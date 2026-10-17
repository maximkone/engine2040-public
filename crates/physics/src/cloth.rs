//! cloth: CPU Verlet cloth for fighter capes and arena banners.
//!
//! Consumer: the fighter look pins a cape at the shoulders ([`Cloth::pin_top_row`]) and steps it
//! once per sim tick; the arena kit hangs banners from a rod the same way. The renderer reads
//! [`Cloth::positions`]. Presentation only: floats, never hashed, never read by the simulation,
//! so a cloth bug can look wrong but can never desync a replay.
//!
//! Units: meters, seconds, y up. The step is fixed at 60 Hz and takes no time argument; each
//! tick runs `substeps` Verlet substeps with `iterations` link passes each, both from
//! [`ClothParams`] ([`ClothTier`] fills them in). Math: position Verlet with linear air drag
//! toward the wind, Jakobsen tension-only links (structure plus shear), one long-range tether per
//! free particle to its nearest pin, and one-way capsule colliders that push cloth and nothing
//! back. A sleeping cloth costs nothing until a pin moves, the wind changes or a collider
//! touches it. Tearing is off unless `tear_strain` is set.
//!
//! Errors are typed [`ClothError`]; bad caller input is refused before any state changes, and
//! `step` allocates nothing.

use engine_core::vec3::dist2;

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
}
