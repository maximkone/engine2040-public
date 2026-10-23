//! battle: the 500-vs-500 battle fixture for [`Legion`].
//!
//! One seeded tape runs a west shield wall against an east spearhead:
//! * west seats [`CONTRACT_SOLDIERS`] on [`Formation::ShieldWall`] at
//!   [`WALL_ANCHOR`], every squad on [`Order::Hold`]: its marks never move
//!   and its braces engage as the enemy closes;
//! * east seats [`CONTRACT_SOLDIERS`] on [`Formation::Wedge`] and marches
//!   its anchor one cell a tick until the spearhead meets the wall;
//! * at [`FLANK_ORDER_TICK`] east wheels its front squads onto
//!   [`Order::FlankLeft`]/[`Order::FlankRight`]. Those bodies reach the wall
//!   off its front bearing, so the engaged wall squads take flank pressure
//!   and turn; a side breaks when half its engaged squads have turned.
//!
//! The tape proves four things:
//! 1. the west wall holds every mark and braces (shield wall holds), yet the
//!    flanked west is the side that breaks first;
//! 2. without the wheel the same battle never breaks (flanking decides);
//! 3. an order shows on the next decided tick, within the posted
//!    [`ORDER_LATENCY_BOUND_TICKS`];
//! 4. the routers repath on NV3 tile versions mid-battle: sealing a gap
//!    rebakes its tile and exactly the walks that crossed it repath.
//!
//! Everything is integer math over the public [`Legion`] API plus a body
//! field. Bodies never share a cell and walk one cell per axis per tick
//! toward their decided slot (the shape the arena steers through nav). The
//! same seed replays byte-identical states, bodies and repaths; a second
//! seed changes the tape, so the gate can say no.

use super::*;
use engine_core::{CounterCursor, Fnv64};
use engine_nav::tiles::{StampedPath, TiledNav};

/// Soldiers a side seats: the 500-soldier contract size.
const BATTLE_SOLDIERS: usize = CONTRACT_SOLDIERS;
/// Cells the battle stands on.
const BATTLE_GRID: (usize, usize) = (1024, 1024);
/// West wall anchor: it never moves.
const WALL_ANCHOR: (usize, usize) = (512, 700);
/// East wedge anchor at the muster.
const WEDGE_ANCHOR: (usize, usize) = (512, 300);
/// Seeded muster y spread: the wedge starts `WEDGE_ANCHOR.1 - draw` cells up.
const WEDGE_Y_SPREAD: u64 = 3;
/// Y where the east anchor halts: the spearhead meets the contact band.
const EAST_HALT_Y: usize = 694;
/// Tick east wheels its front squads.
const FLANK_ORDER_TICK: u32 = 300;
/// Front squads that wheel at least, from the seeded first squad on.
const FLANK_SQUADS: usize = 20;
/// Seeded extra wheel squads: the wheel width varies by seed.
const FLANK_SQUAD_SPREAD: u64 = 3;
/// Squad centers this close (Chebyshev, cells) are in contact.
const SQUAD_CONTACT_CELLS: i32 = 40;
/// Ticks of off-bearing pressure that turn a squad.
const FLANK_TURN_TICKS: u32 = 16;
/// Ticks one battle tape runs.
const BATTLE_TICKS: u32 = 520;
/// Ticks from the break to the rout order.
const ROUT_DELAY_TICKS: u32 = 24;
/// Ticks from the rout order to the mid-rout repath.
const REPATH_DELAY_TICKS: u32 = 24;
/// Posted bound: a new order shows within this many ticks of issue.
const ORDER_LATENCY_BOUND_TICKS: u32 = 2;
/// Replay seed.
const BATTLE_SEED: u64 = 2040;
/// Second seed: its tape must differ.
const BATTLE_SEED_OTHER: u64 = 2041;
/// Rout nav side in cells.
const ROUT_NAV_EDGE: usize = 128;
/// Rout nav tile side in cells (one NV3 tile).
const ROUT_TILE_CELLS: usize = 16;
/// Rout band y: the old wall line the routers must cross.
const ROUT_BAND_Y: usize = 100;
/// West gap x in the rout band; the breach seals it mid-rout.
const ROUT_GAP_WEST: usize = 48;
/// East gap x in the rout band; it stays open.
const ROUT_GAP_EAST: usize = 80;
/// Rally cell the routers walk to.
const ROUT_RALLY: (usize, usize) = (64, 120);
/// Battle cells per rout nav cell.
const ROUT_CELLS_PER_NAV: usize = 8;

/// One side of the field.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    West,
    East,
}

impl Side {
    /// Every side, west first: the body field walks them in this order.
    const ALL: [Self; 2] = [Self::West, Self::East];

    /// Field mark this side's bodies claim.
    const fn mark(self) -> u8 {
        match self {
            Self::West => 1,
            Self::East => 2,
        }
    }

    /// Facing on the y axis: west faces -y (toward the east muster).
    const fn front(self) -> i32 {
        match self {
            Self::West => -1,
            Self::East => 1,
        }
    }

    /// Lane index in per-side tuples.
    const fn slot(self) -> usize {
        match self {
            Self::West => 0,
            Self::East => 1,
        }
    }

    /// Short name for the receipts.
    const fn name(self) -> &'static str {
        match self {
            Self::West => "west",
            Self::East => "east",
        }
    }
}

/// The first tick one side broke, with the counts that decided it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct BreakRecord {
    tick: u32,
    side: Side,
    turned: usize,
    engaged: usize,
}

/// The mid-rout NV3 repath: stale counts, repaths and the sealed gap.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct RoutRecord {
    routers: usize,
    stale_before: usize,
    stale_after: usize,
    repathed: usize,
    bytes_changed: usize,
    cells_changed: usize,
    stale_end: usize,
    rebaked_tiles: usize,
    gap_used_before: usize,
    gap_used_after: usize,
}

/// One battle tape: per-tick state hashes, both break records, the order
/// latencies and the mid-rout repath.
struct BattleRun {
    states: Vec<u64>,
    final_digest: u64,
    breaks: [Option<BreakRecord>; 2],
    first: Option<BreakRecord>,
    wall_marks_held: bool,
    contact_tick: Option<u32>,
    braced_peak: usize,
    braced_tick: Option<u32>,
    east_lead_y: usize,
    flank_latency: Option<u32>,
    rout_latency: Option<u32>,
    rout: Option<RoutRecord>,
}

impl BattleRun {
    /// Fold the whole per-tick trace into one `u64`, so a replay compares
    /// every tick while printing one number.
    fn trace(&self) -> u64 {
        let mut hash = Fnv64::new();
        hash.write_len(self.states.len());
        for state in &self.states {
            hash.write_u64(*state);
        }
        hash.finish()
    }
}

/// The body field: one cell holds at most one body.
///
/// Bodies claim their cell in a fixed order, so the field replays exactly.
/// The east spearhead packs against the wall instead of passing it.
struct Field {
    grid: (usize, usize),
    occupancy: Vec<u8>,
}

impl Field {
    fn new(grid: (usize, usize)) -> Self {
        Self {
            grid,
            occupancy: vec![0; grid.0 * grid.1],
        }
    }

    /// Claim `cell` for `mark`; false when another body already holds it.
    fn claim(&mut self, cell: (usize, usize), mark: u8) -> bool {
        let index = cell.1 * self.grid.0 + cell.0;
        if self.occupancy[index] == 0 {
            self.occupancy[index] = mark;
            true
        } else {
            false
        }
    }

    /// Claim `cell`, else its nearest free orthogonal neighbour.
    fn claim_near(&mut self, cell: (usize, usize), mark: u8) -> Option<(usize, usize)> {
        let (w, h) = self.grid;
        let candidates = [
            cell,
            (cell.0.saturating_add(1).min(w - 1), cell.1),
            (cell.0.saturating_sub(1), cell.1),
            (cell.0, cell.1.saturating_add(1).min(h - 1)),
            (cell.0, cell.1.saturating_sub(1)),
        ];
        candidates
            .into_iter()
            .find(|candidate| self.claim(*candidate, mark))
    }

    /// Seat the mustered bodies, nudging a duplicate slot to a neighbour.
    fn place(&mut self, mark: u8, slots: &[(usize, usize)]) -> Vec<(usize, usize)> {
        let mut seated = Vec::with_capacity(slots.len());
        for slot in slots {
            seated.push(self.claim_near(*slot, mark).unwrap_or(*slot));
        }
        seated
    }

    /// Clear every body's cell so the tick re-claims the whole field.
    fn lift(&mut self, bodies: &[(usize, usize)]) {
        for (x, y) in bodies {
            self.occupancy[y * self.grid.0 + x] = 0;
        }
    }

    /// Walk every body one cell per axis toward its slot; a taken cell holds
    /// the body instead. Integer only, fixed body order.
    fn advance(&mut self, mark: u8, bodies: &mut [(usize, usize)], slots: &[(usize, usize)]) {
        for (body, slot) in bodies.iter_mut().zip(slots) {
            let from = *body;
            let dx = axis(from.0, slot.0);
            let dy = axis(from.1, slot.1);
            let tx = from.0.saturating_add_signed(dx);
            let ty = from.1.saturating_add_signed(dy);
            let mut landed = None;
            for candidate in [(tx, ty), (tx, from.1), (from.0, ty)] {
                if self.claim(candidate, mark) {
                    landed = Some(candidate);
                    break;
                }
            }
            *body = landed
                .or_else(|| self.claim_near(from, mark))
                .unwrap_or(from);
        }
    }
}

/// Signed one-cell axis delta from `from` toward `to`.
fn axis(from: usize, to: usize) -> isize {
    match from.cmp(&to) {
        std::cmp::Ordering::Less => 1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => -1,
    }
}

/// True when the order commits its squad to a flanking approach.
fn flank_order(order: Order) -> bool {
    matches!(order, Order::FlankLeft | Order::FlankRight)
}

/// One side's morale: squad centers, squad orders, flank strain and breaks.
struct Front {
    side: Side,
    squads: usize,
    centers: [(i32, i32); MAX_SQUADS],
    orders: [Order; MAX_SQUADS],
    strain: [u32; MAX_SQUADS],
    turned: [bool; MAX_SQUADS],
    turned_count: usize,
    engaged_count: usize,
    break_at: Option<BreakRecord>,
}

impl Front {
    fn new(side: Side, squads: usize) -> Self {
        Self {
            side,
            squads,
            centers: [(0, 0); MAX_SQUADS],
            orders: [Order::Hold; MAX_SQUADS],
            strain: [0; MAX_SQUADS],
            turned: [false; MAX_SQUADS],
            turned_count: 0,
            engaged_count: 0,
            break_at: None,
        }
    }

    /// Mirror the live squad orders (the wheel is what a flank reads).
    fn sync_orders(&mut self, legion: &Legion) {
        for squad in 0..self.squads {
            self.orders[squad] = legion.order_of(squad).unwrap_or(Order::Hold);
        }
    }

    /// Recompute every squad center as the integer mean of its body cells.
    fn refresh(&mut self, bodies: &[(usize, usize)]) {
        for squad in 0..self.squads {
            let start = squad * SQUAD_SIZE;
            let end = (start + SQUAD_SIZE).min(bodies.len());
            let count = i32::try_from(end.saturating_sub(start).max(1)).unwrap_or(1);
            let mut sum_x = 0i32;
            let mut sum_y = 0i32;
            for (x, y) in &bodies[start..end] {
                sum_x += i32::try_from(*x).unwrap_or(0);
                sum_y += i32::try_from(*y).unwrap_or(0);
            }
            self.centers[squad] = (sum_x / count, sum_y / count);
        }
    }

    /// Fold this tick: a squad in contact is engaged, and a squad held off
    /// its front bearing by a flank-committed enemy squad turns after
    /// [`FLANK_TURN_TICKS`]. Half the engaged squads turning breaks the side.
    fn observe(&mut self, tick: u32, enemy: &Self) {
        let mut turned = 0usize;
        let mut engaged = 0usize;
        for squad in 0..self.squads {
            let own = self.centers[squad];
            let mut in_contact = false;
            let mut threatened = false;
            for other in 0..enemy.squads {
                let dx = (enemy.centers[other].0 - own.0).abs();
                let forward = (enemy.centers[other].1 - own.1) * self.side.front();
                if dx.max(forward.abs()) > SQUAD_CONTACT_CELLS {
                    continue;
                }
                in_contact = true;
                if dx > forward && flank_order(enemy.orders[other]) {
                    threatened = true;
                }
            }
            if in_contact {
                engaged += 1;
            }
            self.strain[squad] = if threatened {
                self.strain[squad].saturating_add(1)
            } else {
                0
            };
            if self.strain[squad] >= FLANK_TURN_TICKS {
                self.turned[squad] = true;
            }
            if self.turned[squad] {
                turned += 1;
            }
        }
        self.turned_count = turned;
        self.engaged_count = engaged;
        if self.break_at.is_none() && engaged > 0 && turned > 0 && turned * 2 >= engaged {
            self.break_at = Some(BreakRecord {
                tick,
                side: self.side,
                turned,
                engaged,
            });
        }
    }
}

/// One tick's state hash: both decided legions, both body lanes, and the
/// morale counts, so a replay compares bodies and break timing too.
fn state_hash(
    wall: &Legion,
    east: &Legion,
    west_bodies: &[(usize, usize)],
    east_bodies: &[(usize, usize)],
    west_front: &Front,
    east_front: &Front,
) -> u64 {
    let mut hash = Fnv64::new();
    hash.write_u64(wall.digest());
    hash.write_u64(east.digest());
    for (x, y) in west_bodies.iter().chain(east_bodies) {
        hash.write_u64(*x as u64);
        hash.write_u64(*y as u64);
    }
    for front in [west_front, east_front] {
        hash.write_u64(front.turned_count as u64);
        hash.write_u64(front.engaged_count as u64);
        hash.write_bool(front.break_at.is_some());
    }
    hash.finish()
}

/// Fold the tape's records into the final digest.
fn final_digest(
    states: &[u64],
    breaks: &[Option<BreakRecord>; 2],
    rout: Option<&RoutRecord>,
) -> u64 {
    let mut hash = Fnv64::new();
    hash.write_len(states.len());
    for state in states {
        hash.write_u64(*state);
    }
    for record in breaks.iter().flatten() {
        hash.write_bool(true);
        hash.write_u32(record.tick);
        hash.write_u8(record.side.mark());
        hash.write_len(record.turned);
        hash.write_len(record.engaged);
    }
    match rout {
        Some(record) => {
            hash.write_bool(true);
            for count in [
                record.routers,
                record.stale_before,
                record.stale_after,
                record.repathed,
                record.bytes_changed,
                record.cells_changed,
                record.stale_end,
                record.rebaked_tiles,
                record.gap_used_before,
                record.gap_used_after,
            ] {
                hash.write_len(count);
            }
        }
        None => hash.write_bool(false),
    }
    hash.finish()
}

/// Run one seeded battle tape. `wheel` false is the negative control: the
/// same musters and march, no flank orders, so the wall is never flanked.
fn run_battle(seed: u64, wheel: bool) -> BattleRun {
    let grid = BATTLE_GRID;
    let mut cursor = CounterCursor::new(seed);
    let wheel_first = cursor.next_range(0, 3) as usize;
    let wheel_count = FLANK_SQUADS + cursor.next_range(0, FLANK_SQUAD_SPREAD) as usize;
    let muster_y = WEDGE_ANCHOR.1 - cursor.next_range(0, WEDGE_Y_SPREAD) as usize;
    let wedge_anchor = (WEDGE_ANCHOR.0, muster_y);
    let mut wall = Legion::new(
        BATTLE_SOLDIERS,
        Formation::ShieldWall,
        WALL_ANCHOR,
        wedge_anchor,
        grid,
    )
    .expect("the wall seats");
    let mut east = Legion::new(
        BATTLE_SOLDIERS,
        Formation::Wedge,
        wedge_anchor,
        WALL_ANCHOR,
        grid,
    )
    .expect("the east seats");
    let mut field = Field::new(grid);
    let wall_marks = wall.slots().to_vec();
    let mut west_bodies = field.place(Side::West.mark(), &wall_marks);
    let mut east_bodies = field.place(Side::East.mark(), east.slots());
    let mut wall_slots = vec![(0usize, 0usize); BATTLE_SOLDIERS];
    let mut wall_brace = vec![false; BATTLE_SOLDIERS];
    let mut east_slots = vec![(0usize, 0usize); BATTLE_SOLDIERS];
    let mut east_brace = vec![false; BATTLE_SOLDIERS];
    let mut west_front = Front::new(Side::West, wall.squad_count());
    let mut east_front = Front::new(Side::East, east.squad_count());
    let mut states = Vec::with_capacity(BATTLE_TICKS as usize);
    let mut breaks: [Option<BreakRecord>; 2] = [None, None];
    let mut wall_marks_held = true;
    let mut contact_tick = None;
    let mut braced_peak = 0usize;
    let mut braced_tick = None;
    let mut east_lead_y = 0usize;
    let mut flank_issued = None;
    let mut flank_latency = None;
    let mut rout_issued = None;
    let mut rout_latency = None;
    let mut rout = None;
    // Each wheeled squad leader's plain wedge x offset: the wheel moves it.
    let mut wheel_probe: Vec<(usize, i32)> = Vec::new();
    for squad in wheel_first..(wheel_first + wheel_count).min(east.squad_count()) {
        let soldier = squad * SQUAD_SIZE;
        let offset = formation_slot(Formation::Wedge, soldier, BATTLE_SOLDIERS, 1)
            .map(|(x, _)| x)
            .unwrap_or(0);
        wheel_probe.push((soldier, offset));
    }

    for tick in 0..BATTLE_TICKS {
        let (ex, ey) = east.anchor();
        if ey < EAST_HALT_Y {
            east.set_anchor((ex, ey + 1))
                .expect("the march anchor stays on the grid");
        }
        wall.set_enemy(east.anchor())
            .expect("the wall's foe stays on the grid");
        wall.step(tick, &west_bodies, &mut wall_slots, &mut wall_brace)
            .expect("the wall lane matches");
        east.step(tick, &east_bodies, &mut east_slots, &mut east_brace)
            .expect("the east lane matches");
        field.lift(&west_bodies);
        field.lift(&east_bodies);
        field.advance(Side::West.mark(), &mut west_bodies, &wall_slots);
        field.advance(Side::East.mark(), &mut east_bodies, &east_slots);
        west_front.sync_orders(&wall);
        east_front.sync_orders(&east);
        west_front.refresh(&west_bodies);
        east_front.refresh(&east_bodies);
        west_front.observe(tick, &east_front);
        east_front.observe(tick, &west_front);
        // The lead is read on the fighting line: once the rout order lands the
        // east walks into the wall's vacated marks and the reading is stale.
        if rout_issued.is_none() {
            east_lead_y = east_bodies.iter().map(|cell| cell.1).max().unwrap_or(0);
        }
        let braced = wall_brace.iter().filter(|held| **held).count();
        if braced > braced_peak {
            braced_peak = braced;
            braced_tick = Some(tick);
        }
        if contact_tick.is_none() && west_front.engaged_count > 0 {
            contact_tick = Some(tick);
        }
        if rout_issued.is_none() && wall_slots != wall_marks {
            wall_marks_held = false;
        }
        if let (Some(issued), None) = (flank_issued, flank_latency) {
            if tick > issued {
                let anchor = east.anchor();
                for (soldier, offset) in &wheel_probe {
                    let slot = east_slots[*soldier];
                    let dx =
                        i32::try_from(slot.0).unwrap_or(0) - i32::try_from(anchor.0).unwrap_or(0);
                    if dx != *offset {
                        flank_latency = Some(tick - issued);
                        break;
                    }
                }
            }
        }
        if let (Some(issued), None) = (rout_issued, rout_latency) {
            if tick > issued && wall_slots != wall_marks {
                rout_latency = Some(tick - issued);
            }
        }
        states.push(state_hash(
            &wall,
            &east,
            &west_bodies,
            &east_bodies,
            &west_front,
            &east_front,
        ));
        for side in Side::ALL {
            let front = if side == Side::West {
                &west_front
            } else {
                &east_front
            };
            if breaks[side.slot()].is_none() {
                breaks[side.slot()] = front.break_at;
            }
        }
        // Commands land after this tick's step: the effect is measured on
        // the next decide, never assumed.
        if wheel && flank_issued.is_none() && tick == FLANK_ORDER_TICK {
            for squad in wheel_first..(wheel_first + wheel_count).min(east.squad_count()) {
                let order = if (squad - wheel_first).is_multiple_of(2) {
                    Order::FlankLeft
                } else {
                    Order::FlankRight
                };
                east.set_order(squad, order)
                    .expect("the wheel squad is on the roster");
            }
            flank_issued = Some(tick);
        }
        if let (Some(record), None) = (breaks[Side::West.slot()], rout_issued) {
            if tick == record.tick + ROUT_DELAY_TICKS {
                for squad in 0..wall.squad_count() {
                    wall.set_order(squad, Order::Retreat)
                        .expect("the wall squad is on the roster");
                }
                rout_issued = Some(tick);
            }
        }
        if let (Some(issued), None) = (rout_issued, rout) {
            if tick == issued + REPATH_DELAY_TICKS {
                rout = Some(rout_repath(&wall_slots));
            }
        }
    }

    let first = match (breaks[0], breaks[1]) {
        (Some(west), Some(east)) => Some(if west.tick <= east.tick { west } else { east }),
        (Some(record), None) | (None, Some(record)) => Some(record),
        (None, None) => None,
    };
    let final_digest = final_digest(&states, &breaks, rout.as_ref());
    BattleRun {
        states,
        final_digest,
        breaks,
        first,
        wall_marks_held,
        contact_tick,
        braced_peak,
        braced_tick,
        east_lead_y,
        flank_latency,
        rout_latency,
        rout,
    }
}

/// The rout leg: 63 squad leaders hold NV3 stamped walks through a band with
/// two gaps; sealing the west gap mid-rout rebakes its tile and exactly the
/// walks that crossed it repath.
fn rout_repath(wall_slots: &[(usize, usize)]) -> RoutRecord {
    let side = ROUT_NAV_EDGE;
    let mut blocked = vec![false; side * side];
    for x in 0..side {
        blocked[ROUT_BAND_Y * side + x] = true;
    }
    blocked[ROUT_BAND_Y * side + ROUT_GAP_WEST] = false;
    blocked[ROUT_BAND_Y * side + ROUT_GAP_EAST] = false;
    let mut nav =
        TiledNav::bake(&blocked, side, side, ROUT_TILE_CELLS).expect("the rout nav bakes");
    let mut paths: Vec<StampedPath> = Vec::new();
    for squad in 0..wall_slots.len().div_ceil(SQUAD_SIZE) {
        let leader = wall_slots[squad * SQUAD_SIZE];
        let start = (leader.0 / ROUT_CELLS_PER_NAV, leader.1 / ROUT_CELLS_PER_NAV);
        let path = match nav
            .find_path(start, ROUT_RALLY)
            .expect("the rout query reads")
        {
            Some(path) => path,
            None => panic!("the {squad} router is sealed at {start:?}"),
        };
        paths.push(path);
    }
    let gap = (ROUT_GAP_WEST, ROUT_BAND_Y);
    let stale_before = nav.stale_paths(&paths).len();
    let gap_used_before = paths
        .iter()
        .filter(|path| path.cells().contains(&gap))
        .count();
    // The west gap collapses mid-rout: the cell seals, its tile rebakes and
    // bumps the version the held walks are stamped with.
    nav.set_blocked(ROUT_GAP_WEST, ROUT_BAND_Y, true)
        .expect("the breach cell is on the nav");
    let rebaked_tiles = nav.rebake_budget(ROUT_TILE_CELLS);
    let stale_after = nav.stale_paths(&paths).len();
    let mut repathed = 0usize;
    let mut bytes_changed = 0usize;
    let mut cells_changed = 0usize;
    for path in &mut paths {
        let bytes_before = path.to_bytes();
        let cells_before = path.cells().to_vec();
        if let Some(fresh) = nav.refresh_path(path).expect("the repath reads") {
            if fresh.to_bytes() != bytes_before {
                bytes_changed += 1;
            }
            if fresh.cells() != cells_before.as_slice() {
                cells_changed += 1;
            }
            *path = fresh;
            repathed += 1;
        }
    }
    let stale_end = nav.stale_paths(&paths).len();
    let gap_used_after = paths
        .iter()
        .filter(|path| path.cells().contains(&gap))
        .count();
    RoutRecord {
        routers: paths.len(),
        stale_before,
        stale_after,
        repathed,
        bytes_changed,
        cells_changed,
        stale_end,
        rebaked_tiles,
        gap_used_before,
        gap_used_after,
    }
}

/// Ticks from issuing `after` (over `before`) to the first decided slot or
/// brace that differs, on one squad stepped from a settled clone pair.
fn visible_latency(before: Order, after: Order) -> u32 {
    let grid = (80usize, 40usize);
    let bodies = [(40usize, 30usize); SQUAD_SIZE];
    let mut left = Legion::new(SQUAD_SIZE, Formation::Line, (40, 30), (40, 10), grid)
        .expect("the latency lane seats");
    left.set_order(0, before)
        .expect("one squad is on the roster");
    let mut right = left.clone();
    let mut left_slots = [(0usize, 0usize); SQUAD_SIZE];
    let mut left_brace = [false; SQUAD_SIZE];
    let mut right_slots = [(0usize, 0usize); SQUAD_SIZE];
    let mut right_brace = [false; SQUAD_SIZE];
    // Settle both on `before` at tick 0, then issue `after` on the right.
    left.step(0, &bodies, &mut left_slots, &mut left_brace)
        .expect("the latency lane matches");
    right
        .step(0, &bodies, &mut right_slots, &mut right_brace)
        .expect("the latency lane matches");
    right
        .set_order(0, after)
        .expect("one squad is on the roster");
    for tick in 1..=ORDER_LATENCY_BOUND_TICKS {
        left.step(tick, &bodies, &mut left_slots, &mut left_brace)
            .expect("the latency lane matches");
        right
            .step(tick, &bodies, &mut right_slots, &mut right_brace)
            .expect("the latency lane matches");
        if left_slots != right_slots || left_brace != right_brace {
            return tick;
        }
    }
    u32::MAX
}

#[test]
fn battle_replays_and_the_flanked_side_breaks_first() {
    let first = run_battle(BATTLE_SEED, true);
    let replay = run_battle(BATTLE_SEED, true);
    assert_eq!(
        first.trace(),
        replay.trace(),
        "the same seed changed the state or body trace"
    );
    assert_eq!(first.final_digest, replay.final_digest);
    assert_eq!(first.states, replay.states);
    assert_eq!(first.breaks, replay.breaks);
    let west = first.breaks[Side::West.slot()].expect("the wall breaks");
    assert_eq!(west.side, Side::West);
    assert!(
        first.breaks[Side::East.slot()].is_none(),
        "the flanking east broke too: {:?}",
        first.breaks[Side::East.slot()]
    );
    let first_break = first.first.expect("a side breaks");
    assert_eq!(
        first_break.side,
        Side::West,
        "the flanked side must break first"
    );
    assert!(
        first.wall_marks_held,
        "the wall yielded a mark before the rout"
    );
    assert!(
        first.braced_peak > 0,
        "the wall never braced against the closing spearhead"
    );
    assert!(
        first.braced_peak < BATTLE_SOLDIERS,
        "every wall file braced: the brace range gate never fired"
    );
    let contact_band = WALL_ANCHOR.1 - BRACE_RANGE_CELLS as usize;
    assert!(
        (contact_band..WALL_ANCHOR.1).contains(&first.east_lead_y),
        "the spearhead stopped at y {}, outside the wall's contact band {}..{}",
        first.east_lead_y,
        contact_band,
        WALL_ANCHOR.1
    );
    assert!(
        first_break.tick + ROUT_DELAY_TICKS + REPATH_DELAY_TICKS < BATTLE_TICKS,
        "the tape ends before the rout repath at tick {}",
        first_break.tick + ROUT_DELAY_TICKS + REPATH_DELAY_TICKS
    );
    let other = run_battle(BATTLE_SEED_OTHER, true);
    let other_break = other.first.expect("the second seed breaks too");
    assert_eq!(
        other_break.side,
        Side::West,
        "the second seed broke the other side"
    );
    assert!(other.breaks[Side::East.slot()].is_none());
    assert_ne!(
        first.trace(),
        other.trace(),
        "the second seed replayed the first seed's trace"
    );
    assert_ne!(
        first.final_digest, other.final_digest,
        "the second seed ended in the first seed's state"
    );
    eprintln!(
        "battle seed {BATTLE_SEED}: {} breaks first at tick {} ({} of {} engaged squads \
         turned; {} files braced at tick {:?}; contact tick {:?}); east never breaks; \
         trace {:016x}, final {:016x}. Seed {BATTLE_SEED_OTHER}: {} breaks first at tick {} \
         ({} of {}), trace {:016x}.",
        first_break.side.name(),
        first_break.tick,
        first_break.turned,
        first_break.engaged,
        first.braced_peak,
        first.braced_tick,
        first.contact_tick,
        first.trace(),
        first.final_digest,
        other_break.side.name(),
        other_break.tick,
        other_break.turned,
        other_break.engaged,
        other.trace()
    );
}

#[test]
fn flank_orders_decide_the_battle() {
    let wheeled = run_battle(BATTLE_SEED, true);
    let held = run_battle(BATTLE_SEED, false);
    assert!(wheeled.first.is_some(), "the wheeled battle never broke");
    assert!(
        held.first.is_none(),
        "the wall broke with no flank orders: {:?}",
        held.first
    );
    assert!(
        wheeled.contact_tick.is_some(),
        "the wheeled fronts never met"
    );
    assert!(
        held.contact_tick.is_some(),
        "the control fronts never met: the standoff must be a contact, not an absence"
    );
    assert!(
        held.east_lead_y <= EAST_HALT_Y,
        "the control spearhead enveloped the wall at y {}",
        held.east_lead_y
    );
    assert!(held.rout.is_none(), "the control ran a rout with no break");
    eprintln!(
        "flank control: fronts meet at tick {:?} with the wheel and {:?} without; \
         the wheeled west breaks at {:?}, neither side breaks without it; \
         lead y {} with the wheel vs {} without.",
        wheeled.contact_tick,
        held.contact_tick,
        wheeled.first.map(|record| record.tick),
        wheeled.east_lead_y,
        held.east_lead_y
    );
}

#[test]
fn orders_land_within_the_posted_latency_bound() {
    let pairs = [
        (Order::Hold, Order::Advance),
        (Order::Advance, Order::Hold),
        (Order::Charge, Order::Hold),
        (Order::Retreat, Order::Hold),
        (Order::FlankLeft, Order::Hold),
        (Order::FlankRight, Order::Hold),
    ];
    for (before, after) in pairs {
        let latency = visible_latency(before, after);
        assert!(
            (1..=ORDER_LATENCY_BOUND_TICKS).contains(&latency),
            "{before:?} -> {after:?} landed after {latency} ticks, \
             past the posted {ORDER_LATENCY_BOUND_TICKS}-tick bound"
        );
        eprintln!("order {before:?} -> {after:?}: visible after {latency} tick(s)");
    }
    let battle = run_battle(BATTLE_SEED, true);
    for (name, latency) in [
        ("flank wheel", battle.flank_latency),
        ("rout retreat", battle.rout_latency),
    ] {
        let latency = latency.expect("the tape order landed");
        assert!(
            (1..=ORDER_LATENCY_BOUND_TICKS).contains(&latency),
            "the {name} order landed after {latency} ticks, \
             past the posted {ORDER_LATENCY_BOUND_TICKS}-tick bound"
        );
        eprintln!("battle order {name}: visible after {latency} tick(s)");
    }
}

#[test]
fn shield_wall_holds_its_marks_and_braces() {
    let run = run_battle(BATTLE_SEED, true);
    assert!(
        run.wall_marks_held,
        "the wall yielded a mark before the rout"
    );
    let contact = run.contact_tick.expect("the fronts meet");
    assert!(
        run.braced_peak > 0,
        "the wall never braced at contact (tick {contact})"
    );
    assert!(
        run.braced_peak < BATTLE_SOLDIERS,
        "every wall file braced: the brace range gate never fired"
    );
    let braced = run.braced_tick.expect("a brace engaged");
    assert!(braced >= contact, "a brace engaged before the fronts met");
    eprintln!(
        "shield wall: held all {} marks from tick 0 to the rout; {} files braced \
         at tick {braced} (first contact tick {contact}); the spearhead stopped at y {}.",
        BATTLE_SOLDIERS, run.braced_peak, run.east_lead_y
    );
}

#[test]
fn routers_repath_on_nv3_tile_versions_mid_battle() {
    let run = run_battle(BATTLE_SEED, true);
    let rout = run.rout.expect("the rout repath ran mid-battle");
    assert_eq!(rout.routers, BATTLE_SOLDIERS.div_ceil(SQUAD_SIZE));
    assert_eq!(rout.stale_before, 0, "a fresh rout walk was already stale");
    assert!(rout.rebaked_tiles >= 1, "the sealed gap rebaked no tile");
    assert!(
        rout.stale_after >= 1,
        "the sealed gap left every held walk fresh"
    );
    assert_eq!(rout.repathed, rout.stale_after);
    assert!(rout.bytes_changed >= 1, "no repathed walk changed bytes");
    assert!(
        rout.cells_changed >= 1,
        "no repathed walk changed cells around the sealed gap"
    );
    assert_eq!(rout.stale_end, 0, "a repathed walk stayed stale");
    assert!(
        rout.gap_used_before >= 1,
        "no router used the west gap before the breach"
    );
    assert_eq!(
        rout.gap_used_after, 0,
        "a repathed router still walks the sealed gap"
    );
    eprintln!(
        "rout repath: {} routers, {} stale before, {} after {} tile rebake, {} repathed \
         ({} bytes, {} cells changed), {} stale after; west gap used {} before, {} after.",
        rout.routers,
        rout.stale_before,
        rout.stale_after,
        rout.rebaked_tiles,
        rout.repathed,
        rout.bytes_changed,
        rout.cells_changed,
        rout.stale_end,
        rout.gap_used_before,
        rout.gap_used_after
    );
}
