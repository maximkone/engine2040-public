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

use engine_core::fixed::Fixed;
use engine_core::vec3::dist2;
use engine_core::Fnv64;

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

/// One-way capsule (a sphere when both ends meet) that pushes cloth aside; cloth pushes nothing
/// back. Intended consumer: the fighter body (torso and legs) for capes, rods and poles for
/// banners.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClothCollider {
    a: [f32; 3],
    b: [f32; 3],
    radius: f32,
}

impl ClothCollider {
    /// A ball at `center`.
    pub fn sphere(center: [f32; 3], radius: f32) -> Result<Self, ClothError> {
        Self::capsule(center, center, radius)
    }

    /// A segment `a` to `b` swollen by `radius` meters.
    pub fn capsule(a: [f32; 3], b: [f32; 3], radius: f32) -> Result<Self, ClothError> {
        check_point(a, "collider")?;
        check_point(b, "collider")?;
        if !(0.001..=MAX_SIZE).contains(&radius) {
            return Err(ClothError::BadParam("collider radius"));
        }
        Ok(Self { a, b, radius })
    }

    /// The point of the axis segment nearest `p`.
    fn nearest_on_axis(&self, p: [f32; 3]) -> [f32; 3] {
        let ab = sub(self.b, self.a);
        let len2 = dot(ab, ab);
        if len2 < MIN_LEN2 {
            return self.a;
        }
        let t = (dot(sub(p, self.a), ab) / len2).clamp(0.0, 1.0);
        [
            self.a[0] + ab[0] * t,
            self.a[1] + ab[1] * t,
            self.a[2] + ab[2] * t,
        ]
    }

    /// True when `p` is inside the shell.
    fn contains(&self, p: [f32; 3]) -> bool {
        dist2(p, self.nearest_on_axis(p)) < self.radius * self.radius
    }

    /// `p` moved straight out to the shell surface; points outside stay put. A point exactly on
    /// the axis leaves along +Z.
    fn push_out(&self, p: [f32; 3]) -> [f32; 3] {
        let axis = self.nearest_on_axis(p);
        let away = sub(p, axis);
        let d2 = dot(away, away);
        if d2 >= self.radius * self.radius {
            return p;
        }
        if d2 < MIN_LEN2 {
            return [axis[0], axis[1], axis[2] + self.radius];
        }
        let k = self.radius / d2.sqrt();
        [
            axis[0] + away[0] * k,
            axis[1] + away[1] * k,
            axis[2] + away[2] * k,
        ]
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
    pinned: Vec<bool>,
    links: Vec<Link>,
    tethers: Vec<Tether>,
    params: ClothParams,
    torn: usize,
    still_ticks: u32,
    asleep: bool,
    ticks: u64,
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
        let mut cloth = Self {
            cols: grid.cols,
            rows: grid.rows,
            spacing,
            pos: Vec::with_capacity(n),
            prev: Vec::with_capacity(n),
            anchor: Vec::with_capacity(n),
            pinned: Vec::with_capacity(n),
            links: build_links(grid, spacing),
            tethers: Vec::with_capacity(n),
            params,
            torn: 0,
            still_ticks: 0,
            asleep: false,
            ticks: 0,
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
        for s in 0..substeps {
            self.substep(substeps - s, h, colliders);
        }
        self.settle(h);
        self.ticks += 1;
        true
    }

    /// True when a free particle sits inside a collider.
    fn touched(&self, colliders: &[ClothCollider]) -> bool {
        colliders.iter().any(|c| {
            self.pos
                .iter()
                .zip(&self.pinned)
                .any(|(p, pinned)| !pinned && c.contains(*p))
        })
    }

    /// One Verlet substep of length `h`; `left` counts the substeps still to run this tick,
    /// this one included.
    fn substep(&mut self, left: usize, h: f32, colliders: &[ClothCollider]) {
        self.integrate(left, h);
        self.tear_overstretched();
        for _ in 0..self.params.iterations {
            self.solve_links();
        }
        if self.torn == 0 {
            self.solve_tethers();
        }
        for collider in colliders {
            for (p, pinned) in self.pos.iter_mut().zip(&self.pinned) {
                if !pinned {
                    *p = collider.push_out(*p);
                }
            }
        }
    }

    /// Verlet integration with gravity and air drag toward the wind. Pinned particles walk to
    /// their anchor in equal shares over the tick's substeps (the last lands exactly); free
    /// ones keep their motion, capped at `MAX_SPEED`.
    fn integrate(&mut self, left: usize, h: f32) {
        let params = self.params;
        let keep = 1.0 - params.air_drag * h;
        let hh = h * h;
        let push = [
            params.air_drag * params.wind[0] * hh,
            (params.air_drag * params.wind[1] - params.gravity) * hh,
            params.air_drag * params.wind[2] * hh,
        ];
        let max_move2 = MAX_SPEED * MAX_SPEED * hh;
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
            let moved2 = dot(moved, moved);
            if moved2 > max_move2 {
                let k = (max_move2 / moved2).sqrt();
                moved = [moved[0] * k, moved[1] * k, moved[2] * k];
            }
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

/// Largest particle count of one hair lock or rope.
pub const MAX_STRAND_NODES: usize = 64;

/// Downward pull added to a free strand particle each tick, Q16 meters: 9.81 m/s squared at
/// 60 Hz is 179 / 65536 m.
const STRAND_GRAVITY_Q16: i64 = 179;
/// Fraction of the last tick's motion a free strand particle keeps, Q16: 99% for hair and rope.
const STRAND_KEEP_Q16: i64 = 64_881;
/// Largest gravity a caller may set, Q16 meters per tick squared (40 m/s squared).
const MAX_STRAND_GRAVITY_Q16: i64 = 728;
/// Largest push a caller may set per axis, Q16 meters per tick squared (one meter).
const MAX_STRAND_PUSH_Q16: i64 = 65_536;
/// Largest per-tick displacement of a free strand particle, Q16 meters (half a meter).
const MAX_STRAND_STEP_Q16: i64 = 32_768;
/// Largest strand length, meters, in Q16; matches the cloth sheet cap.
const MAX_STRAND_LEN_Q16: i64 = 20 * 65_536;
/// Largest absolute coordinate of a strand pin, Q16 meters; matches `WORLD_LIMIT`.
const STRAND_WORLD_LIMIT_Q16: i64 = 1000 * 65_536;
/// Percent of a rope's span added as slack between its pins.
const ROPE_SLACK_PERCENT: i64 = 1;

/// Typed errors for strand setup and pins. The stepper cannot fail.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrandError {
    /// A strand has `2..=MAX_STRAND_NODES` particles.
    BadNodes,
    /// A named value is not in range.
    BadParam(&'static str),
    /// Particle index is out of range.
    BadIndex(usize),
}

impl core::fmt::Display for StrandError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            StrandError::BadNodes => {
                write!(f, "strand: particle count must be 2..={MAX_STRAND_NODES}")
            }
            StrandError::BadParam(what) => write!(f, "strand: bad {what}"),
            StrandError::BadIndex(i) => write!(f, "strand: particle {i} out of range"),
        }
    }
}

impl std::error::Error for StrandError {}

/// Tune of one strand, in Q16 per-tick units: the step takes no time argument, so a caller's
/// seconds become tick-squared displacements here (60 Hz).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StrandParams {
    /// Downward displacement added to a free particle each tick, Q16 meters, `0..=728`.
    pub gravity_q16: i64,
    /// Fraction of the last tick's motion kept, Q16, `0..=65_536` (zero stops on a dime).
    pub keep_q16: i64,
    /// Wind and drag push per tick, Q16 meters per axis, within `+-65_536`.
    pub push_q16: [i64; 3],
    /// Link passes per tick, `1..=MAX_ITERATIONS`.
    pub iterations: usize,
}

impl StrandParams {
    /// Hair and rope tune: full gravity, 99% of the motion kept, no wind, six link passes.
    pub const fn calm() -> Self {
        Self {
            gravity_q16: STRAND_GRAVITY_Q16,
            keep_q16: STRAND_KEEP_Q16,
            push_q16: [0; 3],
            iterations: 6,
        }
    }

    /// Range-check every field; a failing tune is never applied.
    pub fn validate(&self) -> Result<(), StrandError> {
        let in_range = |v: i64, lo: i64, hi: i64| (lo..=hi).contains(&v);
        if !in_range(self.gravity_q16, 0, MAX_STRAND_GRAVITY_Q16) {
            return Err(StrandError::BadParam("gravity"));
        }
        if !in_range(self.keep_q16, 0, 65_536) {
            return Err(StrandError::BadParam("keep"));
        }
        if self
            .push_q16
            .iter()
            .any(|p| !in_range(*p, -MAX_STRAND_PUSH_Q16, MAX_STRAND_PUSH_Q16))
        {
            return Err(StrandError::BadParam("push"));
        }
        if !(1..=MAX_ITERATIONS).contains(&self.iterations) {
            return Err(StrandError::BadParam("iterations"));
        }
        Ok(())
    }
}

/// A presentation hair lock or rope: a chain of Q16 Verlet particles.
///
/// The first particle starts pinned to a root; [`pin`](Strand::pin) pins more (both ends of a
/// rope) and [`unpin`](Strand::unpin) frees them again. Links resist stretching only, so a strand
/// sags and swings like a lock of hair or a slack rope. Presentation only: floats leave only
/// through [`positions`](Strand::positions), nothing here is hashed or snapshotted, and no sim
/// state reads it. [`digest`](Strand::digest) is for tests and tapes.
#[derive(Clone, Debug)]
pub struct Strand {
    pos: Vec<[Fixed; 3]>,
    prev: Vec<[Fixed; 3]>,
    anchor: Vec<[Fixed; 3]>,
    pinned: Vec<bool>,
    rest: Fixed,
    params: StrandParams,
    ticks: u64,
    view: Vec<[f32; 3]>,
}

impl Strand {
    /// A straight chain of `nodes` particles hanging down -Y from `root`, its root pinned.
    /// `length` is the whole chain, meters, split evenly between the links.
    pub fn new(
        nodes: usize,
        length: Fixed,
        root: [Fixed; 3],
        params: StrandParams,
    ) -> Result<Self, StrandError> {
        if !(2..=MAX_STRAND_NODES).contains(&nodes) {
            return Err(StrandError::BadNodes);
        }
        params.validate()?;
        check_strand_point(root, "root")?;
        let len = length.to_bits();
        if !(1..=MAX_STRAND_LEN_Q16).contains(&len) {
            return Err(StrandError::BadParam("length"));
        }
        let links = i64::try_from(nodes - 1).unwrap_or(i64::MAX);
        if len / links < 1 {
            return Err(StrandError::BadParam("length"));
        }
        let mut strand = Self {
            pos: Vec::with_capacity(nodes),
            prev: Vec::with_capacity(nodes),
            anchor: Vec::with_capacity(nodes),
            pinned: Vec::with_capacity(nodes),
            rest: Fixed::from_bits(len / links),
            params,
            ticks: 0,
            view: vec![[0.0; 3]; nodes],
        };
        strand.layout(root);
        Ok(strand)
    }

    /// A hair lock: eight particles over `length` meters, the root pinned at `root`.
    pub fn hair(root: [Fixed; 3], length: Fixed) -> Result<Self, StrandError> {
        Self::new(8, length, root, StrandParams::calm())
    }

    /// A rope of `nodes` particles hung between `a` and `b`, both ends pinned, with one percent
    /// of slack over the span so gravity bows it.
    pub fn rope(a: [Fixed; 3], b: [Fixed; 3], nodes: usize) -> Result<Self, StrandError> {
        if !(2..=MAX_STRAND_NODES).contains(&nodes) {
            return Err(StrandError::BadNodes);
        }
        check_strand_point(a, "root")?;
        check_strand_point(b, "pin")?;
        let span = dist_q16(a, b);
        if span <= 0 {
            return Err(StrandError::BadParam("length"));
        }
        let length = span + span / 100 * ROPE_SLACK_PERCENT;
        if length > MAX_STRAND_LEN_Q16 {
            return Err(StrandError::BadParam("length"));
        }
        let mut rope = Self::new(nodes, Fixed::from_bits(length), a, StrandParams::calm())?;
        rope.layout_between(a, b);
        Ok(rope)
    }

    /// Lay the chain straight down from `root`, root pinned, every other particle free.
    fn layout(&mut self, root: [Fixed; 3]) {
        let nodes = self.view.len();
        self.pos.clear();
        self.prev.clear();
        self.anchor.clear();
        self.pinned.clear();
        for k in 0..nodes {
            let p = [
                root[0],
                root[1] - self.rest * Fixed::from_int(i64::try_from(k).unwrap_or(0)),
                root[2],
            ];
            self.pos.push(p);
            self.prev.push(p);
            self.anchor.push(root);
            self.pinned.push(k == 0);
        }
        self.ticks = 0;
        self.refresh_view();
    }

    /// Lay the chain along the segment `a` to `b`, both ends pinned.
    fn layout_between(&mut self, a: [Fixed; 3], b: [Fixed; 3]) {
        let nodes = self.view.len();
        let last = i64::try_from(nodes - 1).unwrap_or(i64::MAX);
        self.pos.clear();
        self.prev.clear();
        self.anchor.clear();
        self.pinned.clear();
        for k in 0..nodes {
            let at = Fixed::from_int(i64::try_from(k).unwrap_or(0)) / Fixed::from_int(last);
            let p = [
                a[0] + (b[0] - a[0]) * at,
                a[1] + (b[1] - a[1]) * at,
                a[2] + (b[2] - a[2]) * at,
            ];
            self.pos.push(p);
            self.prev.push(p);
            self.anchor.push(if k + 1 == nodes { b } else { p });
            self.pinned.push(k == 0 || k + 1 == nodes);
        }
        self.ticks = 0;
        self.refresh_view();
    }

    /// Pin particle `particle` to `at`, or move its pin. A pinned particle lands there at once.
    pub fn pin(&mut self, particle: usize, at: [Fixed; 3]) -> Result<(), StrandError> {
        if particle >= self.pos.len() {
            return Err(StrandError::BadIndex(particle));
        }
        check_strand_point(at, "pin")?;
        self.anchor[particle] = at;
        self.pinned[particle] = true;
        self.pos[particle] = at;
        self.prev[particle] = at;
        self.refresh_view();
        Ok(())
    }

    /// Release a particle back to free motion; it keeps its place and loses its velocity.
    pub fn unpin(&mut self, particle: usize) -> Result<(), StrandError> {
        let Some(pinned) = self.pinned.get_mut(particle) else {
            return Err(StrandError::BadIndex(particle));
        };
        if *pinned {
            *pinned = false;
            self.prev[particle] = self.pos[particle];
        }
        Ok(())
    }

    /// Replace the tune; a bad one changes nothing.
    pub fn set_params(&mut self, params: StrandParams) -> Result<(), StrandError> {
        params.validate()?;
        self.params = params;
        Ok(())
    }

    /// Replace the wind push (a gust); a bad one changes nothing.
    pub fn set_push(&mut self, push: [i64; 3]) -> Result<(), StrandError> {
        let params = StrandParams {
            push_q16: push,
            ..self.params
        };
        self.set_params(params)
    }

    /// Particle count.
    pub fn particle_count(&self) -> usize {
        self.pos.len()
    }

    /// Link count (`particles - 1`).
    pub fn link_count(&self) -> usize {
        self.pos.len().saturating_sub(1)
    }

    /// True when the particle is pinned; false for an index out of range.
    pub fn is_pinned(&self, particle: usize) -> bool {
        self.pinned.get(particle).copied().unwrap_or(false)
    }

    /// Ticks stepped since construction or the last layout.
    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// Current tune.
    pub fn params(&self) -> StrandParams {
        self.params
    }

    /// Particle positions in meters for the renderer, root first, converted from Q16.
    pub fn positions(&self) -> &[[f32; 3]] {
        &self.view
    }

    /// FNV-1a over every particle's Q16 position bits: the replay check for tests and tapes.
    /// Presentation only; the match hash never reads it.
    pub fn digest(&self) -> u64 {
        let mut h = Fnv64::new();
        for p in &self.pos {
            for v in p {
                h.write_i64(v.to_bits());
            }
        }
        h.finish()
    }

    /// Advance one fixed 1/60 s tick: free particles keep 99% of their motion and fall, then the
    /// links pull taut pairs back toward rest. Never allocates.
    pub fn step(&mut self) {
        self.integrate();
        for _ in 0..self.params.iterations {
            self.solve_links();
        }
        self.clamp_world();
        self.refresh_view();
        self.ticks += 1;
    }

    /// Q16 position Verlet with gravity and wind push; pins land on their anchors exactly. A
    /// free particle's per-tick move is capped so a yanked scalp can never fling the lock.
    fn integrate(&mut self) {
        let keep = Fixed::from_bits(self.params.keep_q16);
        let push = [
            Fixed::from_bits(self.params.push_q16[0]),
            Fixed::from_bits(self.params.push_q16[1] - self.params.gravity_q16),
            Fixed::from_bits(self.params.push_q16[2]),
        ];
        let cap = Fixed::from_bits(MAX_STRAND_STEP_Q16);
        let cap2 = cap * cap;
        let particles = self
            .pos
            .iter_mut()
            .zip(&mut self.prev)
            .zip(&self.anchor)
            .zip(&self.pinned);
        for (((pos, prev), anchor), pinned) in particles {
            if *pinned {
                *pos = *anchor;
                *prev = *anchor;
                continue;
            }
            let cur = *pos;
            let mut moved = [
                (cur[0] - prev[0]) * keep,
                (cur[1] - prev[1]) * keep,
                (cur[2] - prev[2]) * keep,
            ];
            let moved2 = moved[0] * moved[0] + moved[1] * moved[1] + moved[2] * moved[2];
            if moved2 > cap2 {
                let len = moved2.sqrt();
                if len > Fixed::ZERO {
                    let k = cap / len;
                    moved = [moved[0] * k, moved[1] * k, moved[2] * k];
                }
            }
            *prev = cur;
            *pos = [
                cur[0] + moved[0] + push[0],
                cur[1] + moved[1] + push[1],
                cur[2] + moved[2] + push[2],
            ];
        }
    }

    /// One pass over the links in index order. A link longer than its rest length pulls its ends
    /// together, split between the free ends (a pinned end never moves); slack links do nothing.
    fn solve_links(&mut self) {
        let rest = self.rest;
        for i in 0..self.pos.len() - 1 {
            let (pa, pb) = (self.pos[i], self.pos[i + 1]);
            let d = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            if len <= rest {
                continue;
            }
            let share = (len - rest) / len;
            let (ka, kb) = match (self.pinned[i], self.pinned[i + 1]) {
                (true, true) => continue,
                (true, false) => (Fixed::ZERO, share),
                (false, true) => (share, Fixed::ZERO),
                (false, false) => (share * Fixed::HALF, share * Fixed::HALF),
            };
            self.pos[i] = [pa[0] + d[0] * ka, pa[1] + d[1] * ka, pa[2] + d[2] * ka];
            self.pos[i + 1] = [pb[0] - d[0] * kb, pb[1] - d[1] * kb, pb[2] - d[2] * kb];
        }
    }

    /// Hard stop at the world limit, the same +-1000 m the cloth pins obey.
    fn clamp_world(&mut self) {
        let limit = Fixed::from_bits(STRAND_WORLD_LIMIT_Q16);
        for p in &mut self.pos {
            for v in p {
                *v = (*v).clamp(-limit, limit);
            }
        }
    }

    /// Copy the Q16 state to the f32 view the renderer reads.
    fn refresh_view(&mut self) {
        for (out, p) in self.view.iter_mut().zip(&self.pos) {
            *out = [metres(p[0]), metres(p[1]), metres(p[2])];
        }
    }
}

/// Squared length of `b - a` in Q32, square rooted back to Q16 meters.
fn dist_q16(a: [Fixed; 3], b: [Fixed; 3]) -> i64 {
    let d = [
        i128::from((b[0] - a[0]).to_bits()),
        i128::from((b[1] - a[1]).to_bits()),
        i128::from((b[2] - a[2]).to_bits()),
    ];
    let sq: i128 = d.iter().map(|v| v * v).sum();
    let n = u128::try_from(sq).unwrap_or(0);
    i64::try_from(engine_core::fixed::isqrt(n)).unwrap_or(i64::MAX)
}

/// A strand pin must be finite and inside the world.
fn check_strand_point(p: [Fixed; 3], what: &'static str) -> Result<(), StrandError> {
    if p.iter()
        .all(|v| (-STRAND_WORLD_LIMIT_Q16..=STRAND_WORLD_LIMIT_Q16).contains(&v.to_bits()))
    {
        Ok(())
    } else {
        Err(StrandError::BadParam(what))
    }
}

/// Q16 meters as f32 meters: a power-of-two scale, exact inside the world limit.
fn metres(v: Fixed) -> f32 {
    let metres = engine_core::checked::f32_from_u64(v.to_bits().unsigned_abs()) / 65_536.0;
    if v.to_bits() < 0 {
        -metres
    } else {
        metres
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

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::Rng;

    /// A fighter cape: half a meter wide, a meter long, hung from shoulder height.
    const CAPE_SIZE: [f32; 2] = [0.5, 1.0];
    const SHOULDERS: [f32; 3] = [0.0, 1.6, 0.0];

    fn cape_with(tier: ClothTier, tear_strain: Option<f32>) -> Cloth {
        let params = ClothParams {
            tear_strain,
            ..ClothParams::for_tier(tier)
        };
        Cloth::new(tier.knobs().cape, CAPE_SIZE, SHOULDERS, params).expect("cape builds")
    }

    fn cape(tier: ClothTier) -> Cloth {
        cape_with(tier, None)
    }

    /// The cape's top row at the shoulders, moved by `shift`.
    fn shoulder_row(cols: usize, shift: [f32; 3]) -> Vec<[f32; 3]> {
        (0..cols)
            .map(|c| {
                let across = CAPE_SIZE[0] * c as f32 / (cols - 1) as f32;
                [
                    SHOULDERS[0] + across + shift[0],
                    SHOULDERS[1] + shift[1],
                    SHOULDERS[2] + shift[2],
                ]
            })
            .collect()
    }

    /// Triangle wave in `-1..=1` with a 48 tick period; no libm.
    fn tri(tick: usize, phase: usize) -> f32 {
        let k = (tick + phase) % 48;
        let up = if k < 24 { k } else { 48 - k };
        up as f32 / 12.0 - 1.0
    }

    fn run(cloth: &mut Cloth, ticks: usize) {
        for _ in 0..ticks {
            cloth.step(&[]);
        }
    }

    fn sleep_it(cloth: &mut Cloth) {
        for _ in 0..1200 {
            if cloth.is_asleep() {
                return;
            }
            cloth.step(&[]);
        }
        assert!(cloth.is_asleep(), "a still sheet must fall asleep");
    }

    fn finite(cloth: &Cloth) -> bool {
        cloth
            .positions()
            .iter()
            .all(|p| p.iter().all(|v| v.is_finite() && v.abs() < 200.0))
    }

    #[test]
    fn tier_table_grows_with_the_tier_and_fits_the_limits() {
        let mut last = (0, 0, 0, 0);
        for tier in ClothTier::ALL {
            let k = tier.knobs();
            assert_eq!(k.cape.validate(), Ok(()));
            assert_eq!(k.banner.validate(), Ok(()));
            assert!((1..=MAX_SUBSTEPS).contains(&k.substeps));
            assert!((1..=MAX_ITERATIONS).contains(&k.iterations));
            let now = (
                k.cape.particles(),
                k.banner.particles(),
                k.substeps,
                k.iterations,
            );
            assert!(
                now.0 > last.0 && now.1 > last.1 && now.2 >= last.2 && now.3 >= last.3,
                "{tier:?} must cost more than the tier below: {now:?} vs {last:?}"
            );
            last = now;
        }
        let potato = ClothTier::Potato.knobs();
        assert!(potato.cape.particles() <= 32 && potato.banner.particles() <= 32);
        assert_eq!(ClothTier::Comfort.knobs().cape.particles(), 60);
    }

    #[test]
    fn tier_params_follow_the_table_and_sheets_build_to_size() {
        for tier in ClothTier::ALL {
            let k = tier.knobs();
            let params = ClothParams::for_tier(tier);
            assert_eq!(params.validate(), Ok(()));
            assert_eq!(
                (params.substeps, params.iterations),
                (k.substeps, k.iterations)
            );
            assert_eq!(params.tear_strain, None, "tearing is off unless asked for");
            for (grid, size) in [(k.cape, CAPE_SIZE), (k.banner, [3.0, 2.0])] {
                let cloth = Cloth::new(grid, size, SHOULDERS, params).expect("sheet builds");
                let (c, r) = (grid.cols, grid.rows);
                assert_eq!(cloth.particle_count(), c * r);
                assert_eq!(
                    cloth.link_count(),
                    (c - 1) * r + c * (r - 1) + 2 * (c - 1) * (r - 1)
                );
                assert_eq!(cloth.pinned_count(), c, "the top row starts pinned");
                assert!(cloth.is_pinned(c - 1) && !cloth.is_pinned(c));
            }
        }
    }

    #[test]
    fn bad_input_is_refused_before_any_state_changes() {
        let grid = ClothTier::Web.knobs().cape;
        let params = ClothParams::for_tier(ClothTier::Web);
        let build = |g, s, o, p| Cloth::new(g, s, o, p).err();
        let one_wide = ClothGrid { cols: 1, rows: 4 };
        assert_eq!(
            build(one_wide, CAPE_SIZE, SHOULDERS, params),
            Some(ClothError::BadGrid)
        );
        let huge = ClothGrid {
            cols: MAX_SIDE + 1,
            rows: 4,
        };
        assert_eq!(
            build(huge, CAPE_SIZE, SHOULDERS, params),
            Some(ClothError::BadGrid)
        );
        assert_eq!(
            build(grid, [0.0, 1.0], SHOULDERS, params),
            Some(ClothError::BadParam("size"))
        );
        assert_eq!(
            build(grid, [f32::NAN, 1.0], SHOULDERS, params),
            Some(ClothError::BadParam("size"))
        );
        assert_eq!(
            build(grid, CAPE_SIZE, [0.0, f32::INFINITY, 0.0], params),
            Some(ClothError::BadParam("origin"))
        );
        let bad_tunes = [
            ClothParams {
                stiffness: 1.5,
                ..params
            },
            ClothParams {
                iterations: 0,
                ..params
            },
            ClothParams {
                substeps: MAX_SUBSTEPS + 1,
                ..params
            },
            ClothParams {
                air_drag: f32::NAN,
                ..params
            },
            ClothParams {
                wind: [0.0, 0.0, 99.0],
                ..params
            },
            ClothParams {
                tear_strain: Some(-1.0),
                ..params
            },
        ];
        for bad in bad_tunes {
            assert!(bad.validate().is_err(), "{bad:?}");
            assert!(build(grid, CAPE_SIZE, SHOULDERS, bad).is_some(), "{bad:?}");
        }

        let mut cloth = cape(ClothTier::Web);
        let before = cloth.positions().to_vec();
        let short = shoulder_row(grid.cols - 1, [0.0; 3]);
        assert_eq!(
            cloth.pin_top_row(&short),
            Err(ClothError::BadPins {
                expected: grid.cols,
                got: grid.cols - 1
            })
        );
        let mut poisoned = shoulder_row(grid.cols, [0.0; 3]);
        poisoned[2][1] = f32::NAN;
        assert_eq!(
            cloth.pin_top_row(&poisoned),
            Err(ClothError::BadParam("pin"))
        );
        assert_eq!(cloth.pin(999, SHOULDERS), Err(ClothError::BadIndex(999)));
        assert_eq!(cloth.unpin(999), Err(ClothError::BadIndex(999)));
        assert_eq!(
            cloth.set_wind([f32::NAN, 0.0, 0.0]),
            Err(ClothError::BadParam("wind"))
        );
        assert_eq!(cloth.params(), params, "a refused tune changes nothing");
        assert_eq!(cloth.positions(), &*before);
        assert_eq!(
            cloth.reset([f32::NAN; 3]),
            Err(ClothError::BadParam("origin"))
        );
        assert!(ClothCollider::sphere(SHOULDERS, 0.0).is_err());
        assert!(ClothCollider::capsule([f32::NAN; 3], SHOULDERS, 0.1).is_err());
        assert!(ClothCollider::capsule(SHOULDERS, SHOULDERS, f32::INFINITY).is_err());
    }

    #[test]
    fn pins_hold_while_the_free_cloth_swings() {
        let mut cloth = cape(ClothTier::Comfort);
        let cols = cloth.cols;
        let wind = [6.0, 0.0, 2.0];
        cloth.set_wind(wind).expect("wind in range");
        let mut moved = 0.0_f32;
        for t in 0..240 {
            let row = shoulder_row(cols, [0.3 * tri(t, 0), 0.0, 0.2 * tri(t, 12)]);
            cloth.pin_top_row(&row).expect("pins in range");
            cloth.step(&[]);
            for (c, want) in row.iter().enumerate() {
                assert_eq!(
                    &cloth.positions()[c],
                    want,
                    "pin {c} left its shoulder at tick {t}"
                );
            }
            let last = cloth.positions()[cloth.particle_count() - 1];
            moved = moved.max(dist2(last, [0.5, 0.6, 0.0]).sqrt());
        }
        assert!(
            moved > 0.2,
            "the free hem must swing while the pins stay put: {moved}"
        );
    }

    /// Scatter the free particles up to 0.15 m off their rest places, motionless.
    fn crumple(cloth: &mut Cloth, seed: u64) {
        let mut rng = Rng::new(seed);
        let mut jitter = move || rng.next_range(0, 301) as f32 / 1000.0 - 0.15;
        let free = cloth.pos.iter_mut().zip(&mut cloth.prev).zip(&cloth.pinned);
        for ((p, q), pinned) in free {
            if !pinned {
                *p = [p[0] + jitter(), p[1] + jitter(), p[2] + jitter()];
                *q = *p;
            }
        }
    }

    #[test]
    fn a_crumpled_cape_relaxes_to_a_hang_at_rest_length_on_every_tier() {
        for tier in ClothTier::ALL {
            let mut cloth = cape(tier);
            crumple(&mut cloth, 11);
            run(&mut cloth, 300);
            let hem = cloth.positions()[cloth.particle_count() - 1];
            assert!(
                (SHOULDERS[1] - hem[1] - CAPE_SIZE[1]).abs() < 0.1 && hem[2].abs() < 0.02,
                "{tier:?} hem at {hem:?}"
            );
            assert!(
                cloth.max_stretch() < 1.15,
                "{tier:?} stretch {}",
                cloth.max_stretch()
            );
        }
    }

    #[test]
    fn wind_blows_the_hem_downwind() {
        // The cape hangs in the xy plane, so a wind along z hits it face on.
        let mean_hem_z = |wind: [f32; 3]| {
            let mut cloth = cape(ClothTier::Comfort);
            cloth.set_wind(wind).expect("wind in range");
            run(&mut cloth, 180);
            let cols = cloth.cols;
            let hem = &cloth.positions()[cloth.particle_count() - cols..];
            hem.iter().map(|p| p[2]).sum::<f32>() / cols as f32
        };
        assert!(
            mean_hem_z([0.0, 0.0, 8.0]) > 0.3,
            "a steady wind must blow the hem out"
        );
        assert!(
            mean_hem_z([0.0, 0.0, -8.0]) < -0.3,
            "and the other way round"
        );
        assert!(mean_hem_z([0.0; 3]).abs() < 0.02, "no wind, no drift");
    }

    #[test]
    fn no_nan_after_600_ticks_of_gusts_and_a_swinging_body() {
        let mut rng = Rng::new(2040);
        let mut gust = move || rng.next_range(0, 4001) as f32 / 100.0 - 20.0;
        for tier in ClothTier::ALL {
            let mut cloth = cape(tier);
            let mut hem_x = (f32::MAX, f32::MIN);
            for t in 0..600 {
                if t % 12 == 0 {
                    cloth
                        .set_wind([gust(), gust() / 4.0, gust()])
                        .expect("gust in range");
                }
                let sway = 0.4 * tri(t, 0);
                cloth
                    .pin_top_row(&shoulder_row(cloth.cols, [sway, 0.0, 0.2 * tri(t, 12)]))
                    .expect("pins in range");
                let body = ClothCollider::capsule(
                    [0.25 + sway, 0.6, -0.08],
                    [0.25 + sway, 1.5, -0.08],
                    0.14,
                )
                .expect("body");
                cloth.step(&[body]);
                assert!(finite(&cloth), "{tier:?} blew up at tick {t}");
                let hem = cloth.positions()[cloth.particle_count() - 1];
                hem_x = (hem_x.0.min(hem[0]), hem_x.1.max(hem[0]));
            }
            assert!(
                cloth.max_stretch() < 2.0,
                "{tier:?} stretch {}",
                cloth.max_stretch()
            );
            assert!(
                hem_x.1 - hem_x.0 > 0.5,
                "{tier:?}: the gusts must have thrown the hem about: {hem_x:?}"
            );
        }
    }

    /// Thirty settled ticks, then the shoulders jump five meters sideways in one tick.
    fn yanked(tear_strain: Option<f32>) -> Cloth {
        let mut cloth = cape_with(ClothTier::Comfort, tear_strain);
        run(&mut cloth, 30);
        let cols = cloth.cols;
        cloth
            .pin_top_row(&shoulder_row(cols, [5.0, 0.0, 0.0]))
            .expect("pins in range");
        run(&mut cloth, 300);
        cloth
    }

    #[test]
    fn tearing_off_survives_a_violent_yank_whole() {
        let cloth = yanked(None);
        assert!(finite(&cloth));
        assert_eq!(cloth.torn_count(), 0, "tearing is off");
        assert!(cloth.max_stretch() < 1.3, "stretch {}", cloth.max_stretch());
        let hem = cloth.positions()[cloth.particle_count() - 1];
        assert!(hem[0] > 4.5, "the cape must follow its shoulders: {hem:?}");
        assert!(
            hem[1] > SHOULDERS[1] - 1.2,
            "the tethers keep the hem hanging from the pins: {hem:?}"
        );
    }

    #[test]
    fn tearing_on_cuts_a_yanked_cape_and_lets_the_pieces_fall() {
        let cloth = yanked(Some(0.5));
        assert!(finite(&cloth));
        assert!(cloth.torn_count() > 0, "a five meter yank must tear links");
        let hem = cloth.positions()[cloth.particle_count() - 1];
        assert!(
            hem[1] < SHOULDERS[1] - 3.0,
            "the torn piece falls away: {hem:?}"
        );
    }

    #[test]
    fn tearing_on_leaves_gentle_motion_alone() {
        let mut cloth = cape_with(ClothTier::Comfort, Some(0.5));
        let mut hem_x = (f32::MAX, f32::MIN);
        for t in 0..300 {
            let row = shoulder_row(cloth.cols, [0.3 * tri(t, 12), 0.0, 0.0]);
            cloth.pin_top_row(&row).expect("pins in range");
            cloth.step(&[]);
            let hem = cloth.positions()[cloth.particle_count() - 1];
            hem_x = (hem_x.0.min(hem[0]), hem_x.1.max(hem[0]));
        }
        assert!(
            hem_x.1 - hem_x.0 > 0.2,
            "the hem must follow the sway: {hem_x:?}"
        );
        assert_eq!(cloth.torn_count(), 0);
    }

    #[test]
    fn a_still_cape_sleeps_and_wakes_on_pin_wind_or_touch() {
        let mut cloth = cape(ClothTier::Comfort);
        let cols = cloth.cols;
        sleep_it(&mut cloth);
        assert!(
            cloth.ticks() < 600,
            "slept only after {} ticks",
            cloth.ticks()
        );

        let frozen = cloth.positions().to_vec();
        let ticks = cloth.ticks();
        for _ in 0..30 {
            assert!(!cloth.step(&[]), "a sleeping sheet does no work");
        }
        assert_eq!(cloth.positions(), &*frozen);
        assert_eq!(cloth.ticks(), ticks);

        // Sub-millimeter pin drift and the same wind do not wake it.
        cloth
            .pin_top_row(&shoulder_row(cols, [0.0002, 0.0, 0.0]))
            .expect("pins in range");
        cloth.set_wind([0.0; 3]).expect("wind in range");
        assert!(cloth.is_asleep());
        // A collider far away does not either.
        let far = ClothCollider::sphere([9.0, 0.0, 9.0], 0.5).expect("ball");
        assert!(!cloth.step(&[far]));

        // A real pin move wakes it, and it moves.
        cloth
            .pin_top_row(&shoulder_row(cols, [0.05, 0.0, 0.0]))
            .expect("pins in range");
        assert!(!cloth.is_asleep());
        assert!(cloth.step(&[]));
        assert_ne!(cloth.positions(), &*frozen);

        sleep_it(&mut cloth);
        cloth.set_wind([5.0, 0.0, 0.0]).expect("wind in range");
        assert!(!cloth.is_asleep(), "a gust wakes the sheet");

        sleep_it(&mut cloth);
        let hem = cloth.positions()[cloth.particle_count() - 1];
        let poke = ClothCollider::sphere(hem, 0.1).expect("ball");
        assert!(cloth.step(&[poke]), "a collider on the hem wakes the sheet");
    }

    #[test]
    fn a_capsule_keeps_the_cape_off_the_body() {
        // The body stands just behind the hanging cape, so the cape must ride up over it.
        let body = ClothCollider::capsule([0.25, 0.7, -0.1], [0.25, 1.5, -0.1], 0.2).expect("body");
        let mut cloth = cape(ClothTier::Comfort);
        let mut free = Cloth::clone(&cloth);
        for _ in 0..120 {
            cloth.step(&[body]);
            free.step(&[]);
        }
        for (i, p) in cloth.positions().iter().enumerate() {
            if cloth.is_pinned(i) {
                continue;
            }
            let reach = dist2(*p, body.nearest_on_axis(*p)).sqrt();
            assert!(
                reach >= 0.2 - 1e-3,
                "particle {i} sank {reach} m from the axis"
            );
        }
        let pushed = cloth
            .positions()
            .iter()
            .map(|p| p[2])
            .fold(f32::MIN, f32::max);
        assert!(
            pushed > 0.08,
            "the body must push the cape out front: {pushed}"
        );
        let drift = free
            .positions()
            .iter()
            .map(|p| p[2].abs())
            .fold(0.0, f32::max);
        assert!(
            drift < 0.02,
            "without the body the cape stays flat: {drift}"
        );
    }

    #[test]
    fn colliders_push_to_the_surface_and_leave_outside_points() {
        let ball = ClothCollider::sphere([1.0, 1.0, 1.0], 0.5).expect("ball");
        let out = ball.push_out([1.2, 1.0, 1.0]);
        assert!((dist2(out, [1.0, 1.0, 1.0]).sqrt() - 0.5).abs() < 1e-5);
        assert!(
            out[0] > 1.4 && (out[1] - 1.0).abs() < 1e-6,
            "pushed straight out: {out:?}"
        );
        assert_eq!(ball.push_out([3.0, 1.0, 1.0]), [3.0, 1.0, 1.0]);
        assert_eq!(
            ball.push_out([1.0, 1.0, 1.0]),
            [1.0, 1.0, 1.5],
            "dead center leaves along +Z"
        );

        let pole = ClothCollider::capsule([0.0, 0.0, 0.0], [0.0, 2.0, 0.0], 0.25).expect("pole");
        assert_eq!(
            pole.push_out([0.1, 1.0, 0.0]),
            [0.25, 1.0, 0.0],
            "the shell is round around the axis"
        );
        let cap = pole.push_out([0.0, 2.1, 0.0]);
        assert!(
            (cap[1] - 2.25).abs() < 1e-5,
            "and capped at both ends: {cap:?}"
        );
    }

    #[test]
    fn a_banner_pinned_at_two_corners_sags_more_when_the_corners_close_in() {
        let grid = ClothTier::Web.knobs().banner;
        let params = ClothParams::for_tier(ClothTier::Web);
        // Hang a 3 m banner from two corner pins `gap` meters in from its edges; the middle
        // of the top edge is free.
        let sag = |gap: f32| {
            let mut banner = Cloth::new(grid, [3.0, 2.0], [0.0, 4.0, 0.0], params).expect("banner");
            for c in 1..grid.cols - 1 {
                banner.unpin(c).expect("top row index");
            }
            assert_eq!(banner.pinned_count(), 2);
            banner.pin(0, [gap, 4.0, 0.0]).expect("corner");
            banner
                .pin(grid.cols - 1, [3.0 - gap, 4.0, 0.0])
                .expect("corner");
            run(&mut banner, 600);
            assert!(finite(&banner));
            assert_eq!(banner.positions()[0], [gap, 4.0, 0.0], "the corner holds");
            assert!(
                banner.max_stretch() < 1.2,
                "stretch {}",
                banner.max_stretch()
            );
            4.0 - banner.positions()[grid.cols / 2][1]
        };
        let (taut, slack) = (sag(0.0), sag(0.15));
        assert!(taut < 0.3, "a banner at full span barely sags: {taut}");
        assert!(
            slack > taut + 0.2 && slack < 1.5,
            "slack between the corners sags: {slack} vs {taut}"
        );
    }

    #[test]
    fn squeezed_links_do_not_push_back() {
        let mut cloth = cape(ClothTier::Comfort);
        // Shrink the whole sheet to half size about its middle: every link is slack.
        let middle = [0.25, 1.1, 0.0];
        for p in &mut cloth.pos {
            *p = [
                middle[0] + (p[0] - middle[0]) / 2.0,
                middle[1] + (p[1] - middle[1]) / 2.0,
                middle[2],
            ];
        }
        let squeezed = cloth.pos.clone();
        cloth.solve_links();
        assert_eq!(cloth.pos, squeezed, "slack links must not move anything");
        // The same sheet stretched to double size is pulled back toward rest.
        cloth.pos = squeezed
            .iter()
            .map(|p| {
                [
                    middle[0] + (p[0] - middle[0]) * 4.0,
                    middle[1] + (p[1] - middle[1]) * 4.0,
                    middle[2],
                ]
            })
            .collect();
        let excess = |c: &Cloth| -> f32 {
            let over = |l: &Link| (dist2(c.pos[l.a], c.pos[l.b]).sqrt() - l.rest).max(0.0);
            c.links.iter().map(over).sum()
        };
        let before = excess(&cloth);
        cloth.solve_links();
        assert!(
            excess(&cloth) < before,
            "a pass must pull stretched links in"
        );
    }

    #[test]
    fn the_tether_pulls_an_overstretched_particle_back_to_its_reach() {
        let mut cloth = cape(ClothTier::Potato);
        let node = cloth.particle_count() - 1;
        let tether = *cloth
            .tethers
            .iter()
            .find(|t| t.node == node)
            .expect("hem is tethered");
        cloth.pos[node] = [0.0, -10.0, 0.0];
        cloth.solve_tethers();
        let reach = dist2(cloth.pos[node], cloth.pos[tether.pin]).sqrt();
        assert!(
            (reach - tether.max).abs() < 1e-4,
            "reach {reach} vs {}",
            tether.max
        );
        assert!(
            (tether.max - CAPE_SIZE[1]).abs() < 1e-5,
            "straight down is the full hang"
        );
    }

    /// A 32x32 sheet pinned along its top row, dropped crumpled and run 300 ticks.
    fn settled_big_sheet() -> Cloth {
        let params = ClothParams::for_tier(ClothTier::Comfort);
        let grid = ClothGrid { cols: 32, rows: 32 };
        let mut cloth = Cloth::new(grid, [3.1, 3.1], SHOULDERS, params).expect("32x32 sheet");
        crumple(&mut cloth, 7);
        run(&mut cloth, 300);
        cloth
    }

    #[test]
    fn a_32x32_sheet_settles_with_every_link_within_a_quarter_of_rest_and_replays_exactly() {
        let cloth = settled_big_sheet();
        assert!(finite(&cloth));
        assert!(
            cloth.max_stretch() <= 1.25,
            "tethers must carry the long hanging chain: {}",
            cloth.max_stretch()
        );
        let again = settled_big_sheet();
        assert_eq!(cloth.positions(), again.positions(), "bit-exact replay");
    }

    #[test]
    fn the_awake_step_with_pin_moves_wind_and_a_body_never_touches_the_heap() {
        for tier in ClothTier::ALL {
            let mut cloth = cape(tier);
            let body = [
                ClothCollider::capsule([0.25, 0.6, -0.1], [0.25, 1.5, -0.1], 0.15).expect("body"),
            ];
            let rows: Vec<Vec<[f32; 3]>> = (0..120)
                .map(|t| shoulder_row(cloth.cols, [0.3 * tri(t, 0), 0.0, 0.1 * tri(t, 12)]))
                .collect();
            let ((), heap) = engine_core::alloc_probe::measure(|| {
                for (t, row) in rows.iter().enumerate() {
                    cloth.pin_top_row(row).expect("pins in range");
                    if t == 60 {
                        cloth.set_wind([6.0, 0.0, 1.0]).expect("wind in range");
                    }
                    assert!(
                        cloth.step(&body),
                        "the cape must stay awake while pinned moves"
                    );
                }
            });
            assert_eq!(heap.calls, 0, "{tier:?}: cloth tick allocated: {heap:?}");
        }
    }

    #[test]
    fn a_tick_is_one_sixtieth_of_a_second_for_any_substep_count() {
        for substeps in [1, 2, 4, MAX_SUBSTEPS] {
            let params = ClothParams {
                air_drag: 0.0,
                substeps,
                ..ClothParams::for_tier(ClothTier::Comfort)
            };
            let mut cloth = Cloth::new(
                ClothTier::Comfort.knobs().cape,
                CAPE_SIZE,
                SHOULDERS,
                params,
            )
            .expect("cape");
            for c in 0..cloth.cols {
                cloth.unpin(c).expect("top row index");
            }
            let top = cloth.positions()[0][1];
            run(&mut cloth, 60);
            let fall = top - cloth.positions()[0][1];
            let want = 0.5 * 9.81;
            assert!(
                (fall - want).abs() / want < 0.02,
                "{substeps} substeps fell {fall} m in one second, free fall is {want}"
            );
        }
    }

    #[test]
    fn the_same_inputs_give_the_same_bits_twice() {
        let play = || {
            let mut cloth = cape(ClothTier::High);
            let mut rng = Rng::new(7);
            for t in 0..300 {
                if t % 20 == 0 {
                    let w = rng.next_range(0, 1500) as f32 / 100.0;
                    cloth.set_wind([w, 0.0, w / 2.0]).expect("wind in range");
                }
                let row = shoulder_row(cloth.cols, [0.3 * tri(t, 0), 0.0, 0.0]);
                cloth.pin_top_row(&row).expect("pins in range");
                cloth.step(&[]);
            }
            cloth
                .positions()
                .iter()
                .flatten()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>()
        };
        let rest: Vec<u32> = cape(ClothTier::High)
            .positions()
            .iter()
            .flatten()
            .map(|v| v.to_bits())
            .collect();
        assert_eq!(play(), play());
        assert_ne!(play(), rest, "the run must have moved the cape");
    }

    #[test]
    fn reset_puts_a_wrecked_sheet_back_at_rest() {
        let fresh = cape_with(ClothTier::Comfort, Some(0.5));
        let mut cloth = yanked(Some(0.5));
        assert!(cloth.torn_count() > 0);
        cloth.reset(SHOULDERS).expect("origin in range");
        assert_eq!(cloth.positions(), fresh.positions());
        assert_eq!((cloth.torn_count(), cloth.is_asleep()), (0, false));
        assert_eq!(cloth.pinned_count(), cloth.cols);
        run(&mut cloth, 60);
        assert!(cloth.max_stretch() < 1.15);
    }

    /// Wall-clock nanoseconds of `f`. Tests may read the clock; the tick never does.
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn nanos(f: impl FnOnce()) -> u128 {
        let start = std::time::Instant::now();
        f();
        start.elapsed().as_nanos()
    }

    /// One game tick for a crowd: shoulders sway, the wind gusts every half second, each cape
    /// meets its fighter's body.
    fn crowd_tick(capes: &mut [Cloth], bodies: &[ClothCollider], t: usize) {
        for (k, cloth) in capes.iter_mut().enumerate() {
            let sway = [0.3 * tri(t, 5 * k), 0.0, 0.1 * tri(t, 5 * k + 12)];
            let row = shoulder_row(cloth.cols, [1.5 * k as f32 + sway[0], sway[1], sway[2]]);
            cloth.pin_top_row(&row).expect("pins in range");
            if t.is_multiple_of(30) {
                let gust = 2.0 + 6.0 * tri(t / 30, k).abs();
                cloth.set_wind([gust, 0.0, 1.0]).expect("wind in range");
            }
            cloth.step(&bodies[k..=k]);
        }
    }

    #[test]
    fn eight_capes_step_under_half_a_millisecond_in_release() {
        for tier in ClothTier::ALL {
            let knobs = tier.knobs();
            let mut capes: Vec<Cloth> = (0..8)
                .map(|k| {
                    let origin = [1.5 * k as f32, SHOULDERS[1], 0.0];
                    Cloth::new(knobs.cape, CAPE_SIZE, origin, ClothParams::for_tier(tier))
                        .expect("cape")
                })
                .collect();
            let bodies: Vec<ClothCollider> = (0..8)
                .map(|k| {
                    let x = 1.5 * k as f32 + 0.25;
                    ClothCollider::capsule([x, 0.6, -0.1], [x, 1.5, -0.1], 0.15).expect("body")
                })
                .collect();
            let mut t = 0;
            for _ in 0..60 {
                crowd_tick(&mut capes, &bodies, t);
                t += 1;
            }
            let mut per_tick = [0u128; 7];
            for slot in &mut per_tick {
                *slot = nanos(|| {
                    for _ in 0..60 {
                        crowd_tick(&mut capes, &bodies, t);
                        t += 1;
                    }
                }) / 60;
            }
            per_tick.sort_unstable();
            assert!(
                capes.iter().all(|c| !c.is_asleep()),
                "the budget must be measured awake"
            );
            let us = |ns: u128| ns as f64 / 1000.0;
            println!(
                "cloth cost {tier:?}: 8 capes of {} particles, {} substeps x {} passes: median {:.1} us/tick (min {:.1}, max {:.1}; bar 500 us in release)",
                knobs.cape.particles(),
                knobs.substeps,
                knobs.iterations,
                us(per_tick[3]),
                us(per_tick[0]),
                us(per_tick[6]),
            );
            if !cfg!(debug_assertions) {
                assert!(
                    per_tick[3] < 500_000,
                    "{tier:?}: eight capes took {} us per tick",
                    us(per_tick[3])
                );
            }
        }
    }

    /// Q16 meters from a test literal: 1.0 is one meter.
    fn q16(metres: f32) -> Fixed {
        Fixed::from_bits((metres * 65_536.0).round() as i64)
    }

    /// A hair lock and a rope of the sizes a fighter look asks for, both at rest.
    fn strands() -> (Strand, Strand) {
        let hair = Strand::hair([q16(0.0), q16(1.7), q16(0.0)], q16(0.4)).expect("hair builds");
        let rope = Strand::rope(
            [q16(-1.0), q16(2.0), q16(0.0)],
            [q16(1.0), q16(2.0), q16(0.0)],
            12,
        )
        .expect("rope builds");
        (hair, rope)
    }

    fn run_strands(hair: &mut Strand, rope: &mut Strand, ticks: usize) {
        for _ in 0..ticks {
            hair.step();
            rope.step();
        }
    }

    #[test]
    fn hair_hangs_down_and_a_rope_sags_between_its_pins() {
        let (mut hair, mut rope) = strands();
        run_strands(&mut hair, &mut rope, 300);
        let root = hair.positions()[0];
        let tail = hair.positions()[hair.particle_count() - 1];
        assert!(
            tail[1] < root[1] - 0.35,
            "the lock must hang from its root: {tail:?}"
        );
        assert!(tail[0].abs() < 0.05, "it hangs straight: {tail:?}");
        assert_eq!(
            (
                rope.positions()[0],
                rope.positions()[rope.particle_count() - 1]
            ),
            ([-1.0, 2.0, 0.0], [1.0, 2.0, 0.0]),
            "both rope ends hold their pins"
        );
        let mid = rope.positions()[rope.particle_count() / 2][1];
        assert!(mid < 1.9, "the slack rope must sag below its pins: {mid}");
        for strand in [&hair, &rope] {
            let rest = strand.rest.to_bits() as f32 / 65_536.0;
            for pair in strand.positions().windows(2) {
                let len = dist2(pair[0], pair[1]).sqrt();
                assert!(
                    len < rest * 1.5 + 0.001,
                    "a link stretched to {len} m over a {rest} m rest"
                );
            }
        }
    }

    /// One replay of a gusty scalp: the hair root sways, the wind gusts every third of a
    /// second, and the rope ends stay pinned.
    fn strand_play() -> (u64, u64) {
        let (mut hair, mut rope) = strands();
        for t in 0..600_i64 {
            if t % 20 == 0 {
                let gust = (t % 40 - 20) * 400;
                hair.set_push([gust, 0, gust / 2]).expect("push in range");
                rope.set_push([gust / 2, gust, 0]).expect("push in range");
            }
            let sway = Fixed::from_bits((t % 48 - 24) * 120);
            hair.pin(0, [q16(0.0) + sway, q16(1.7), q16(0.0)])
                .expect("root in range");
            hair.step();
            rope.step();
        }
        (hair.digest(), rope.digest())
    }

    #[test]
    fn hair_and_rope_q16_verlet_replays_the_same_bits_twice() {
        let (hair, rope) = strands();
        let rest = (hair.digest(), rope.digest());
        let first = strand_play();
        let again = strand_play();
        assert_eq!(first, again, "Q16 Verlet x2: same bits twice");
        assert_ne!(first.0, rest.0, "the hair must have moved");
        assert_ne!(first.1, rest.1, "the rope must have moved");
    }

    /// One game tick for a head of hair and a rope: the locks sway from their roots, the
    /// wind changes twice a second, and the rope ends stay pinned.
    fn strand_crowd_tick(locks: &mut [Strand], rope: &mut Strand, t: usize) {
        let sway = Fixed::from_bits((t as i64 % 48 - 24) * 120);
        for (k, lock) in locks.iter_mut().enumerate() {
            lock.pin(0, [q16(k as f32 * 0.02) + sway, q16(1.7), q16(0.0)])
                .expect("root in range");
            if t.is_multiple_of(30) {
                let gust = 500 * (1 + t / 30 % 8) as i64;
                lock.set_push([gust, 0, 0]).expect("push in range");
            }
            lock.step();
        }
        if t.is_multiple_of(30) {
            rope.set_push([0, 0, 1500]).expect("push in range");
        }
        rope.step();
    }

    #[test]
    fn a_head_of_hair_and_a_rope_meet_the_half_millisecond_bar() {
        let mut locks: Vec<Strand> = (0..12)
            .map(|k| {
                Strand::new(
                    12,
                    q16(0.35),
                    [q16(k as f32 * 0.02), q16(1.7), q16(0.0)],
                    StrandParams::calm(),
                )
                .expect("lock")
            })
            .collect();
        let mut rope = Strand::rope(
            [q16(-2.0), q16(3.0), q16(0.0)],
            [q16(2.0), q16(3.0), q16(0.0)],
            16,
        )
        .expect("rope");
        let mut t = 0;
        for _ in 0..60 {
            strand_crowd_tick(&mut locks, &mut rope, t);
            t += 1;
        }
        let mut per_tick = [0u128; 7];
        for slot in &mut per_tick {
            *slot = nanos(|| {
                for _ in 0..60 {
                    strand_crowd_tick(&mut locks, &mut rope, t);
                    t += 1;
                }
            }) / 60;
        }
        per_tick.sort_unstable();
        let us = |ns: u128| ns as f64 / 1000.0;
        println!(
            "hair+rope cost: 12 hair locks of 12 nodes + a 16-node rope: median {:.1} us/tick (min {:.1}, max {:.1}; bar 500 us in release)",
            us(per_tick[3]),
            us(per_tick[0]),
            us(per_tick[6]),
        );
        if !cfg!(debug_assertions) {
            assert!(
                per_tick[3] < 500_000,
                "hair and rope took {} us per tick",
                us(per_tick[3])
            );
        }
    }

    #[test]
    fn a_strand_tick_never_touches_the_heap() {
        let (mut hair, mut rope) = strands();
        let ((), heap) = engine_core::alloc_probe::measure(|| {
            for t in 0..120_i64 {
                let sway = Fixed::from_bits((t % 48 - 24) * 120);
                hair.pin(0, [q16(0.0) + sway, q16(1.7), q16(0.0)])
                    .expect("root in range");
                hair.step();
                rope.step();
            }
        });
        assert_eq!(heap.calls, 0, "a strand tick allocated: {heap:?}");
    }

    /// March two physics bodies 240 ticks on the same input; `with_strands` also presents a
    /// hair lock and a rope driven by those bodies every tick. Returns the sim state hash.
    fn sim_march(with_strands: bool) -> u64 {
        let mut space = crate::Space::new();
        space.add(crate::Body {
            x: -1.0,
            y: 4.0,
            vx: 0.4,
            vy: 0.0,
            hx: 0.5,
            hy: 0.5,
        });
        space.add(crate::Body {
            x: 1.0,
            y: 5.0,
            vx: -0.4,
            vy: 0.0,
            hx: 0.5,
            hy: 0.5,
        });
        let (mut hair, mut rope) = strands();
        let rest = (hair.digest(), rope.digest());
        for _ in 0..240 {
            space.step(1.0 / 60.0);
            if with_strands {
                let (b0, b1) = (&space.bodies[0], &space.bodies[1]);
                hair.pin(0, [q16(b0.x), q16(b0.y + 0.6), q16(0.0)])
                    .expect("hair root in range");
                hair.step();
                rope.pin(0, [q16(b0.x), q16(b0.y + 0.3), q16(0.0)])
                    .expect("rope end in range");
                rope.pin(
                    rope.particle_count() - 1,
                    [q16(b1.x), q16(b1.y + 0.3), q16(0.0)],
                )
                .expect("rope end in range");
                rope.step();
            }
        }
        if with_strands {
            assert_ne!(
                (hair.digest(), rope.digest()),
                rest,
                "the presented strands must have moved"
            );
        }
        space.state_hash()
    }

    #[test]
    fn the_match_hash_is_the_same_with_hair_and_rope_on_and_off() {
        let on = sim_march(true);
        let off = sim_march(false);
        assert_eq!(on, off, "hair and rope on and off: equal sim hashes");
        assert_eq!(sim_march(true), on, "and the match hash repeats");
    }
}
