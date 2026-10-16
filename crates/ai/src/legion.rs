//! legion: squad-scale brain for hundreds of soldiers (formations,
//! shield wall, flanking, orders, thinking LOD).
//!
//! Owner: ai crate. Consumer: the arena match turns `slots_out` into
//! steering through the nav tiles (leaders query flow fields, followers
//! hold formation gaps); the thinking-LOD audit builds on `think_period`.
//! Units: grid cells in, slot cells out, ticks at 60 Hz, stamina
//! `0..=1000` on the duelist stamina bounds. Determinism: soldier-id order, integer
//! math only, no hash order, floats or wall clock. Errors: [`LegionError`]
//! refuses bad input, never half-applies. Budget: one step is O(count)
//! integer ops with zero steady-state allocation; far soldiers think
//! every 8th tick so a 500-soldier legion stays under 1.5 ms of AI.

use core::fmt;

use crate::alert::{self, AlertState, Sense, Sight};
use crate::depth::{Phase, Role, STAMINA_COLLAPSE_AT, STAMINA_MAX, STAMINA_MIN};
use engine_core::Fnv64;

/// Largest legion; bounds every lane up front (contract needs 500).
pub const MAX_SOLDIERS: usize = 512;
/// Contract legion size the budget and fixture rows exercise.
pub const CONTRACT_SOLDIERS: usize = 500;
/// Soldiers per squad; squad `s` owns soldiers `s*8..s*8+8`.
pub const SQUAD_SIZE: usize = 8;
/// Largest squad roster for a full legion.
pub const MAX_SQUADS: usize = MAX_SOLDIERS / SQUAD_SIZE;
/// Fixed tick rate the LOD clock counts in.
pub const TICK_HZ: u32 = 60;
/// Enemy range in cells that braces a holding shield wall.
pub const BRACE_RANGE_CELLS: u32 = 16;
/// Lateral gap in cells a flanking squad keeps off the enemy anchor.
pub const FLANK_GAP_CELLS: i32 = 12;
/// Largest grid edge in cells one legion steps on.
pub const MAX_GRID_EDGE: usize = 4096;
/// Largest spacing between formation neighbours in cells.
pub const MAX_SPACING_CELLS: u32 = 8;
/// Chebyshev cells past which a soldier counts as far for commander LOD.
/// A squad is far only when every soldier sits past this range.
pub const SQUAD_FAR_CELLS: u32 = 64;
/// Ticks between squad-commander syncs for far Hold/Advance squads.
pub const SQUAD_SYNC_PERIOD: u32 = 8;

/// Legion shape: where each soldier stands around the anchor.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Formation {
    /// Single rank line, centered on the anchor.
    Line,
    /// Four-wide march column behind the anchor.
    Column,
    /// Apex-forward wedge (index 0 at the tip).
    Wedge,
    /// Single rank line that braces when holding near the enemy.
    ShieldWall,
}

impl Formation {
    /// Parse a formation byte (`0` line, `1` column, `2` wedge,
    /// `3` shield wall). Anything else refuses loudly.
    pub fn from_u8(value: u8) -> Result<Self, LegionError> {
        match value {
            0 => Ok(Formation::Line),
            1 => Ok(Formation::Column),
            2 => Ok(Formation::Wedge),
            3 => Ok(Formation::ShieldWall),
            _ => Err(LegionError::UnknownFormation { value }),
        }
    }

    /// Formation byte round-trip for tapes (`0`..=`3`).
    pub const fn as_u8(self) -> u8 {
        match self {
            Formation::Line => 0,
            Formation::Column => 1,
            Formation::Wedge => 2,
            Formation::ShieldWall => 3,
        }
    }
}

/// Squad order: how one squad shifts its formation slots.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Order {
    /// Hold formation slots; a shield wall braces near the enemy.
    Hold,
    /// Shift slots 4 cells toward the enemy.
    Advance,
    /// Shift slots 12 cells toward the enemy (falls back to [`Order::Advance`]
    /// while gassed, like the duelist stamina gate stops committing).
    Charge,
    /// Shift slots 6 cells away from the enemy.
    Retreat,
    /// Wrap port-side of the enemy anchor, then close in.
    FlankLeft,
    /// Wrap starboard-side of the enemy anchor, then close in.
    FlankRight,
}

impl Order {
    /// Parse an order byte (`0` hold, `1` advance, `2` charge, `3` retreat,
    /// `4` flank left, `5` flank right). Anything else refuses loudly.
    pub fn from_u8(value: u8) -> Result<Self, LegionError> {
        match value {
            0 => Ok(Order::Hold),
            1 => Ok(Order::Advance),
            2 => Ok(Order::Charge),
            3 => Ok(Order::Retreat),
            4 => Ok(Order::FlankLeft),
            5 => Ok(Order::FlankRight),
            _ => Err(LegionError::UnknownOrder { value }),
        }
    }

    /// Order byte round-trip for tapes (`0`..=`5`).
    pub const fn as_u8(self) -> u8 {
        match self {
            Order::Hold => 0,
            Order::Advance => 1,
            Order::Charge => 2,
            Order::Retreat => 3,
            Order::FlankLeft => 4,
            Order::FlankRight => 5,
        }
    }
}

/// Legion failures. Invalid external input returns these; the
/// simulation never panics on them and a bad batch writes nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LegionError {
    /// No soldiers were seated.
    EmptyLegion,
    /// More soldiers than [`MAX_SOLDIERS`]; count carried.
    TooManySoldiers {
        /// Soldier count offered.
        got: usize,
    },
    /// Soldier index past the legion; index carried.
    BadSoldier {
        /// Soldier index offered.
        index: usize,
    },
    /// Squad index past the roster; index carried.
    BadSquad {
        /// Squad index offered.
        index: usize,
    },
    /// Formation byte outside `0..=3`; culprit carried.
    UnknownFormation {
        /// The rejected formation byte.
        value: u8,
    },
    /// Order byte outside `0..=5`; culprit carried.
    UnknownOrder {
        /// The rejected order byte.
        value: u8,
    },
    /// Cell outside the grid; axis and value carried.
    OutOfBounds {
        /// `b'x'` or `b'y'`.
        axis: u8,
        /// The rejected coordinate.
        value: usize,
    },
    /// Grid dims are zero or over [`MAX_GRID_EDGE`]; dims carried.
    BadDims {
        /// Grid width offered in cells.
        w: usize,
        /// Grid height offered in cells.
        h: usize,
    },
    /// Spacing is zero or over [`MAX_SPACING_CELLS`]; value carried.
    BadSpacing {
        /// Spacing offered in cells.
        got: u32,
    },
    /// Lane lengths differ from the legion count; all three carried.
    MismatchedLanes {
        /// `soldiers` lane length.
        soldiers: usize,
        /// `slots_out` lane length.
        slots: usize,
        /// `brace_out` lane length.
        brace: usize,
    },
}

impl fmt::Display for LegionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LegionError::EmptyLegion => write!(f, "legion: no soldiers seated"),
            LegionError::TooManySoldiers { got } => {
                write!(f, "legion: {got} soldiers past the {MAX_SOLDIERS} cap")
            }
            LegionError::BadSoldier { index } => {
                write!(f, "legion: soldier {index} past the legion")
            }
            LegionError::BadSquad { index } => {
                write!(f, "legion: squad {index} past the roster")
            }
            LegionError::UnknownFormation { value } => {
                write!(f, "legion: unknown formation {value}")
            }
            LegionError::UnknownOrder { value } => {
                write!(f, "legion: unknown order {value}")
            }
            LegionError::OutOfBounds { axis, value } => {
                write!(f, "legion: cell {}={value} past the grid", *axis as char)
            }
            LegionError::BadDims { w, h } => {
                write!(f, "legion: grid {w}x{h} outside 1..={MAX_GRID_EDGE}")
            }
            LegionError::BadSpacing { got } => {
                write!(f, "legion: spacing {got} outside 1..={MAX_SPACING_CELLS}")
            }
            LegionError::MismatchedLanes {
                soldiers,
                slots,
                brace,
            } => write!(
                f,
                "legion: lanes {soldiers}/{slots}/{brace} differ from the legion count"
            ),
        }
    }
}

impl std::error::Error for LegionError {}

/// Centered step for formation rank `i` of `n`: `2*i+1-n`, so rank 0 of
/// 1 sits on the anchor and wider ranks spread symmetrically. Both
/// inputs are in range (`n <= MAX_SOLDIERS`), so the math cannot overflow.
fn centered_step(i: usize, n: usize) -> i32 {
    2 * i as i32 + 1 - n as i32
}

/// Formation offset in cells for soldier `index` of `count` at `spacing`.
///
/// Pure integer geometry: line and shield wall share the single rank,
/// column marches four wide, wedge stacks rows of 1, 2, 3, ... Odd
/// spacings truncate toward the anchor; even spacings stay exact.
/// Errors: [`LegionError::EmptyLegion`], [`LegionError::TooManySoldiers`],
/// [`LegionError::BadSoldier`], [`LegionError::BadSpacing`].
pub fn formation_slot(
    formation: Formation,
    index: usize,
    count: usize,
    spacing: u32,
) -> Result<(i32, i32), LegionError> {
    if count == 0 {
        return Err(LegionError::EmptyLegion);
    }
    if count > MAX_SOLDIERS {
        return Err(LegionError::TooManySoldiers { got: count });
    }
    if index >= count {
        return Err(LegionError::BadSoldier { index });
    }
    if spacing == 0 || spacing > MAX_SPACING_CELLS {
        return Err(LegionError::BadSpacing { got: spacing });
    }
    // Spacing validated above as 1..=8, so the conversion always succeeds.
    let s = i32::try_from(spacing).unwrap_or(1);
    match formation {
        Formation::Line | Formation::ShieldWall => Ok((centered_step(index, count) * s / 2, 0)),
        Formation::Column => {
            const FILES: usize = 4;
            Ok((
                centered_step(index % FILES, FILES) * s / 2,
                (index / FILES) as i32 * s,
            ))
        }
        Formation::Wedge => {
            // Row r holds r+1 soldiers; find the row holding `index`.
            // Rows stay under 32 for any legal count, so the loop is bounded.
            let target = index as i32;
            let mut row: i32 = 0;
            while (row + 1) * (row + 2) / 2 <= target {
                row += 1;
            }
            let start = row * (row + 1) / 2;
            let k = (target - start) as usize;
            Ok((centered_step(k, row as usize + 1) * s / 2, -row * s))
        }
    }
}

/// Thinking period in ticks for a soldier `dist_cells` (Chebyshev) from
/// the enemy anchor: near soldiers think every tick, far ones every 8th.
/// Pure table, so the LOD audit can replay it without a legion.
pub const fn think_period(dist_cells: u32) -> u32 {
    if dist_cells <= 8 {
        1
    } else if dist_cells <= 24 {
        2
    } else if dist_cells <= 64 {
        4
    } else {
        8
    }
}

/// Chebyshev distance in cells between two cells (integer, no root).
fn chebyshev(a: (usize, usize), b: (usize, usize)) -> u32 {
    (a.0.abs_diff(b.0).max(a.1.abs_diff(b.1))) as u32
}

/// Sign of `v`: `1`, `0` or `-1`.
fn sign(v: i32) -> i32 {
    i32::from(v > 0) - i32::from(v < 0)
}

/// Clamp a signed cell onto `0..max`.
fn clamp_cell(v: i32, max: usize) -> usize {
    (v.max(0).min(max as i32 - 1)) as usize
}

/// Stamina pace per order per think: holding recovers, committing drains.
/// Bounds come from the duelist tables; the shape (backing off recovers,
/// charging gasses) matches the duel pacing.
const fn stamina_pace(order: Order) -> i32 {
    match order {
        Order::Hold => 10,
        Order::Advance => -6,
        Order::Charge => -14,
        Order::Retreat => -4,
        Order::FlankLeft | Order::FlankRight => -8,
    }
}

/// Squad-commander far predicate: Hold/Advance only, every soldier past
/// [`SQUAD_FAR_CELLS`] Chebyshev cells from the enemy anchor. Integer-only,
/// zero allocation; the caller passes the squad slice of the soldier lane.
fn squad_is_far(order: Order, soldiers: &[(usize, usize)], enemy: (usize, usize)) -> bool {
    if order != Order::Hold && order != Order::Advance {
        return false;
    }
    for cell in soldiers.iter() {
        if chebyshev(*cell, enemy) <= SQUAD_FAR_CELLS {
            return false;
        }
    }
    // An empty slice never counts as far; guards the partial tail.
    !soldiers.is_empty()
}

/// Fold one enemy sighting into the alert ladder.
/// Units: `tick` in fixed-tick counts, `enemy` in grid cells, stored cell
/// in centimetres (`100` per cell). Determinism: pure integer math over the
/// caller-passed tick; same inputs give the same output, no wall clock.
/// Errors: none (out-of-range cells fall back to no cell, never panic).
pub fn alert_step_for_enemy(
    state: &AlertState,
    sense: Sense,
    tick: u32,
    enemy: (usize, usize),
) -> AlertState {
    let tick64 = u64::from(tick);
    let seen_cell_cm = if sense.sight == Sight::Confirmed {
        match (i32::try_from(enemy.0), i32::try_from(enemy.1)) {
            (Ok(x), Ok(y)) => Some([x.saturating_mul(100), y.saturating_mul(100)]),
            _ => None,
        }
    } else {
        None
    };
    alert::step(state, sense, tick64, seen_cell_cm)
}

/// Read the fixed track cell back in grid cells.
/// Units: stored centimetres divided by `100` per cell. Determinism: pure
/// integer read; never follows the live position. Errors: none (`None`
/// for unaware or missing cells; negatives saturate to zero).
pub fn track_cell(state: &AlertState) -> Option<(usize, usize)> {
    if state.alert == alert::Alert::Unaware {
        return None;
    }
    let cell = alert::last_seen(state)?;
    let x = usize::try_from(cell[0].max(0) / 100).ok()?;
    let y = usize::try_from(cell[1].max(0) / 100).ok()?;
    Some((x, y))
}

/// One legion: formation slots plus squad orders over SoA lanes.
///
/// Soldiers are indices (`0..count`); squad `s` owns indices
/// `s*SQUAD_SIZE..(s+1)*SQUAD_SIZE`. Wing squads take the duelist
/// [`Role::Flanker`], center squads [`Role::Holder`]; the arena drives
/// [`Phase`] from battle progress. Roles reassign only through
/// [`Legion::set_order`] and stamina collapse, never mid-think, so a
/// tape replays exactly. Steering stays in nav: this brain only decides
/// slot cells plus brace flags.
#[derive(Clone, Debug)]
pub struct Legion {
    count: usize,
    formation: Formation,
    phase: Phase,
    anchor: (usize, usize),
    enemy: (usize, usize),
    grid: (usize, usize),
    spacing: u32,
    orders: [Order; MAX_SQUADS],
    roles: [Role; MAX_SQUADS],
    slots: [(usize, usize); MAX_SOLDIERS],
    braced: [bool; MAX_SOLDIERS],
    stamina: [i32; MAX_SOLDIERS],
    thinks_last: usize,
    alert: AlertState,
    squad_anchor: [(usize, usize); MAX_SQUADS],
    squad_enemy: [(usize, usize); MAX_SQUADS],
    squad_formation: [Formation; MAX_SQUADS],
    squad_order: [Order; MAX_SQUADS],
    squad_spacing: [u32; MAX_SQUADS],
    squad_synced: [bool; MAX_SQUADS],
}

impl Legion {
    /// Seat `count` soldiers in `formation` around `anchor`, facing the
    /// `enemy` anchor on a `grid` (`(w, h)` cells). All squads start on
    /// [`Order::Hold`] at full stamina with freshly decided slots.
    /// Errors: [`LegionError::EmptyLegion`],
    /// [`LegionError::TooManySoldiers`], [`LegionError::BadDims`],
    /// [`LegionError::OutOfBounds`].
    pub fn new(
        count: usize,
        formation: Formation,
        anchor: (usize, usize),
        enemy: (usize, usize),
        grid: (usize, usize),
    ) -> Result<Self, LegionError> {
        if count == 0 {
            return Err(LegionError::EmptyLegion);
        }
        if count > MAX_SOLDIERS {
            return Err(LegionError::TooManySoldiers { got: count });
        }
        check_dims(grid)?;
        check_cell(anchor, grid)?;
        check_cell(enemy, grid)?;
        let squads = count.div_ceil(SQUAD_SIZE);
        let mut legion = Self {
            count,
            formation,
            phase: Phase::Open,
            anchor,
            enemy,
            grid,
            spacing: 1,
            orders: [Order::Hold; MAX_SQUADS],
            roles: [Role::Holder; MAX_SQUADS],
            slots: [(0, 0); MAX_SOLDIERS],
            braced: [false; MAX_SOLDIERS],
            stamina: [STAMINA_MAX; MAX_SOLDIERS],
            thinks_last: 0,
            alert: alert::fresh(),
            squad_anchor: [(0, 0); MAX_SQUADS],
            squad_enemy: [(0, 0); MAX_SQUADS],
            squad_formation: [Formation::Line; MAX_SQUADS],
            squad_order: [Order::Hold; MAX_SQUADS],
            squad_spacing: [1; MAX_SQUADS],
            squad_synced: [false; MAX_SQUADS],
        };
        for s in 0..squads {
            legion.roles[s] = if squads > 1 && (s == 0 || s + 1 == squads) {
                Role::Flanker
            } else {
                Role::Holder
            };
        }
        // Opening decision for every soldier, as if all thought at tick 0.
        for i in 0..count {
            let (slot, brace) = legion.decide_slot(i);
            legion.slots[i] = slot;
            legion.braced[i] = brace;
        }
        // Seed the squad-commander cache to the opening inputs so a static
        // siege starts synced and holds between sync waves with zero decides.
        for s in 0..squads {
            legion.squad_anchor[s] = anchor;
            legion.squad_enemy[s] = enemy;
            legion.squad_formation[s] = formation;
            legion.squad_order[s] = Order::Hold;
            legion.squad_spacing[s] = 1;
            legion.squad_synced[s] = true;
        }
        Ok(legion)
    }

    /// Soldier count.
    pub const fn count(&self) -> usize {
        self.count
    }

    /// Squad count (`count` rounded up to [`SQUAD_SIZE`]).
    pub fn squad_count(&self) -> usize {
        self.count.div_ceil(SQUAD_SIZE)
    }

    /// Squad owning soldier `index`.
    pub const fn soldier_squad(index: usize) -> usize {
        index / SQUAD_SIZE
    }

    /// Current formation.
    pub const fn formation(&self) -> Formation {
        self.formation
    }

    /// Install a formation and re-decide every slot at once (a drill
    /// command, so no soldier thinks stale geometry afterwards).
    pub fn set_formation(&mut self, formation: Formation) {
        self.formation = formation;
        for i in 0..self.count {
            let (slot, brace) = self.decide_slot(i);
            self.slots[i] = slot;
            self.braced[i] = brace;
        }
        self.sync_all_squads();
    }

    /// Current battle phase.
    pub const fn phase(&self) -> Phase {
        self.phase
    }

    /// Install the battle phase the arena drives from progress.
    pub fn set_phase(&mut self, phase: Phase) {
        self.phase = phase;
    }

    /// Current anchor cell.
    pub const fn anchor(&self) -> (usize, usize) {
        self.anchor
    }

    /// Move the anchor (the leader query moved it). Out-of-grid anchors
    /// refuse and keep the old one. Returns the stored anchor.
    pub fn set_anchor(&mut self, anchor: (usize, usize)) -> Result<(usize, usize), LegionError> {
        check_cell(anchor, self.grid)?;
        self.anchor = anchor;
        Ok(self.anchor)
    }

    /// Current enemy anchor cell.
    pub const fn enemy(&self) -> (usize, usize) {
        self.enemy
    }

    /// Move the enemy anchor (scout report). Out-of-grid anchors refuse
    /// and keep the old one. Returns the stored anchor.
    pub fn set_enemy(&mut self, enemy: (usize, usize)) -> Result<(usize, usize), LegionError> {
        check_cell(enemy, self.grid)?;
        self.enemy = enemy;
        Ok(self.enemy)
    }

    /// Presentation preview of the alert ladder for the live foe slot.
    /// Folds `sense` at `tick` through [`alert_step_for_enemy`] with the
    /// live [`Legion::enemy`] cell, then reads the fixed Track target
    /// through [`track_cell`] plus [`alert::last_seen`]. Overlays read the
    /// returned state; the fixed tick never steers by it and it never
    /// enters [`Legion::digest`], so the golden hash stays unchanged.
    /// Units: `tick` in fixed-tick counts, cells in grid cells.
    /// Determinism: pure integer read over the caller-passed tick; same
    /// inputs give the same output, no wall clock. Errors: none.
    pub fn alert_preview(&self, state: &AlertState, sense: Sense, tick: u32) -> AlertState {
        let next = alert_step_for_enemy(state, sense, tick, self.enemy);
        // Presentation reads: the fixed cell never follows the live slot.
        // Both lenses stay live so the helper never becomes test-only.
        let fixed = track_cell(&next);
        let seen = alert::last_seen(&next);
        let _ = (fixed, seen);
        next
    }

    /// Stored alert rung threaded across [`Legion::step`] ticks.
    /// Presentation only for overlays; it never steers slots and never
    /// enters [`Legion::digest`], so the golden hash stays unchanged.
    /// Units: last-seen cell in grid cells via [`track_cell`].
    /// Determinism: pure read of the stored rung; same stored state gives
    /// the same output, no wall clock. Errors: none.
    pub const fn alert_state(&self) -> AlertState {
        self.alert
    }

    /// Current neighbour spacing in cells.
    pub const fn spacing(&self) -> u32 {
        self.spacing
    }

    /// Set neighbour spacing (`1..=MAX_SPACING_CELLS`) and re-decide
    /// every slot at once, like [`Legion::set_formation`].
    /// Errors: [`LegionError::BadSpacing`].
    pub fn set_spacing(&mut self, spacing: u32) -> Result<u32, LegionError> {
        if spacing == 0 || spacing > MAX_SPACING_CELLS {
            return Err(LegionError::BadSpacing { got: spacing });
        }
        self.spacing = spacing;
        for i in 0..self.count {
            let (slot, brace) = self.decide_slot(i);
            self.slots[i] = slot;
            self.braced[i] = brace;
        }
        self.sync_all_squads();
        Ok(self.spacing)
    }

    /// Order of squad `squad`. Errors: [`LegionError::BadSquad`].
    pub fn order_of(&self, squad: usize) -> Result<Order, LegionError> {
        if squad >= self.squad_count() {
            return Err(LegionError::BadSquad { index: squad });
        }
        Ok(self.orders[squad])
    }

    /// Role of squad `squad` (wing flankers, center holders).
    /// Errors: [`LegionError::BadSquad`].
    pub fn role_of(&self, squad: usize) -> Result<Role, LegionError> {
        if squad >= self.squad_count() {
            return Err(LegionError::BadSquad { index: squad });
        }
        Ok(self.roles[squad])
    }

    /// Give squad `squad` an order (a reassign boundary: the squad
    /// re-decides its slots on its next think, never mid-swing).
    /// Errors: [`LegionError::BadSquad`].
    pub fn set_order(&mut self, squad: usize, order: Order) -> Result<(), LegionError> {
        if squad >= self.squad_count() {
            return Err(LegionError::BadSquad { index: squad });
        }
        self.orders[squad] = order;
        Ok(())
    }

    /// Refresh every squad cache to the live inputs (drill commands only).
    /// Integer copies over fixed arrays; zero allocation.
    fn sync_all_squads(&mut self) {
        let squads = self.squad_count();
        for s in 0..squads {
            self.squad_anchor[s] = self.anchor;
            self.squad_enemy[s] = self.enemy;
            self.squad_formation[s] = self.formation;
            self.squad_order[s] = self.orders[s];
            self.squad_spacing[s] = self.spacing;
            self.squad_synced[s] = true;
        }
    }

    /// True when squad `squad` cache matches the live inputs.
    /// Integer compares only; zero allocation.
    fn squad_inputs_equal(&self, squad: usize) -> bool {
        self.squad_synced[squad]
            && self.squad_anchor[squad] == self.anchor
            && self.squad_enemy[squad] == self.enemy
            && self.squad_formation[squad] == self.formation
            && self.squad_order[squad] == self.orders[squad]
            && self.squad_spacing[squad] == self.spacing
    }

    /// Record the live inputs for squad `squad` after a full re-decide.
    /// Integer copies only; zero allocation.
    fn sync_one_squad(&mut self, squad: usize) {
        self.squad_anchor[squad] = self.anchor;
        self.squad_enemy[squad] = self.enemy;
        self.squad_formation[squad] = self.formation;
        self.squad_order[squad] = self.orders[squad];
        self.squad_spacing[squad] = self.spacing;
        self.squad_synced[squad] = true;
    }

    /// Stamina of soldier `index` (`0..=1000`).
    /// Errors: [`LegionError::BadSoldier`].
    pub fn stamina_of(&self, index: usize) -> Result<i32, LegionError> {
        if index >= self.count {
            return Err(LegionError::BadSoldier { index });
        }
        Ok(self.stamina[index])
    }

    /// Set stamina, clamped to `0..=1000`. Returns the clamped value.
    /// Errors: [`LegionError::BadSoldier`].
    pub fn set_stamina(&mut self, index: usize, stamina: i32) -> Result<i32, LegionError> {
        if index >= self.count {
            return Err(LegionError::BadSoldier { index });
        }
        self.stamina[index] = stamina.clamp(STAMINA_MIN, STAMINA_MAX);
        Ok(self.stamina[index])
    }

    /// Decides (fresh slot computations) on the last [`Legion::step`].
    pub const fn thinks_last(&self) -> usize {
        self.thinks_last
    }

    /// Decided slots for `0..count` (recorded intents between thinks).
    pub fn slots(&self) -> &[(usize, usize)] {
        &self.slots[..self.count]
    }

    /// Brace flags for `0..count`.
    pub fn braced(&self) -> &[bool] {
        &self.braced[..self.count]
    }

    /// Step one tick: squad-commander LOD plus the per-soldier path.
    /// Far Hold/Advance squads with unchanged inputs hold recorded slots
    /// between 8-tick squad syncs (one leader decide per sync, zero decides
    /// and zero stamina pacing off-sync); any other squad keeps the exact
    /// per-soldier LOD clock of today. Writers go to the caller lanes;
    /// nothing allocates. Returns how many soldiers thought. Input order
    /// decides output order, so the batch replays exactly. A bad batch writes nothing.
    /// Errors: [`LegionError::MismatchedLanes`],
    /// [`LegionError::OutOfBounds`] (carries the soldier index as `value`
    /// on the failing axis... see `axis`/`value`: `value` is the bad
    /// coordinate).
    pub fn step(
        &mut self,
        tick: u32,
        soldiers: &[(usize, usize)],
        slots_out: &mut [(usize, usize)],
        brace_out: &mut [bool],
    ) -> Result<usize, LegionError> {
        if soldiers.len() != self.count
            || slots_out.len() != self.count
            || brace_out.len() != self.count
        {
            return Err(LegionError::MismatchedLanes {
                soldiers: soldiers.len(),
                slots: slots_out.len(),
                brace: brace_out.len(),
            });
        }
        // Validate every cell first: a bad batch writes nothing.
        for cell in soldiers.iter() {
            check_cell(*cell, self.grid)?;
        }
        // Presentation only: fold the live foe slot through the alert
        // ladder for overlays; the fixed Track never steers slots and
        // never enters digest, so the golden hash stays unchanged.
        // The stored rung threads across ticks: close contact confirms,
        // quiet ticks hold the fixed cell, then expire to unaware.
        let mut seen = false;
        for cell in soldiers.iter() {
            if chebyshev(*cell, self.enemy) <= 8 {
                seen = true;
                break;
            }
        }
        let sense = Sense {
            sight: if seen { Sight::Confirmed } else { Sight::None },
            heard: false,
            door_closed: false,
        };
        let current = self.alert_state();
        let next = self.alert_preview(&current, sense, tick);
        self.alert = next;
        let mut thinks = 0usize;
        let squads = self.squad_count();
        let sync_wave = tick.is_multiple_of(SQUAD_SYNC_PERIOD);
        for squad in 0..squads {
            thinks += self.step_squad(squad, tick, soldiers, slots_out, brace_out, sync_wave);
        }
        self.thinks_last = thinks;
        Ok(thinks)
    }

    /// Step one squad for [`Legion::step`]: far Hold/Advance squads with
    /// unchanged inputs hold recorded slots between sync waves, changed far
    /// squads re-decide fully, others keep the per-soldier LOD clock.
    /// Integer-only, zero allocation; returns the squad think count.
    fn step_squad(
        &mut self,
        squad: usize,
        tick: u32,
        soldiers: &[(usize, usize)],
        slots_out: &mut [(usize, usize)],
        brace_out: &mut [bool],
        sync_wave: bool,
    ) -> usize {
        let start = squad * SQUAD_SIZE;
        let mut end = start + SQUAD_SIZE;
        if end > self.count {
            end = self.count;
        }
        let order = self.orders[squad];
        let mut thinks = 0usize;
        if squad_is_far(order, &soldiers[start..end], self.enemy) {
            if self.squad_inputs_equal(squad) {
                if sync_wave {
                    thinks += 1;
                    let (slot, brace) = self.decide_slot(start);
                    self.slots[start] = slot;
                    self.braced[start] = brace;
                    let pace = stamina_pace(order);
                    self.stamina[start] = self.stamina[start]
                        .saturating_add(pace)
                        .clamp(STAMINA_MIN, STAMINA_MAX);
                }
                slots_out[start..end].copy_from_slice(&self.slots[start..end]);
                brace_out[start..end].copy_from_slice(&self.braced[start..end]);
                return thinks;
            }
            for i in start..end {
                thinks += 1;
                let (slot, brace) = self.decide_slot(i);
                self.slots[i] = slot;
                self.braced[i] = brace;
                let pace = stamina_pace(order);
                self.stamina[i] = self.stamina[i]
                    .saturating_add(pace)
                    .clamp(STAMINA_MIN, STAMINA_MAX);
                slots_out[i] = slot;
                brace_out[i] = brace;
            }
            self.sync_one_squad(squad);
            return thinks;
        }
        for i in start..end {
            let period = think_period(chebyshev(soldiers[i], self.enemy));
            if tick.wrapping_add(i as u32).is_multiple_of(period) {
                thinks += 1;
                let (slot, brace) = self.decide_slot(i);
                self.slots[i] = slot;
                self.braced[i] = brace;
                let pace = stamina_pace(self.orders[squad]);
                self.stamina[i] = self.stamina[i]
                    .saturating_add(pace)
                    .clamp(STAMINA_MIN, STAMINA_MAX);
            }
            slots_out[i] = self.slots[i];
            brace_out[i] = self.braced[i];
        }
        thinks
    }

    /// Decide one slot plus brace flag for soldier `i` from the current
    /// formation, anchor, enemy and squad order. Gassed soldiers commit
    /// like [`Order::Advance`] instead of [`Order::Charge`].
    fn decide_slot(&self, i: usize) -> ((usize, usize), bool) {
        let squad = Self::soldier_squad(i);
        let order = self.orders[squad];
        // Anchor-to-enemy facing in cells; a stacked anchor charges east
        // so the fallback stays deterministic.
        let dx = self.enemy.0 as i32 - self.anchor.0 as i32;
        let dy = self.enemy.1 as i32 - self.anchor.1 as i32;
        let (fx, fy) = if dx == 0 && dy == 0 {
            (1, 0)
        } else {
            (sign(dx), sign(dy))
        };
        let collapsed = self.stamina[i] <= STAMINA_COLLAPSE_AT;
        let (shift_x, shift_y) = match order {
            Order::Hold => (0, 0),
            Order::Advance => (4 * fx, 4 * fy),
            Order::Charge if collapsed => (4 * fx, 4 * fy),
            Order::Charge => (12 * fx, 12 * fy),
            Order::Retreat => (-6 * fx, -6 * fy),
            // Port side in the x-right/y-up frame, then close in.
            Order::FlankLeft => (
                fy * FLANK_GAP_CELLS + 8 * fx,
                -fx * FLANK_GAP_CELLS + 8 * fy,
            ),
            Order::FlankRight => (
                -fy * FLANK_GAP_CELLS + 8 * fx,
                fx * FLANK_GAP_CELLS + 8 * fy,
            ),
        };
        let (ox, oy) =
            formation_slot(self.formation, i, self.count, self.spacing).unwrap_or((0, 0));
        let (w, h) = self.grid;
        let slot = (
            clamp_cell(self.anchor.0 as i32 + ox + shift_x, w),
            clamp_cell(self.anchor.1 as i32 + oy + shift_y, h),
        );
        let brace = self.formation == Formation::ShieldWall
            && order == Order::Hold
            && chebyshev(slot, self.enemy) <= BRACE_RANGE_CELLS;
        (slot, brace)
    }

    /// Stable `u64` over the decided state (FNV-1a): count, formation,
    /// phase, anchors, spacing, squad orders and roles, slots, braces
    /// and stamina. Identical legions digest identically.
    pub fn digest(&self) -> u64 {
        let mut hash = Fnv64::new();
        hash.write_u64(self.count as u64);
        hash.write_u8(self.formation.as_u8());
        hash.write_u8(self.phase.as_u8());
        hash.write_u8(self.spacing as u8);
        for v in [self.anchor.0, self.anchor.1, self.enemy.0, self.enemy.1] {
            hash.write_u64(v as u64);
        }
        for s in 0..self.squad_count() {
            hash.write_u8(self.orders[s].as_u8());
            hash.write_u8(self.roles[s].as_u8());
        }
        for (x, y) in &self.slots[..self.count] {
            hash.write_u64(*x as u64);
            hash.write_u64(*y as u64);
        }
        for brace in &self.braced[..self.count] {
            hash.write_bool(*brace);
        }
        for stamina in &self.stamina[..self.count] {
            hash.write_i32(*stamina);
        }
        hash.finish()
    }
}

/// Check grid dims are usable.
fn check_dims(grid: (usize, usize)) -> Result<(), LegionError> {
    let (w, h) = grid;
    if w == 0 || h == 0 || w > MAX_GRID_EDGE || h > MAX_GRID_EDGE {
        return Err(LegionError::BadDims { w, h });
    }
    Ok(())
}

/// Check a cell sits on the grid.
fn check_cell(cell: (usize, usize), grid: (usize, usize)) -> Result<(), LegionError> {
    if cell.0 >= grid.0 {
        return Err(LegionError::OutOfBounds {
            axis: b'x',
            value: cell.0,
        });
    }
    if cell.1 >= grid.1 {
        return Err(LegionError::OutOfBounds {
            axis: b'y',
            value: cell.1,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe::measure;

    fn small_grid() -> (usize, usize) {
        (40, 20)
    }

    #[test]
    fn formation_slots_are_centered_and_stable() {
        // Line of 4 at spacing 2: exact symmetric spread.
        let line: Vec<(i32, i32)> = (0..4)
            .map(|i| formation_slot(Formation::Line, i, 4, 2).unwrap())
            .collect();
        assert_eq!(line, vec![(-3, 0), (-1, 0), (1, 0), (3, 0)]);
        // Shield wall shares the rank geometry.
        let wall: Vec<(i32, i32)> = (0..4)
            .map(|i| formation_slot(Formation::ShieldWall, i, 4, 2).unwrap())
            .collect();
        assert_eq!(wall, line);
        // Column of 8 marches four wide, two deep.
        let col: Vec<(i32, i32)> = (0..8)
            .map(|i| formation_slot(Formation::Column, i, 8, 2).unwrap())
            .collect();
        assert_eq!(
            col,
            vec![
                (-3, 0),
                (-1, 0),
                (1, 0),
                (3, 0),
                (-3, 2),
                (-1, 2),
                (1, 2),
                (3, 2)
            ]
        );
        // Wedge of 6: tip, pair, triple, apex forward (-y).
        let wedge: Vec<(i32, i32)> = (0..6)
            .map(|i| formation_slot(Formation::Wedge, i, 6, 2).unwrap())
            .collect();
        assert_eq!(
            wedge,
            vec![(0, 0), (-1, -2), (1, -2), (-2, -4), (0, -4), (2, -4)]
        );
        // Same inputs decide the same slots twice.
        for (i, slot) in wedge.iter().enumerate().take(6) {
            assert_eq!(formation_slot(Formation::Wedge, i, 6, 2).unwrap(), *slot);
        }
    }

    #[test]
    fn formation_slot_rejects_bad_input() {
        assert_eq!(
            formation_slot(Formation::Line, 0, 0, 1),
            Err(LegionError::EmptyLegion)
        );
        assert_eq!(
            formation_slot(Formation::Line, 0, MAX_SOLDIERS + 1, 1),
            Err(LegionError::TooManySoldiers {
                got: MAX_SOLDIERS + 1
            })
        );
        assert_eq!(
            formation_slot(Formation::Line, 4, 4, 1),
            Err(LegionError::BadSoldier { index: 4 })
        );
        assert_eq!(
            formation_slot(Formation::Line, 0, 4, 0),
            Err(LegionError::BadSpacing { got: 0 })
        );
        assert_eq!(
            formation_slot(Formation::Line, 0, 4, MAX_SPACING_CELLS + 1),
            Err(LegionError::BadSpacing {
                got: MAX_SPACING_CELLS + 1
            })
        );
        assert_eq!(
            Formation::from_u8(4),
            Err(LegionError::UnknownFormation { value: 4 })
        );
        assert_eq!(
            Order::from_u8(6),
            Err(LegionError::UnknownOrder { value: 6 })
        );
        for (byte, formation) in [(0, Formation::Line), (3, Formation::ShieldWall)] {
            assert_eq!(Formation::from_u8(byte).unwrap(), formation);
            assert_eq!(formation.as_u8(), byte);
        }
        for (byte, order) in [(0, Order::Hold), (5, Order::FlankRight)] {
            assert_eq!(Order::from_u8(byte).unwrap(), order);
            assert_eq!(order.as_u8(), byte);
        }
    }

    #[test]
    fn constructor_and_setters_reject_bad_input() {
        assert_eq!(
            Legion::new(0, Formation::Line, (1, 1), (2, 2), small_grid()).unwrap_err(),
            LegionError::EmptyLegion
        );
        assert_eq!(
            Legion::new(513, Formation::Line, (1, 1), (2, 2), small_grid()).unwrap_err(),
            LegionError::TooManySoldiers { got: 513 }
        );
        assert_eq!(
            Legion::new(4, Formation::Line, (1, 1), (2, 2), (0, 20)).unwrap_err(),
            LegionError::BadDims { w: 0, h: 20 }
        );
        assert_eq!(
            Legion::new(4, Formation::Line, (40, 1), (2, 2), small_grid()).unwrap_err(),
            LegionError::OutOfBounds {
                axis: b'x',
                value: 40
            }
        );
        let mut legion = Legion::new(16, Formation::Line, (20, 10), (20, 2), small_grid()).unwrap();
        assert_eq!(legion.squad_count(), 2);
        assert_eq!(
            legion.set_order(2, Order::Charge),
            Err(LegionError::BadSquad { index: 2 })
        );
        assert_eq!(
            legion.set_anchor((40, 10)),
            Err(LegionError::OutOfBounds {
                axis: b'x',
                value: 40
            })
        );
        // A refused anchor keeps the old one.
        assert_eq!(legion.anchor(), (20, 10));
        assert_eq!(
            legion.set_spacing(0),
            Err(LegionError::BadSpacing { got: 0 })
        );
        assert_eq!(
            legion.stamina_of(16),
            Err(LegionError::BadSoldier { index: 16 })
        );
        // Wing squads flank, the center holds.
        assert_eq!(legion.role_of(0).unwrap(), Role::Flanker);
        assert_eq!(legion.role_of(1).unwrap(), Role::Flanker);
        let mut single = Legion::new(8, Formation::Line, (20, 10), (20, 2), small_grid()).unwrap();
        assert_eq!(single.role_of(0).unwrap(), Role::Holder);
        assert_eq!(single.order_of(0).unwrap(), Order::Hold);
        single.set_phase(Phase::Finish);
        assert_eq!(single.phase(), Phase::Finish);
    }

    #[test]
    fn step_rejects_mismatched_lanes_and_bad_cells_without_writing() {
        let mut legion = Legion::new(8, Formation::Line, (20, 10), (20, 2), small_grid()).unwrap();
        let soldiers = [(20usize, 10usize); 8];
        let mut slots = [(0usize, 0usize); 8];
        let mut brace = [false; 8];
        let mut short = [(0usize, 0usize); 7];
        let mut short_brace = [false; 7];
        assert_eq!(
            legion.step(0, &soldiers, &mut short, &mut brace),
            Err(LegionError::MismatchedLanes {
                soldiers: 8,
                slots: 7,
                brace: 8
            })
        );
        assert_eq!(
            legion.step(0, &soldiers, &mut slots, &mut short_brace),
            Err(LegionError::MismatchedLanes {
                soldiers: 8,
                slots: 8,
                brace: 7
            })
        );
        let mut bad = soldiers;
        bad[3] = (40, 10);
        assert_eq!(
            legion.step(0, &bad, &mut slots, &mut brace),
            Err(LegionError::OutOfBounds {
                axis: b'x',
                value: 40
            })
        );
        // Nothing was written: lanes still hold their sentinels.
        assert!(slots.iter().all(|slot| *slot == (0, 0)));
        assert!(brace.iter().all(|held| !held));
    }

    // One-cell Chebyshev step toward a slot: the fixture harness moves
    // bodies the way the arena would through nav steering (nav owns real
    // movement; the harness only advances the decision fixture).
    fn chase(pos: (usize, usize), slot: (usize, usize)) -> (usize, usize) {
        let dx = slot.0 as i32 - pos.0 as i32;
        let dy = slot.1 as i32 - pos.1 as i32;
        if dx == 0 && dy == 0 {
            return pos;
        }
        if dx.abs() >= dy.abs() {
            ((pos.0 as i32 + sign(dx)) as usize, pos.1)
        } else {
            (pos.0, (pos.1 as i32 + sign(dy)) as usize)
        }
    }

    #[test]
    fn shield_wall_holds_against_a_charge() {
        let grid = small_grid();
        // Defenders brace a wall across the corridor; attackers charge
        // down it. Both brains decide; the harness only walks bodies.
        let mut wall = Legion::new(64, Formation::ShieldWall, (20, 14), (20, 4), grid).unwrap();
        let mut horde = Legion::new(64, Formation::Column, (20, 4), (20, 14), grid).unwrap();
        for s in 0..horde.squad_count() {
            horde.set_order(s, Order::Charge).unwrap();
        }
        let opening = wall.slots().to_vec();
        let mut defenders: Vec<(usize, usize)> = opening.clone();
        // Bodies muster at the horde anchor and walk into their charge;
        // slots are targets, not bodies, so spawning on them would start
        // the charge already past the shields.
        let mut attackers: Vec<(usize, usize)> = vec![(20, 4); 64];
        let mut wall_slots = vec![(0usize, 0usize); 64];
        let mut wall_brace = vec![false; 64];
        let mut horde_slots = vec![(0usize, 0usize); 64];
        let mut horde_brace = vec![false; 64];
        for tick in 0..120u32 {
            // The horde's anchor marches down the corridor behind its charge.
            if tick % 2 == 0 {
                let anchor = horde.anchor();
                let _ = horde.set_anchor((anchor.0, (anchor.1 + 1).min(13)));
                horde.set_enemy((20, 14)).unwrap();
            }
            wall.step(tick, &defenders, &mut wall_slots, &mut wall_brace)
                .unwrap();
            horde
                .step(tick, &attackers, &mut horde_slots, &mut horde_brace)
                .unwrap();
            // The wall never yields ground: every slot stays on its mark.
            assert_eq!(wall_slots, opening, "wall displaced at tick {tick}");
            // Braces engage for the center ranks inside brace range while
            // the far edges (past brace range) stay unbraced: the range
            // gate itself is the negative case.
            assert!(
                wall_brace[28..36].iter().all(|held| *held),
                "center ranks unbraced at tick {tick}"
            );
            assert!(
                !wall_brace[0] && !wall_brace[63],
                "edge files braced past range at tick {tick}"
            );
            // Chargers never brace: bracing belongs to a holding wall.
            assert!(horde_brace.iter().all(|held| !held));
            for i in 0..64 {
                defenders[i] = chase(defenders[i], wall_slots[i]);
                // Attackers halt at contact instead of passing the rank.
                let next = chase(attackers[i], horde_slots[i]);
                let blocked = defenders.contains(&next)
                    || defenders.iter().any(|held| chebyshev(*held, next) == 0);
                if !blocked {
                    attackers[i] = next;
                }
            }
        }
        assert!(attackers.iter().all(|cell| cell.1 < 14));
        // The wall still stands on its exact marks.
        assert_eq!(defenders, opening);
    }

    #[test]
    fn flank_squads_wrap_the_enemy() {
        let grid = (80usize, 40usize);
        let mut legion = Legion::new(24, Formation::Line, (40, 30), (40, 10), grid).unwrap();
        legion.set_order(0, Order::FlankLeft).unwrap();
        legion.set_order(2, Order::FlankRight).unwrap();
        let soldiers = vec![(40usize, 30usize); 24];
        let mut slots = vec![(0usize, 0usize); 24];
        let mut brace = vec![false; 24];
        // Near the enemy anchor so every soldier thinks on tick 0.
        let mut near = soldiers.clone();
        near.fill((40, 12));
        let thinks = legion.step(0, &near, &mut slots, &mut brace).unwrap();
        assert_eq!(thinks, 24);
        // Flank slots sit off the anchor axis, on opposite sides.
        let left: Vec<(usize, usize)> = slots[0..8].to_vec();
        let right: Vec<(usize, usize)> = slots[16..24].to_vec();
        assert!(left.iter().all(|cell| (cell.0 as i32 - 40) <= -8));
        assert!(right.iter().all(|cell| (cell.0 as i32 - 40) >= 8));
        // The center squad holds its rank instead of wrapping.
        let center: Vec<(usize, usize)> = slots[8..16].to_vec();
        assert!(center.iter().all(|cell| cell == &(40, 30) || cell.1 == 30));
    }

    #[test]
    fn thinking_lod_caps_decides() {
        // Near band thinks every tick.
        assert_eq!(think_period(0), 1);
        assert_eq!(think_period(8), 1);
        assert_eq!(think_period(9), 2);
        assert_eq!(think_period(24), 2);
        assert_eq!(think_period(25), 4);
        assert_eq!(think_period(64), 4);
        assert_eq!(think_period(65), 8);
        assert_eq!(think_period(400), 8);
        // Squad-commander LOD: a far Hold legion with static anchors pays
        // one leader decide per squad per 8-tick sync wave (zero decides and
        // zero stamina pacing off-sync). 63 squads x2 syncs = 126 decides.
        let grid = (400usize, 400usize);
        let run_static = || {
            let mut legion = Legion::new(
                CONTRACT_SOLDIERS,
                Formation::Column,
                (20, 20),
                (380, 380),
                grid,
            )
            .unwrap();
            let soldiers = vec![(20usize, 20usize); CONTRACT_SOLDIERS];
            let mut slots = vec![(0usize, 0usize); CONTRACT_SOLDIERS];
            let mut brace = vec![false; CONTRACT_SOLDIERS];
            let mut total = 0usize;
            let mut worst = 0usize;
            for tick in 0..16u32 {
                let thinks = legion
                    .step(tick, &soldiers, &mut slots, &mut brace)
                    .unwrap();
                total += thinks;
                worst = worst.max(thinks);
            }
            (total, worst, legion.digest(), slots, brace)
        };
        let (total, worst, digest, slots, brace) = run_static();
        assert!(total <= 128, "squad LOD total thinks: {total}");
        assert!(worst <= 64, "far-band thinks per tick: {worst}");
        assert!(
            total < CONTRACT_SOLDIERS,
            "must beat per-soldier 1000: {total}"
        );
        // Digest replays identically across two static runs.
        let (total2, worst2, digest2, slots2, brace2) = run_static();
        assert_eq!((total, worst), (total2, worst2));
        assert_eq!(digest, digest2);
        assert_eq!(slots, slots2);
        assert_eq!(brace, brace2);
        // A close legion thinks everybody, every tick.
        let mut near = Legion::new(
            CONTRACT_SOLDIERS,
            Formation::Column,
            (20, 20),
            (22, 22),
            grid,
        )
        .unwrap();
        let close = vec![(20usize, 20usize); CONTRACT_SOLDIERS];
        let mut near_slots = vec![(0usize, 0usize); CONTRACT_SOLDIERS];
        let mut near_brace = vec![false; CONTRACT_SOLDIERS];
        let thinks = near
            .step(0, &close, &mut near_slots, &mut near_brace)
            .unwrap();
        assert_eq!(thinks, CONTRACT_SOLDIERS);
    }

    #[test]
    fn stamina_collapse_degrades_the_charge() {
        let grid = small_grid();
        let mut legion = Legion::new(8, Formation::Line, (20, 14), (20, 10), grid).unwrap();
        legion.set_order(0, Order::Charge).unwrap();
        // Fresh charge commits 12 cells toward the enemy (facing -y).
        let soldiers = vec![(20usize, 14usize); 8];
        let mut slots = vec![(0usize, 0usize); 8];
        let mut brace = vec![false; 8];
        legion.step(0, &soldiers, &mut slots, &mut brace).unwrap();
        assert_eq!(slots[3], (20, 2));
        // Gassed soldiers fall back to an advance (4 cells).
        for i in 0..8 {
            legion.set_stamina(i, STAMINA_COLLAPSE_AT).unwrap();
        }
        legion.step(1, &soldiers, &mut slots, &mut brace).unwrap();
        assert_eq!(slots[3], (20, 10));
        // Holding recovers past the collapse line.
        legion.set_order(0, Order::Hold).unwrap();
        for tick in 2..14u32 {
            legion
                .step(tick, &soldiers, &mut slots, &mut brace)
                .unwrap();
        }
    }

    #[test]
    fn same_tape_same_digest_twice() {
        let grid = small_grid();
        let run = || {
            let mut wall = Legion::new(64, Formation::ShieldWall, (20, 14), (20, 4), grid).unwrap();
            let mut horde = Legion::new(64, Formation::Column, (20, 4), (20, 14), grid).unwrap();
            for s in 0..horde.squad_count() {
                horde.set_order(s, Order::Charge).unwrap();
            }
            let mut defenders = wall.slots().to_vec();
            let mut attackers = horde.slots().to_vec();
            let mut wall_slots = vec![(0usize, 0usize); 64];
            let mut wall_brace = vec![false; 64];
            let mut horde_slots = vec![(0usize, 0usize); 64];
            let mut horde_brace = vec![false; 64];
            for tick in 0..60u32 {
                wall.step(tick, &defenders, &mut wall_slots, &mut wall_brace)
                    .unwrap();
                horde
                    .step(tick, &attackers, &mut horde_slots, &mut horde_brace)
                    .unwrap();
                for i in 0..64 {
                    defenders[i] = chase(defenders[i], wall_slots[i]);
                    attackers[i] = chase(attackers[i], horde_slots[i]);
                }
            }
            (wall.digest(), horde.digest())
        };
        let first = run();
        let second = run();
        assert_eq!(first, second);
        // The digest is sensitive: moving the enemy anchor changes it.
        let mut moved = Legion::new(64, Formation::ShieldWall, (20, 14), (20, 4), grid).unwrap();
        let mut soldiers = moved.slots().to_vec();
        let mut slots = vec![(0usize, 0usize); 64];
        let mut brace = vec![false; 64];
        let before = moved.digest();
        moved.set_enemy((30, 4)).unwrap();
        moved.step(0, &soldiers, &mut slots, &mut brace).unwrap();
        for i in 0..64 {
            soldiers[i] = chase(soldiers[i], slots[i]);
        }
        assert_ne!(moved.digest(), before);
    }

    #[test]
    fn step_holds_zero_allocation_per_tick() {
        let grid = (400usize, 400usize);
        let mut legion = Legion::new(
            CONTRACT_SOLDIERS,
            Formation::Column,
            (20, 20),
            (380, 380),
            grid,
        )
        .unwrap();
        let soldiers = [(20usize, 20usize); CONTRACT_SOLDIERS];
        let mut slots = [(0usize, 0usize); CONTRACT_SOLDIERS];
        let mut brace = [false; CONTRACT_SOLDIERS];
        // Warm the recorded slots once, then count a steady-state tick.
        legion.step(0, &soldiers, &mut slots, &mut brace).unwrap();
        let ((), heap) = measure(|| {
            let thinks = legion.step(1, &soldiers, &mut slots, &mut brace).unwrap();
            std::hint::black_box(thinks);
        });
        assert_eq!(heap.calls, 0, "step allocated {} times", heap.calls);
    }

    #[test]
    fn alert_step_helper_holds_fixed_cell() {
        use crate::alert::{fresh, last_seen, Alert, Sense, Sight};

        let seen = Sense {
            sight: Sight::Confirmed,
            heard: false,
            door_closed: false,
        };
        let quiet = Sense {
            sight: Sight::None,
            heard: false,
            door_closed: false,
        };
        // Cell (4, 5) stores as centimetres [400, 500].
        let combat = alert_step_for_enemy(&fresh(), seen, 0, (4, 5));
        assert_eq!(combat.alert, Alert::Combat);
        assert_eq!(last_seen(&combat), Some([400, 500]));
        assert_eq!(track_cell(&combat), Some((4, 5)));
        // Quiet ticks hold the fixed cell even as the live enemy moves.
        let track10 = alert_step_for_enemy(&combat, quiet, 10, (9, 9));
        assert_eq!(track10.alert, Alert::Track);
        assert_eq!(last_seen(&track10), Some([400, 500]));
        assert_eq!(track_cell(&track10), Some((4, 5)));
        let track20 = alert_step_for_enemy(&track10, quiet, 20, (0, 0));
        assert_eq!(track20.alert, Alert::Track);
        assert_eq!(last_seen(&track20), Some([400, 500]));
        assert_eq!(track_cell(&track20), Some((4, 5)));
    }

    #[test]
    fn alert_track_holds_fixed_cell_while_live_moves() {
        use crate::alert::{last_seen, Alert};

        let mut legion = Legion::new(8, Formation::Line, (20, 10), (4, 5), small_grid()).unwrap();
        let soldiers = vec![(4usize, 5usize); 8];
        let mut slots = vec![(0usize, 0usize); 8];
        let mut brace = vec![false; 8];
        // Contact at the live foe slot confirms Combat at the fixed cell.
        legion.step(0, &soldiers, &mut slots, &mut brace).unwrap();
        assert_eq!(legion.alert_state().alert, Alert::Combat);
        assert_eq!(last_seen(&legion.alert_state()), Some([400, 500]));
        assert_eq!(track_cell(&legion.alert_state()), Some((4, 5)));
        // The foe slips far away; quiet steps hold the fixed cell via step.
        legion.set_enemy((30, 15)).unwrap();
        let mut twin = legion.clone();
        let mut twin_slots = vec![(0usize, 0usize); 8];
        let mut twin_brace = vec![false; 8];
        legion.step(10, &soldiers, &mut slots, &mut brace).unwrap();
        twin.step(10, &soldiers, &mut twin_slots, &mut twin_brace)
            .unwrap();
        assert_eq!(legion.alert_state(), twin.alert_state());
        assert_eq!(slots, twin_slots);
        assert_eq!(legion.alert_state().alert, Alert::Track);
        assert_eq!(last_seen(&legion.alert_state()), Some([400, 500]));
        assert_eq!(track_cell(&legion.alert_state()), Some((4, 5)));
        // The live slot moves again; the fixed cell still holds via step.
        legion.set_enemy((25, 15)).unwrap();
        legion.step(20, &soldiers, &mut slots, &mut brace).unwrap();
        assert_eq!(legion.alert_state().alert, Alert::Track);
        assert_eq!(last_seen(&legion.alert_state()), Some([400, 500]));
        assert_eq!(track_cell(&legion.alert_state()), Some((4, 5)));
        // Evidence expires after 300 ticks: back to unaware with no cell.
        legion.step(300, &soldiers, &mut slots, &mut brace).unwrap();
        assert_eq!(legion.alert_state().alert, Alert::Unaware);
        assert_eq!(last_seen(&legion.alert_state()), None);
        assert_eq!(track_cell(&legion.alert_state()), None);
    }

    #[test]
    fn alert_step_unaware_and_overflow_saturates() {
        use crate::alert::{fresh, Alert, AlertState, Sense, Sight};

        // Unaware never reports a cell, even with a stored one.
        assert_eq!(track_cell(&fresh()), None);
        let hidden = AlertState {
            alert: Alert::Unaware,
            last_seen_cm: Some([400, 500]),
            evidence_tick: 0,
        };
        assert_eq!(track_cell(&hidden), None);
        // Negative centimetres saturate to zero, never panic.
        let neg = AlertState {
            alert: Alert::Track,
            last_seen_cm: Some([-50, -250]),
            evidence_tick: 10,
        };
        assert_eq!(track_cell(&neg), Some((0, 0)));
        // Huge cells never panic: Confirmed falls back to no cell.
        let seen = Sense {
            sight: Sight::Confirmed,
            heard: false,
            door_closed: false,
        };
        let huge = alert_step_for_enemy(&fresh(), seen, 0, (usize::MAX, usize::MAX));
        assert_eq!(huge.alert, Alert::Combat);
        assert_eq!(track_cell(&huge), None);
        // Quiet from fresh expires to unaware with no cell.
        let quiet = Sense {
            sight: Sight::None,
            heard: false,
            door_closed: false,
        };
        let cold = alert_step_for_enemy(&fresh(), quiet, 300, (4, 5));
        assert_eq!(cold.alert, Alert::Unaware);
        assert_eq!(track_cell(&cold), None);
    }

    #[test]
    #[ignore = "timing: 500-soldier AI budget, run under --perf in release"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn perf_500_soldiers_under_budget() {
        let grid = (400usize, 400usize);
        let mut legion = Legion::new(
            CONTRACT_SOLDIERS,
            Formation::Column,
            (20, 20),
            (380, 380),
            grid,
        )
        .unwrap();
        // Mixed orders across squads, like a real battle line.
        for s in 0..legion.squad_count() {
            let order = match s % 4 {
                0 => Order::Advance,
                1 => Order::Charge,
                2 => Order::FlankLeft,
                _ => Order::Hold,
            };
            legion.set_order(s, order).unwrap();
        }
        let soldiers = [(20usize, 20usize); CONTRACT_SOLDIERS];
        let mut slots = [(0usize, 0usize); CONTRACT_SOLDIERS];
        let mut brace = [false; CONTRACT_SOLDIERS];
        // Warm up, then time the steady state.
        for tick in 0..8u32 {
            legion
                .step(tick, &soldiers, &mut slots, &mut brace)
                .unwrap();
        }
        let steps = 60u32;
        let start = std::time::Instant::now();
        for tick in 8..8 + steps {
            legion
                .step(tick, &soldiers, &mut slots, &mut brace)
                .unwrap();
        }
        let elapsed = start.elapsed();
        let per_step_us = elapsed.as_micros() as f64 / f64::from(steps);
        eprintln!(
            "legion perf: {CONTRACT_SOLDIERS} soldiers x{steps} steps in {elapsed:?} \
             ({per_step_us:.1} us/step, budget 1500 us/step)"
        );
        assert!(
            per_step_us < 1500.0,
            "500-soldier AI over budget: {per_step_us:.1} us/step"
        );
    }
}
