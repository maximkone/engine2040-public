//! legion_bind: the contubernium falls in.
//!
//! The arena binds the legion brain (`engine_ai::legion`) and the squad
//! orders (`engine_ai::orders`) to the eight men of a contubernium: slot 0
//! the player, seven comrades beside him. The brain owns the formation: it
//! decides the slot cells of the eight soldiers, the anchor the line
//! stands on, the foe it watches and the order, and it paces the thinking.
//! The bind owns the line in millimetres: the frontage one man holds and
//! the gap between two ranks come from `content/rome/line.toml` (3 ft and
//! 6 ft), every man takes the nearest free slot and keeps his row and
//! file, and the pace is the slowest man's speed, so the line never
//! outruns its weakest leg.
//!
//! Units: millimetres on the ground, millimetres a tick for the pace.
//! Determinism: integer math only, the brain's own integer decisions and
//! the order of its slots; the same drill digests the same twice, with no
//! wall clock and no float. Errors: [`BindError`] refuses a bad table, an
//! order outside the drill or a brain that will not take the line; a
//! refused order changes nothing.

use core::fmt;

use engine_ai::legion::{Formation, Legion, LegionError, Order, SQUAD_SIZE};
use engine_ai::orders::SquadOrder;
use engine_core::fixed::isqrt;
use engine_core::Fnv64;

/// Men in a contubernium: the early Imperial tent group of eight.
pub const CONTUBERNIUM: usize = SQUAD_SIZE;

/// The brain's grid: cells on a side. One cell is half the frontage, so
/// the grid covers 227 m of line.
const GRID_EDGE: usize = 512;

/// The brain's file step in cells: two cells make one frontage.
const FILE_STEP_CELLS: u32 = 2;

/// A walking soldier: 25 mm a tick, 1.5 m/s.
pub const DEFAULT_SPEED_MM_PER_TICK: i32 = 25;

/// The Roman line: the frontage one man holds and the gap between two
/// ranks, in millimetres.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Line {
    /// Frontage per man: 3 ft is 888 mm.
    pub frontage_mm: i64,
    /// Gap between two ranks: 6 ft is 1776 mm.
    pub rank_gap_mm: i64,
}

impl Line {
    /// The shipped line table, embedded at build time so the tick never
    /// reads a file.
    pub const SHIPPED: &'static str = include_str!("../../../content/rome/line.toml");

    /// Read the shipped line table.
    ///
    /// Errors: [`BindError::Table`] when the table does not hold the
    /// numbers.
    pub fn shipped() -> Result<Self, BindError> {
        Self::parse(Self::SHIPPED)
    }

    /// Read one line table: `mm_per_ft` from `[units]`, `frontage_ft` and
    /// `rank_gap_ft` from `[line]`.
    ///
    /// Errors: [`BindError::Table`] when a key is missing, is not a plain
    /// number or reads zero.
    pub fn parse(text: &str) -> Result<Self, BindError> {
        let mm_per_ft = table_int(text, "units", "mm_per_ft").ok_or(BindError::Table {
            key: "units.mm_per_ft",
        })?;
        let frontage_ft = table_int(text, "line", "frontage_ft").ok_or(BindError::Table {
            key: "line.frontage_ft",
        })?;
        let rank_gap_ft = table_int(text, "line", "rank_gap_ft").ok_or(BindError::Table {
            key: "line.rank_gap_ft",
        })?;
        if mm_per_ft == 0 {
            return Err(BindError::Table {
                key: "units.mm_per_ft",
            });
        }
        let frontage_mm = i64::from(frontage_ft) * i64::from(mm_per_ft);
        let rank_gap_mm = i64::from(rank_gap_ft) * i64::from(mm_per_ft);
        if frontage_mm == 0 || rank_gap_mm == 0 {
            return Err(BindError::Table { key: "line" });
        }
        Ok(Self {
            frontage_mm,
            rank_gap_mm,
        })
    }
}

/// Which way the line faces. Cardinal, so the geometry stays integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Facing {
    /// The line faces east; its right hand points south.
    East,
    /// The line faces west; its right hand points north.
    West,
    /// The line faces north; its right hand points east.
    North,
    /// The line faces south; its right hand points west.
    South,
}

impl Facing {
    /// The line's lateral axis, to the right of the facing.
    pub const fn right(self) -> (i64, i64) {
        match self {
            Facing::East => (0, -1),
            Facing::West => (0, 1),
            Facing::North => (1, 0),
            Facing::South => (-1, 0),
        }
    }

    /// The line's depth axis: behind the front rank.
    pub const fn back(self) -> (i64, i64) {
        match self {
            Facing::East => (-1, 0),
            Facing::West => (1, 0),
            Facing::North => (0, -1),
            Facing::South => (0, 1),
        }
    }
}

/// The order the contubernium obeys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindOrder {
    /// Close up on the leader's slots: every man takes the nearest free
    /// slot and keeps his row and file.
    FallIn,
    /// Stand fast: the line stops where it stands.
    Hold,
    /// March on the point the order named.
    Advance,
}

impl BindOrder {
    /// The order's code, for the digest.
    pub const fn code(self) -> u8 {
        match self {
            BindOrder::FallIn => 0,
            BindOrder::Hold => 1,
            BindOrder::Advance => 2,
        }
    }
}

/// One man of the contubernium: where he stands and which slot he holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Body {
    pos_mm: (i64, i64),
    slot: Option<u8>,
    speed_mm_per_tick: i32,
}

impl Body {
    /// The man's place on the ground, in millimetres.
    pub const fn pos_mm(&self) -> (i64, i64) {
        self.pos_mm
    }

    /// The slot he holds, or `None` while he is still loose.
    pub const fn slot(&self) -> Option<u8> {
        self.slot
    }

    /// His own speed, in millimetres a tick. The slowest man of the line
    /// sets the pace.
    pub const fn speed_mm_per_tick(&self) -> i32 {
        self.speed_mm_per_tick
    }
}

/// The contubernium: eight men bound to the legion brain's slots.
pub struct Contubernium {
    line: Line,
    legion: Legion,
    bodies: [Body; CONTUBERNIUM],
    /// The line's centre on the ground, in millimetres.
    anchor_mm: (i64, i64),
    facing: Facing,
    order: BindOrder,
    /// The point the line advances on, in millimetres.
    target_mm: Option<(i64, i64)>,
    /// Files a rank holds, read from the brain's slots.
    files: u8,
    /// Ranks the line is deep, read from the brain's slots.
    ranks: u8,
    /// The brain's slot cells, one per soldier.
    slots: [(usize, usize); CONTUBERNIUM],
    /// The ranks' depths, shallowest first.
    depths: [usize; CONTUBERNIUM],
    depth_count: usize,
    /// The brace flags the brain set for the eight men.
    braced: [bool; CONTUBERNIUM],
}

impl Contubernium {
    /// A contubernium of eight at `anchor_mm`, facing `facing`, in
    /// `formation`, the eight men where `positions` puts them.
    ///
    /// Errors: [`BindError::Brain`] when the brain will not take the line.
    pub fn new(
        line: Line,
        anchor_mm: (i64, i64),
        facing: Facing,
        formation: Formation,
        positions: [(i64, i64); CONTUBERNIUM],
    ) -> Result<Self, BindError> {
        let anchor_cell = cell_of(anchor_mm, cell_mm(line));
        let mut legion = Legion::new(
            CONTUBERNIUM,
            formation,
            anchor_cell,
            anchor_cell,
            (GRID_EDGE, GRID_EDGE),
        )
        .map_err(brain_error)?;
        legion.set_spacing(FILE_STEP_CELLS).map_err(brain_error)?;
        let bodies = core::array::from_fn(|index| Body {
            pos_mm: positions[index],
            slot: None,
            speed_mm_per_tick: DEFAULT_SPEED_MM_PER_TICK,
        });
        Ok(Self {
            line,
            legion,
            bodies,
            anchor_mm,
            facing,
            order: BindOrder::Hold,
            target_mm: None,
            files: 1,
            ranks: 1,
            slots: [(0, 0); CONTUBERNIUM],
            depths: [0; CONTUBERNIUM],
            depth_count: 0,
            braced: [false; CONTUBERNIUM],
        })
    }

    /// Give the contubernium a squad order: `fall in` closes the line up,
    /// `hold` stops it, `move` marches it on the point the order names.
    /// Any other order is refused and changes nothing.
    ///
    /// Errors: [`BindError::UnknownOrder`] for an order outside the drill,
    /// [`BindError::Brain`] when the brain will not watch the point.
    pub fn apply_order(&mut self, order: SquadOrder) -> Result<(), BindError> {
        match order {
            SquadOrder::FallIn => {
                self.order = BindOrder::FallIn;
                self.target_mm = None;
            }
            SquadOrder::Hold => {
                self.order = BindOrder::Hold;
                self.target_mm = None;
            }
            SquadOrder::Move(point) => {
                let target = (i64::from(point.x_mm), i64::from(point.y_mm));
                self.order = BindOrder::Advance;
                self.target_mm = Some(target);
                // The brain watches the point the line marches on, so its
                // advance shift points the same way the line walks.
                let cell = cell_of(target, cell_mm(self.line));
                self.legion.set_enemy(cell).map_err(brain_error)?;
            }
            other => {
                return Err(BindError::UnknownOrder {
                    name: order_name(other),
                })
            }
        }
        Ok(())
    }

    /// Step one tick: the brain thinks, the line marches on its order,
    /// every loose man takes the nearest free slot, and the men close up
    /// on their slots at the pace of the slowest man.
    ///
    /// Errors: [`BindError::Brain`] when the brain will not take the line.
    pub fn step(&mut self, tick: u32) -> Result<(), BindError> {
        // The brain's anchor follows the line's centre.
        let anchor_cell = cell_of(self.anchor_mm, cell_mm(self.line));
        self.legion.set_anchor(anchor_cell).map_err(brain_error)?;
        self.legion
            .set_order(0, self.brain_order())
            .map_err(brain_error)?;
        let cell = cell_mm(self.line);
        let mut soldiers = [(0usize, 0usize); CONTUBERNIUM];
        for (lane, body) in soldiers.iter_mut().zip(self.bodies.iter()) {
            *lane = cell_of(body.pos_mm, cell);
        }
        let mut slots = [(0usize, 0usize); CONTUBERNIUM];
        let mut brace = [false; CONTUBERNIUM];
        self.legion
            .step(tick, &soldiers, &mut slots, &mut brace)
            .map_err(brain_error)?;
        self.slots = slots;
        self.braced = brace;
        self.read_shape();
        if self.order == BindOrder::FallIn {
            self.seat();
        }
        self.march();
        self.close_up();
        Ok(())
    }

    /// The line's centre on the ground, in millimetres.
    pub const fn anchor_mm(&self) -> (i64, i64) {
        self.anchor_mm
    }

    /// The order the contubernium obeys.
    pub const fn order(&self) -> BindOrder {
        self.order
    }

    /// The point the line advances on, in millimetres.
    pub const fn target_mm(&self) -> Option<(i64, i64)> {
        self.target_mm
    }

    /// Which way the line faces.
    pub const fn facing(&self) -> Facing {
        self.facing
    }

    /// The man at `index`, or `None` past the contubernium.
    pub fn body(&self, index: usize) -> Option<&Body> {
        self.bodies.get(index)
    }

    /// The eight men, in slot order.
    pub const fn bodies(&self) -> &[Body; CONTUBERNIUM] {
        &self.bodies
    }

    /// The brain behind the line.
    pub const fn legion(&self) -> &Legion {
        &self.legion
    }

    /// The brace flags the brain set for the eight men.
    pub const fn braced(&self) -> &[bool; CONTUBERNIUM] {
        &self.braced
    }

    /// The pace of the line: the slowest man's speed, in millimetres a
    /// tick. Nobody outruns the slowest leg.
    pub fn pace_mm_per_tick(&self) -> i32 {
        let mut pace = i32::MAX;
        for body in self.bodies.iter() {
            pace = pace.min(body.speed_mm_per_tick);
        }
        pace
    }

    /// Set a man's own speed, in millimetres a tick. The slowest man of
    /// the line sets the pace of the whole line. False past the
    /// contubernium.
    pub fn set_speed(&mut self, index: usize, speed_mm_per_tick: i32) -> bool {
        let Some(body) = self.bodies.get_mut(index) else {
            return false;
        };
        body.speed_mm_per_tick = speed_mm_per_tick.max(0);
        true
    }

    /// Place a man on the ground, in millimetres. The fall-in seats him by
    /// the slot nearest to where he stands. False past the contubernium.
    pub fn place(&mut self, index: usize, pos_mm: (i64, i64)) -> bool {
        let Some(body) = self.bodies.get_mut(index) else {
            return false;
        };
        body.pos_mm = pos_mm;
        true
    }

    /// Where a formation slot stands on the ground, in millimetres. The
    /// front rank stands on the anchor's line and the ranks behind it one
    /// rank gap back; the files spread evenly along the front, in the
    /// order the brain decided them.
    pub fn slot_pos_mm(&self, slot: u8) -> (i64, i64) {
        let Some((file, rank)) = self.file_and_rank(slot) else {
            return self.anchor_mm;
        };
        // The files spread evenly about the anchor: an even number of
        // files leaves the anchor between the two middle men.
        let spread = 2 * i64::try_from(file).unwrap_or(0) + 1 - i64::from(self.files);
        let lateral = spread * self.line.frontage_mm / 2;
        let depth = i64::try_from(rank).unwrap_or(0) * self.line.rank_gap_mm;
        let (right_x, right_y) = self.facing.right();
        let (back_x, back_y) = self.facing.back();
        (
            self.anchor_mm.0 + right_x * lateral + back_x * depth,
            self.anchor_mm.1 + right_y * lateral + back_y * depth,
        )
    }

    /// A stable digest of the line: the anchor, the order, the point, the
    /// shape, the men's places and the brain's own state. The same drill
    /// digests the same twice.
    pub fn digest(&self) -> u64 {
        let mut hash = Fnv64::new();
        hash.write_i64(self.anchor_mm.0);
        hash.write_i64(self.anchor_mm.1);
        hash.write_u8(self.order.code());
        hash.write_u8(self.legion.formation().as_u8());
        hash.write_u8(self.files);
        hash.write_u8(self.ranks);
        match self.target_mm {
            Some(target) => {
                hash.write_bool(true);
                hash.write_i64(target.0);
                hash.write_i64(target.1);
            }
            None => hash.write_bool(false),
        }
        for body in self.bodies.iter() {
            hash.write_i64(body.pos_mm.0);
            hash.write_i64(body.pos_mm.1);
            match body.slot {
                Some(slot) => {
                    hash.write_bool(true);
                    hash.write_u8(slot);
                }
                None => hash.write_bool(false),
            }
            hash.write_i32(body.speed_mm_per_tick);
        }
        hash.write_u64(self.legion.digest());
        hash.finish()
    }

    /// The order the brain obeys: the line holds while it falls in and
    /// advances on the point it marches to.
    fn brain_order(&self) -> Order {
        match self.order {
            BindOrder::FallIn | BindOrder::Hold => Order::Hold,
            BindOrder::Advance => Order::Advance,
        }
    }

    /// Read the formation's shape from the brain's slots: the ranks by
    /// their depth, the files by their order along the front. The brain's
    /// formation decides the shape; the line table gives it in
    /// millimetres.
    fn read_shape(&mut self) {
        let mut depths = [0usize; CONTUBERNIUM];
        let mut count = 0usize;
        for slot in self.slots.iter() {
            if depths[..count].contains(&slot.1) {
                continue;
            }
            depths[count] = slot.1;
            count += 1;
        }
        // The ranks run shallowest first: at most eight entries.
        for index in 1..count {
            let mut cursor = index;
            while cursor > 0 && depths[cursor - 1] > depths[cursor] {
                depths.swap(cursor - 1, cursor);
                cursor -= 1;
            }
        }
        let front = depths.first().copied().unwrap_or(0);
        let files = self.slots.iter().filter(|slot| slot.1 == front).count();
        self.depths = depths;
        self.depth_count = count;
        self.files = u8::try_from(files).unwrap_or(1);
        self.ranks = u8::try_from(count).unwrap_or(1);
    }

    /// The formation slot a soldier holds: the brain's slot read as a rank
    /// and a file, numbered along the front rank first.
    fn slot_index(&self, soldier: usize) -> Option<u8> {
        let cell = *self.slots.get(soldier)?;
        let (file, rank) = self.place_of(cell)?;
        let index = rank * usize::from(self.files) + file;
        u8::try_from(index).ok()
    }

    /// The file and the rank a formation slot stands in, read from the
    /// brain's own slot cells.
    fn file_and_rank(&self, slot: u8) -> Option<(usize, usize)> {
        for soldier in 0..CONTUBERNIUM {
            if self.slot_index(soldier) == Some(slot) {
                return self.place_of(self.slots[soldier]);
            }
        }
        None
    }

    /// The place of a brain slot cell in the formation: the file along the
    /// front and the rank behind it.
    fn place_of(&self, cell: (usize, usize)) -> Option<(usize, usize)> {
        let rank = self.depths[..self.depth_count]
            .iter()
            .position(|depth| *depth == cell.1)?;
        let mut file = 0usize;
        for other in self.slots.iter() {
            if other.1 == cell.1 && other.0 < cell.0 {
                file += 1;
            }
        }
        Some((file, rank))
    }

    /// Seat the loose men: each takes the nearest free slot and keeps it,
    /// so every man holds his row and file from then on.
    fn seat(&mut self) {
        let mut taken = [false; CONTUBERNIUM];
        for body in self.bodies.iter() {
            if let Some(slot) = body.slot {
                taken[usize::from(slot)] = true;
            }
        }
        for index in 0..CONTUBERNIUM {
            if self.bodies[index].slot.is_some() {
                continue;
            }
            let place = self.bodies[index].pos_mm;
            let Some(slot) = self.nearest_free_slot(place, &taken) else {
                continue;
            };
            taken[usize::from(slot)] = true;
            self.bodies[index].slot = Some(slot);
        }
    }

    /// The free slot nearest to `place`: the closest by distance, then the
    /// lowest slot, so the choice never depends on the order of the
    /// search.
    fn nearest_free_slot(&self, place: (i64, i64), taken: &[bool; CONTUBERNIUM]) -> Option<u8> {
        let mut best: Option<(i64, u8)> = None;
        for (slot, held) in taken.iter().enumerate() {
            if *held {
                continue;
            }
            let index = u8::try_from(slot).unwrap_or(0);
            let candidate = (distance_mm(place, self.slot_pos_mm(index)), index);
            match best {
                Some((best_distance, best_slot)) if (best_distance, best_slot) <= candidate => {}
                _ => best = Some(candidate),
            }
        }
        best.map(|(_, slot)| slot)
    }

    /// The line walks on the point the advance order named, at the pace of
    /// the slowest man.
    fn march(&mut self) {
        if self.order != BindOrder::Advance {
            return;
        }
        let Some(target) = self.target_mm else {
            return;
        };
        let pace = i64::from(self.pace_mm_per_tick());
        if pace <= 0 {
            return;
        }
        self.anchor_mm = step_toward(self.anchor_mm, target, pace);
    }

    /// Every man walks toward his slot at the pace of the slowest man. A
    /// held line does not move at all.
    fn close_up(&mut self) {
        if self.order == BindOrder::Hold {
            return;
        }
        let pace = i64::from(self.pace_mm_per_tick());
        if pace <= 0 {
            return;
        }
        for index in 0..CONTUBERNIUM {
            let Some(slot) = self.bodies[index].slot else {
                continue;
            };
            let target = self.slot_pos_mm(slot);
            let from = self.bodies[index].pos_mm;
            self.bodies[index].pos_mm = step_toward(from, target, pace);
        }
    }
}

/// Why the bind refused an input.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BindError {
    /// The line table does not hold the number.
    Table {
        /// The table key that is missing or unreadable.
        key: &'static str,
    },
    /// The order is not one of fall in, hold or advance.
    UnknownOrder {
        /// The refused order's name.
        name: &'static str,
    },
    /// The brain will not take the line.
    Brain {
        /// The brain's own refusal.
        error: LegionError,
    },
}

impl fmt::Display for BindError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BindError::Table { key } => write!(f, "legion bind: the line table has no {key}"),
            BindError::UnknownOrder { name } => {
                write!(f, "legion bind: the order {name} is not a drill order")
            }
            BindError::Brain { error } => write!(f, "legion bind: {error}"),
        }
    }
}

impl std::error::Error for BindError {}

/// Wrap the brain's own refusal.
fn brain_error(error: LegionError) -> BindError {
    BindError::Brain { error }
}

/// The brain's cell for a point on the ground. One cell is half the
/// frontage, so the brain's two-cell file step is the Roman frontage. The
/// grid's centre is the origin: a line that marches away from it keeps
/// its cells, and the formation's offsets around the anchor never fall
/// off the grid.
fn cell_of(pos_mm: (i64, i64), cell_mm: i64) -> (usize, usize) {
    let edge = i64::try_from(GRID_EDGE - 1).unwrap_or(0);
    let centre = i64::try_from(GRID_EDGE / 2).unwrap_or(0);
    let x = (centre + div_floor(pos_mm.0, cell_mm)).clamp(0, edge);
    let y = (centre + div_floor(pos_mm.1, cell_mm)).clamp(0, edge);
    (
        usize::try_from(x).unwrap_or(0),
        usize::try_from(y).unwrap_or(0),
    )
}

/// The brain's cell size in millimetres: half the frontage, so two cells
/// make one frontage.
fn cell_mm(line: Line) -> i64 {
    (line.frontage_mm / 2).max(1)
}

/// Floor division, so a point west of the origin keeps its cell.
fn div_floor(value: i64, divisor: i64) -> i64 {
    let quotient = value / divisor;
    let remainder = value % divisor;
    if remainder != 0 && (remainder < 0) != (divisor < 0) {
        quotient - 1
    } else {
        quotient
    }
}

/// Read one integer from a table section: the value of `key` under
/// `[section]`. `None` when the section or the key is missing or the
/// value is not a plain integer.
fn table_int(text: &str, section: &str, key: &str) -> Option<u32> {
    let mut in_section = false;
    for raw in text.lines() {
        let line = match raw.find('#') {
            Some(at) => &raw[..at],
            None => raw,
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            in_section = name.trim() == section;
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim() != key {
            continue;
        }
        return value.trim().parse::<u32>().ok();
    }
    None
}

/// The name of a squad order, for the refusal message.
fn order_name(order: SquadOrder) -> &'static str {
    match order {
        SquadOrder::Move(_) => "move",
        SquadOrder::Hold => "hold",
        SquadOrder::FallIn => "fall in",
        SquadOrder::Stack(_) => "stack",
        SquadOrder::Mirror(_) => "mirror",
        SquadOrder::Wedge(_) => "wedge",
        SquadOrder::Breach { .. } => "breach",
        SquadOrder::Restrain(_) => "restrain",
        SquadOrder::Collect(_) => "collect",
        SquadOrder::Split(_) => "split",
    }
}

/// The distance in millimetres between two points, rounded down.
fn distance_mm(a: (i64, i64), b: (i64, i64)) -> i64 {
    let dx = b.0.saturating_sub(a.0);
    let dy = b.1.saturating_sub(a.1);
    let squared = u128::from(dx.unsigned_abs()) * u128::from(dx.unsigned_abs())
        + u128::from(dy.unsigned_abs()) * u128::from(dy.unsigned_abs());
    i64::try_from(isqrt(squared)).unwrap_or(i64::MAX)
}

/// Walk from `from` toward `to` by at most `max_mm`: the step lands on the
/// target when it is close enough, so a man reaches his slot exactly.
fn step_toward(from: (i64, i64), to: (i64, i64), max_mm: i64) -> (i64, i64) {
    let distance = distance_mm(from, to);
    if distance == 0 {
        return to;
    }
    let step = max_mm.clamp(0, distance);
    if step >= distance {
        return to;
    }
    let dx = to.0.saturating_sub(from.0);
    let dy = to.1.saturating_sub(from.1);
    // The offsets stay on the segment, so they fit an i64.
    let ox = i64::try_from(i128::from(dx) * i128::from(step) / i128::from(distance)).unwrap_or(0);
    let oy = i64::try_from(i128::from(dy) * i128::from(step) / i128::from(distance)).unwrap_or(0);
    (from.0.saturating_add(ox), from.1.saturating_add(oy))
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_ai::orders::{DoorId, PointMm};

    /// The shipped line table.
    fn line() -> Line {
        Line::shipped().expect("the shipped line table reads")
    }

    /// Eight men scattered about the anchor: the muster before the
    /// fall-in.
    fn muster(anchor: (i64, i64)) -> [(i64, i64); CONTUBERNIUM] {
        [
            anchor,
            (anchor.0 - 300, anchor.1 + 120),
            (anchor.0 + 260, anchor.1 - 90),
            (anchor.0 - 150, anchor.1 - 200),
            (anchor.0 + 420, anchor.1 + 60),
            (anchor.0 - 500, anchor.1 - 40),
            (anchor.0 + 90, anchor.1 + 310),
            (anchor.0 - 60, anchor.1 - 380),
        ]
    }

    /// A contubernium of eight at the origin, facing east.
    fn contubernium(formation: Formation) -> Contubernium {
        Contubernium::new(line(), (0, 0), Facing::East, formation, muster((0, 0)))
            .expect("the contubernium builds")
    }

    /// How far a man stands from his slot, in millimetres.
    fn off_slot_mm(bind: &Contubernium, index: usize) -> i64 {
        let body = bind.body(index).expect("a man of the contubernium");
        let slot = body.slot().expect("a slot");
        distance_mm(body.pos_mm(), bind.slot_pos_mm(slot))
    }

    /// Where the eight men stand, in slot order.
    fn places(bind: &Contubernium) -> Vec<(i64, i64)> {
        (0..CONTUBERNIUM)
            .map(|index| {
                bind.body(index)
                    .expect("a man of the contubernium")
                    .pos_mm()
            })
            .collect()
    }

    /// Run the whole drill: fall in, then march the line 50 m east.
    fn drill(bind: &mut Contubernium) {
        bind.apply_order(SquadOrder::FallIn).expect("fall in");
        for tick in 0..300u32 {
            bind.step(tick).expect("the line steps");
        }
        bind.apply_order(SquadOrder::Move(PointMm {
            x_mm: 50_000,
            y_mm: 0,
        }))
        .expect("advance");
        for tick in 300..3_000u32 {
            bind.step(tick).expect("the line steps");
        }
    }

    #[test]
    fn fall_in_holds_the_slots_while_the_line_advances_fifty_metres() {
        let mut bind = contubernium(Formation::Line);
        drill(&mut bind);
        assert_eq!(bind.anchor_mm(), (50_000, 0));
        for index in 0..CONTUBERNIUM {
            assert!(
                off_slot_mm(&bind, index) <= 150,
                "man {index} is off his slot after the march"
            );
        }
    }

    #[test]
    fn hold_stops_the_line_within_a_second() {
        let mut bind = contubernium(Formation::Line);
        bind.apply_order(SquadOrder::FallIn).expect("fall in");
        for tick in 0..300u32 {
            bind.step(tick).expect("the line steps");
        }
        bind.apply_order(SquadOrder::Move(PointMm {
            x_mm: 50_000,
            y_mm: 0,
        }))
        .expect("advance");
        for tick in 300..400u32 {
            bind.step(tick).expect("the line steps");
        }
        // The line is marching; the hold must stop it inside a second.
        bind.apply_order(SquadOrder::Hold).expect("hold");
        let before = places(&bind);
        let anchor_before = bind.anchor_mm();
        for tick in 400..460u32 {
            bind.step(tick).expect("the line steps");
        }
        assert_eq!(before, places(&bind), "the line moved after the hold");
        assert_eq!(
            anchor_before,
            bind.anchor_mm(),
            "the anchor moved after the hold"
        );
    }

    #[test]
    fn two_runs_hash_the_same() {
        let mut first = contubernium(Formation::Line);
        drill(&mut first);
        let mut second = contubernium(Formation::Line);
        drill(&mut second);
        assert_eq!(first.digest(), second.digest());
    }

    #[test]
    fn the_shipped_line_table_gives_the_roman_frontage() {
        let line = line();
        assert_eq!(line.frontage_mm, 888);
        assert_eq!(line.rank_gap_mm, 1776);
    }

    #[test]
    fn a_table_without_the_numbers_is_refused() {
        assert!(Line::parse("[units]\nmm_per_ft = 296\n").is_err());
        assert!(Line::parse("[line]\nfrontage_ft = 3\nrank_gap_ft = 6\n").is_err());
        assert!(
            Line::parse("[units]\nmm_per_ft = 0\n[line]\nfrontage_ft = 3\nrank_gap_ft = 6\n")
                .is_err()
        );
    }

    #[test]
    fn the_line_holds_its_frontage_and_its_rank_gap() {
        let mut bind = contubernium(Formation::Line);
        bind.apply_order(SquadOrder::FallIn).expect("fall in");
        bind.step(0).expect("the line steps");
        // The eight files stand one frontage apart.
        assert_eq!(distance_mm(bind.slot_pos_mm(0), bind.slot_pos_mm(1)), 888);
        // The column marches four abreast and two ranks deep: the second
        // rank stands one rank gap back.
        let mut column = contubernium(Formation::Column);
        column.apply_order(SquadOrder::FallIn).expect("fall in");
        column.step(0).expect("the line steps");
        assert_eq!(
            distance_mm(column.slot_pos_mm(0), column.slot_pos_mm(4)),
            1776
        );
    }

    #[test]
    fn a_man_takes_the_slot_nearest_to_him() {
        let mut bind = contubernium(Formation::Line);
        // The brain decides the slots, so the line knows its shape before
        // the fall-in seats the men.
        bind.step(0).expect("the line steps");
        // The fourth man stands by the first file of the line.
        let first_file = bind.slot_pos_mm(0);
        assert!(bind.place(3, first_file));
        bind.apply_order(SquadOrder::FallIn).expect("fall in");
        bind.step(1).expect("the line steps");
        assert_eq!(bind.body(3).expect("a man").slot(), Some(0));
    }

    #[test]
    fn a_man_keeps_his_row_and_file() {
        let mut bind = contubernium(Formation::Line);
        drill(&mut bind);
        let slots: Vec<Option<u8>> = (0..CONTUBERNIUM)
            .map(|index| bind.body(index).expect("a man").slot())
            .collect();
        let mut held = slots.clone();
        held.sort_unstable();
        let every_slot: Vec<Option<u8>> = (0..CONTUBERNIUM)
            .map(|slot| Some(u8::try_from(slot).unwrap_or(0)))
            .collect();
        assert_eq!(held, every_slot, "two men share a slot");
        // A second fall-in moves nobody.
        let before = places(&bind);
        bind.apply_order(SquadOrder::FallIn).expect("fall in");
        for tick in 3_000..3_020u32 {
            bind.step(tick).expect("the line steps");
        }
        assert_eq!(before, places(&bind), "a man changed his place");
        let after: Vec<Option<u8>> = (0..CONTUBERNIUM)
            .map(|index| bind.body(index).expect("a man").slot())
            .collect();
        assert_eq!(slots, after, "a man changed his slot");
    }

    #[test]
    fn the_slowest_man_sets_the_pace() {
        let mut bind = contubernium(Formation::Line);
        assert!(bind.set_speed(4, 10));
        assert_eq!(bind.pace_mm_per_tick(), 10);
        bind.apply_order(SquadOrder::Move(PointMm {
            x_mm: 1_000,
            y_mm: 0,
        }))
        .expect("advance");
        bind.step(0).expect("the line steps");
        assert_eq!(bind.anchor_mm(), (10, 0));
    }

    #[test]
    fn an_order_outside_the_drill_is_refused_and_changes_nothing() {
        let mut bind = contubernium(Formation::Line);
        let before = bind.digest();
        let error = bind
            .apply_order(SquadOrder::Stack(DoorId(1)))
            .expect_err("the order is refused");
        assert_eq!(error, BindError::UnknownOrder { name: "stack" });
        assert_eq!(bind.digest(), before);
    }

    #[test]
    fn the_brain_decides_the_slots_the_bind_reads() {
        let mut bind = contubernium(Formation::Line);
        bind.apply_order(SquadOrder::FallIn).expect("fall in");
        bind.step(0).expect("the line steps");
        // Every soldier of the brain holds a slot of the formation.
        for soldier in 0..CONTUBERNIUM {
            assert!(
                bind.slot_index(soldier).is_some(),
                "soldier {soldier} has no slot"
            );
        }
        assert_eq!(bind.legion().count(), CONTUBERNIUM);
    }
}
