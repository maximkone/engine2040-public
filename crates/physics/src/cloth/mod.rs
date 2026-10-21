//! cloth: CPU Verlet cloth for fighter capes and arena banners, plus Q16 hair and rope strands.
//!
//! Consumer: the fighter look pins a cape at the shoulders ([`Cloth::pin_top_row`]) and steps it
//! once per sim tick; the arena kit hangs banners from a rod the same way. The renderer reads
//! [`Cloth::positions`]. Presentation only: floats, never hashed, never read by the simulation,
//! so a cloth bug can look wrong but can never desync a replay.
//!
//! [`Strand`] is the same idea for hair and rope: a chain of particles stepped in Q47.16 fixed
//! point, the root pinned to a scalp or both ends pinned between two points, tension-only links
//! and a sagging hang. It is presentation only too: [`Strand::positions`] is the renderer's view
//! and [`Strand::digest`] is a test-and-tape check, never the match hash.
//!
//! Units: meters, seconds, y up. The step is fixed at 60 Hz and takes no time argument; each
//! tick runs `substeps` Verlet substeps with `iterations` link passes each, both from
//! [`ClothParams`] ([`ClothTier`] fills them in). Math: position Verlet with linear air drag
//! toward the wind, Jakobsen tension-only links (structure plus shear), one long-range tether per
//! free particle to its nearest pin, and one-way capsule colliders that push cloth and nothing
//! back. A sleeping cloth costs nothing until a pin moves, the wind changes or a collider
//! touches it. Tearing is off unless `tear_strain` is set.
//!
//! Errors are typed [`ClothError`] and [`StrandError`]; bad caller input is refused before any
//! state changes, and both steppers allocate nothing.
//!
//! Collisions (CLX): a collider carries its layer and the surface penetration the cloth may
//! keep, and the layer table in [`crate::layers`] decides which pairs may meet (cloth meets
//! fighters, props, the world and other cloth; it never meets debris). A particle whose path
//! crosses a collider stops at the time of impact instead of passing through it: the swept test
//! uses the hashed tick's own Q16 detector ([`crate::query::swept_cast`]), so a fast gust cannot
//! tunnel a cape through a body. [`Cloth::set_self_collision_set`] names the intercollision
//! particle set: only those particles collide with each other, and a spatial hash keeps that
//! pass off the quadratic path. [`Cloth::set_max_distance_gradient`] gives every particle its
//! own Max Distance, so a painted gradient is data and not a second solver.

mod collide;
mod convert;
mod intercollision;
mod max_distance;
mod strand;
#[cfg(test)]
mod tests;

pub use collide::ClothCollider;
pub use strand::{Strand, StrandError, StrandParams, MAX_STRAND_NODES};

use crate::query::SweptHit;
use collide::cloth_meets;
use engine_core::vec3::dist2;
use intercollision::hash_shape;

/// One tick of the fixed 60 Hz step, seconds.
const TICK_DT: f32 = 1.0 / 60.0;
/// Largest grid side, in particles.
pub const MAX_SIDE: usize = 32;
/// Largest substep count per tick.
pub const MAX_SUBSTEPS: usize = 8;
/// Largest link passes per substep.
pub const MAX_ITERATIONS: usize = 16;
/// Largest particle speed, m/s: a yank can never throw a particle across the arena.
const MAX_SPEED: f32 = 30.0;
/// Largest absolute coordinate of a pin, origin or collider point, meters.
const WORLD_LIMIT: f32 = 1000.0;
/// Largest cloth side or collider radius, meters.
const MAX_SIZE: f32 = 20.0;
/// Largest gravity, m/s squared.
const MAX_GRAVITY: f32 = 40.0;
/// Largest air drag, 1/s. With the 60 Hz step this keeps `drag * dt` under one half.
const MAX_DRAG: f32 = 30.0;
/// Largest wind speed per axis, m/s.
const MAX_WIND: f32 = 60.0;
/// A free particle slower than this, m/s, counts as still.
const SLEEP_SPEED: f32 = 0.02;
/// Consecutive still ticks before the cloth sleeps (half a second).
const SLEEP_TICKS: u32 = 30;
/// A pin target must move this far, meters, to wake the cloth.
const PIN_EPS: f32 = 0.0005;
/// Links and colliders shorter than this squared are treated as zero length.
const MIN_LEN2: f32 = 1e-12;
/// Largest intercollision sweeps per tick; the sweep exits early once every pair in the set
/// clears the separation. A settled sheet rides the early-out.
const MAX_SELF_PASSES: usize = 4;
/// Share of the overlap one intercollision pass takes away. A full push puts the pair exactly
/// at the separation, the way the PX29 shape did.
const SELF_RELAX: f32 = 1.0;
/// Largest spatial hash cells per axis; the cell edge grows when a sheet is wider.
const MAX_HASH_AXIS: usize = 64;
/// The same 64 as a float, for the cell arithmetic that divides a sheet's extent. Written out so
/// the hash shape needs no cast and the pair cannot drift apart.
const MAX_HASH_AXIS_F32: f32 = 64.0;
/// How far outside the shell a stopped particle is placed, meters: the touch epsilon that keeps
/// the push-out from shoving it back in.
const TOUCH_EPS: f32 = 1.0e-4;

/// Typed errors for cloth setup, pins, tuning and colliders. The stepper cannot fail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClothError {
    /// Both grid sides must be `2..=MAX_SIDE` particles.
    BadGrid,
    /// A named value is not finite or is outside its range.
    BadParam(&'static str),
    /// Particle index is out of range.
    BadIndex(usize),
    /// The top row takes exactly one pin point per column.
    BadPins {
        /// Pin points the grid needs (its column count).
        expected: usize,
        /// Pin points the caller passed.
        got: usize,
    },
}

impl core::fmt::Display for ClothError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ClothError::BadGrid => write!(f, "cloth: grid sides must be 2..={MAX_SIDE}"),
            ClothError::BadParam(what) => write!(f, "cloth: bad {what}"),
            ClothError::BadIndex(i) => write!(f, "cloth: particle {i} out of range"),
            ClothError::BadPins { expected, got } => {
                write!(f, "cloth: {got} pin points for a top row of {expected}")
            }
        }
    }
}

impl std::error::Error for ClothError {}

/// Particles per side of a cloth sheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClothGrid {
    /// Particles along the pinned top edge.
    pub cols: usize,
    /// Particles down the hang.
    pub rows: usize,
}

impl ClothGrid {
    /// Particle count of the sheet.
    pub const fn particles(self) -> usize {
        self.cols * self.rows
    }

    fn validate(self) -> Result<(), ClothError> {
        let sides = 2..=MAX_SIDE;
        if sides.contains(&self.cols) && sides.contains(&self.rows) {
            Ok(())
        } else {
            Err(ClothError::BadGrid)
        }
    }
}

/// The hardware class a client runs on. The floor is an integrated GPU with 8 GB of RAM (every
/// game must run there); a 3050 class card with 16 GB is the comfortable tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClothTier {
    /// Weak GPU, little RAM: a small cape and a light banner, one substep.
    Potato,
    /// A browser tab: single thread, wasm speed.
    Web,
    /// The comfortable desktop tier.
    Comfort,
    /// Headroom: finer sheets and more link passes.
    High,
}

/// What one tier pays for cloth. Plain data: the quality ladder reads it and the fighter look
/// and arena kit size their sheets from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClothKnobs {
    /// Particles of one fighter cape.
    pub cape: ClothGrid,
    /// Particles of one arena banner (wider than tall).
    pub banner: ClothGrid,
    /// Verlet substeps per 60 Hz tick.
    pub substeps: usize,
    /// Link passes per substep.
    pub iterations: usize,
}

impl ClothTier {
    /// Every tier, weakest first.
    pub const ALL: [ClothTier; 4] = [
        ClothTier::Potato,
        ClothTier::Web,
        ClothTier::Comfort,
        ClothTier::High,
    ];

    /// The cost table: every per-tier number lives in this one place.
    pub const fn knobs(self) -> ClothKnobs {
        match self {
            ClothTier::Potato => ClothKnobs {
                cape: ClothGrid { cols: 4, rows: 6 },
                banner: ClothGrid { cols: 6, rows: 4 },
                substeps: 1,
                iterations: 3,
            },
            ClothTier::Web => ClothKnobs {
                cape: ClothGrid { cols: 5, rows: 8 },
                banner: ClothGrid { cols: 8, rows: 5 },
                substeps: 1,
                iterations: 4,
            },
            ClothTier::Comfort => ClothKnobs {
                cape: ClothGrid { cols: 6, rows: 10 },
                banner: ClothGrid { cols: 10, rows: 6 },
                substeps: 2,
                iterations: 4,
            },
            ClothTier::High => ClothKnobs {
                cape: ClothGrid { cols: 8, rows: 14 },
                banner: ClothGrid { cols: 14, rows: 8 },
                substeps: 2,
                iterations: 5,
            },
        }
    }
}

/// Physical tune of one cloth sheet.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClothParams {
    /// Gravity pulling down, m/s squared, `0..=40`.
    pub gravity: f32,
    /// Air drag, 1/s, `0..=30`: slows the cloth and pulls it toward the wind speed.
    pub air_drag: f32,
    /// Wind velocity, m/s per axis, within `-60..=60`.
    pub wind: [f32; 3],
    /// Fraction of each link error removed per pass, `0.05..=1`.
    pub stiffness: f32,
    /// Link passes per substep, `1..=MAX_ITERATIONS`.
    pub iterations: usize,
    /// Verlet substeps per 60 Hz tick, `1..=MAX_SUBSTEPS`.
    pub substeps: usize,
    /// `None` keeps tearing off. `Some(s)` cuts a link stretched past `1 + s` times its rest
    /// length, `s` in `0.05..=4`; once any link is cut the tethers stop holding the pieces up.
    pub tear_strain: Option<f32>,
}

impl ClothParams {
    /// Cape and banner tune with the substep and pass counts of `tier`: light air drag, no
    /// wind, tearing off.
    pub const fn for_tier(tier: ClothTier) -> Self {
        let knobs = tier.knobs();
        Self {
            gravity: 9.81,
            air_drag: 1.5,
            wind: [0.0; 3],
            stiffness: 1.0,
            iterations: knobs.iterations,
            substeps: knobs.substeps,
            tear_strain: None,
        }
    }

    /// Range-check every field; a failing tune is never applied.
    pub fn validate(&self) -> Result<(), ClothError> {
        let ok = |v: f32, lo: f32, hi: f32| (lo..=hi).contains(&v);
        if !ok(self.gravity, 0.0, MAX_GRAVITY) {
            return Err(ClothError::BadParam("gravity"));
        }
        if !ok(self.air_drag, 0.0, MAX_DRAG) {
            return Err(ClothError::BadParam("air drag"));
        }
        if !self.wind.iter().all(|w| ok(*w, -MAX_WIND, MAX_WIND)) {
            return Err(ClothError::BadParam("wind"));
        }
        if !ok(self.stiffness, 0.05, 1.0) {
            return Err(ClothError::BadParam("stiffness"));
        }
        if !(1..=MAX_ITERATIONS).contains(&self.iterations) {
            return Err(ClothError::BadParam("iterations"));
        }
        if !(1..=MAX_SUBSTEPS).contains(&self.substeps) {
            return Err(ClothError::BadParam("substeps"));
        }
        if self.tear_strain.is_some_and(|s| !ok(s, 0.05, 4.0)) {
            return Err(ClothError::BadParam("tear strain"));
        }
        Ok(())
    }
}

/// One distance link between two particles.
#[derive(Clone, Copy, Debug)]
struct Link {
    a: usize,
    b: usize,
    rest: f32,
    alive: bool,
}

/// Long-range cap on how far a free particle may hang from its nearest pin. Local links alone
/// cannot carry a tall sheet in a few passes, so its top links would stretch.
#[derive(Clone, Copy, Debug)]
struct Tether {
    node: usize,
    pin: usize,
    max: f32,
}

/// A presentation cloth sheet: `cols` by `rows` particles hung from the pinned top row.
///
/// There is deliberately no hash, snapshot or restore: cloth never enters checkpoints.
#[derive(Clone, Debug)]
pub struct Cloth {
    cols: usize,
    rows: usize,
    /// Rest spacing between neighbours: along the top edge, then down the hang.
    spacing: [f32; 2],
    pos: Vec<[f32; 3]>,
    prev: Vec<[f32; 3]>,
    anchor: Vec<[f32; 3]>,
    /// The layout positions: the reference a Max Distance limit is measured from.
    rest: Vec<[f32; 3]>,
    pinned: Vec<bool>,
    links: Vec<Link>,
    tethers: Vec<Tether>,
    /// The intercollision particle set as a bitset over particle ids. Only the
    /// particles in the set collide with each other; an empty set is no
    /// self-collision at all.
    self_set: Vec<u64>,
    /// The particles of the intercollision set, in index order.
    self_list: Vec<usize>,
    /// Per-particle Max Distance, meters from the rest position. Zero means the
    /// tether reach applies and no extra limit is set.
    max_distance: Vec<f32>,
    /// The spatial hash of the intercollision set: cells per axis, the cell
    /// edge and the grid origin, all fixed so the pass never allocates.
    hash_dims: [usize; 2],
    hash_cell: f32,
    hash_origin: [f32; 2],
    hash_count: Vec<u32>,
    hash_start: Vec<u32>,
    hash_items: Vec<usize>,
    /// Swept-cast scratch, reused so the step never allocates.
    cast_hits: Vec<SweptHit>,
    params: ClothParams,
    torn: usize,
    still_ticks: u32,
    asleep: bool,
    ticks: u64,
    /// How many particles the swept collider test stopped on the last tick.
    swept_stops: u32,
}

impl Cloth {
    /// Build a flat sheet of `size` meters (width, height) hanging down -Y from `origin`, the
    /// left end of its pinned top row. Callers [`reset`](Cloth::reset) it onto the shoulders or
    /// the rod when the owner spawns.
    pub fn new(
        grid: ClothGrid,
        size: [f32; 2],
        origin: [f32; 3],
        params: ClothParams,
    ) -> Result<Self, ClothError> {
        grid.validate()?;
        params.validate()?;
        if !size.iter().all(|s| (0.01..=MAX_SIZE).contains(s)) {
            return Err(ClothError::BadParam("size"));
        }
        check_point(origin, "origin")?;
        let spacing = [
            size[0] / count_f32(grid.cols - 1),
            size[1] / count_f32(grid.rows - 1),
        ];
        let n = grid.particles();
        let (hash_dims, hash_cell) = hash_shape(spacing, size);
        let cells = hash_dims[0] * hash_dims[1];
        let mut cloth = Self {
            cols: grid.cols,
            rows: grid.rows,
            spacing,
            pos: Vec::with_capacity(n),
            prev: Vec::with_capacity(n),
            anchor: Vec::with_capacity(n),
            rest: Vec::with_capacity(n),
            pinned: Vec::with_capacity(n),
            links: build_links(grid, spacing),
            tethers: Vec::with_capacity(n),
            self_set: vec![0; n.div_ceil(64)],
            self_list: Vec::with_capacity(n),
            max_distance: vec![0.0; n],
            hash_dims,
            hash_cell,
            hash_origin: [0.0; 2],
            hash_count: vec![0; cells],
            hash_start: vec![0; cells + 1],
            hash_items: vec![0; n],
            cast_hits: Vec::with_capacity(4),
            params,
            torn: 0,
            still_ticks: 0,
            asleep: false,
            ticks: 0,
            swept_stops: 0,
        };
        cloth.layout(origin);
        Ok(cloth)
    }

    /// Lay the sheet flat and at rest from `origin`, top row pinned, every link whole, awake.
    fn layout(&mut self, origin: [f32; 3]) {
        self.pos.clear();
        self.pinned.clear();
        for r in 0..self.rows {
            for c in 0..self.cols {
                self.pos.push([
                    origin[0] + count_f32(c) * self.spacing[0],
                    origin[1] - count_f32(r) * self.spacing[1],
                    origin[2],
                ]);
                self.pinned.push(r == 0);
            }
        }
        self.prev.clone_from(&self.pos);
        self.anchor.clone_from(&self.pos);
        self.rest.clone_from(&self.pos);
        for link in &mut self.links {
            link.alive = true;
        }
        self.torn = 0;
        self.wake();
        self.rebuild_tethers();
    }

    /// Put the sheet back at rest from `origin` with no motion: a respawn or a teleport.
    pub fn reset(&mut self, origin: [f32; 3]) -> Result<(), ClothError> {
        check_point(origin, "origin")?;
        self.layout(origin);
        Ok(())
    }

    /// Particle count (`cols * rows`).
    pub fn particle_count(&self) -> usize {
        self.pos.len()
    }

    /// Links the sheet was built with, torn ones included.
    pub fn link_count(&self) -> usize {
        self.links.len()
    }

    /// Links cut by tearing since the last [`reset`](Cloth::reset).
    pub fn torn_count(&self) -> usize {
        self.torn
    }

    /// Pinned particle count.
    pub fn pinned_count(&self) -> usize {
        self.pinned.iter().filter(|p| **p).count()
    }

    /// True when the particle is pinned; false for an index out of range.
    pub fn is_pinned(&self, particle: usize) -> bool {
        self.pinned.get(particle).copied().unwrap_or(false)
    }

    /// Ticks stepped awake since construction.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// True while the sheet is asleep and [`step`](Cloth::step) costs nothing.
    pub fn is_asleep(&self) -> bool {
        self.asleep
    }

    /// Current tune.
    pub fn params(&self) -> ClothParams {
        self.params
    }

    /// Particle positions in meters, row by row from the pinned top row: the renderer's verts.
    pub fn positions(&self) -> &[[f32; 3]] {
        &self.pos
    }

    /// Longest whole link over its rest length (1.0 is relaxed): the stretch a capture shows.
    pub fn max_stretch(&self) -> f32 {
        self.links
            .iter()
            .filter(|l| l.alive)
            .map(|l| dist2(self.pos[l.a], self.pos[l.b]).sqrt() / l.rest)
            .fold(0.0, f32::max)
    }

    /// Replace the tune. A changed tune wakes the sheet; a bad one changes nothing.
    pub fn set_params(&mut self, params: ClothParams) -> Result<(), ClothError> {
        params.validate()?;
        if params != self.params {
            self.params = params;
            self.wake();
        }
        Ok(())
    }

    /// Replace the wind velocity (arena weather, a gust); a changed wind wakes the sheet.
    pub fn set_wind(&mut self, wind: [f32; 3]) -> Result<(), ClothError> {
        self.set_params(ClothParams {
            wind,
            ..self.params
        })
    }

    /// Wake the sheet; it sleeps again after half a second of calm.
    pub fn wake(&mut self) {
        self.asleep = false;
        self.still_ticks = 0;
    }

    /// Move the pinned top row to `points`, one per column left to right: a cape follows its
    /// shoulders, a banner its rod. The sheet is stepped toward the new pins over the next tick.
    /// Moving a pin more than half a millimeter wakes the sheet.
    pub fn pin_top_row(&mut self, points: &[[f32; 3]]) -> Result<(), ClothError> {
        if points.len() != self.cols {
            return Err(ClothError::BadPins {
                expected: self.cols,
                got: points.len(),
            });
        }
        for p in points {
            check_point(*p, "pin")?;
        }
        let mut pin_set_changed = false;
        for (particle, p) in points.iter().enumerate() {
            pin_set_changed |= self.set_pin(particle, *p);
        }
        if pin_set_changed {
            self.rebuild_tethers();
        }
        Ok(())
    }

    /// Pin one particle (a banner corner) to `to`, or move its pin.
    pub fn pin(&mut self, particle: usize, to: [f32; 3]) -> Result<(), ClothError> {
        if particle >= self.pos.len() {
            return Err(ClothError::BadIndex(particle));
        }
        check_point(to, "pin")?;
        if self.set_pin(particle, to) {
            self.rebuild_tethers();
        }
        Ok(())
    }

    /// Release a particle back to free motion; it keeps its place and loses its velocity.
    pub fn unpin(&mut self, particle: usize) -> Result<(), ClothError> {
        let Some(pinned) = self.pinned.get_mut(particle) else {
            return Err(ClothError::BadIndex(particle));
        };
        if *pinned {
            *pinned = false;
            self.prev[particle] = self.pos[particle];
            self.wake();
            self.rebuild_tethers();
        }
        Ok(())
    }

    /// Point one particle's pin at `to`; true when the particle was not pinned before.
    fn set_pin(&mut self, particle: usize, to: [f32; 3]) -> bool {
        let newly = !self.pinned[particle];
        if newly || dist2(self.anchor[particle], to) > PIN_EPS * PIN_EPS {
            self.anchor[particle] = to;
            self.pinned[particle] = true;
            self.prev[particle] = self.pos[particle];
            self.wake();
        }
        newly
    }

    /// One tether per free particle to its nearest pin, capped at their flat rest distance.
    /// Nearest compares squared grid distances; ties keep the smallest pin index.
    fn rebuild_tethers(&mut self) {
        self.tethers.clear();
        for node in (0..self.pos.len()).filter(|i| !self.pinned[*i]) {
            let nearest = (0..self.pos.len())
                .filter(|j| self.pinned[*j])
                .map(|j| (self.flat_rest2(node, j), j))
                .fold(None, |best: Option<(f32, usize)>, cand| match best {
                    Some(b) if b.0 <= cand.0 => Some(b),
                    _ => Some(cand),
                });
            if let Some((rest2, pin)) = nearest {
                self.tethers.push(Tether {
                    node,
                    pin,
                    max: rest2.sqrt(),
                });
            }
        }
    }

    /// Squared rest distance between two particles on the flat sheet, meters squared.
    fn flat_rest2(&self, a: usize, b: usize) -> f32 {
        let dc = (count_f32(a % self.cols) - count_f32(b % self.cols)) * self.spacing[0];
        let dr = (count_f32(a / self.cols) - count_f32(b / self.cols)) * self.spacing[1];
        dc * dc + dr * dr
    }

    /// Advance one fixed 1/60 s tick. Returns false, doing no work, while the sheet is asleep
    /// and nothing touched it.
    pub fn step(&mut self, colliders: &[ClothCollider]) -> bool {
        if self.asleep {
            if !self.touched(colliders) {
                return false;
            }
            self.wake();
        }
        let substeps = self.params.substeps;
        let h = TICK_DT / count_f32(substeps);
        self.swept_stops = 0;
        for s in 0..substeps {
            self.substep(substeps - s, h, colliders);
        }
        self.settle(h);
        self.ticks += 1;
        true
    }

    /// One Verlet substep of length `h`; `left` counts the substeps still to run this tick,
    /// this one included.
    ///
    /// The order is the one the PX29 shape used: intercollision runs last, so the tick rests
    /// separated. The swept collider test runs before the push-out, so a particle that would
    /// cross a shell stops at the time of impact instead of being pushed out on the far side.
    fn substep(&mut self, left: usize, h: f32, colliders: &[ClothCollider]) {
        self.integrate(left, h);
        self.cap_speed(h);
        self.tear_overstretched();
        for _ in 0..self.params.iterations {
            self.solve_links();
        }
        if self.torn == 0 {
            self.solve_tethers();
        }
        self.solve_max_distance();
        self.swept_stops += self.sweep_colliders(colliders);
        for collider in colliders {
            if !cloth_meets(collider) {
                continue;
            }
            for (p, pinned) in self.pos.iter_mut().zip(&self.pinned) {
                if !pinned {
                    *p = collider.push_out(*p);
                }
            }
        }
        self.resolve_self();
    }

    /// Verlet integration with gravity and air drag toward the wind. Pinned particles walk to
    /// their anchor in equal shares over the tick's substeps (the last lands exactly); free
    /// ones keep their motion. The speed cap is [`Cloth::cap_speed`], which runs after the
    /// whole move so the swept collider test sees the real travel.
    fn integrate(&mut self, left: usize, h: f32) {
        let params = self.params;
        let keep = 1.0 - params.air_drag * h;
        let hh = h * h;
        let push = [
            params.air_drag * params.wind[0] * hh,
            (params.air_drag * params.wind[1] - params.gravity) * hh,
            params.air_drag * params.wind[2] * hh,
        ];
        let share = 1.0 / count_f32(left);
        let particles = self
            .pos
            .iter_mut()
            .zip(&mut self.prev)
            .zip(&self.anchor)
            .zip(&self.pinned);
        for (((pos, prev), anchor), pinned) in particles {
            let cur = *pos;
            if *pinned {
                *pos = if left == 1 {
                    *anchor
                } else {
                    [
                        cur[0] + (anchor[0] - cur[0]) * share,
                        cur[1] + (anchor[1] - cur[1]) * share,
                        cur[2] + (anchor[2] - cur[2]) * share,
                    ]
                };
                *prev = *pos;
                continue;
            }
            let mut moved = sub(cur, *prev);
            moved = [moved[0] * keep, moved[1] * keep, moved[2] * keep];
            *prev = cur;
            *pos = [
                cur[0] + moved[0] + push[0],
                cur[1] + moved[1] + push[1],
                cur[2] + moved[2] + push[2],
            ];
        }
    }

    /// Cut every live link stretched past the tear threshold. Does nothing while tearing is off.
    fn tear_overstretched(&mut self) {
        let Some(strain) = self.params.tear_strain else {
            return;
        };
        for link in self.links.iter_mut().filter(|l| l.alive) {
            let limit = link.rest * (1.0 + strain);
            if dist2(self.pos[link.a], self.pos[link.b]) > limit * limit {
                link.alive = false;
                self.torn += 1;
            }
        }
    }

    /// One pass over the live links in index order: a stretched pair moves toward its rest
    /// length by `stiffness` of the error, split between the free ends (a pinned end never
    /// moves). Links resist stretching only, like fabric: a squeezed or jammed sheet falls and
    /// unfolds under gravity instead of standing up like a truss.
    fn solve_links(&mut self) {
        let stiffness = self.params.stiffness;
        for link in self.links.iter().filter(|l| l.alive) {
            let (pinned_a, pinned_b) = (self.pinned[link.a], self.pinned[link.b]);
            if pinned_a && pinned_b {
                continue;
            }
            let (pa, pb) = (self.pos[link.a], self.pos[link.b]);
            let between = sub(pb, pa);
            let len2 = dot(between, between);
            if len2 < MIN_LEN2 {
                continue;
            }
            let len = len2.sqrt();
            if len <= link.rest {
                continue;
            }
            // A pinned end takes no share, so the free end closes the whole error.
            let share = stiffness * (len - link.rest) / len;
            let (ka, kb) = match (pinned_a, pinned_b) {
                (true, _) => (0.0, share),
                (_, true) => (share, 0.0),
                _ => (share / 2.0, share / 2.0),
            };
            self.pos[link.a] = [
                pa[0] + between[0] * ka,
                pa[1] + between[1] * ka,
                pa[2] + between[2] * ka,
            ];
            self.pos[link.b] = [
                pb[0] - between[0] * kb,
                pb[1] - between[1] * kb,
                pb[2] - between[2] * kb,
            ];
        }
    }

    /// Pull every free particle that hangs past its tether back onto the reach ball around its
    /// pin.
    fn solve_tethers(&mut self) {
        for tether in &self.tethers {
            let pin = self.pos[tether.pin];
            let away = sub(self.pos[tether.node], pin);
            let len2 = dot(away, away);
            if len2 > tether.max * tether.max {
                let k = tether.max / len2.sqrt();
                self.pos[tether.node] = [
                    pin[0] + away[0] * k,
                    pin[1] + away[1] * k,
                    pin[2] + away[2] * k,
                ];
            }
        }
    }

    /// Count the tick as still or moving; enough still ticks put the sheet to sleep.
    fn settle(&mut self, h: f32) {
        let limit = SLEEP_SPEED * h;
        let calm = self
            .pos
            .iter()
            .zip(&self.prev)
            .zip(&self.pinned)
            .all(|((p, q), pinned)| *pinned || dist2(*p, *q) <= limit * limit);
        if !calm {
            self.still_ticks = 0;
            return;
        }
        self.still_ticks += 1;
        if self.still_ticks >= SLEEP_TICKS {
            self.asleep = true;
            self.prev.clone_from(&self.pos);
        }
    }
}

/// Links in index order per cell: right, down, then both shear diagonals.
fn build_links(grid: ClothGrid, spacing: [f32; 2]) -> Vec<Link> {
    let diagonal = (spacing[0] * spacing[0] + spacing[1] * spacing[1]).sqrt();
    let at = |c: usize, r: usize| r * grid.cols + c;
    let link = |a: usize, b: usize, rest: f32| Link {
        a,
        b,
        rest,
        alive: true,
    };
    let mut links = Vec::with_capacity(4 * grid.particles());
    for r in 0..grid.rows {
        for c in 0..grid.cols {
            if c + 1 < grid.cols {
                links.push(link(at(c, r), at(c + 1, r), spacing[0]));
            }
            if r + 1 < grid.rows {
                links.push(link(at(c, r), at(c, r + 1), spacing[1]));
            }
            if c + 1 < grid.cols && r + 1 < grid.rows {
                links.push(link(at(c, r), at(c + 1, r + 1), diagonal));
            }
            if c > 0 && r + 1 < grid.rows {
                links.push(link(at(c, r), at(c - 1, r + 1), diagonal));
            }
        }
    }
    links
}

/// A pin, origin or collider point must be finite and inside the world.
fn check_point(p: [f32; 3], what: &'static str) -> Result<(), ClothError> {
    if p.iter().all(|v| (-WORLD_LIMIT..=WORLD_LIMIT).contains(v)) {
        Ok(())
    } else {
        Err(ClothError::BadParam(what))
    }
}

/// A small count as a float; sheet sides never pass `MAX_SIDE`, so nothing saturates.
fn count_f32(n: usize) -> f32 {
    f32::from(u16::try_from(n).unwrap_or(u16::MAX))
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
