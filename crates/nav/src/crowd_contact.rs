//! crowd_contact: position-based contact projection for dense crowds, in
//! integer millimetres. One projection pass per call, run after ORCA and the
//! caller's integration: every disc is pushed to [`WALL_SKIN_MM`] clear of
//! every wall segment, every overlapping disc pair is pushed apart to its
//! reach plus [`PAIR_SKIN_MM`], the discs near a wall are cleared again, and
//! a pair a later correction re-opened runs through up to [`REPAIR_ROUNDS`]
//! exact sweeps. The pass writes each agent's pressure in newton-equivalents;
//! a pressure over the crush edge hands TC3 one shove band.
//! Owner: nav crate; callers pass the discs and walls of the crowd they
//! simulate (the arena passes its near-field crowd after ORCA) and read back
//! corrected positions, one pressure lane and the shoves in agent order.
//! Units: positions and radii in millimetres, pressure in newtons-equivalent.
//! Pressure is the contact compression a projection sees, scaled by
//! [`CONTACT_STIFFNESS_N_PER_MM`]: a light brush stays under the first crush
//! edge, a packed gate passes it.
//! Determinism: `i64` arithmetic and [`isqrt`] only, pairs in slot order
//! (lower slot first), walls in input order, a counting-sort grid whose cells
//! list indices ascending, no floats, no hash order, no clock. Two runs over
//! the same complete input write the same bytes.
//! Errors: typed [`ContactError`]; a bad batch writes nothing.
//! Budgets: a step is O(discs + discs x walls) with a grid-accelerated pair
//! scan and wall frames cached once per step, and the scratch lanes keep
//! their capacity, so a warm step allocates nothing. The sweep is
//! Gauss-Seidel, not a converged solve: a pair projected early can be
//! re-opened by a later neighbour correction, so the crowd is driven a few
//! millimetres per tick and a step that ends over [`SETTLE_MM`] runs the
//! closing repair.
//! Inputs are bounded by `|position| <= 2^20 mm` and `0 < radius <= 4096 mm`,
//! so every product fits `i64`.
//! IDEA(Weiss et al., Computers and Graphics 78 (2019),
//! DOI 10.1016/j.cag.2018.10.008): agents as discs with contact constraints
//! solved by projection, the same loop as the cloth. The paper is read for
//! the idea; this is an integer port with no ported code.

use std::fmt;

use engine_core::fixed::isqrt;

/// Unit scale of direction vectors (Q16).
const ONE: i64 = 1 << 16;
/// Most discs one step accepts.
pub const MAX_DISCS: usize = 4096;
/// Largest body radius one step accepts, in millimetres.
pub const MAX_RADIUS_MM: i32 = 4096;
/// Largest accepted coordinate magnitude, in millimetres (2^20).
pub const MAX_COORD_MM: i32 = 1 << 20;
/// Largest grid cell count one step builds; wider crowds refuse.
pub const MAX_CELLS: usize = 1 << 18;
/// Spring scale of body compression: newtons-equivalent per millimetre of
/// overlap. Chosen so a brush (a millimetre or two) stays under the first
/// crush edge and a packed gate (several contacts) passes it.
pub const CONTACT_STIFFNESS_N_PER_MM: i32 = 32;

/// Skin a pair projection leaves between two discs that were overlapping,
/// in millimetres. A later correction in the same sweep can close the gap
/// again; the skin absorbs that residual so one pass clears the 5 mm bar.
const PAIR_SKIN_MM: i64 = 2;

/// Overlap a settled crowd keeps, in millimetres. A step that ends over this
/// bar runs the closing repair.
const SETTLE_MM: i32 = 4;

/// Repair rounds the closing repair may run. Each round clears the walls and
/// runs one exact pair sweep (no skin, no pressure), so a jammed throat
/// cannot report a residual over the settle bar.
const REPAIR_ROUNDS: usize = 4;

/// Extra clearance that marks a disc as near a wall. The closing wall sweep
/// re-checks only these discs, so a pair push cannot leave a disc inside a
/// wall without paying for a scan of the whole crowd.
const NEAR_WALL_MM: i64 = 16;

/// Collision slop a wall projection leaves, in millimetres. A crowded throat
/// can press a disc into a wall between two pair corrections; the slop keeps
/// the reported penetration under the settle bar without letting the disc
/// touch the wall.
const WALL_SKIN_MM: i64 = 2;

/// One row of the crush shove table: the pressure edge in
/// newtons-equivalent and the impulse the band hands the balance, Q8 units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CrushRow {
    /// Pressure edge: an agent at or over this pressure lands the band.
    pub over_pressure_n: i32,
    /// Impulse the band hands TC3, Q8 impulse units (the arena's balance
    /// bands: block, parry, hit, fall).
    pub impulse_q8: u16,
}

/// The crush table: pressure bands, smallest edge first. An agent under the
/// first edge lands no shove; one past the last edge lands the last row.
/// The Q8 values are the arena's four balance bands, so a crush reads like a
/// landed blow: 128 brace, 256 stagger, 384 hit, 640 fall.
pub const CRUSH_SHOVE_TABLE: [CrushRow; 4] = [
    CrushRow {
        over_pressure_n: 320,
        impulse_q8: 128,
    },
    CrushRow {
        over_pressure_n: 640,
        impulse_q8: 256,
    },
    CrushRow {
        over_pressure_n: 960,
        impulse_q8: 384,
    },
    CrushRow {
        over_pressure_n: 1600,
        impulse_q8: 640,
    },
];

/// The shove a crushed agent hands TC3: which agent and which Q8 impulse.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shove {
    /// Index of the crushed agent in the step's disc lane.
    pub agent: u32,
    /// Impulse for the balance, Q8 units; never zero in a step's output.
    pub impulse_q8: u16,
}

/// One crowd disc: centre on the XZ plane and body radius, in millimetres.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Disc {
    /// Centre in millimetres.
    pub pos: [i32; 2],
    /// Body radius in millimetres.
    pub radius_mm: i32,
}

/// One wall segment: a disc keeps its radius clear of the segment. The wall
/// has no free side; a disc on either side is pushed to that side.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wall {
    /// First endpoint in millimetres.
    pub a: [i32; 2],
    /// Second endpoint in millimetres.
    pub b: [i32; 2],
}

/// What one projection pass left behind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StepReport {
    /// Shoves written to the caller's lane, in agent order.
    pub shoves: usize,
    /// Largest disc-pair overlap still present after the pass, in mm.
    pub max_overlap_mm: i32,
    /// Largest disc-to-wall penetration still present after the pass, in mm.
    pub max_wall_penetration_mm: i32,
}

/// Typed step failures; a bad batch writes no position, pressure or shove.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ContactError {
    /// The pressure lane length differs from the disc count.
    MismatchedLanes {
        /// Discs offered.
        discs: usize,
        /// Pressure slots offered.
        pressure: usize,
    },
    /// The shove lane is smaller than the disc count, so not every crushed
    /// agent could be written.
    ShoveLanes {
        /// Discs offered.
        discs: usize,
        /// Shove slots offered.
        shoves: usize,
    },
    /// More discs than [`MAX_DISCS`].
    TooManyDiscs {
        /// Discs offered.
        discs: usize,
    },
    /// A disc has a non-positive or oversized radius.
    BadRadius {
        /// Index of the disc.
        disc: usize,
    },
    /// A disc centre is outside [`MAX_COORD_MM`].
    BadPosition {
        /// Index of the disc.
        disc: usize,
    },
    /// A wall endpoint is outside [`MAX_COORD_MM`] or the wall is a point.
    BadWall {
        /// Index of the wall.
        wall: usize,
    },
    /// The crowd spans more than [`MAX_CELLS`] grid cells.
    WorldTooWide,
}

impl fmt::Display for ContactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MismatchedLanes { discs, pressure } => {
                write!(
                    f,
                    "crowd_contact: {discs} discs but {pressure} pressure slots"
                )
            }
            Self::ShoveLanes { discs, shoves } => {
                write!(f, "crowd_contact: {discs} discs but {shoves} shove slots")
            }
            Self::TooManyDiscs { discs } => {
                write!(f, "crowd_contact: {discs} discs over the {MAX_DISCS} bound")
            }
            Self::BadRadius { disc } => {
                write!(f, "crowd_contact: disc {disc} radius is out of range")
            }
            Self::BadPosition { disc } => {
                write!(f, "crowd_contact: disc {disc} centre is out of range")
            }
            Self::BadWall { wall } => {
                write!(f, "crowd_contact: wall {wall} is a point or out of range")
            }
            Self::WorldTooWide => {
                write!(f, "crowd_contact: crowd spans more than {MAX_CELLS} cells")
            }
        }
    }
}

impl std::error::Error for ContactError {}

/// Reusable contact stepper: owns the broad-phase grid and its scratch lanes.
#[derive(Clone, Debug, Default)]
pub struct Contact {
    /// Counting-sort cell starts, one past the last cell.
    starts: Vec<u32>,
    /// Disc indices grouped by cell, ascending inside each cell.
    items: Vec<u32>,
    /// Cell index of each disc at broad-phase build time.
    cells: Vec<u32>,
    /// Wall frame of each wall for this step (segment unit and length).
    frames: Vec<StepWall>,
    /// Discs a pair push can have pressed into a wall this step.
    near: Vec<u32>,
    /// Pressure lane the closing repair reuses, so a repair sweep allocates
    /// nothing and adds no pressure to the caller's lane.
    scratch: Vec<i32>,
}

/// One wall prepared for a step: origin, Q16 unit direction, length and the
/// axis-aligned bounds the near test reads.
#[derive(Clone, Copy, Debug)]
struct StepWall {
    /// First endpoint.
    a: [i64; 2],
    /// Q16 unit direction from `a` to the second endpoint.
    unit: [i64; 2],
    /// Segment length in millimetres.
    length: i64,
    /// Lower corner of the segment bounds.
    min: [i64; 2],
    /// Upper corner of the segment bounds.
    max: [i64; 2],
}

/// Grid geometry of one build.
#[derive(Clone, Copy)]
struct Grid {
    min: [i64; 2],
    cell: i64,
    cols: usize,
    rows: usize,
}

impl Grid {
    /// Cell of a position, in `(x, y)` cell coordinates.
    fn cell_of(&self, pos: [i32; 2]) -> (usize, usize) {
        let at = |axis: usize| {
            let offset = i64::from(pos[axis]) - self.min[axis];
            usize::try_from(offset / self.cell).unwrap_or(0)
        };
        (at(0), at(1))
    }
}

impl Contact {
    /// Empty stepper; the first step sizes the lanes.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Run one projection pass and write pressure and shoves.
    ///
    /// Every wall is projected for every disc first; then every pair with a
    /// higher slot is projected once, in lower-slot order, leaving
    /// [`PAIR_SKIN_MM`] of clearance; then the discs near a wall are cleared
    /// once more, so a pair push cannot leave a disc inside a wall. A crowded
    /// throat can still leave a pair over [`SETTLE_MM`] because a later
    /// correction re-opens it, so the pass closes with up to
    /// [`REPAIR_ROUNDS`] exact pair sweeps that add no pressure. Positions
    /// are corrected in place; `pressure[i]` is the compression agent `i` saw
    /// (wall penetration at the pre-pass positions plus each pair's overlap
    /// when it is projected), and `shoves` receives one entry per crushed
    /// agent in ascending agent order.
    ///
    /// # Errors
    /// [`ContactError`]: a lane length differs, a disc or wall is out of
    /// range, or the crowd spans too many grid cells. A bad batch writes
    /// nothing.
    pub fn step(
        &mut self,
        discs: &mut [Disc],
        walls: &[Wall],
        pressure: &mut [i32],
        shoves: &mut [Shove],
    ) -> Result<StepReport, ContactError> {
        validate(discs, walls, pressure, shoves)?;
        let grid = self.build_grid(discs)?;
        self.frames.clear();
        for wall in walls {
            self.frames.push(step_wall(wall));
        }
        self.near.clear();
        pressure.fill(0);
        for (index, (disc, seen)) in discs.iter_mut().zip(pressure.iter_mut()).enumerate() {
            let mut near = false;
            for frame in &self.frames {
                near |= project_wall(disc, frame, seen);
            }
            if near {
                self.near.push(u32::try_from(index).unwrap_or(u32::MAX));
            }
        }
        Self::sweep_pairs(
            &self.starts,
            &self.items,
            &self.cells,
            discs,
            &grid,
            pressure,
            PAIR_SKIN_MM,
        );
        // Walls are hard: a pair push can press a disc into one, so the pass
        // ends by clearing the walls of the discs a push could have reached.
        // This can re-open a pair by at most the wall push, which the report
        // then measures.
        for &index in &self.near {
            let slot = usize::try_from(index).unwrap_or(0);
            for frame in &self.frames {
                push_clear(&mut discs[slot], frame);
            }
        }
        let mut written = 0;
        for (agent, seen) in pressure.iter().enumerate() {
            let Some(impulse_q8) = crush_impulse(*seen) else {
                continue;
            };
            shoves[written] = Shove {
                agent: u32::try_from(agent).unwrap_or(u32::MAX),
                impulse_q8,
            };
            written += 1;
        }
        let (max_overlap_mm, max_wall_penetration_mm) = self.close(discs)?;
        Ok(StepReport {
            shoves: written,
            max_overlap_mm,
            max_wall_penetration_mm,
        })
    }

    /// Counting-sort the discs into square cells of four largest radii. Four
    /// radii leave room for the corrections of one pass, so a pair pushed
    /// together after the build is still inside the next query window.
    fn build_grid(&mut self, discs: &[Disc]) -> Result<Grid, ContactError> {
        let mut min = [i64::MAX; 2];
        let mut max = [i64::MIN; 2];
        for disc in discs {
            for axis in 0..2 {
                min[axis] = min[axis].min(i64::from(disc.pos[axis]));
                max[axis] = max[axis].max(i64::from(disc.pos[axis]));
            }
        }
        if discs.is_empty() {
            min = [0, 0];
            max = [0, 0];
        }
        let largest = discs.iter().map(|disc| disc.radius_mm).max().unwrap_or(1);
        let cell = i64::from(largest).saturating_mul(4).max(4);
        let cols = usize::try_from((max[0] - min[0]) / cell + 1).unwrap_or(usize::MAX);
        let rows = usize::try_from((max[1] - min[1]) / cell + 1).unwrap_or(usize::MAX);
        if cols.saturating_mul(rows) > MAX_CELLS {
            return Err(ContactError::WorldTooWide);
        }
        let grid = Grid {
            min,
            cell,
            cols,
            rows,
        };
        self.starts.clear();
        self.starts.resize(cols * rows + 1, 0);
        for disc in discs {
            let (cx, cy) = grid.cell_of(disc.pos);
            self.starts[cy * cols + cx + 1] += 1;
        }
        for cell in 0..cols * rows {
            self.starts[cell + 1] += self.starts[cell];
        }
        self.items.clear();
        self.items.resize(discs.len(), 0);
        self.cells.clear();
        self.cells.resize(discs.len(), 0);
        self.scratch.clear();
        self.scratch.resize(discs.len(), 0);
        for (index, disc) in discs.iter().enumerate() {
            let (cx, cy) = grid.cell_of(disc.pos);
            let cell = cy * cols + cx;
            let cursor = &mut self.starts[cell];
            self.items[usize::try_from(*cursor).unwrap_or(0)] =
                u32::try_from(index).unwrap_or(u32::MAX);
            *cursor += 1;
            self.cells[index] = u32::try_from(cell).unwrap_or(u32::MAX);
        }
        self.starts.rotate_right(1);
        self.starts[0] = 0;
        Ok(grid)
    }

    /// One pair sweep in slot order: every disc against the discs in its
    /// window with a higher slot. `pressure` may be the scratch lane when the
    /// caller does not want the compression counted, as the closing repair
    /// does.
    fn sweep_pairs(
        starts: &[u32],
        items: &[u32],
        cells: &[u32],
        discs: &mut [Disc],
        grid: &Grid,
        pressure: &mut [i32],
        skin: i64,
    ) {
        for i in 0..discs.len() {
            let (before, after) = discs.split_at_mut(i + 1);
            let (pressure_before, pressure_after) = pressure.split_at_mut(i + 1);
            let disc = &mut before[i];
            let seen = &mut pressure_before[i];
            let cell = cells[i];
            Self::for_each_in_window(starts, items, grid, cell, |j| {
                if j <= i {
                    return;
                }
                let other = &mut after[j - i - 1];
                let other_pressure = &mut pressure_after[j - i - 1];
                project_pair(disc, other, seen, other_pressure, skin);
            });
        }
    }

    /// Call `visit` for every disc in the cell window around `cell`.
    fn for_each_in_window(
        starts: &[u32],
        items: &[u32],
        grid: &Grid,
        cell: u32,
        mut visit: impl FnMut(usize),
    ) {
        let cell = usize::try_from(cell).unwrap_or(0);
        let cx = cell % grid.cols;
        let cy = cell / grid.cols;
        for y in cy.saturating_sub(1)..=(cy + 1).min(grid.rows - 1) {
            for x in cx.saturating_sub(1)..=(cx + 1).min(grid.cols - 1) {
                let at = y * grid.cols + x;
                let begin = usize::try_from(starts[at]).unwrap_or(0);
                let end = usize::try_from(starts[at + 1]).unwrap_or(0);
                for &item in &items[begin..end] {
                    visit(usize::try_from(item).unwrap_or(0));
                }
            }
        }
    }

    /// Largest pair overlap and wall penetration on the current positions.
    /// The grid is rebuilt on those positions, so the reading is exact for
    /// every touching pair. The grid is returned for the next sweep.
    fn scan(&mut self, discs: &[Disc]) -> Result<(i32, i32, Grid), ContactError> {
        let grid = self.build_grid(discs)?;
        let mut overlap = 0;
        let mut penetration = 0;
        for i in 0..discs.len() {
            let cell = self.cells[i];
            let first = &discs[i];
            Self::for_each_in_window(&self.starts, &self.items, &grid, cell, |j| {
                if j <= i {
                    return;
                }
                overlap = overlap.max(pair_overlap(first, &discs[j]));
            });
            for wall in &self.frames {
                penetration = penetration.max(wall_penetration(&discs[i], wall));
            }
        }
        Ok((overlap, penetration, grid))
    }

    /// Bring the pass under [`SETTLE_MM`]. A crowded throat can leave a pair
    /// over the bar because a later correction re-opens it, so the closing
    /// repair clears the walls, runs one more exact pair sweep (no skin, no
    /// pressure) and measures again, at most [`REPAIR_ROUNDS`] times. The
    /// returned reading is the true one after the last round.
    fn close(&mut self, discs: &mut [Disc]) -> Result<(i32, i32), ContactError> {
        let (mut overlap, mut penetration, mut grid) = self.scan(discs)?;
        let mut rounds = 0;
        while (overlap > SETTLE_MM || penetration > SETTLE_MM) && rounds < REPAIR_ROUNDS {
            rounds += 1;
            for disc in discs.iter_mut() {
                for frame in &self.frames {
                    push_clear(disc, frame);
                }
            }
            Self::sweep_pairs(
                &self.starts,
                &self.items,
                &self.cells,
                discs,
                &grid,
                &mut self.scratch,
                0,
            );
            (overlap, penetration, grid) = self.scan(discs)?;
        }
        Ok((overlap, penetration))
    }
}

/// Validate the whole batch before any lane is written.
fn validate(
    discs: &[Disc],
    walls: &[Wall],
    pressure: &[i32],
    shoves: &[Shove],
) -> Result<(), ContactError> {
    if discs.len() > MAX_DISCS {
        return Err(ContactError::TooManyDiscs { discs: discs.len() });
    }
    if pressure.len() != discs.len() {
        return Err(ContactError::MismatchedLanes {
            discs: discs.len(),
            pressure: pressure.len(),
        });
    }
    if shoves.len() < discs.len() {
        return Err(ContactError::ShoveLanes {
            discs: discs.len(),
            shoves: shoves.len(),
        });
    }
    for (index, disc) in discs.iter().enumerate() {
        if disc.radius_mm <= 0 || disc.radius_mm > MAX_RADIUS_MM {
            return Err(ContactError::BadRadius { disc: index });
        }
        if !inside(disc.pos) {
            return Err(ContactError::BadPosition { disc: index });
        }
    }
    for (index, wall) in walls.iter().enumerate() {
        if wall.a == wall.b || !inside(wall.a) || !inside(wall.b) {
            return Err(ContactError::BadWall { wall: index });
        }
    }
    Ok(())
}

/// True when a point is inside the accepted coordinate bound.
fn inside(point: [i32; 2]) -> bool {
    let limit = i64::from(MAX_COORD_MM);
    point.iter().all(|value| i64::from(*value).abs() <= limit)
}

/// Impulse a pressure hands the balance, or `None` under the first crush edge.
#[must_use]
pub fn crush_impulse(pressure_n: i32) -> Option<u16> {
    let mut impulse = None;
    for row in CRUSH_SHOVE_TABLE {
        if pressure_n >= row.over_pressure_n {
            impulse = Some(row.impulse_q8);
        }
    }
    impulse
}

/// Push two overlapping discs half the separation apart and add the
/// compression each one sees to its pressure lane. `skin` extra clearance is
/// left between them: the sweep uses [`PAIR_SKIN_MM`] so a later correction
/// does not push the pair back over the crush bar, the closing repair uses
/// zero so it injects no energy.
fn project_pair(
    first: &mut Disc,
    second: &mut Disc,
    first_pressure: &mut i32,
    second_pressure: &mut i32,
    skin: i64,
) {
    let dx = i64::from(second.pos[0]) - i64::from(first.pos[0]);
    let dy = i64::from(second.pos[1]) - i64::from(first.pos[1]);
    let reach = i64::from(first.radius_mm) + i64::from(second.radius_mm);
    let distance_sq = dx * dx + dy * dy;
    if distance_sq >= reach * reach {
        return;
    }
    let distance = root(distance_sq);
    let compression = reach - distance;
    let force = narrow(compression.saturating_mul(i64::from(CONTACT_STIFFNESS_N_PER_MM)));
    *first_pressure = first_pressure.saturating_add(force);
    *second_pressure = second_pressure.saturating_add(force);
    let push = compression + skin;
    if distance == 0 {
        // Coincident centres: the higher slot yields along +x, the lower
        // slot takes the rest along -x, so the pair separates the same way
        // on every run.
        let half = push / 2;
        shift(first, -half, 0);
        shift(second, half, 0);
        return;
    }
    let unit_x = dx * ONE / distance;
    let unit_y = dy * ONE / distance;
    let half = push / 2;
    let rest = push - half;
    shift(first, -mul_q16(half, unit_x), -mul_q16(half, unit_y));
    shift(second, mul_q16(rest, unit_x), mul_q16(rest, unit_y));
}

/// Build the step frame of a validated wall.
fn step_wall(wall: &Wall) -> StepWall {
    let a = [i64::from(wall.a[0]), i64::from(wall.a[1])];
    let b = [i64::from(wall.b[0]), i64::from(wall.b[1])];
    let ex = b[0] - a[0];
    let ey = b[1] - a[1];
    let length = root(ex * ex + ey * ey);
    StepWall {
        a,
        unit: [ex * ONE / length, ey * ONE / length],
        length,
        min: [a[0].min(b[0]), a[1].min(b[1])],
        max: [a[0].max(b[0]), a[1].max(b[1])],
    }
}

/// True when a disc centre is within `reach` of the wall's bounds. The bounds
/// are conservative, so a false answer is exact and a true one still gets the
/// full segment test.
fn near_wall(disc: &Disc, wall: &StepWall, reach: i64) -> bool {
    let x = i64::from(disc.pos[0]);
    let y = i64::from(disc.pos[1]);
    x >= wall.min[0] - reach
        && x <= wall.max[0] + reach
        && y >= wall.min[1] - reach
        && y <= wall.max[1] + reach
}

/// Push a disc its radius plus [`WALL_SKIN_MM`] clear of a wall segment, add
/// the true penetration it sees to its pressure lane, and report whether the
/// disc was near the wall.
fn project_wall(disc: &mut Disc, wall: &StepWall, pressure: &mut i32) -> bool {
    let radius = i64::from(disc.radius_mm);
    let target = radius + WALL_SKIN_MM;
    if !near_wall(disc, wall, target + NEAR_WALL_MM) {
        return false;
    }
    let (point, unit) = wall_frame(disc, wall);
    if point.distance >= radius {
        return true;
    }
    let penetration = radius - point.distance;
    let force = narrow(penetration.saturating_mul(i64::from(CONTACT_STIFFNESS_N_PER_MM)));
    *pressure = pressure.saturating_add(force);
    push_out(disc, &point, unit, target);
    true
}

/// Push a disc out of one wall without charging pressure.
fn push_clear(disc: &mut Disc, wall: &StepWall) {
    let radius = i64::from(disc.radius_mm);
    let target = radius + WALL_SKIN_MM;
    if !near_wall(disc, wall, target + NEAR_WALL_MM) {
        return;
    }
    let (point, unit) = wall_frame(disc, wall);
    if point.distance < radius {
        push_out(disc, &point, unit, target);
    }
}

/// Set a disc one radius clear of a wall frame.
fn push_out(disc: &mut Disc, point: &FramePoint, unit: [i64; 2], clearance: i64) {
    let normal = if point.distance == 0 {
        [-unit[1], unit[0]]
    } else {
        point.normal
    };
    let x = point.at[0] + mul_q16(clearance, normal[0]);
    let y = point.at[1] + mul_q16(clearance, normal[1]);
    disc.pos = [clamp_coord(x), clamp_coord(y)];
}

/// Wall frame at the disc: the projection point with its outward normal, and
/// the segment's Q16 unit direction.
fn wall_frame(disc: &Disc, wall: &StepWall) -> (FramePoint, [i64; 2]) {
    let px = i64::from(disc.pos[0]) - wall.a[0];
    let py = i64::from(disc.pos[1]) - wall.a[1];
    let along = (px * wall.unit[0] + py * wall.unit[1]) / ONE;
    let clamped = along.clamp(0, wall.length);
    let at = [
        wall.a[0] + mul_q16(clamped, wall.unit[0]),
        wall.a[1] + mul_q16(clamped, wall.unit[1]),
    ];
    let rx = i64::from(disc.pos[0]) - at[0];
    let ry = i64::from(disc.pos[1]) - at[1];
    let distance_sq = rx * rx + ry * ry;
    let distance = root(distance_sq);
    let normal = if distance == 0 {
        [0, 0]
    } else {
        [rx * ONE / distance, ry * ONE / distance]
    };
    (
        FramePoint {
            at,
            normal,
            distance,
        },
        wall.unit,
    )
}

/// Projection point of a disc on a wall: the point, the outward normal and
/// the distance from the disc centre.
#[derive(Clone, Copy)]
struct FramePoint {
    at: [i64; 2],
    normal: [i64; 2],
    distance: i64,
}

/// Disc-pair overlap in millimetres, zero when apart.
fn pair_overlap(first: &Disc, second: &Disc) -> i32 {
    let dx = i64::from(second.pos[0]) - i64::from(first.pos[0]);
    let dy = i64::from(second.pos[1]) - i64::from(first.pos[1]);
    let reach = i64::from(first.radius_mm) + i64::from(second.radius_mm);
    let distance_sq = dx * dx + dy * dy;
    if distance_sq >= reach * reach {
        return 0;
    }
    narrow(reach - root(distance_sq))
}

/// Disc-to-wall penetration in millimetres, zero when clear.
fn wall_penetration(disc: &Disc, wall: &StepWall) -> i32 {
    let (point, _) = wall_frame(disc, wall);
    narrow(i64::from(disc.radius_mm) - point.distance)
}

/// Move a disc by an integer millimetre delta, clamped to the world bound.
fn shift(disc: &mut Disc, dx: i64, dy: i64) {
    disc.pos[0] = clamp_coord(i64::from(disc.pos[0]) + dx);
    disc.pos[1] = clamp_coord(i64::from(disc.pos[1]) + dy);
}

/// Clamp a coordinate to [`MAX_COORD_MM`].
fn clamp_coord(value: i64) -> i32 {
    let limit = i64::from(MAX_COORD_MM);
    narrow(value.clamp(-limit, limit))
}

/// Floor square root of a non-negative `i64`, as `i64`.
fn root(value: i64) -> i64 {
    let non_negative = u128::try_from(value).unwrap_or(0);
    i64::try_from(isqrt(non_negative)).unwrap_or(i64::MAX)
}

/// Narrow an `i64` to `i32`, saturating instead of wrapping.
fn narrow(value: i64) -> i32 {
    let window = value.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
    i32::try_from(window).unwrap_or(0)
}

/// Multiply a length by a Q16 factor, rounded half away from zero.
fn mul_q16(value: i64, factor: i64) -> i64 {
    let scaled = value * factor;
    if scaled >= 0 {
        (scaled + ONE / 2) / ONE
    } else {
        (scaled - ONE / 2) / ONE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Body radius of the gate crowd, mm.
    const RADIUS_MM: i32 = 160;
    /// Half the 2 m gate opening, mm.
    const GATE_HALF_MM: i32 = 1_000;
    /// Fence half length, mm.
    const FENCE_MM: i32 = 8_000;
    /// Funnel length upstream of the gate, mm.
    const FUNNEL_MM: i32 = 12_000;
    /// Half width of the approach channel, mm; one more lane than the gate.
    const FUNNEL_HALF_MM: i32 = 1_075;
    /// Crowd grid spacing, mm.
    const SPACING_MM: i32 = 350;
    /// Drive per tick toward the gate, mm.
    const DRIVE_MM: i64 = 2;
    /// Downstream distance that counts as through the gate, mm.
    const PASS_X_MM: i32 = 2_000;
    /// Most ticks the gate proof may take.
    const MAX_TICKS: usize = 30_000;
    /// Overlap the pass must keep, mm.
    const MAX_RESIDUAL_MM: i32 = 5;

    /// The fence with a 2 m gate at the origin, plus the funnel that gathers
    /// the crowd into it from upstream.
    fn gate_walls() -> [Wall; 4] {
        [
            Wall {
                a: [-FUNNEL_MM, -FUNNEL_HALF_MM],
                b: [0, -GATE_HALF_MM],
            },
            Wall {
                a: [-FUNNEL_MM, FUNNEL_HALF_MM],
                b: [0, GATE_HALF_MM],
            },
            Wall {
                a: [0, -FENCE_MM],
                b: [0, -GATE_HALF_MM],
            },
            Wall {
                a: [0, GATE_HALF_MM],
                b: [0, FENCE_MM],
            },
        ]
    }

    /// A six-lane crowd in an approach channel thinned evenly to `count`
    /// discs: the sixth lane has to merge into the five the gate passes, and
    /// that merge is where the crowd presses hardest.
    fn gate_crowd(count: usize) -> Vec<Disc> {
        let mut candidates = Vec::new();
        for column in 0..43 {
            let x = -14_950 + column * SPACING_MM;
            for row in 0..6 {
                let y: i32 = -875 + row * SPACING_MM;
                candidates.push(Disc {
                    pos: [x, y],
                    radius_mm: RADIUS_MM,
                });
            }
        }
        assert!(
            candidates.len() >= count,
            "only {} candidates for {count} discs",
            candidates.len()
        );
        let mut discs = Vec::with_capacity(count);
        for index in 0..count {
            let pick = index * candidates.len() / count;
            discs.push(candidates[pick]);
        }
        discs
    }

    /// Drive one disc straight at the gate; the funnel walls turn the side
    /// discs inward.
    fn drive(disc: &mut Disc) {
        disc.pos[0] += i32::try_from(DRIVE_MM).unwrap();
    }

    /// One warmed gate step: drive, then project.
    fn step_gate(
        contact: &mut Contact,
        discs: &mut [Disc],
        pressure: &mut [i32],
        shoves: &mut [Shove],
    ) -> StepReport {
        for disc in discs.iter_mut() {
            drive(disc);
        }
        contact
            .step(discs, &gate_walls(), pressure, shoves)
            .expect("gate batch is valid")
    }

    /// Bytes of every lane, so two runs compare exactly.
    fn digest(
        discs: &[Disc],
        pressure: &[i32],
        shoves: &[Shove],
        reports: &[StepReport],
    ) -> Vec<u8> {
        let mut bytes = Vec::new();
        for disc in discs {
            for axis in disc.pos {
                bytes.extend_from_slice(&axis.to_le_bytes());
            }
            bytes.extend_from_slice(&disc.radius_mm.to_le_bytes());
        }
        for seen in pressure {
            bytes.extend_from_slice(&seen.to_le_bytes());
        }
        for shove in shoves {
            bytes.extend_from_slice(&shove.agent.to_le_bytes());
            bytes.extend_from_slice(&shove.impulse_q8.to_le_bytes());
        }
        for report in reports {
            bytes.extend_from_slice(&u32::try_from(report.shoves).unwrap().to_le_bytes());
            bytes.extend_from_slice(&report.max_overlap_mm.to_le_bytes());
            bytes.extend_from_slice(&report.max_wall_penetration_mm.to_le_bytes());
        }
        bytes
    }

    /// A 512-disc block at a chosen spacing, for the step-cost and
    /// allocation proofs.
    fn block_512(spacing_mm: i32) -> Vec<Disc> {
        let mut discs = Vec::with_capacity(512);
        for row in 0..16 {
            for column in 0..32 {
                discs.push(Disc {
                    pos: [1_000 + column * spacing_mm, row * spacing_mm],
                    radius_mm: RADIUS_MM,
                });
            }
        }
        discs
    }

    #[test]
    fn crowd_contact_gate_256_keep_overlap_under_five_mm() {
        let mut discs = gate_crowd(256);
        let mut contact = Contact::new();
        let mut pressure = vec![0; discs.len()];
        let mut shoves = vec![Shove::default(); discs.len()];
        let mut peak = (0i32, [0i32; 2]);
        let mut ticks = 0usize;
        while ticks < MAX_TICKS {
            let report = step_gate(&mut contact, &mut discs, &mut pressure, &mut shoves);
            assert!(
                report.max_overlap_mm <= MAX_RESIDUAL_MM,
                "tick {ticks}: pair overlap {} mm",
                report.max_overlap_mm
            );
            assert!(
                report.max_wall_penetration_mm <= MAX_RESIDUAL_MM,
                "tick {ticks}: wall penetration {} mm",
                report.max_wall_penetration_mm
            );
            for (index, seen) in pressure.iter().enumerate() {
                if *seen > peak.0 {
                    peak = (*seen, discs[index].pos);
                }
            }
            ticks += 1;
            if discs.iter().all(|disc| disc.pos[0] > PASS_X_MM) {
                break;
            }
        }
        let passed = discs.iter().filter(|disc| disc.pos[0] > PASS_X_MM).count();
        println!(
            "crowd_contact gate: {passed} discs through in {ticks} ticks, peak pressure {} N at x = {}",
            peak.0, peak.1[0]
        );
        assert_eq!(
            passed,
            discs.len(),
            "only {passed} of {} discs passed in {ticks} ticks",
            discs.len()
        );
        assert!(
            peak.0 >= CONTACT_STIFFNESS_N_PER_MM,
            "peak pressure {} N never pressed",
            peak.0
        );
        assert!(
            (-1_500..=500).contains(&peak.1[0]),
            "peak pressure {} N at x = {}, not the gate",
            peak.0,
            peak.1[0]
        );
    }

    #[test]
    fn crowd_contact_two_runs_write_identical_bytes() {
        let run = || {
            let mut discs = gate_crowd(256);
            let mut contact = Contact::new();
            let mut pressure = vec![0; discs.len()];
            let mut shoves = vec![Shove::default(); discs.len()];
            let mut reports = Vec::with_capacity(1_200);
            for _ in 0..1_200 {
                let report = step_gate(&mut contact, &mut discs, &mut pressure, &mut shoves);
                reports.push(report);
            }
            digest(&discs, &pressure, &shoves, &reports)
        };
        let first = run();
        let second = run();
        assert_eq!(first, second, "two runs wrote different bytes");
        assert!(!first.is_empty());
    }

    #[test]
    fn crowd_contact_crush_hands_tc3_shoves_in_agent_order() {
        let mut discs = vec![
            Disc {
                pos: [0, 0],
                radius_mm: 300,
            },
            Disc {
                pos: [590, 0],
                radius_mm: 300,
            },
            Disc {
                pos: [-590, 0],
                radius_mm: 300,
            },
        ];
        let mut pressure = vec![0; discs.len()];
        let mut shoves = vec![Shove::default(); discs.len()];
        let report = Contact::new()
            .step(&mut discs, &[], &mut pressure, &mut shoves)
            .expect("valid batch");
        assert_eq!(report.shoves, 3, "all three discs are crushed");
        let handed = &shoves[..report.shoves];
        assert!(
            handed.windows(2).all(|pair| pair[0].agent < pair[1].agent),
            "shoves arrive in agent order"
        );
        for shove in handed {
            let index = usize::try_from(shove.agent).unwrap();
            assert_eq!(Some(shove.impulse_q8), crush_impulse(pressure[index]));
            assert!(shove.impulse_q8 >= CRUSH_SHOVE_TABLE[0].impulse_q8);
        }
        assert_eq!(crush_impulse(0), None);
        assert_eq!(crush_impulse(319), None);
        assert_eq!(crush_impulse(320), Some(128));
        assert_eq!(crush_impulse(639), Some(128));
        assert_eq!(crush_impulse(640), Some(256));
        assert_eq!(crush_impulse(960), Some(384));
        assert_eq!(crush_impulse(1_600), Some(640));
        assert_eq!(crush_impulse(50_000), Some(640));
    }

    #[test]
    fn crowd_contact_coincident_discs_separate_the_same_way_twice() {
        let start = [
            Disc {
                pos: [100, 100],
                radius_mm: 200,
            },
            Disc {
                pos: [100, 100],
                radius_mm: 200,
            },
        ];
        let run = || {
            let mut discs = start;
            let mut pressure = [0; 2];
            let mut shoves = [Shove::default(); 2];
            Contact::new()
                .step(&mut discs, &[], &mut pressure, &mut shoves)
                .expect("valid batch");
            discs
        };
        let first = run();
        assert_eq!(first, run(), "coincident centres separate the same way");
        assert!(first[0].pos[0] < first[1].pos[0], "lower slot yields -x");
        let reach = 400 + i32::try_from(PAIR_SKIN_MM).unwrap();
        assert_eq!(
            first[1].pos[0] - first[0].pos[0],
            reach,
            "coincident discs separate to reach plus the pair skin"
        );
    }

    #[test]
    fn crowd_contact_wall_pushes_a_disc_clear() {
        let mut discs = vec![Disc {
            pos: [100, 0],
            radius_mm: 200,
        }];
        let wall = Wall {
            a: [0, -1_000],
            b: [0, 1_000],
        };
        let mut pressure = [0; 1];
        let mut shoves = [Shove::default(); 1];
        let report = Contact::new()
            .step(&mut discs, &[wall], &mut pressure, &mut shoves)
            .expect("valid batch");
        let clear = 200 + i32::try_from(WALL_SKIN_MM).unwrap();
        assert_eq!(
            discs[0].pos,
            [clear, 0],
            "pushed one radius plus the wall skin clear"
        );
        assert_eq!(pressure[0], 100 * CONTACT_STIFFNESS_N_PER_MM);
        assert_eq!(report.max_wall_penetration_mm, 0);
        assert_eq!(report.shoves, 1, "100 mm of compression is a crush");
    }

    #[test]
    fn crowd_contact_step_allocates_nothing_warm() {
        let mut discs = block_512(420);
        let mut contact = Contact::new();
        let mut pressure = vec![0; discs.len()];
        let mut shoves = vec![Shove::default(); discs.len()];
        for _ in 0..3 {
            contact
                .step(&mut discs, &[], &mut pressure, &mut shoves)
                .expect("warm step");
        }
        let (report, count) = engine_core::alloc_probe::measure(|| {
            contact
                .step(&mut discs, &[], &mut pressure, &mut shoves)
                .expect("measured step")
        });
        assert_eq!(count.calls, 0, "warm step allocated {} times", count.calls);
        assert_eq!(report.shoves, 0, "spaced discs are not crushed");
    }

    #[test]
    fn crowd_contact_bad_batch_writes_nothing() {
        let start = vec![
            Disc {
                pos: [0, 0],
                radius_mm: 100,
            },
            Disc {
                pos: [50, 0],
                radius_mm: 100,
            },
        ];
        let walls = [Wall {
            a: [0, -1_000],
            b: [0, 1_000],
        }];
        let mut contact = Contact::new();
        let mut discs = start.clone();
        let mut pressure = [0; 2];
        let mut shoves = [Shove::default(); 2];
        let mut narrow_pressure = [0; 1];
        assert_eq!(
            contact.step(&mut discs, &walls, &mut narrow_pressure, &mut shoves),
            Err(ContactError::MismatchedLanes {
                discs: 2,
                pressure: 1
            })
        );
        assert_eq!(discs, start);
        assert_eq!(narrow_pressure, [0]);
        let mut narrow_shoves = [Shove::default(); 1];
        assert_eq!(
            contact.step(&mut discs, &walls, &mut pressure, &mut narrow_shoves),
            Err(ContactError::ShoveLanes {
                discs: 2,
                shoves: 1
            })
        );
        assert_eq!(discs, start);
        assert_eq!(pressure, [0; 2]);
        let mut bad_radius = start.clone();
        bad_radius[1].radius_mm = 0;
        let expected = bad_radius.clone();
        assert_eq!(
            contact.step(&mut bad_radius, &walls, &mut pressure, &mut shoves),
            Err(ContactError::BadRadius { disc: 1 })
        );
        assert_eq!(bad_radius, expected);
        let mut bad_place = start.clone();
        bad_place[0].pos[0] = MAX_COORD_MM + 1;
        assert_eq!(
            contact.step(&mut bad_place, &walls, &mut pressure, &mut shoves),
            Err(ContactError::BadPosition { disc: 0 })
        );
        assert_eq!(bad_place[0].pos[0], MAX_COORD_MM + 1);
        let point_wall = [Wall {
            a: [5, 5],
            b: [5, 5],
        }];
        assert_eq!(
            contact.step(&mut discs, &point_wall, &mut pressure, &mut shoves),
            Err(ContactError::BadWall { wall: 0 })
        );
        let wide = [
            Disc {
                pos: [-MAX_COORD_MM, 0],
                radius_mm: 1,
            },
            Disc {
                pos: [MAX_COORD_MM, 0],
                radius_mm: 1,
            },
        ];
        let mut wide = wide;
        let mut wide_pressure = [0; 2];
        assert_eq!(
            contact.step(&mut wide, &[], &mut wide_pressure, &mut shoves),
            Err(ContactError::WorldTooWide)
        );
        let mut too_many = vec![
            Disc {
                pos: [0, 0],
                radius_mm: 1,
            };
            MAX_DISCS + 1
        ];
        let mut big_pressure = vec![0; too_many.len()];
        let mut big_shoves = vec![Shove::default(); too_many.len()];
        assert_eq!(
            contact.step(&mut too_many, &[], &mut big_pressure, &mut big_shoves),
            Err(ContactError::TooManyDiscs {
                discs: MAX_DISCS + 1
            })
        );
    }

    #[test]
    #[ignore = "timing: release-only 1 ms gate for 512 discs; run with --release --ignored --nocapture"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn crowd_contact_512_discs_step_under_one_ms() {
        let walls = [Wall {
            a: [0, -10_000],
            b: [0, 10_000],
        }];
        let mut discs = block_512(360);
        let mut contact = Contact::new();
        let mut pressure = vec![0; discs.len()];
        let mut shoves = vec![Shove::default(); discs.len()];
        for _ in 0..8 {
            contact
                .step(&mut discs, &walls, &mut pressure, &mut shoves)
                .expect("warm step");
        }
        let mut samples = Vec::with_capacity(64);
        for _ in 0..64 {
            let started = std::time::Instant::now();
            contact
                .step(&mut discs, &walls, &mut pressure, &mut shoves)
                .expect("timed step");
            samples.push(started.elapsed().as_secs_f64() * 1_000.0);
        }
        samples.sort_by(|left, right| left.partial_cmp(right).unwrap());
        let p50 = samples[32];
        let p95 = samples[60];
        println!(
            "crowd_contact 512 discs warm step: p50 {p50:.3} ms, p95 {p95:.3} ms (gate < 1.000 ms)"
        );
        assert!(p95 < 1.0, "p95 {p95:.3} ms is not under the 1 ms gate");
    }
}
