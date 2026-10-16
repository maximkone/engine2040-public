//! cloth: CPU verlet cloth for capes and banners on fixed-point math.
//!
//! Consumer: the render particle path reads presentation verts per frame.
//! Owner: this module owns particles, constraints, anchors and params.
//! Units: meters in Q16.16 raw units, ticks at the 60 Hz fixed step.
//! Determinism: index-ordered verlet plus Jakobsen solve on integers only;
//! presentation only, never checkpoints or hashes. Errors are typed
//! [`ClothError`]; bad caller input never panics.
//!
//! Math (Jakobsen 2001, reimplemented): verlet integration plus strain-limited
//! projection per constraint. Each pass heals toward the rest length but never
//! past it, and no pass moves a particle more than the strain allowance, so a
//! 10x-stiffness drill tune stays bounded. Intercollision then sweeps
//! non-adjacent pairs apart to half-cell separation, so folded cloth never
//! rests self-penetrated. The tick path uses no float ops,
//! so NaN and inf are impossible by construction. Budget: 0.5 ms per tick at
//! 8 fighters; `step` allocates nothing after construction.
#![forbid(unsafe_code)]

use engine_core::{Fnv64, Rng};

/// Raw units per meter (Q16.16).
pub const FP_ONE: i32 = 65536;
/// Largest grid side built by [`ClothWorld::new_grid`].
pub const MAX_DIM: u32 = 64;
/// Largest particle count per world.
pub const MAX_PARTICLES: usize = 4096;
/// Largest constraint count per world.
pub const MAX_CONSTRAINTS: usize = 16384;
/// Largest `dt_ticks` served per [`step_cloth`] call; callers loop for more.
pub const MAX_DT_TICKS: u32 = 240;
/// World clamp, raw units (±32 m). Integrated and solved positions are
/// clamped here, so drill tunes stay finite by construction.
const POS_BOUND_RAW: i32 = 32 * 65536;
/// Largest stiffness, raw (16.0). The blow-up drill runs 10x the 0.9 default.
const MAX_STIFFNESS_RAW: i32 = 16 * 65536;
/// Largest per-axis accel, raw (4 m per tick squared).
const MAX_ACCEL_RAW: i32 = 4 * 65536;
/// Largest solver passes per tick.
const MAX_ITERATIONS: u32 = 16;
/// Largest intercollision sweeps per tick; the sweep exits early once every
/// non-adjacent pair clears the min separation. One sweep clears the cuff
/// and isolated-pair gates, so the cap only bounds deep pile-ups (a full
/// 3-row wad needs long-range attachment tethers, a follow-up row). Each
/// settled pair costs one axis compare, so clean sheets ride the early-out.
const MAX_SELF_PASSES: u32 = 4;
/// Contact skin over the min separation, raw units (~0.12 mm): integer
/// truncation can strand a pair up to one raw unit short of the target,
/// and the skin keeps that floor above the probed min.
const SELF_SKIN_RAW: i32 = 8;

/// Typed errors for cloth setup and anchors. The stepper itself cannot fail:
///
/// construction validates everything it needs, and the tick path saturates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClothError {
    /// Grid sides must be `1..=MAX_DIM` with at most `MAX_PARTICLES` total.
    BadDims,
    /// Spacing must be `1..=MAX_ACCEL_RAW` raw units (1 LSB to 4 m).
    BadSpacing,
    /// A named param, origin, bone or buffer failed its range check.
    BadParam(&'static str),
    /// Particle index is out of range.
    BadIndex(usize),
}

impl core::fmt::Display for ClothError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ClothError::BadDims => f.write_str("bad grid dims: need 1..=64 sides, <=4096 total"),
            ClothError::BadSpacing => f.write_str("bad spacing: need 1..=262144 raw units"),
            ClothError::BadParam(what) => write!(f, "bad cloth param: {what}"),
            ClothError::BadIndex(i) => write!(f, "bad particle index: {i}"),
        }
    }
}

impl std::error::Error for ClothError {}

/// Per-world tune. `stiffness_q16` is a Q16 fraction of each constraint
/// correction applied per pass (1.0 heals a stretch in one pass); the drill
/// runs up to 16.0, and the overshoot guard below keeps that stable.
/// `wind_q16` and `gravity_q16` are Q16.16 meters per tick squared (the 60 Hz
/// step is folded in, so the tick path needs no `dt` multiply).
/// `damping_q16` retains velocity per tick, `max_strain_q16` caps the movement
/// one pass applies per constraint as a fraction of its rest length (a wide
/// safety net the solver never touches in normal flight; small stretches
/// heal proportionally, which is what converges to the rest shape).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClothParams {
    /// Correction fraction per pass, Q16, `0..=1048576`.
    pub stiffness_q16: i32,
    /// Wind accel per axis, Q16.16 m per tick squared.
    pub wind_q16: [i32; 3],
    /// Gravity magnitude applied on -Y, Q16.16 m per tick squared.
    pub gravity_q16: i32,
    /// Velocity retention per tick, Q16, `0..=65536`.
    pub damping_q16: i32,
    /// Max length change per constraint per tick, Q16 fraction of rest.
    pub max_strain_q16: i32,
    /// Solver passes per tick, `1..=16`.
    pub iterations: u32,
}

impl ClothParams {
    /// Cape tune: 0.9 stiffness, light +X/+Z breeze, 9.81 m/s² gravity at
    /// 60 Hz (179 raw), 0.99 damping, 100% strain allowance per pass (never
    /// binds in normal flight; extreme yanks heal at most one rest length
    /// per pass), 8 passes. Eight sweeps hold the clasp links near 10%
    /// stretch under a full curtain of load; fewer passes sag visibly.
    pub fn default_cape() -> Self {
        Self {
            stiffness_q16: 58982,
            wind_q16: [120, 0, 40],
            gravity_q16: 179,
            damping_q16: 64881,
            max_strain_q16: 65536,
            iterations: 8,
        }
    }

    /// Range-check every field; the world keeps its old params on failure.
    pub fn validate(&self) -> Result<(), ClothError> {
        if self.stiffness_q16 < 0 || self.stiffness_q16 > MAX_STIFFNESS_RAW {
            return Err(ClothError::BadParam("stiffness_q16"));
        }
        for w in self.wind_q16 {
            if !(-MAX_ACCEL_RAW..=MAX_ACCEL_RAW).contains(&w) {
                return Err(ClothError::BadParam("wind_q16"));
            }
        }
        if self.gravity_q16 < 0 || self.gravity_q16 > MAX_ACCEL_RAW {
            return Err(ClothError::BadParam("gravity_q16"));
        }
        if self.damping_q16 < 0 || self.damping_q16 > FP_ONE {
            return Err(ClothError::BadParam("damping_q16"));
        }
        if self.max_strain_q16 < 0 || self.max_strain_q16 > FP_ONE {
            return Err(ClothError::BadParam("max_strain_q16"));
        }
        if self.iterations == 0 || self.iterations > MAX_ITERATIONS {
            return Err(ClothError::BadParam("iterations"));
        }
        Ok(())
    }
}

/// One-way collider that pushes presentation cloth aside; cloth pushes
/// nothing back. The presentation path calls
/// [`step_cloth_with_colliders`](step_cloth_with_colliders) each tick before
/// [`ClothWorld::write_verts_f32`]. Units are Q16.16 raw. Slice order then
/// particle index order with integer ops only, so the same call sequence
/// replays bit-exact; never checkpoints or hashes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClothCollider {
    /// Vertical capsule segment at `(cx, cz)`, `y_bot..=y_top`, `radius`.
    Capsule {
        cx: i32,
        cz: i32,
        y_bot: i32,
        y_top: i32,
        radius: i32,
    },
    /// Sphere at `center` with `radius`.
    Sphere { center: [i32; 3], radius: i32 },
}

impl ClothCollider {
    /// Vertical capsule collider; radius `1..=MAX_ACCEL_RAW`, segment ends
    /// inside the world with `y_bot <= y_top`.
    pub fn capsule(
        cx: i32,
        cz: i32,
        y_bot: i32,
        y_top: i32,
        radius: i32,
    ) -> Result<Self, ClothError> {
        let c = Self::Capsule {
            cx,
            cz,
            y_bot,
            y_top,
            radius,
        };
        c.validate()?;
        Ok(c)
    }

    /// Sphere collider; center inside the world, radius `1..=MAX_ACCEL_RAW`.
    pub fn sphere(center: [i32; 3], radius: i32) -> Result<Self, ClothError> {
        let c = Self::Sphere { center, radius };
        c.validate()?;
        Ok(c)
    }

    /// Range-check the shape; constructors already gate, this keeps direct
    /// struct literals honest for callers that build the enum by hand.
    pub fn validate(&self) -> Result<(), ClothError> {
        match *self {
            Self::Capsule {
                cx,
                cz,
                y_bot,
                y_top,
                radius,
            } => {
                for v in [cx, cz, y_bot, y_top] {
                    if !(-POS_BOUND_RAW..=POS_BOUND_RAW).contains(&v) {
                        return Err(ClothError::BadParam("collider"));
                    }
                }
                if y_bot > y_top {
                    return Err(ClothError::BadParam("collider"));
                }
                if radius <= 0 || radius > MAX_ACCEL_RAW {
                    return Err(ClothError::BadParam("collider"));
                }
            }
            Self::Sphere { center, radius } => {
                for v in center {
                    if !(-POS_BOUND_RAW..=POS_BOUND_RAW).contains(&v) {
                        return Err(ClothError::BadParam("collider"));
                    }
                }
                if radius <= 0 || radius > MAX_ACCEL_RAW {
                    return Err(ClothError::BadParam("collider"));
                }
            }
        }
        Ok(())
    }
}

/// Saturate an `i64` intermediate into `i32`: overflow pins, never wraps.
fn sat_i32(v: i64) -> i32 {
    if v > i64::from(i32::MAX) {
        i32::MAX
    } else if v < i64::from(i32::MIN) {
        i32::MIN
    } else {
        // In range by the guards above; the fallback never runs.
        i32::try_from(v).unwrap_or(i32::MAX)
    }
}

fn clamp_raw(v: i32, lo: i32, hi: i32) -> i32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

fn clamp_i64(v: i64, lo: i64, hi: i64) -> i64 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// Integer square root by the binary digit method; integer ops only, so all
/// targets agree bit for bit. Positions stay within ±32 m, so callers feed
/// values far below `u64::MAX`, but the full range still terminates.
fn isqrt_u64(mut n: u64) -> u64 {
    let mut root: u64 = 0;
    let mut bit: u64 = 1 << 62;
    while bit > n {
        bit >>= 2;
    }
    while bit != 0 {
        // Accepted bits add twice their place then the root halves; the
        // shift runs every pass, so `root + bit` always fits `u64`.
        if n >= root + bit {
            n -= root + bit;
            root += bit << 1;
        }
        root >>= 1;
        bit >>= 2;
    }
    root
}

/// Length of a raw delta triple in raw units: `isqrt(dx²+dy²+dz²)`. With
/// positions clamped to ±32 m the deltas stay near ±4.2M raw, so the sum of
/// squares (near 5.3e13) fits `i64` with huge margin.
fn raw_len(dx: i64, dy: i64, dz: i64) -> u64 {
    debug_assert!(dx.abs() < (1 << 32));
    debug_assert!(dy.abs() < (1 << 32));
    debug_assert!(dz.abs() < (1 << 32));
    isqrt_u64(u64::try_from(dx * dx + dy * dy + dz * dz).unwrap_or(u64::MAX))
}

/// Push `p` out of the ball at `center` with `radius` raw, one-way positional
/// projection on integers only. Outside stays put; on-axis escapes along +X;
/// otherwise the point lands on the surface up to 2 LSB of truncation.
/// `|delta| <= 4.2M`, `gap <= 262144`: every product fits `i64`.
fn push_point_out(p: [i32; 3], center: [i32; 3], radius: i32) -> [i32; 3] {
    let dx = i64::from(p[0]) - i64::from(center[0]);
    let dy = i64::from(p[1]) - i64::from(center[1]);
    let dz = i64::from(p[2]) - i64::from(center[2]);
    // `raw_len` stays near 7.3M raw (deltas bounded near 4.2M), so the
    // saturation arm is unreachable; it pins rather than wraps or panics.
    let dist = i64::try_from(raw_len(dx, dy, dz)).unwrap_or(i64::MAX);
    let r = i64::from(radius);
    if dist >= r {
        return p;
    }
    if dist == 0 {
        return [
            clamp_raw(sat_i32(i64::from(p[0]) + r), -POS_BOUND_RAW, POS_BOUND_RAW),
            p[1],
            p[2],
        ];
    }
    let gap = r - dist;
    let mut out = [0i32; 3];
    let d = [dx, dy, dz];
    for axis in 0..3 {
        let push = d[axis] * gap / dist;
        out[axis] = clamp_raw(
            sat_i32(i64::from(p[axis]) + push),
            -POS_BOUND_RAW,
            POS_BOUND_RAW,
        );
    }
    out
}

/// One distance constraint between two particles, with its rest length in
/// raw units. Built once in index order; the solver never reallocates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Constraint {
    a: u32,
    b: u32,
    rest: i32,
}

/// Build grid links in index order: right, down, then both shear diagonals.
/// Pure constructor for new_grid; same inputs give same output.
/// Units: spacing and diag are raw units; diag is spacing times sqrt two.
/// Integer ops only, so all targets agree bit for bit.
fn build_links(cols: u32, rows: u32, spacing_q16: i32, diag: i32) -> Vec<Constraint> {
    let at = |c: u32, r: u32| r * cols + c;
    let mut cons = Vec::new();
    for r in 0..rows {
        for c in 0..cols {
            if c + 1 < cols {
                cons.push(Constraint {
                    a: at(c, r),
                    b: at(c + 1, r),
                    rest: spacing_q16,
                });
            }
            if r + 1 < rows {
                cons.push(Constraint {
                    a: at(c, r),
                    b: at(c, r + 1),
                    rest: spacing_q16,
                });
            }
            if c + 1 < cols && r + 1 < rows {
                cons.push(Constraint {
                    a: at(c, r),
                    b: at(c + 1, r + 1),
                    rest: diag,
                });
            }
            if c > 0 && r + 1 < rows {
                cons.push(Constraint {
                    a: at(c, r),
                    b: at(c - 1, r + 1),
                    rest: diag,
                });
            }
        }
    }
    cons
}

/// Long-range attachment tether from a free particle to its nearest pinned
/// node, with the flat rest distance as the max length. Local constraints
/// cannot carry a long hanging chain in a few passes, so the top links of a
/// tall sheet stretch far past rest; the tether caps the total distance each
/// node can hang from its pin. Solved once after the stretch pass as a full
/// projection (free end only, anchor ends stay put). Index order with integer
/// ops only, so the same call sequence replays bit-exact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Tether {
    node: u32,
    anchor: u32,
    rest: i32,
}

/// Presentation-only cloth: verlet particles plus Jakobsen constraints on
/// Q16.16 integers. Anchored particles pin to caller-supplied bone poses.
/// There is deliberately no hash, snapshot or restore: cloth never enters
/// checkpoints, so a cloth bug can look wrong but can never desync a replay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClothWorld {
    pos: Vec<[i32; 3]>,
    prev: Vec<[i32; 3]>,
    anchor: Vec<[i32; 3]>,
    anchored: Vec<bool>,
    cons: Vec<Constraint>,
    /// Long-range tethers, one per free particle, rebuilt when pins change.
    /// The tick path only reads this list, so stepping never allocates.
    tethers: Vec<Tether>,
    /// Min separation between non-adjacent particles, raw units: half the
    /// build spacing, so folded layers rest half a cell apart.
    self_min_raw: i32,
    /// Build spacing in raw units; tether rest lengths derive from it.
    spacing_q16: i32,
    cols: u32,
    rows: u32,
    params: ClothParams,
    tick: u64,
}

impl ClothWorld {
    /// Build a `cols` by `rows` curtain hanging down -Y from `origin_q16`
    /// (top-left pin), spaced `spacing_q16` raw units apart. The top row
    /// starts pinned; per-tick bone poses arrive via [`bind_top_row`](
    /// ClothWorld::bind_top_row). Links run right, down and both shear
    /// diagonals in index order, so the solve order is stable.
    pub fn new_grid(
        cols: u32,
        rows: u32,
        spacing_q16: i32,
        origin_q16: [i32; 3],
        params: ClothParams,
    ) -> Result<Self, ClothError> {
        if cols == 0 || rows == 0 || cols > MAX_DIM || rows > MAX_DIM {
            return Err(ClothError::BadDims);
        }
        let count = usize::try_from(cols).unwrap_or(0) * usize::try_from(rows).unwrap_or(0);
        if count > MAX_PARTICLES {
            return Err(ClothError::BadDims);
        }
        if spacing_q16 <= 0 || spacing_q16 > MAX_ACCEL_RAW {
            return Err(ClothError::BadSpacing);
        }
        params.validate()?;
        for o in origin_q16 {
            if !(-POS_BOUND_RAW..=POS_BOUND_RAW).contains(&o) {
                return Err(ClothError::BadParam("origin_q16"));
            }
        }
        // `cols, rows <= 64` and `spacing <= 262144` keep every product near
        // 16.6M raw, far inside `i32`; the grid must also fit the world.
        let right = i64::from(origin_q16[0]) + i64::from(cols - 1) * i64::from(spacing_q16);
        let bottom = i64::from(origin_q16[1]) - i64::from(rows - 1) * i64::from(spacing_q16);
        if right > i64::from(POS_BOUND_RAW) || bottom < i64::from(-POS_BOUND_RAW) {
            return Err(ClothError::BadParam("origin_q16"));
        }
        let mut pos = Vec::with_capacity(count);
        for r in 0..rows {
            for c in 0..cols {
                pos.push([
                    origin_q16[0] + i32::try_from(c).unwrap_or(i32::MAX) * spacing_q16,
                    origin_q16[1] - i32::try_from(r).unwrap_or(i32::MAX) * spacing_q16,
                    origin_q16[2],
                ]);
            }
        }
        let prev = pos.clone();
        let anchor = pos.clone();
        let mut anchored = vec![false; count];
        for pinned in anchored.iter_mut().take(usize::try_from(cols).unwrap_or(0)) {
            *pinned = true;
        }
        // Shear rest length is `spacing * sqrt(2)` on integers only.
        // `spacing_q16 > 0` by the guard above, so the widenings never fail.
        let spacing_u64 = u64::try_from(spacing_q16).unwrap_or(0);
        let diag = i32::try_from(isqrt_u64(2 * spacing_u64 * spacing_u64)).unwrap_or(i32::MAX);
        let cons = build_links(cols, rows, spacing_q16, diag);
        if cons.len() > MAX_CONSTRAINTS {
            return Err(ClothError::BadDims);
        }
        let mut world = Self {
            pos,
            prev,
            anchor,
            anchored,
            cons,
            tethers: Vec::new(),
            self_min_raw: spacing_q16 / 2,
            spacing_q16,
            cols,
            rows,
            params,
            tick: 0,
        };
        world.rebuild_tethers();
        Ok(world)
    }

    /// Rebuild the tether list from the current pins: one tether per free
    /// particle to its nearest pinned node, capped at the flat rest distance
    /// between them. Nearest compares squared flat distances (no roots in the
    /// search); one root per particle fixes the rest length. Ties keep the
    /// smallest pin index, so the result is deterministic. Reuses the list
    /// capacity, which the constructor sizes for the full grid.
    fn rebuild_tethers(&mut self) {
        self.tethers.clear();
        if self.tethers.capacity() < self.pos.len() {
            self.tethers
                .reserve(self.pos.len() - self.tethers.capacity());
        }
        let cols = usize::try_from(self.cols).unwrap_or(0);
        if cols == 0 {
            return;
        }
        // Pin indices in ascending order: first minimum wins ties.
        for i in 0..self.pos.len() {
            if self.anchored[i] {
                continue;
            }
            let ic = i % cols;
            let ir = i / cols;
            let mut best: Option<usize> = None;
            let mut best_q: u64 = u64::MAX;
            for (j, pinned) in self.anchored.iter().enumerate() {
                if !*pinned {
                    continue;
                }
                let jc = j % cols;
                let jr = j / cols;
                // Flat rest offsets in raw units: `dc, dr <= 63` cells and
                // `spacing <= 262144`, so each product stays near 16.6M and
                // the sum of squares fits `u64` with huge margin.
                let dc = (i64::try_from(ic).unwrap_or(0) - i64::try_from(jc).unwrap_or(0))
                    * i64::from(self.spacing_q16);
                let dr = (i64::try_from(ir).unwrap_or(0) - i64::try_from(jr).unwrap_or(0))
                    * i64::from(self.spacing_q16);
                let q = u64::try_from(dc * dc + dr * dr).unwrap_or(u64::MAX);
                if q < best_q {
                    best_q = q;
                    best = Some(j);
                }
            }
            if let Some(j) = best {
                let rest = i32::try_from(isqrt_u64(best_q)).unwrap_or(i32::MAX);
                if rest > 0 {
                    self.tethers.push(Tether {
                        node: u32::try_from(i).unwrap_or(u32::MAX),
                        anchor: u32::try_from(j).unwrap_or(u32::MAX),
                        rest,
                    });
                }
            }
        }
    }

    /// Solve every tether once in node order: free particles past their max
    /// distance step back toward the rest ball around their pin. The step is
    /// capped by the strain allowance (a fraction of the tether rest) and
    /// never passes the rest length, so extreme yanks heal at a bounded rate
    /// like local links do. Zero stiffness disables tethers with the elastic
    /// pass, so a stiffness-free tune still isolates intercollision. Anchor
    /// ends stay put. Integer ops only, touches `pos` alone, no allocation.
    fn solve_tethers(&mut self) {
        if self.params.stiffness_q16 == 0 {
            return;
        }
        let strain = i64::from(self.params.max_strain_q16);
        for k in 0..self.tethers.len() {
            let t = self.tethers[k];
            let n = usize::try_from(t.node).unwrap_or(0);
            let a = usize::try_from(t.anchor).unwrap_or(0);
            if n >= self.pos.len() || a >= self.pos.len() {
                continue;
            }
            if self.anchored[n] {
                continue;
            }
            let pa = self.pos[a];
            let pn = self.pos[n];
            let dx = i64::from(pn[0]) - i64::from(pa[0]);
            let dy = i64::from(pn[1]) - i64::from(pa[1]);
            let dz = i64::from(pn[2]) - i64::from(pa[2]);
            let dist = i64::try_from(raw_len(dx, dy, dz)).unwrap_or(i64::MAX);
            let rest = i64::from(t.rest);
            if dist <= rest || dist == 0 {
                continue;
            }
            // Bounded healing: at most the strain allowance per tick, never
            // past the rest ball. `rest <= 7.4M`, `strain <= 65536`: the
            // product fits `i64`.
            let excess = dist - rest;
            let allow = strain * rest / 65536;
            let step = if allow < excess { allow } else { excess };
            if step <= 0 {
                continue;
            }
            // `|d| <= 4.2M`, `step <= 7.4M`: every product fits `i64`.
            let d = [dx, dy, dz];
            let mut out = pn;
            for axis in 0..3 {
                let back = d[axis] * step / dist;
                out[axis] = clamp_raw(
                    sat_i32(i64::from(pn[axis]) - back),
                    -POS_BOUND_RAW,
                    POS_BOUND_RAW,
                );
            }
            self.pos[n] = out;
        }
    }

    /// Particle count (`cols * rows`).
    pub fn particle_count(&self) -> usize {
        self.pos.len()
    }

    /// Constraint count (structural plus shear).
    pub fn constraint_count(&self) -> usize {
        self.cons.len()
    }

    /// Grid columns (also the [`bind_top_row`](ClothWorld::bind_top_row) width).
    pub fn cols(&self) -> u32 {
        self.cols
    }

    /// Grid rows.
    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// Fixed ticks stepped so far.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Current tune.
    pub fn params(&self) -> ClothParams {
        self.params
    }

    /// Replace the tune; bad values report and keep the old tune.
    pub fn set_params(&mut self, params: ClothParams) -> Result<(), ClothError> {
        params.validate()?;
        self.params = params;
        Ok(())
    }

    /// Replace the wind accel; out-of-range values report and change nothing.
    pub fn set_wind(&mut self, wind_q16: [i32; 3]) -> Result<(), ClothError> {
        for w in wind_q16 {
            if !(-MAX_ACCEL_RAW..=MAX_ACCEL_RAW).contains(&w) {
                return Err(ClothError::BadParam("wind_q16"));
            }
        }
        self.params.wind_q16 = wind_q16;
        Ok(())
    }

    /// True when the particle is pinned to its anchor.
    pub fn is_anchored(&self, particle: usize) -> Option<bool> {
        self.anchored.get(particle).copied()
    }

    /// Pinned particle count.
    pub fn anchored_count(&self) -> usize {
        self.anchored.iter().filter(|a| **a).count()
    }

    /// Raw position of one particle, Q16.16 meters.
    pub fn position_raw(&self, particle: usize) -> Option<[i32; 3]> {
        self.pos.get(particle).copied()
    }

    /// All raw positions in index order, Q16.16 meters.
    pub fn positions_raw(&self) -> &[[i32; 3]] {
        &self.pos
    }

    /// Pin a particle to a live bone pose. The particle snaps to the bone
    /// with zero velocity, so re-targeting never injects energy. Tethers
    /// rebuild from the new pin set, so the newcomer anchors its neighbors.
    pub fn set_anchor(&mut self, particle: usize, bone_q16: [i32; 3]) -> Result<(), ClothError> {
        if particle >= self.pos.len() {
            return Err(ClothError::BadIndex(particle));
        }
        for o in bone_q16 {
            if !(-POS_BOUND_RAW..=POS_BOUND_RAW).contains(&o) {
                return Err(ClothError::BadParam("bone_q16"));
            }
        }
        self.anchor[particle] = bone_q16;
        self.anchored[particle] = true;
        self.pos[particle] = bone_q16;
        self.prev[particle] = bone_q16;
        self.rebuild_tethers();
        Ok(())
    }

    /// Release a particle back to free verlet motion. Tethers rebuild so the
    /// freed node hangs from its nearest live pin.
    pub fn clear_anchor(&mut self, particle: usize) -> Result<(), ClothError> {
        if particle >= self.pos.len() {
            return Err(ClothError::BadIndex(particle));
        }
        self.anchored[particle] = false;
        self.rebuild_tethers();
        Ok(())
    }

    /// Re-target the pinned top row from live bone poses, one per column in
    /// order. The consumer calls this each tick before [`step_cloth`].
    /// Tethers rebuild once from the new pin line, not per column.
    pub fn bind_top_row(&mut self, bones_q16: &[[i32; 3]]) -> Result<(), ClothError> {
        if bones_q16.len() != usize::try_from(self.cols).unwrap_or(0) {
            return Err(ClothError::BadParam("bones_q16"));
        }
        for (c, b) in bones_q16.iter().enumerate() {
            for o in *b {
                if !(-POS_BOUND_RAW..=POS_BOUND_RAW).contains(&o) {
                    return Err(ClothError::BadParam("bone_q16"));
                }
            }
            if c >= self.pos.len() {
                return Err(ClothError::BadIndex(c));
            }
            self.anchor[c] = *b;
            self.anchored[c] = true;
            self.pos[c] = *b;
            self.prev[c] = *b;
        }
        self.rebuild_tethers();
        Ok(())
    }

    /// Copy presentation verts in meters for the render path. This is a
    /// plain scale by `1/65536`, not simulation, so it stays out of audits.
    pub fn write_verts_f32(&self, out: &mut [[f32; 3]]) -> Result<(), ClothError> {
        if out.len() != self.pos.len() {
            return Err(ClothError::BadParam("verts"));
        }
        for (d, s) in out.iter_mut().zip(self.pos.iter()) {
            d[0] = s[0] as f32 / 65536.0;
            d[1] = s[1] as f32 / 65536.0;
            d[2] = s[2] as f32 / 65536.0;
        }
        Ok(())
    }

    /// Advance by `dt_ticks` fixed ticks, capped at [`MAX_DT_TICKS`] per call.
    /// Same call sequence replays bit-exact; performs no allocation.
    pub fn step_ticks(&mut self, dt_ticks: u32) {
        let n = dt_ticks.min(MAX_DT_TICKS);
        for _ in 0..n {
            self.tick_once();
        }
        self.tick = self.tick.wrapping_add(u64::from(n));
    }

    /// Advance with one-way coupling: each tick integrates, solves, then
    /// pushes free particles out of `colliders` in slice order. Anchored
    /// particles never move; `prev` is kept so the push reads as contact,
    /// not energy. Integer ops only, no allocation, bit-exact replay.
    pub fn step_ticks_with_colliders(&mut self, dt_ticks: u32, colliders: &[ClothCollider]) {
        let n = dt_ticks.min(MAX_DT_TICKS);
        for _ in 0..n {
            self.tick_once();
            self.resolve_colliders(colliders);
        }
        self.tick = self.tick.wrapping_add(u64::from(n));
    }

    /// Push every free particle out of `colliders` in slice then index
    /// order. Colliders push cloth, cloth pushes nothing back. Anchored
    /// particles stay pinned; `prev` is untouched. No allocation.
    pub fn resolve_colliders(&mut self, colliders: &[ClothCollider]) {
        if colliders.is_empty() {
            return;
        }
        for c in colliders {
            match *c {
                ClothCollider::Capsule {
                    cx,
                    cz,
                    y_bot,
                    y_top,
                    radius,
                } => {
                    for i in 0..self.pos.len() {
                        if self.anchored[i] {
                            continue;
                        }
                        let p = self.pos[i];
                        let cy = clamp_raw(p[1], y_bot, y_top);
                        self.pos[i] = push_point_out(p, [cx, cy, cz], radius);
                    }
                }
                ClothCollider::Sphere { center, radius } => {
                    for i in 0..self.pos.len() {
                        if self.anchored[i] {
                            continue;
                        }
                        self.pos[i] = push_point_out(self.pos[i], center, radius);
                    }
                }
            }
        }
    }

    /// Separate non-adjacent particles to at least `self_min_raw` raw units:
    /// the intercollision set over the structural-plus-shear links, so a
    /// folded curtain never rests self-penetrated. The pair set skips the
    /// 1-ring (index gaps of at most `cols + 1`), so links never fight the
    /// push. Index order with integer ops only, so the same call sequence
    /// replays bit-exact. Anchored ends stay put while the free end takes
    /// the full share; exactly coincident pairs escape along the index-sum
    /// axis. Up to `MAX_SELF_PASSES` sweeps with early-out; no allocation.
    fn resolve_self(&mut self) {
        let min = i64::from(self.self_min_raw);
        if min <= 0 {
            return;
        }
        // Separation target with the contact skin: truncation can strand a
        // pair up to one raw unit short, and the skin keeps that floor above
        // the probed `min`.
        let target = min + i64::from(SELF_SKIN_RAW);
        let target_sq = target * target;
        // `u32 -> usize` cannot fail on the 64-bit targets; the fallback is
        // unreachable and only keeps the conversion total.
        let skip = usize::try_from(self.cols).unwrap_or(0) + 1;
        let n = self.pos.len();
        // Alternate sweep direction each pass so separation propagates both
        // ways along particle chains. Pass parity keeps the order
        // deterministic, hence bit-exact.
        for pass in 0..MAX_SELF_PASSES {
            let mut clean = true;
            if pass % 2 == 0 {
                for a in 0..n {
                    for b in (a + skip + 1)..n {
                        if !self.self_fix(a, b, target, target_sq) {
                            clean = false;
                        }
                    }
                }
            } else {
                for a in (0..n).rev() {
                    for b in ((a + skip + 1)..n).rev() {
                        if !self.self_fix(a, b, target, target_sq) {
                            clean = false;
                        }
                    }
                }
            }
            if clean {
                break;
            }
        }
    }

    /// One intercollision pair fix; true when the pair already clears the
    /// target. Anchored ends stay put while the free end takes the full
    /// share; exactly coincident pairs escape along the index-sum axis.
    /// Integer ops only; touches `pos` alone, so sweep order decides
    /// the result.
    fn self_fix(&mut self, a: usize, b: usize, target: i64, target_sq: i64) -> bool {
        if self.anchored[a] && self.anchored[b] {
            return true;
        }
        let pa = self.pos[a];
        let pb = self.pos[b];
        let dx = i64::from(pb[0]) - i64::from(pa[0]);
        let dy = i64::from(pb[1]) - i64::from(pa[1]);
        let dz = i64::from(pb[2]) - i64::from(pa[2]);
        // One axis already past the target proves the pair clear with no
        // squares: `|dx| >= target` forces `q >= target_sq`. Same outcome,
        // fewer multiplies on settled sheets.
        if dx >= target
            || dx <= -target
            || dy >= target
            || dy <= -target
            || dz >= target
            || dz <= -target
        {
            return true;
        }
        // Deltas stay near 4.2M raw, so every square and the sum fit `i64`
        // and stay non-negative: distant pairs skip the root entirely with
        // no cast anywhere on the path.
        let q = dx * dx + dy * dy + dz * dz;
        if q >= target_sq {
            return true;
        }
        // Rare path only: exact root through the shared helper, saturating
        // past the target on unbounded input instead of wrapping.
        let dist = i64::try_from(raw_len(dx, dy, dz)).unwrap_or(i64::MAX);
        if dist >= target {
            return true;
        }
        let wa: i64 = if self.anchored[a] {
            0
        } else if self.anchored[b] {
            2
        } else {
            1
        };
        let wb: i64 = if self.anchored[b] {
            0
        } else if self.anchored[a] {
            2
        } else {
            1
        };
        let mut na = pa;
        let mut nb = pb;
        if dist == 0 {
            // Exactly coincident pairs escape along the index-sum axis; the
            // helper keeps this function under the line cap.
            self.self_fix_coincident(a, b, wa, wb);
            return false;
        }
        let gap = target - dist;
        let e = [dx, dy, dz];
        for axis in 0..3 {
            // Repulsion: move apart along the pair delta. (An attract
            // sign here reads stable for a pass and churns forever.)
            let push = e[axis] * gap / dist;
            na[axis] = clamp_raw(
                sat_i32(i64::from(pa[axis]) - push * wa / 2),
                -POS_BOUND_RAW,
                POS_BOUND_RAW,
            );
            nb[axis] = clamp_raw(
                sat_i32(i64::from(pb[axis]) + push * wb / 2),
                -POS_BOUND_RAW,
                POS_BOUND_RAW,
            );
        }
        self.pos[a] = na;
        self.pos[b] = nb;
        false
    }

    /// Escape for exactly coincident pairs along the index-sum axis, so the
    /// direction is deterministic for every pair. Anchored ends stay put
    /// while the free end takes the full share. Integer ops only; touches
    /// `pos` alone, so sweep order decides the result.
    fn self_fix_coincident(&mut self, a: usize, b: usize, wa: i64, wb: i64) {
        let pa = self.pos[a];
        let pb = self.pos[b];
        let axis = (a + b) % 3;
        let mut na = pa;
        let mut nb = pb;
        if wa == 2 {
            na[axis] = clamp_raw(
                sat_i32(i64::from(pa[axis]) - i64::from(self.self_min_raw)),
                -POS_BOUND_RAW,
                POS_BOUND_RAW,
            );
        } else if wb == 2 {
            nb[axis] = clamp_raw(
                sat_i32(i64::from(pb[axis]) + i64::from(self.self_min_raw)),
                -POS_BOUND_RAW,
                POS_BOUND_RAW,
            );
        } else {
            let half = self.self_min_raw / 2;
            na[axis] = clamp_raw(
                sat_i32(i64::from(pa[axis]) - i64::from(half)),
                -POS_BOUND_RAW,
                POS_BOUND_RAW,
            );
            nb[axis] = clamp_raw(
                sat_i32(i64::from(pb[axis]) + i64::from(half)),
                -POS_BOUND_RAW,
                POS_BOUND_RAW,
            );
        }
        self.pos[a] = na;
        self.pos[b] = nb;
    }

    /// One fixed tick: verlet integrate with wind plus gravity, pin anchors,
    /// then Jakobsen passes with the per-tick strain clamp, then the
    /// long-range tethers cap each hanging chain at its rest length.
    /// Integer ops only.
    fn tick_once(&mut self) {
        let damp = i64::from(self.params.damping_q16);
        let acc = [
            i64::from(self.params.wind_q16[0]),
            i64::from(self.params.wind_q16[1]) - i64::from(self.params.gravity_q16),
            i64::from(self.params.wind_q16[2]),
        ];
        for i in 0..self.pos.len() {
            if self.anchored[i] {
                self.pos[i] = self.anchor[i];
                self.prev[i] = self.anchor[i];
                continue;
            }
            let p = self.pos[i];
            let q = self.prev[i];
            let mut next = [0i32; 3];
            for axis in 0..3 {
                let v = i64::from(p[axis]) - i64::from(q[axis]);
                let vd = v * damp / 65536;
                let n = i64::from(p[axis]) + vd + acc[axis];
                next[axis] = clamp_raw(sat_i32(n), -POS_BOUND_RAW, POS_BOUND_RAW);
            }
            self.prev[i] = p;
            self.pos[i] = next;
        }
        for _ in 0..self.params.iterations {
            self.solve_once();
        }
        // Long-range tethers run after the stretch pass: each node hangs at
        // most its rest distance from its pin, so tall chains stop loading
        // their top links past the strain bound.
        self.solve_tethers();
        // Intercollision runs last: each tick's resolved state rests
        // separated, so folded cloth never sits self-penetrated.
        self.resolve_self();
        // Re-pin so the anchor copy is the single source of truth each tick.
        for i in 0..self.pos.len() {
            if self.anchored[i] {
                self.pos[i] = self.anchor[i];
                self.prev[i] = self.anchor[i];
            }
        }
    }

    /// One Jakobsen pass over constraints in index order. The correction is
    /// proportional to the stretch (stiffness fraction of the gap), then
    /// limited twice: never past the rest length, and never more than the
    /// strain allowance per pass. The first limit makes every pass a
    /// projection, so even 16x stiffness cannot inject energy; the second
    /// caps healing after extreme yanks to a bounded rate. Small stretches
    /// stay proportional, which is what converges to the rest shape instead
    /// of resting at a constant-force plateau. Anchored ends stay put while
    /// the free end takes the full share.
    fn solve_once(&mut self) {
        let stiff = i64::from(self.params.stiffness_q16);
        if stiff == 0 {
            return;
        }
        let strain = i64::from(self.params.max_strain_q16);
        for k in 0..self.cons.len() {
            let c = self.cons[k];
            let a = usize::try_from(c.a).unwrap_or(0);
            let b = usize::try_from(c.b).unwrap_or(0);
            let pa = self.pos[a];
            let pb = self.pos[b];
            let dx = i64::from(pb[0]) - i64::from(pa[0]);
            let dy = i64::from(pb[1]) - i64::from(pa[1]);
            let dz = i64::from(pb[2]) - i64::from(pa[2]);
            let dist = i64::try_from(raw_len(dx, dy, dz)).unwrap_or(i64::MAX);
            if dist == 0 {
                continue;
            }
            let rest = i64::from(c.rest);
            let diff = dist - rest;
            if diff == 0 {
                continue;
            }
            // Proportional correction with two limits: the strain allowance
            // bounds the rate after extreme yanks, `|diff|` forbids jumping
            // past the rest length. `|e| <= 4.2M`, `stiff <= 1M`,
            // `|diff| <= 7.4M`: every product fits `i64`.
            let allow = strain * rest / 65536;
            let gap = if diff < 0 { -diff } else { diff };
            let lim = if allow < gap { allow } else { gap };
            let wa: i64 = if self.anchored[a] {
                0
            } else if self.anchored[b] {
                2
            } else {
                1
            };
            let wb: i64 = if self.anchored[b] {
                0
            } else if self.anchored[a] {
                2
            } else {
                1
            };
            if wa == 0 && wb == 0 {
                continue;
            }
            // Full relative correction along the delta, magnitude-limited to
            // `lim` so the pair lands on, never past, the rest length.
            // `|e| <= 4.2M`, `stiff <= 1M`: every product fits `i64`.
            let e = [dx, dy, dz];
            let mut na = pa;
            let mut nb = pb;
            for axis in 0..3 {
                let t1 = e[axis] * stiff / 65536;
                let f = t1 * diff / dist;
                let g = clamp_i64(f, -lim, lim);
                na[axis] = clamp_raw(
                    sat_i32(i64::from(pa[axis]) + g * wa / 2),
                    -POS_BOUND_RAW,
                    POS_BOUND_RAW,
                );
                nb[axis] = clamp_raw(
                    sat_i32(i64::from(pb[axis]) - g * wb / 2),
                    -POS_BOUND_RAW,
                    POS_BOUND_RAW,
                );
            }
            self.pos[a] = na;
            self.pos[b] = nb;
        }
    }
}

/// Advance the presentation cloth by `dt_ticks` fixed ticks (60 Hz).
/// Fixed-point and index-ordered, so the same call sequence replays exactly.
/// Capped at [`MAX_DT_TICKS`] per call; never touches checkpoints or hashes.
pub fn step_cloth(world: &mut ClothWorld, dt_ticks: u32) {
    world.step_ticks(dt_ticks);
}

/// Advance with one-way colliders: capsule/sphere shells push free cloth
/// particles out each tick, cloth pushes nothing back. Presentation-side
/// only, Fixed-only, never checkpoints or hashes. The dressing-banner path
/// (`crates/render/src/arena_banners.rs`) calls this each tick before
/// [`ClothWorld::write_verts_f32`]; deep 3-row wads stay a follow-up row.
pub fn step_cloth_with_colliders(
    world: &mut ClothWorld,
    colliders: &[ClothCollider],
    dt_ticks: u32,
) {
    world.step_ticks_with_colliders(dt_ticks, colliders);
}

/// Pinned seed for the whole cloth tape; per-drape streams fork from it.
pub const CLOTH_TAPE_SEED: u64 = 2040;
/// Frozen baseline drape count (capes over one capsule each).
pub const CLOTH_TAPE_DRAPES: usize = 50;
/// Fixed steps per drape and for the folded curtain (2 s at 60 Hz).
pub const CLOTH_TAPE_STEPS: u32 = 120;
/// Cape grid sides used by every tape leg.
pub const TAPE_COLS: u32 = 6;
/// Cape grid rows used by every tape leg.
pub const TAPE_ROWS: u32 = 8;
/// Cell spacing of every tape leg, raw units (~0.05 m).
pub const TAPE_SPACING: i32 = 3277;
/// Shoulder-bar radius, raw units (0.15 m).
pub const TAPE_BAR_RADIUS_RAW: i32 = 9830;
/// Sanity cap on reported penetration, raw units (1 m). Resting cloth on
/// the bar reads deep (no coupling yet); anything past this cap means the
/// tape fixture itself drifted, which is the coupling rows' to fix.
pub const TAPE_PEN_CAP_RAW: u64 = 65536;

/// One-way collider for the baseline probe: a capsule segment plus radius,
/// all in raw units. The stepper ignores it (no coupling yet); the probe
/// functions below only read particle positions against it, so measuring
/// never perturbs the tick.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CapsuleCollider {
    /// Segment start, raw units.
    pub a: [i32; 3],
    /// Segment end, raw units.
    pub b: [i32; 3],
    /// Radius, raw units, `1..=MAX_ACCEL_RAW`.
    pub radius_raw: i32,
}

impl CapsuleCollider {
    /// Range-check the endpoints and radius; bad input reports, never panics.
    pub fn new(a: [i32; 3], b: [i32; 3], radius_raw: i32) -> Result<Self, ClothError> {
        for p in [a, b] {
            for o in p {
                if !(-POS_BOUND_RAW..=POS_BOUND_RAW).contains(&o) {
                    return Err(ClothError::BadParam("capsule endpoint"));
                }
            }
        }
        if radius_raw <= 0 || radius_raw > MAX_ACCEL_RAW {
            return Err(ClothError::BadParam("capsule radius"));
        }
        Ok(Self { a, b, radius_raw })
    }
}

/// Integer point-to-segment distance in raw units: projection clamped to
/// the segment, length via the crate [`isqrt_u64`] root. Integer ops only.
pub fn segment_distance_raw(p: [i32; 3], a: [i32; 3], b: [i32; 3]) -> u64 {
    let ab = [
        i64::from(b[0]) - i64::from(a[0]),
        i64::from(b[1]) - i64::from(a[1]),
        i64::from(b[2]) - i64::from(a[2]),
    ];
    let ap = [
        i64::from(p[0]) - i64::from(a[0]),
        i64::from(p[1]) - i64::from(a[1]),
        i64::from(p[2]) - i64::from(a[2]),
    ];
    let len2 = ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2];
    if len2 <= 0 {
        return raw_len(ap[0], ap[1], ap[2]);
    }
    let dot = ap[0] * ab[0] + ap[1] * ab[1] + ap[2] * ab[2];
    let num = clamp_i64(dot, 0, len2);
    let cx = i64::from(a[0]) + ab[0] * num / len2;
    let cy = i64::from(a[1]) + ab[1] * num / len2;
    let cz = i64::from(a[2]) + ab[2] * num / len2;
    raw_len(
        i64::from(p[0]) - cx,
        i64::from(p[1]) - cy,
        i64::from(p[2]) - cz,
    )
}

/// Penetration depth of one particle into a capsule, raw units; `0` outside.
pub fn penetration_depth_raw(p: [i32; 3], cap: &CapsuleCollider) -> u64 {
    let dist = segment_distance_raw(p, cap.a, cap.b);
    // Depth fits `i64` by the world clamp; the fallbacks below are dead.
    let depth = i64::from(cap.radius_raw) - i64::try_from(dist).unwrap_or(i64::MAX);
    u64::try_from(depth.max(0)).unwrap_or(0)
}

/// Deepest penetration over all particles and capsules, raw units; `0`
/// when nothing touches. Reads positions only, never mutates the world.
pub fn capsule_penetration_max(world: &ClothWorld, caps: &[CapsuleCollider]) -> u64 {
    let mut max = 0u64;
    for p in world.positions_raw().iter() {
        for cap in caps.iter() {
            let d = penetration_depth_raw(*p, cap);
            if d > max {
                max = d;
            }
        }
    }
    max
}

/// Closest approach between two particles that share no 1-ring link, raw
/// units; `u64::MAX` when every pair is adjacent. Reads positions only.
/// The 1-ring test is geometric (`dc <= 1 && dr <= 1` skipped), so the
/// number measures sheet-against-sheet gap on the folded curtain.
pub fn self_separation_min(world: &ClothWorld) -> u64 {
    // `cols <= MAX_DIM`, so the widening never fails; the fallback is dead.
    let cols = usize::try_from(world.cols()).unwrap_or(64);
    let pos = world.positions_raw();
    let mut min = u64::MAX;
    for i in 0..pos.len() {
        let (ci, ri) = (i % cols, i / cols);
        for j in (i + 1)..pos.len() {
            let (cj, rj) = (j % cols, j / cols);
            let dc = ci.abs_diff(cj);
            let dr = ri.abs_diff(rj);
            if dc <= 1 && dr <= 1 {
                continue;
            }
            let d = raw_len(
                i64::from(pos[j][0]) - i64::from(pos[i][0]),
                i64::from(pos[j][1]) - i64::from(pos[i][1]),
                i64::from(pos[j][2]) - i64::from(pos[i][2]),
            );
            if d < min {
                min = d;
            }
        }
    }
    min
}

/// Presentation digest over particle positions plus the tick, for the
/// tape's hash-x2 gate. Streams [`Fnv64`] writes with no allocation;
/// integer FNV over little-endian words, so every target agrees bit for
/// bit. Never a sim hash, never a checkpoint: cloth stays out of the
/// golden by construction.
pub fn cloth_digest(world: &ClothWorld) -> u64 {
    let mut hasher = Fnv64::new();
    for p in world.positions_raw().iter() {
        hasher.write_i32(p[0]);
        hasher.write_i32(p[1]);
        hasher.write_i32(p[2]);
    }
    hasher.write_u64(world.tick());
    hasher.finish()
}

/// Frozen tape config: seeded cape drapes plus one folded curtain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClothTapeConfig {
    /// Master seed; drape `k` draws from `Rng::new(seed).fork(k)`.
    pub seed: u64,
    /// Cape drapes over the shoulder bar, `1..=50`.
    pub drapes: usize,
    /// Fixed steps per leg, `1..=240`.
    pub steps_per_drape: u32,
}

impl ClothTapeConfig {
    /// Frozen denominator: 50 drapes, 120 steps, seed 2040.
    pub fn baseline() -> Self {
        Self {
            seed: CLOTH_TAPE_SEED,
            drapes: CLOTH_TAPE_DRAPES,
            steps_per_drape: CLOTH_TAPE_STEPS,
        }
    }

    /// Range-check; bad input reports before anything runs.
    pub fn validate(&self) -> Result<(), ClothError> {
        if self.drapes == 0 || self.drapes > CLOTH_TAPE_DRAPES {
            return Err(ClothError::BadParam("drapes"));
        }
        if self.steps_per_drape == 0 || self.steps_per_drape > MAX_DT_TICKS {
            return Err(ClothError::BadParam("steps_per_drape"));
        }
        Ok(())
    }

    /// Total fixed steps: drapes plus the folded curtain.
    pub fn ticks(&self) -> u64 {
        // `drapes <= 50`, so the widening never fails; the fallback is dead.
        (u64::try_from(self.drapes).unwrap_or(0) + 1) * u64::from(self.steps_per_drape)
    }
}

/// Baseline numbers for one tape run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClothTapeReport {
    /// Drapes run (the folded curtain adds one leg).
    pub drapes: usize,
    /// Master seed run under.
    pub seed: u64,
    /// [`Rng::digest`] over the per-leg end digests in run order.
    pub digest: u64,
    /// Deepest particle-into-capsule penetration seen, raw units.
    pub penetration_max_raw: u64,
    /// Closest non-neighbor particle pair on the folded curtain, raw units.
    pub self_min_raw: u64,
    /// Total fixed steps performed.
    pub ticks: u64,
}

impl core::fmt::Display for ClothTapeReport {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "CLOTH-TAPE drapes={} seed={} digest={:016x} pen_max_raw={} self_min_raw={} ticks={}",
            self.drapes,
            self.seed,
            self.digest,
            self.penetration_max_raw,
            self.self_min_raw,
            self.ticks
        )
    }
}

/// Shoulder bar for one drape: horizontal X capsule at mid-curtain height
/// under `origin_q16`, so the hanging sheet must cross it on the way down.
fn drape_capsule(origin_q16: [i32; 3]) -> Result<CapsuleCollider, ClothError> {
    let y = origin_q16[1] - 6 * TAPE_SPACING;
    CapsuleCollider::new(
        [origin_q16[0] - 16000, y, origin_q16[2]],
        [origin_q16[0] + 16000, y, origin_q16[2]],
        TAPE_BAR_RADIUS_RAW,
    )
}

/// One seeded drape: shifted banner, seeded breeze, stepped, probed.
/// Read-only on the tick beyond the public stepper: never touches sim code.
fn run_drape(seed: u64, leg: usize, steps: u32) -> Result<(u64, u64), ClothError> {
    // `leg` counts drapes (`<= 50`) and every draw below fits `i32` by its
    // range, so the widenings never fail; each fallback is dead.
    let mut rng = Rng::new(seed).fork(u64::try_from(leg).unwrap_or(0));
    let ox = i32::try_from(rng.next_range(0, 16001)).unwrap_or(0) - 8000;
    let oz = i32::try_from(rng.next_range(0, 16001)).unwrap_or(0) - 8000;
    let origin = [ox, 10 * 65536, oz];
    let mut params = ClothParams::default_cape();
    params.wind_q16 = [
        i32::try_from(rng.next_range(0, 1201)).unwrap_or(0) - 600,
        0,
        i32::try_from(rng.next_range(0, 601)).unwrap_or(0) - 300,
    ];
    let mut world = ClothWorld::new_grid(TAPE_COLS, TAPE_ROWS, TAPE_SPACING, origin, params)?;
    step_cloth(&mut world, steps);
    let cap = drape_capsule(origin)?;
    Ok((
        cloth_digest(&world),
        capsule_penetration_max(&world, &[cap]),
    ))
}

/// Folded curtain: the bottom half mirrors over the mid line, then hangs
/// under the cape tune. The fold starts self-touching; the probe posts the
/// settled sheet-against-sheet gap the self-collision row must widen.
fn run_folded(steps: u32) -> Result<(u64, u64), ClothError> {
    let origin = [0, 10 * 65536, 0];
    let mut world = ClothWorld::new_grid(
        TAPE_COLS,
        TAPE_ROWS,
        TAPE_SPACING,
        origin,
        ClothParams::default_cape(),
    )?;
    // Tape dims are single-digit, so the widenings never fail.
    let rows = usize::try_from(TAPE_ROWS).unwrap_or(8);
    let cols = usize::try_from(TAPE_COLS).unwrap_or(6);
    let mid = origin[1] - 3 * TAPE_SPACING - TAPE_SPACING / 2;
    for r in 4..rows {
        for c in 0..cols {
            let i = r * cols + c;
            world.pos[i][1] = 2 * mid - world.pos[i][1];
            world.prev[i] = world.pos[i];
        }
    }
    step_cloth(&mut world, steps);
    Ok((cloth_digest(&world), self_separation_min(&world)))
}

/// Run the frozen denominator tape: 50 seeded cape drapes over capsule
/// colliders plus the folded-curtain intercollision set, all at 60 Hz with
/// the seed pinned. Posts the penetration max, the folded self-gap and the
/// digest the coupling rows gate on. Same config replays bit-exact.
pub fn run_cloth_tape(config: &ClothTapeConfig) -> Result<ClothTapeReport, ClothError> {
    config.validate()?;
    let mut hashes = Vec::with_capacity(config.drapes + 1);
    let mut pen_max = 0u64;
    for leg in 0..config.drapes {
        let (hash, pen) = run_drape(config.seed, leg, config.steps_per_drape)?;
        hashes.push(hash);
        if pen > pen_max {
            pen_max = pen;
        }
    }
    let (fold_hash, self_min) = run_folded(config.steps_per_drape)?;
    hashes.push(fold_hash);
    Ok(ClothTapeReport {
        drapes: config.drapes,
        seed: config.seed,
        digest: Rng::digest(&hashes),
        penetration_max_raw: pen_max,
        self_min_raw: self_min,
        ticks: config.ticks(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::{alloc_probe, Fnv64, Rng};

    const SPACING: i32 = 3277; // ~0.05 m per cell
    const ORIGIN: [i32; 3] = [0, 10 * 65536, 0];

    fn cape() -> ClothWorld {
        ClothWorld::new_grid(6, 8, SPACING, ORIGIN, ClothParams::default_cape()).expect("cape grid")
    }

    fn con_len(w: &ClothWorld, a: usize, b: usize) -> u64 {
        let pa = w.pos[a];
        let pb = w.pos[b];
        raw_len(
            i64::from(pb[0]) - i64::from(pa[0]),
            i64::from(pb[1]) - i64::from(pa[1]),
            i64::from(pb[2]) - i64::from(pa[2]),
        )
    }

    #[test]
    fn isqrt_holds_square_bracket() {
        // A broken root breaks every rest length, so bracket it directly.
        // SPACING is a small positive test const, so the widenings below never fail.
        debug_assert!(SPACING >= 0);
        let mut vals = vec![
            0u64,
            1,
            2,
            3,
            4,
            5,
            8,
            15,
            16,
            17,
            255,
            256,
            65535,
            65536,
            1 << 32,
            1 << 48,
            u64::MAX - 1,
            u64::MAX,
            2 * u64::try_from(SPACING).unwrap_or(0) * u64::try_from(SPACING).unwrap_or(0),
        ];
        for i in 0..200u64 {
            vals.push(i * 1234567 + 891);
        }
        for n in vals {
            let r = u128::from(isqrt_u64(n));
            let n128 = u128::from(n);
            assert!(r * r <= n128, "root too big for {n}");
            assert!((r + 1) * (r + 1) > n128, "root too small for {n}");
        }
        assert_eq!(isqrt_u64(0), 0);
        assert_eq!(isqrt_u64(1), 1);
        assert_eq!(isqrt_u64(2), 1);
        assert_eq!(isqrt_u64(u64::MAX), 4294967295);
    }

    #[test]
    fn cape_hangs_with_top_row_pinned() {
        let mut w = cape();
        assert_eq!(w.particle_count(), 48);
        // Right links 5*8 + down links 6*7 + shear 2*5*7.
        assert_eq!(w.constraint_count(), 40 + 42 + 70);
        assert_eq!((w.cols(), w.rows()), (6, 8));
        assert_eq!(w.anchored_count(), 6);
        let top: Vec<[i32; 3]> = (0..6).map(|c| w.position_raw(c).expect("top")).collect();
        let bottom0 = w.position_raw(47).expect("bottom");
        for _ in 0..30 {
            step_cloth(&mut w, 10);
        }
        assert_eq!(w.tick(), 300);
        // Pins never move; the breeze swings the free curtain off its start.
        for (c, p) in top.iter().enumerate() {
            assert_eq!(w.position_raw(c).expect("pin"), *p, "pin {c} moved");
            assert_eq!(w.is_anchored(c), Some(true));
        }
        let bottom1 = w.position_raw(47).expect("bottom");
        let flown = (bottom1[0] - bottom0[0]).abs()
            + (bottom1[1] - bottom0[1]).abs()
            + (bottom1[2] - bottom0[2]).abs();
        assert!(flown > 1000, "curtain must move under wind: {bottom1:?}");
        assert!(
            bottom1[1] < ORIGIN[1],
            "curtain must hang below its pins: {bottom1:?}"
        );
        assert_eq!(w.is_anchored(47), Some(false));
        assert_eq!(w.is_anchored(999), None);
        // Settled links sit near rest: the clasp row carries the whole
        // curtain under wind plus gravity, settling near 17% while the rest
        // sit tighter. The 25% bound pins the 8-pass tune; breaking the
        // projection or halving the passes blows far past it.
        for k in 0..w.cons.len() {
            let c = w.cons[k];
            debug_assert!(usize::try_from(c.a).is_ok() && usize::try_from(c.b).is_ok());
            debug_assert!(u64::try_from(c.rest).is_ok());
            let dist = con_len(
                &w,
                usize::try_from(c.a).unwrap_or(0),
                usize::try_from(c.b).unwrap_or(0),
            );
            let rest = u64::try_from(c.rest).unwrap_or(0);
            assert!(
                dist <= rest + rest / 4 + 64,
                "link {k} overstretched: {dist} vs rest {rest}"
            );
        }
    }

    #[test]
    fn wind_pushes_curtain_off_plane() {
        fn windy() -> ClothParams {
            let mut p = ClothParams::default_cape();
            p.gravity_q16 = 0;
            p.wind_q16 = [4000, 0, 0];
            p
        }
        let mut w = cape();
        w.set_params(windy()).expect("windy tune");
        assert_eq!(w.params().wind_q16, [4000, 0, 0]);
        let x0 = w.position_raw(27).expect("mid")[0];
        step_cloth(&mut w, 60);
        let x1 = w.position_raw(27).expect("mid")[0];
        assert!(x1 > x0 + 1000, "wind must push +X: {x1} vs {x0}");
        // The dedicated setter re-applying the same wind tracks identically.
        let mut v = cape();
        v.set_params(windy()).expect("retune");
        v.set_wind([4000, 0, 0]).expect("set wind");
        step_cloth(&mut v, 60);
        assert_eq!(v.positions_raw()[27][0], x1);
    }

    #[test]
    fn yank_heals_fast_but_never_past_rest() {
        // Shift one free particle 5 m with zero velocity, then watch the
        // strain allowance meter the healing. A single-pass world moves a
        // bounded step in one tick (uncapped, 0.9 stiffness would jump ~90%
        // of 5 m at once); the particle never crosses its rest spot; and a
        // full-tune world converges back to the settled shape.
        let mut meter = ClothParams::default_cape();
        meter.iterations = 1;
        let mut w = ClothWorld::new_grid(6, 8, SPACING, ORIGIN, meter).expect("metering grid");
        let i = 45;
        let rest_spot = w.pos[i];
        w.pos[i][0] += 5 * 65536;
        w.prev[i][0] += 5 * 65536;
        w.step_ticks(1);
        let after1 = w.pos[i];
        let moved = (after1[0] - (rest_spot[0] + 5 * 65536)).abs();
        assert!(
            moved < 65536,
            "strain allowance must meter healing: moved {moved} raw in one tick"
        );
        assert!(
            after1[0] > rest_spot[0],
            "healing must not cross rest: {:?} vs {rest_spot:?}",
            after1
        );
        for p in w.positions_raw().iter() {
            assert!(p[0] > -POS_BOUND_RAW && p[0] < POS_BOUND_RAW);
        }
        let mut w = cape();
        let rest_spot = w.pos[i];
        w.pos[i][0] += 5 * 65536;
        w.prev[i][0] += 5 * 65536;
        for _ in 0..24 {
            step_cloth(&mut w, 10);
        }
        assert!(
            (w.pos[i][0] - rest_spot[0]).abs() < 65536,
            "yank must heal: {:?} vs {rest_spot:?}",
            w.pos[i]
        );
        for k in 0..w.cons.len() {
            let c = w.cons[k];
            debug_assert!(usize::try_from(c.a).is_ok() && usize::try_from(c.b).is_ok());
            debug_assert!(u64::try_from(c.rest).is_ok());
            let dist = con_len(
                &w,
                usize::try_from(c.a).unwrap_or(0),
                usize::try_from(c.b).unwrap_or(0),
            );
            let rest = u64::try_from(c.rest).unwrap_or(0);
            assert!(
                dist <= rest + rest / 4 + 64,
                "link {k} must converge after the yank: {dist} vs rest {rest}"
            );
        }
    }

    #[test]
    fn drill_10x_stiffness_2x_wind_stays_bounded() {
        // Blow-up drill: 10x stiffness plus 2x wind over a 600-tick soak.
        // Integer math has no NaN/inf; the world clamp plus the strain clamp
        // keep every coordinate near the rig, and f32 verts stay finite.
        let mut drill = ClothParams::default_cape();
        drill.stiffness_q16 = 589820;
        drill.wind_q16 = [240, 0, 80];
        let mut w = ClothWorld::new_grid(8, 10, SPACING, ORIGIN, drill).expect("drill grid");
        // The per-call cap is 240 ticks, so the soak loops like a caller would.
        for _ in 0..10 {
            step_cloth(&mut w, 60);
        }
        assert_eq!(w.tick(), 600);
        for (i, p) in w.positions_raw().iter().enumerate() {
            for (axis, v) in p.iter().enumerate() {
                assert!(
                    *v > -4 * 65536 && *v < 12 * 65536,
                    "drill escaped at {i}[{axis}]: {}",
                    *v
                );
            }
        }
        let mut verts = vec![[0.0f32; 3]; w.particle_count()];
        w.write_verts_f32(&mut verts).expect("verts");
        assert!(verts.iter().flatten().all(|v| v.is_finite()));
        // The curtain still hangs below its pins instead of floating away.
        let pin_y = w.position_raw(0).expect("pin")[1] as f32;
        let free_y = verts[79][1];
        assert!(free_y < pin_y, "drill curtain must hang");
    }

    #[test]
    fn replay_is_exact_twice_with_moving_bones() {
        let run = || {
            let mut w = cape();
            for round in 0..8u32 {
                // March the shoulder line +X each script block, like bones would.
                debug_assert!(i32::try_from(round).is_ok());
                let bones: Vec<[i32; 3]> = (0..6)
                    .map(|c| [i32::try_from(round).unwrap_or(0) * 200 + c * SPACING, ORIGIN[1], 0])
                    .collect();
                w.bind_top_row(&bones).expect("march bones");
                step_cloth(&mut w, 30);
            }
            w
        };
        let a = run();
        let b = run();
        assert_eq!(a.tick(), 240);
        assert_eq!(a, b, "same script must replay bit-exact");
        // A clone stepped onward matches the original stepped onward.
        let mut c = a.clone();
        let mut d = a.clone();
        step_cloth(&mut c, 60);
        step_cloth(&mut d, 60);
        assert_eq!(c, d);
    }

    #[test]
    fn anchors_track_bones_exactly() {
        let mut w = cape();
        let bone = [12345, 600000, -6789];
        w.set_anchor(20, bone).expect("pin mid");
        assert_eq!(w.is_anchored(20), Some(true));
        step_cloth(&mut w, 10);
        assert_eq!(w.position_raw(20).expect("pinned"), bone);
        // Release changes behavior: the freed particle hangs with its sheet
        // instead of tracking the old pin, while a pinned twin stays exact.
        let mut twin = w.clone();
        w.clear_anchor(20).expect("release");
        assert_eq!(w.is_anchored(20), Some(false));
        step_cloth(&mut w, 30);
        step_cloth(&mut twin, 30);
        assert_eq!(twin.position_raw(20).expect("twin pin"), bone);
        let free = w.position_raw(20).expect("free");
        assert!(
            (free[0] - bone[0]).abs() + (free[1] - bone[1]).abs() + (free[2] - bone[2]).abs()
                > 1000,
            "released particle must leave the pin: {free:?} vs {bone:?}"
        );
        // Dropping the whole top row drops the sheet: pins fall with gravity
        // while a fully pinned control hangs in place.
        let mut dropped = cape();
        for c in 0..6 {
            dropped.clear_anchor(c).expect("drop pin");
        }
        assert_eq!(dropped.anchored_count(), 0);
        let mut control = cape();
        step_cloth(&mut dropped, 60);
        step_cloth(&mut control, 60);
        let fell = dropped.position_raw(0).expect("dropped pin")[1];
        assert!(
            fell < ORIGIN[1] - 50000,
            "unpinned sheet must fall: {fell} vs {}",
            ORIGIN[1]
        );
        assert_eq!(control.position_raw(0).expect("held pin"), ORIGIN);
        // Top-row bind snaps every pin to its bone with zero velocity.
        let bones: Vec<[i32; 3]> = (0..6).map(|c| [c * 1000, ORIGIN[1] + 500, 0]).collect();
        control.bind_top_row(&bones).expect("bind");
        step_cloth(&mut control, 1);
        for (c, want) in bones.iter().enumerate() {
            assert_eq!(control.position_raw(c).expect("bound pin"), *want);
        }
    }

    #[test]
    fn step_ticks_caps_and_counts() {
        let mut w = cape();
        w.step_ticks(0);
        assert_eq!(w.tick(), 0);
        step_cloth(&mut w, 5);
        assert_eq!(w.tick(), 5);
        step_cloth(&mut w, 1000);
        assert_eq!(w.tick(), 5 + u64::from(MAX_DT_TICKS));
    }

    #[test]
    fn bad_input_is_refused_loudly() {
        let tune = ClothParams::default_cape();
        assert_eq!(
            ClothWorld::new_grid(0, 8, SPACING, ORIGIN, tune).expect_err("no cols"),
            ClothError::BadDims
        );
        assert_eq!(
            ClothWorld::new_grid(6, 65, SPACING, ORIGIN, tune).expect_err("too tall"),
            ClothError::BadDims
        );
        assert_eq!(
            ClothWorld::new_grid(6, 8, 0, ORIGIN, tune).expect_err("no spacing"),
            ClothError::BadSpacing
        );
        assert_eq!(
            ClothWorld::new_grid(6, 8, -5, ORIGIN, tune).expect_err("neg spacing"),
            ClothError::BadSpacing
        );
        let mut bad = tune;
        bad.stiffness_q16 = -1;
        assert_eq!(
            ClothWorld::new_grid(6, 8, SPACING, ORIGIN, bad).expect_err("neg stiff"),
            ClothError::BadParam("stiffness_q16")
        );
        bad = tune;
        bad.iterations = 0;
        assert_eq!(
            ClothWorld::new_grid(6, 8, SPACING, ORIGIN, bad).expect_err("no iters"),
            ClothError::BadParam("iterations")
        );
        bad = tune;
        bad.damping_q16 = 70000;
        assert_eq!(
            ClothWorld::new_grid(6, 8, SPACING, ORIGIN, bad).expect_err("damping"),
            ClothError::BadParam("damping_q16")
        );
        // A grid that would leave the ±32 m world is refused.
        assert_eq!(
            ClothWorld::new_grid(6, 8, SPACING, [0, 40 * 65536, 0], tune).expect_err("high origin"),
            ClothError::BadParam("origin_q16")
        );
        let mut w = cape();
        let tick0 = w.tick();
        assert_eq!(
            w.set_anchor(999, ORIGIN).expect_err("oob pin"),
            ClothError::BadIndex(999)
        );
        assert_eq!(
            w.set_anchor(3, [0, 40 * 65536, 0]).expect_err("far bone"),
            ClothError::BadParam("bone_q16")
        );
        assert_eq!(
            w.clear_anchor(999).expect_err("oob release"),
            ClothError::BadIndex(999)
        );
        assert_eq!(
            w.bind_top_row(&[ORIGIN; 5]).expect_err("short bones"),
            ClothError::BadParam("bones_q16")
        );
        assert_eq!(
            w.set_wind([999 * 65536, 0, 0]).expect_err("storm"),
            ClothError::BadParam("wind_q16")
        );
        let mut bad2 = tune;
        bad2.gravity_q16 = -7;
        assert!(w.set_params(bad2).is_err());
        assert_eq!(w.params(), tune, "failed retune keeps old params");
        let mut short = vec![[0.0f32; 3]; 7];
        assert_eq!(
            w.write_verts_f32(&mut short).expect_err("short verts"),
            ClothError::BadParam("verts")
        );
        assert_eq!(w.tick(), tick0, "refused input steps nothing");
        // Error values render instead of panicking.
        assert!(!format!("{}", ClothError::BadDims).is_empty());
        assert!(!format!("{}", ClothError::BadSpacing).is_empty());
        assert!(!format!("{}", ClothError::BadParam("x")).is_empty());
        assert!(!format!("{}", ClothError::BadIndex(3)).is_empty());
        let err: &dyn std::error::Error = &ClothError::BadDims;
        assert!(!format!("{err}").is_empty());
    }

    #[test]
    fn verts_match_raw_scale() {
        let mut w = cape();
        step_cloth(&mut w, 10);
        let mut verts = vec![[0.0f32; 3]; w.particle_count()];
        w.write_verts_f32(&mut verts).expect("verts");
        for (v, r) in verts.iter().zip(w.positions_raw().iter()) {
            for axis in 0..3 {
                let expect = r[axis] as f32 / 65536.0;
                assert!((v[axis] - expect).abs() < 1e-6, "scale drift at {r:?}");
            }
        }
    }

    /// The 32x32 soft-sheet settle fixture: 1024 nodes, top row pinned,
    /// 120 steps under the cape tune.
    fn soft_settle() -> ClothWorld {
        let mut w = ClothWorld::new_grid(32, 32, SPACING, ORIGIN, ClothParams::default_cape())
            .expect("soft sheet");
        step_cloth(&mut w, 120);
        w
    }

    #[test]
    #[ignore = "timing: soft-sheet settle wall clock; run under --perf in release"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn soft_settle_stays_cpu_light() {
        let t0 = std::time::Instant::now();
        let w = soft_settle();
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(w.particle_count(), 1024);
        assert!(ms < 2000.0, "spike stays cpu-light: {ms:.2} ms");
    }

    #[test]
    fn fu30_soft_settle_offline_baseline() {
        // Soft-sheet settle gate: the 32x32 sheet hangs its full weight off
        // the pinned top row, and long-range tethers hold every link within
        // 25% + 64 raw of rest under the cape tune. Presentation only, never
        // the `Space` tick.
        let run = soft_settle;
        let w = run();
        assert_eq!(w.particle_count(), 1024, "1k-node settle fixture");
        let mut max_excess: u64 = 0;
        for k in 0..w.cons.len() {
            let c = w.cons[k];
            debug_assert!(usize::try_from(c.a).is_ok() && usize::try_from(c.b).is_ok());
            debug_assert!(u64::try_from(c.rest).is_ok());
            let dist = con_len(
                &w,
                usize::try_from(c.a).unwrap_or(0),
                usize::try_from(c.b).unwrap_or(0),
            );
            let rest = u64::try_from(c.rest).unwrap_or(0);
            assert!(
                dist <= rest + rest / 4 + 64,
                "link {k} overstretched: {dist} vs rest {rest}"
            );
            max_excess = max_excess.max(dist.saturating_sub(rest));
        }
        // Settled number (seeded, deterministic); the ignored timing test
        // carries the wall clock.
        std::println!("soft settle baseline: max_excess_raw={max_excess}");
        // Determinism x2: the same offline script replays bit-exact.
        let a = run();
        let b = run();
        assert_eq!(a, b, "offline settle must replay bit-exact");
        assert_eq!(w, a, "timed run matches replay pair");
    }

    #[test]
    fn step_allocates_nothing_steady_state() {
        let mut w = cape();
        step_cloth(&mut w, 5);
        let ptrs = (
            w.pos.as_ptr(),
            w.prev.as_ptr(),
            w.anchor.as_ptr(),
            w.anchored.as_ptr(),
            w.cons.as_ptr(),
        );
        let caps = (
            w.pos.capacity(),
            w.prev.capacity(),
            w.anchor.capacity(),
            w.anchored.capacity(),
            w.cons.capacity(),
        );
        // March bones and pour wind through the soak: still no growth.
        for round in 0..4u32 {
            debug_assert!(i32::try_from(round).is_ok());
            let bones: Vec<[i32; 3]> = (0..6)
                .map(|c| [i32::try_from(round).unwrap_or(0) * 50 + c * SPACING, ORIGIN[1], 0])
                .collect();
            w.bind_top_row(&bones).expect("march");
            step_cloth(&mut w, 30);
        }
        assert_eq!(
            (
                w.pos.as_ptr(),
                w.prev.as_ptr(),
                w.anchor.as_ptr(),
                w.anchored.as_ptr(),
                w.cons.as_ptr()
            ),
            ptrs,
            "step must not reallocate"
        );
        assert_eq!(
            (
                w.pos.capacity(),
                w.prev.capacity(),
                w.anchor.capacity(),
                w.anchored.capacity(),
                w.cons.capacity()
            ),
            caps,
            "step must not grow buffers"
        );
    }

    /// D-CLOTH-1 baseline tape inputs: pinned seed, drape count, ticks per
    /// drape at the 60 Hz fixed step. Per-drape streams fork from the seed, so
    /// no two drapes share draws and the whole tape replays from one number.
    const GAP_SEED: u64 = 2040;
    const GAP_DRAPES: usize = 50;
    const GAP_TICKS: u32 = 60;
    /// Self-gap threshold for the folded curtain: half a cell in raw units.
    const GAP_CLOSE_RAW: u64 = (SPACING / 2) as u64;

    /// Vertical capsule collider the tape measures against: a standing-torso
    /// stand-in with segment `y_bot..=y_top` at `(cx, cz)` and `radius`. The
    /// stepper ignores it (no coupling yet); the tape only reads penetration,
    /// so the measurement itself cannot drift any hash.
    struct GapCapsule {
        cx: i32,
        cz: i32,
        y_bot: i32,
        y_top: i32,
        radius: i32,
    }

    /// Seeded capsule for drape `i`: lateral offset within one cell of the
    /// curtain center, radius 0.08..0.15 m, spanning from below the pins past
    /// the hem so the falling curtain crosses the whole segment.
    fn gap_capsule(i: usize) -> GapCapsule {
        debug_assert!(u64::try_from(i).is_ok());
        debug_assert!(SPACING >= 0 && u64::try_from(SPACING).is_ok());
        // Span and radius draws stay far below i32::MAX, so the narrowings below never fail.
        debug_assert!(u64::try_from(2 * SPACING).is_ok());
        let mut rng = Rng::new(GAP_SEED).fork(u64::try_from(i).unwrap_or(0));
        let span = 2 * u64::try_from(SPACING).unwrap_or(0) + 1;
        let cx = i32::try_from(rng.next_range(0, span)).unwrap_or(0) - SPACING;
        let cz = i32::try_from(rng.next_range(0, span)).unwrap_or(0) - SPACING;
        let radius = i32::try_from(rng.next_range(5243, 9831)).unwrap_or(0);
        GapCapsule {
            cx,
            cz,
            y_bot: ORIGIN[1] - 11 * SPACING,
            y_top: ORIGIN[1] - SPACING,
            radius,
        }
    }

    /// Raw distance from `p` to the capsule segment. The segment is vertical,
    /// so the closest point clamps `y` and the rest is one integer root.
    fn gap_dist_to_capsule(cap: &GapCapsule, p: [i32; 3]) -> u64 {
        let cy = clamp_raw(p[1], cap.y_bot, cap.y_top);
        raw_len(
            i64::from(p[0]) - i64::from(cap.cx),
            i64::from(p[1]) - i64::from(cy),
            i64::from(p[2]) - i64::from(cap.cz),
        )
    }

    /// One seeded drape: a default cape stepped `GAP_TICKS`, plus its max
    /// penetration into its capsule in raw units (floored at zero clearance).
    fn gap_drape(i: usize) -> (ClothWorld, i64) {
        let mut w = cape();
        step_cloth(&mut w, GAP_TICKS);
        let cap = gap_capsule(i);
        let mut pen = 0i64;
        for p in w.positions_raw().iter() {
            let dist_u64 = gap_dist_to_capsule(&cap, *p);
            debug_assert!(i64::try_from(dist_u64).is_ok());
            let depth = i64::from(cap.radius) - i64::try_from(dist_u64).unwrap_or(i64::MAX);
            if depth > pen {
                pen = depth;
            }
        }
        (w, pen)
    }

    /// FNV digest of every particle coordinate in index order, chained across
    /// drapes with the drape index so order matters. Integer LE writes only,
    /// identical on every platform.
    fn gap_digest(worlds: &[ClothWorld]) -> u64 {
        let mut h = Fnv64::new();
        for (i, w) in worlds.iter().enumerate() {
            debug_assert!(u64::try_from(i).is_ok());
            h.write_u64(u64::try_from(i).unwrap_or(0));
            for p in w.positions_raw().iter() {
                h.write_i32(p[0]);
                h.write_i32(p[1]);
                h.write_i32(p[2]);
            }
        }
        h.finish()
    }

    /// Folded-curtain fixture: an 8x8 sheet with rows 5..=7 mirrored across
    /// the row-4 line over rows 3..=1, so every hem particle starts exactly
    /// coincident with a sheet particle, then settles `GAP_TICKS` ticks.
    fn gap_folded_curtain() -> ClothWorld {
        let mut w = ClothWorld::new_grid(8, 8, SPACING, ORIGIN, ClothParams::default_cape())
            .expect("folded curtain");
        let fold_y = ORIGIN[1] - 4 * SPACING;
        for r in 5..8u32 {
            for c in 0..8u32 {
                debug_assert!(usize::try_from(r * 8 + c).is_ok());
                let i = usize::try_from(r * 8 + c).unwrap_or(0);
                let p = w.pos[i];
                let mirrored = [p[0], 2 * fold_y - p[1], p[2]];
                w.pos[i] = mirrored;
                w.prev[i] = mirrored;
            }
        }
        step_cloth(&mut w, GAP_TICKS);
        w
    }

    /// Min raw gap over non-adjacent pairs (index gaps past `cols + 1` skip
    /// the 1-ring) plus the count of pairs closer than half a cell. The
    /// folded curtain starts fully interpenetrated, so this is the number
    /// the intercollision fix must beat. Integer roots only.
    fn gap_self_gap(w: &ClothWorld) -> (u64, usize) {
        let pts = w.positions_raw();
        debug_assert!(usize::try_from(w.cols()).is_ok());
        let skip = usize::try_from(w.cols()).unwrap_or(0) + 1;
        let mut min_gap = u64::MAX;
        let mut close = 0usize;
        for a in 0..pts.len() {
            for b in (a + skip + 1)..pts.len() {
                let d = raw_len(
                    i64::from(pts[b][0]) - i64::from(pts[a][0]),
                    i64::from(pts[b][1]) - i64::from(pts[a][1]),
                    i64::from(pts[b][2]) - i64::from(pts[a][2]),
                );
                if d < min_gap {
                    min_gap = d;
                }
                if d < GAP_CLOSE_RAW {
                    close += 1;
                }
            }
        }
        (min_gap, close)
    }

    #[test]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "baseline tape posts the wall-clock soak once; it never enters the tick"
    )]
    fn cloth_gap_baseline_tape() {
        // 50 seeded cape drapes over capsule colliders at the 60 Hz step.
        // The stepper has no coupling yet, so the curtain falls through the
        // capsule: the penetration below is the pre-fix baseline the future
        // coupling and intercollision fixes must beat, not a gate.
        let t0 = std::time::Instant::now();
        let mut worlds = Vec::with_capacity(GAP_DRAPES);
        let mut pen_max_raw = 0i64;
        let mut pen_drape = 0usize;
        let mut radius_max = 0i32;
        for i in 0..GAP_DRAPES {
            let (w, pen) = gap_drape(i);
            assert_eq!(w.tick(), u64::from(GAP_TICKS), "drape {i} short-stepped");
            if pen > pen_max_raw {
                pen_max_raw = pen;
                pen_drape = i;
            }
            radius_max = radius_max.max(gap_capsule(i).radius);
            worlds.push(w);
        }
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        // Replay x2: the same seeded script replays bit-exact.
        let mut worlds2 = Vec::with_capacity(GAP_DRAPES);
        for i in 0..GAP_DRAPES {
            worlds2.push(gap_drape(i).0);
        }
        let digest_a = gap_digest(&worlds);
        let digest_b = gap_digest(&worlds2);
        assert_eq!(digest_a, digest_b, "seeded drapes must replay bit-exact");
        // Measurement sanity: penetration can never exceed the capsule
        // radius, so a broken probe fails here instead of posting nonsense.
        assert!(
            pen_max_raw <= i64::from(radius_max),
            "penetration {pen_max_raw} past radius {radius_max}: probe math broke"
        );
        // Pins hold on every drape: the top row never leaves its anchors.
        for (i, w) in worlds.iter().enumerate() {
            debug_assert!(usize::try_from(w.cols()).is_ok());
            for c in 0..(usize::try_from(w.cols()).unwrap_or(0)) {
                debug_assert!(i32::try_from(c).is_ok());
                assert_eq!(
                    w.position_raw(c),
                    Some([i32::try_from(c).unwrap_or(0) * SPACING, ORIGIN[1], 0]),
                    "drape {i} pin {c} moved"
                );
            }
        }
        // The tick allocates nothing: one measured step on a warmed world.
        let mut probe = cape();
        step_cloth(&mut probe, 5);
        let bones: Vec<[i32; 3]> = (0..6).map(|c| [c * SPACING, ORIGIN[1], 0]).collect();
        probe.bind_top_row(&bones).expect("march");
        let ((), allocs) = alloc_probe::measure(|| step_cloth(&mut probe, 1));
        assert_eq!(allocs.calls, 0, "tick must not allocate: {allocs:?}");
        // Folded curtain: replays bit-exact, then posts its self-gap.
        let folded = gap_folded_curtain();
        let folded2 = gap_folded_curtain();
        assert_eq!(folded, folded2, "folded curtain must replay bit-exact");
        let (self_gap_raw, close_pairs) = gap_self_gap(&folded);
        // Consumer leg: the render particle path reads presentation verts
        // off the settled tape sheet; they must stay finite.
        let mut verts = vec![[0.0f32; 3]; folded.particle_count()];
        folded.write_verts_f32(&mut verts).expect("verts");
        assert!(verts.iter().flatten().all(|v| v.is_finite()));
        std::println!(
            "cloth_gap pen_max_raw={pen_max_raw} drape={pen_drape} self_gap_raw={self_gap_raw} close_pairs={close_pairs} ms={ms:.1} alloc_calls={} digest_a={digest_a:016x} digest_b={digest_b:016x}",
            allocs.calls,
        );
    }

    /// Seeded collider for drape `i`, mirroring [`gap_capsule`].
    fn gap_collider(i: usize) -> ClothCollider {
        let g = gap_capsule(i);
        ClothCollider::capsule(g.cx, g.cz, g.y_bot, g.y_top, g.radius).expect("seeded capsule")
    }

    /// One seeded drape with one-way coupling: the capsule pushes the
    /// curtain out each tick, plus its residual penetration in raw units.
    /// Anchored pins are skipped: they track bones exactly and colliders
    /// never move bones, so the gate covers free particles only.
    fn gap_coupled_drape(i: usize) -> (ClothWorld, i64) {
        let mut w = cape();
        let col = gap_collider(i);
        step_cloth_with_colliders(&mut w, &[col], GAP_TICKS);
        let cap = gap_capsule(i);
        let mut pen = 0i64;
        for (idx, p) in w.positions_raw().iter().enumerate() {
            if w.is_anchored(idx) == Some(true) {
                continue;
            }
            let depth = i64::from(cap.radius)
                - i64::try_from(gap_dist_to_capsule(&cap, *p))
                    .expect("capsule distance fits i64: bounded near 4.2M raw");
            if depth > pen {
                pen = depth;
            }
        }
        (w, pen)
    }

    #[test]
    fn cloth_couple_pushes_drapes_out_of_capsules() {
        // 50 seeded drapes with the collider pushing each tick. Without the
        // push the same seeds sit ~9k raw deep (baseline tape); with it no
        // free particle may sit more than 1 cm (655 raw) inside any capsule.
        // Pins track bones exactly and are excluded from the probe.
        let mut worlds = Vec::with_capacity(GAP_DRAPES);
        let mut pen_max_raw = 0i64;
        let mut pen_drape = 0usize;
        for i in 0..GAP_DRAPES {
            let (w, pen) = gap_coupled_drape(i);
            assert_eq!(w.tick(), u64::from(GAP_TICKS), "drape {i} short-stepped");
            if pen > pen_max_raw {
                pen_max_raw = pen;
                pen_drape = i;
            }
            worlds.push(w);
        }
        let mut worlds2 = Vec::with_capacity(GAP_DRAPES);
        for i in 0..GAP_DRAPES {
            worlds2.push(gap_coupled_drape(i).0);
        }
        let digest_a = gap_digest(&worlds);
        let digest_b = gap_digest(&worlds2);
        assert_eq!(digest_a, digest_b, "coupled drapes must replay bit-exact");
        assert!(
            pen_max_raw < 655,
            "1 cm coupling failed: pen_max_raw={pen_max_raw} on drape {pen_drape}"
        );
        for (i, w) in worlds.iter().enumerate() {
            for c in 0..usize::try_from(w.cols()).expect("cols fits usize: dims bounded by MAX_DIM")
            {
                assert_eq!(
                    w.position_raw(c),
                    Some([
                        i32::try_from(c).expect("col fits i32: cols bounded by MAX_DIM") * SPACING,
                        ORIGIN[1],
                        0
                    ]),
                    "drape {i} pin {c} moved"
                );
            }
        }
        let mut probe = cape();
        step_cloth_with_colliders(&mut probe, &[gap_collider(3)], 5);
        let ((), allocs) =
            alloc_probe::measure(|| step_cloth_with_colliders(&mut probe, &[gap_collider(3)], 1));
        assert_eq!(
            allocs.calls, 0,
            "coupled tick must not allocate: {allocs:?}"
        );
        let mut verts = vec![[0.0f32; 3]; worlds[pen_drape].particle_count()];
        worlds[pen_drape]
            .write_verts_f32(&mut verts)
            .expect("verts");
        assert!(verts.iter().flatten().all(|v| v.is_finite()));
        std::println!(
            "cloth_couple pen_max_raw={pen_max_raw} drape={pen_drape} alloc_calls={} digest_a={digest_a:016x} digest_b={digest_b:016x}",
            allocs.calls,
        );
    }

    #[test]
    #[ignore = "timing: run under proof --perf"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "timing test reads the wall clock; it never enters the tick"
    )]
    fn cloth_couple_stays_cpu_light() {
        let t0 = std::time::Instant::now();
        let mut worlds = Vec::with_capacity(GAP_DRAPES);
        for i in 0..GAP_DRAPES {
            worlds.push(gap_coupled_drape(i).0);
        }
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        std::println!(
            "cloth_couple ms={ms:.1} per_tick_ms={:.4}",
            ms / (f64::from(
                i32::try_from(GAP_DRAPES).expect("drape count fits i32: fixed small count"),
            ) * f64::from(GAP_TICKS)),
        );
        assert!(
            ms < 2000.0,
            "50 coupled drapes must stay cpu-light: {ms:.1} ms"
        );
    }

    #[test]
    fn cloth_couple_rejects_bad_colliders() {
        assert_eq!(
            ClothCollider::capsule(0, 0, 0, 65536, 0).expect_err("zero radius"),
            ClothError::BadParam("collider")
        );
        assert_eq!(
            ClothCollider::capsule(0, 0, 65536, 0, 1000).expect_err("inverted segment"),
            ClothError::BadParam("collider")
        );
        assert_eq!(
            ClothCollider::capsule(0, 0, 0, 65536, MAX_ACCEL_RAW + 1).expect_err("fat radius"),
            ClothError::BadParam("collider")
        );
        assert_eq!(
            ClothCollider::sphere([0, 40 * 65536, 0], 1000).expect_err("far center"),
            ClothError::BadParam("collider")
        );
        // Empty slice is a no-op: positions and tick read back identical.
        let mut w = cape();
        let before = w.positions_raw().to_vec();
        w.resolve_colliders(&[]);
        assert_eq!(w.positions_raw(), before.as_slice());
        // Anchored pins never move, even deep inside a collider.
        let mut pinned = cape();
        let pin = pinned.position_raw(0).expect("pin");
        let shell = ClothCollider::sphere(pin, 5000).expect("shell on pin");
        pinned.resolve_colliders(&[shell]);
        assert_eq!(pinned.position_raw(0).expect("pin held"), pin);
        // A free particle at the sphere center escapes along +X by radius.
        let mut free = cape();
        free.clear_anchor(0).expect("free pin");
        let center = [0, 9 * 65536, 0];
        free.pos[10] = center;
        free.prev[10] = center;
        let ball = ClothCollider::sphere(center, 4000).expect("ball");
        free.resolve_colliders(&[ball]);
        let out = free.position_raw(10).expect("pushed");
        assert_eq!(out[0], center[0] + 4000, "on-axis must escape +X");
        assert_eq!((out[1], out[2]), (center[1], center[2]));
        // Off-axis push lands on the surface up to truncation (2 LSB).
        let mut side = cape();
        for c in 0..6 {
            side.clear_anchor(c).expect("drop pin");
        }
        side.pos[20] = [center[0] + 1000, center[1], center[2]];
        side.prev[20] = side.pos[20];
        side.resolve_colliders(&[ball]);
        let got = side.position_raw(20).expect("side push");
        let dist = i64::try_from(gap_dist_to_capsule(
            &GapCapsule {
                cx: center[0],
                cz: center[2],
                y_bot: center[1],
                y_top: center[1],
                radius: 4000,
            },
            got,
        ))
        .expect("capsule distance fits i64: bounded near 4.2M raw");
        assert!(
            (dist - 4000).abs() <= 2,
            "side push must land on surface: dist={dist}"
        );
    }

    /// Cuff fold for the self-collision gate: an 8x8 sheet with the hem row
    /// mirrored across the row-6 line onto row 5, so 8 hem particles start
    /// exactly coincident with the sheet, then settle one tick at a time so
    /// the whole settle is probed, not just the end state.
    fn gap_cuff_fold() -> ClothWorld {
        let mut w = ClothWorld::new_grid(8, 8, SPACING, ORIGIN, ClothParams::default_cape())
            .expect("cuff fold");
        let fold_y = ORIGIN[1] - 6 * SPACING;
        for c in 0..8u32 {
            let i = usize::try_from(7 * 8 + c).expect("cuff index fits usize: dims bounded");
            let p = w.pos[i];
            let mirrored = [p[0], 2 * fold_y - p[1], p[2]];
            w.pos[i] = mirrored;
            w.prev[i] = mirrored;
        }
        w
    }

    #[test]
    fn cloth_self_folded_curtain_separates() {
        // The cuff hem starts exactly coincident with the sheet, so the
        // setup itself is interpenetrated by construction. Without
        // intercollision the mirror pairs ride together and the cuff falls
        // back through the sheet, keeping close pairs through the settle;
        // with it every coincident pair escapes on the first tick and the
        // cuff lands on the sheet instead of crossing it, so every tick is
        // clean. The same script replays bit-exact twice.
        let run = || {
            let mut w = gap_cuff_fold();
            let (_, close0) = gap_self_gap(&w);
            assert!(close0 > 0, "cuff setup must start interpenetrated");
            for t in 0..60 {
                step_cloth(&mut w, 1);
                let (min_gap, close) = gap_self_gap(&w);
                assert!(
                    close == 0,
                    "cuff layers interpenetrate at tick {t}: min_gap_raw={min_gap} close_pairs={close}"
                );
            }
            w
        };
        let a = run();
        let b = run();
        assert_eq!(a.tick(), 60);
        assert_eq!(a, b, "cuff settle must replay bit-exact");
        let digest = gap_digest(core::slice::from_ref(&a));
        // Consumer leg: presentation verts off the separated sheet stay
        // finite, and the tick still allocates nothing steady-state.
        let mut verts = vec![[0.0f32; 3]; a.particle_count()];
        a.write_verts_f32(&mut verts).expect("verts");
        assert!(verts.iter().flatten().all(|v| v.is_finite()));
        let mut probe = gap_cuff_fold();
        step_cloth(&mut probe, 5);
        let ((), allocs) = alloc_probe::measure(|| step_cloth(&mut probe, 1));
        assert_eq!(allocs.calls, 0, "self tick must not allocate: {allocs:?}");
        std::println!(
            "cloth_self close_pairs=0 digest={digest:016x} alloc_calls={}",
            allocs.calls
        );
    }

    /// Stiffness-free tune that isolates intercollision: with no elastic,
    /// wind or gravity forces, only anchors and the self push move
    /// particles, so a pressed pair's separation is the push alone.
    fn still_tune() -> ClothParams {
        let mut p = ClothParams::default_cape();
        p.stiffness_q16 = 0;
        p.wind_q16 = [0, 0, 0];
        p.gravity_q16 = 0;
        p
    }

    #[test]
    fn cloth_self_isolated_pairs_separate() {
        // Two free particles pressed 100 raw apart separate to the skin
        // target in one tick, split evenly; without the push they sit
        // exactly where left. Far from the sheet, so no other pair matters.
        let press = || {
            let mut w =
                ClothWorld::new_grid(8, 8, SPACING, ORIGIN, still_tune()).expect("still world");
            let spot = [100000, 600000, 0];
            for (i, d) in [(10usize, 0i32), (50usize, 100i32)] {
                w.pos[i] = [spot[0] + d, spot[1], spot[2]];
                w.prev[i] = w.pos[i];
            }
            step_cloth(&mut w, 1);
            w
        };
        let a = press();
        let b = press();
        assert_eq!(a, b, "pair press must replay bit-exact");
        let pa = a.position_raw(10).expect("a");
        let pb = a.position_raw(50).expect("b");
        let d = raw_len(
            i64::from(pb[0]) - i64::from(pa[0]),
            i64::from(pb[1]) - i64::from(pa[1]),
            i64::from(pb[2]) - i64::from(pa[2]),
        );
        assert!(
            d >= u64::try_from(SPACING / 2).expect("half spacing fits u64: small positive"),
            "free pair must clear half a cell: dist={d}"
        );
        assert_eq!(
            (pa[0] + pb[0]) / 2,
            100050,
            "free pair must split the push evenly: {pa:?} vs {pb:?}"
        );
        // Anchored share: the pin never moves while the free end takes the
        // whole push.
        let mut w =
            ClothWorld::new_grid(8, 8, SPACING, ORIGIN, still_tune()).expect("pinned world");
        let spot = [100000, 600000, 0];
        w.set_anchor(10, spot).expect("pin one end");
        w.pos[50] = [spot[0] + 100, spot[1], spot[2]];
        w.prev[50] = w.pos[50];
        step_cloth(&mut w, 1);
        assert_eq!(w.position_raw(10).expect("pin"), spot);
        let q = w.position_raw(50).expect("free");
        let dq = raw_len(
            i64::from(q[0]) - i64::from(spot[0]),
            i64::from(q[1]) - i64::from(spot[1]),
            i64::from(q[2]) - i64::from(spot[2]),
        );
        assert!(
            dq >= u64::try_from(SPACING / 2).expect("half spacing fits u64: small positive"),
            "pinned pair must clear half a cell: dist={dq}"
        );
    }

    /// Acceptance tape: 50 seeded cape drapes over the shoulder bar plus
    /// the folded curtain, seed pinned at 60 Hz. Posts penetration max,
    /// folded self-gap, cloth ms, per-tick allocs and the digest twice;
    /// the digest must match across two runs (hash x2). Baseline only:
    /// penetration is measured, never gated to zero (no coupling yet);
    /// the finiteness cap pins the fixture itself.
    #[expect(
        clippy::disallowed_methods,
        reason = "baseline tape posts wall-clock ms; the reading never enters simulation state or hashes"
    )]
    #[expect(
        clippy::disallowed_types,
        reason = "baseline tape posts wall-clock ms; the reading never enters simulation state or hashes"
    )]
    #[test]
    fn cloth_gap() {
        let config = ClothTapeConfig::baseline();
        assert_eq!(config.ticks(), 51 * 120);
        let t0 = std::time::Instant::now();
        let a = run_cloth_tape(&config).expect("tape runs");
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        let b = run_cloth_tape(&config).expect("tape runs again");
        assert_eq!(a.digest, b.digest, "tape digest must repeat: {a} vs {b}");
        assert_eq!(a, b, "tape report must repeat exactly");
        assert_eq!(a.ticks, config.ticks());
        assert_eq!(a.drapes, CLOTH_TAPE_DRAPES);
        assert_eq!(a.seed, CLOTH_TAPE_SEED);
        // Fixture oracle: finite numbers inside the world, never gated to
        // zero before the coupling rows land.
        assert!(
            a.penetration_max_raw < TAPE_PEN_CAP_RAW,
            "penetration out of fixture bounds: {}",
            a.penetration_max_raw
        );
        assert!(
            a.self_min_raw < TAPE_PEN_CAP_RAW,
            "folded self-gap out of fixture bounds: {}",
            a.self_min_raw
        );
        // The tick and the probes allocate nothing steady-state: build the
        // world first (construction owns its buffers), then measure one
        // step plus both probes under the counting allocator.
        let mut w = cape();
        step_cloth(&mut w, 1);
        let ((), tick_allocs) = engine_core::alloc_probe::measure(|| {
            step_cloth(&mut w, 1);
            let cap = drape_capsule(ORIGIN).expect("tape bar fits the world");
            let _ = capsule_penetration_max(&w, &[cap]);
            let _ = self_separation_min(&w);
            let _ = cloth_digest(&w);
        });
        assert_eq!(
            tick_allocs.calls, 0,
            "tape step must not allocate: {tick_allocs:?}"
        );
        assert_eq!(
            tick_allocs.bytes, 0,
            "tape step must not allocate: {tick_allocs:?}"
        );
        std::eprintln!(
            "{a} cloth_ms={ms:.2} per_tick_allocs={}calls/{}B",
            tick_allocs.calls,
            tick_allocs.bytes
        );
    }

    #[test]
    fn cloth_gap_rejects_bad_input() {
        assert_eq!(
            CapsuleCollider::new([0, 0, 0], [1000, 0, 0], 0).expect_err("no radius"),
            ClothError::BadParam("capsule radius")
        );
        assert_eq!(
            CapsuleCollider::new([0, 40 * 65536, 0], [1000, 0, 0], 100).expect_err("far end"),
            ClothError::BadParam("capsule endpoint")
        );
        let mut cfg = ClothTapeConfig::baseline();
        cfg.drapes = 0;
        assert_eq!(
            run_cloth_tape(&cfg).expect_err("no drapes"),
            ClothError::BadParam("drapes")
        );
        cfg = ClothTapeConfig::baseline();
        cfg.steps_per_drape = 0;
        assert_eq!(
            run_cloth_tape(&cfg).expect_err("no steps"),
            ClothError::BadParam("steps_per_drape")
        );
        // Degenerate segment probes as a point; empty collider sets read zero.
        assert_eq!(
            segment_distance_raw([300, 400, 0], [0, 0, 0], [0, 0, 0]),
            500
        );
        assert_eq!(segment_distance_raw([0, 0, 0], [0, 0, 0], [300, 400, 0]), 0);
        let w = cape();
        assert_eq!(capsule_penetration_max(&w, &[]), 0);
    }

    #[test]
    fn build_links_matches_new_grid() {
        let spacing = SPACING;
        let spacing_u64 = u64::try_from(spacing).unwrap_or(0);
        let diag = i32::try_from(isqrt_u64(2 * spacing_u64 * spacing_u64)).unwrap_or(i32::MAX);
        let links = build_links(6, 8, spacing, diag);
        assert_eq!(links.len(), 40 + 42 + 70);
        let w = cape();
        assert_eq!(links, w.cons);
        assert_eq!(
            links[0],
            Constraint {
                a: 0,
                b: 1,
                rest: spacing
            }
        );
        assert_eq!(
            links[1],
            Constraint {
                a: 0,
                b: 6,
                rest: spacing
            }
        );
        assert_eq!(
            links[2],
            Constraint {
                a: 0,
                b: 7,
                rest: diag
            }
        );
        let single = build_links(1, 1, spacing, diag);
        assert!(single.is_empty());
        let row = build_links(2, 1, spacing, diag);
        assert_eq!(row.len(), 1);
        assert_eq!(
            row[0],
            Constraint {
                a: 0,
                b: 1,
                rest: spacing
            }
        );
    }
}
