//! solver: 4x-substep sequential-impulse AABB solver + warm-start + sleep + islands. Fixed order = same hash.
#![forbid(unsafe_code)]

use crate::Body;
use engine_core::hash::FNV64_PRIME;
use engine_core::Fnv64;

/// Fixed substep count per `step`.
pub const SUBSTEPS: usize = 4;

/// Sleep hysteresis: enter rest below 700µm (stacks breathe 100-580µm), wake above 1.4mm.
pub const SLEEP_MOVE2: f32 = 4.9e-7;
pub const WAKE_MOVE2: f32 = 1.96e-6;

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub gravity: f32,
    pub substeps: usize,
    pub iterations: usize,
    pub restitution: f32,
    pub friction: f32,
    pub sleep_v2: f32,
    pub wake_v2: f32,
    pub sleep_steps: u32,
    pub slop: f32,
    pub baumgarte: f32,
    pub wake_pen: f32,
    /// Jolt/Rapier-style island-split solve path for `step` contacts.
    /// Off = legacy loop. XPBD contacts stay on the legacy loop.
    pub island_split: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            gravity: 9.8,
            substeps: SUBSTEPS,
            iterations: 10,
            restitution: 0.0,
            friction: 0.1,
            // Wake at 35cm/s: settled stacks breathe ~11cm/s mid-substep; kicks are m/s.
            // pen backstop instead.
            sleep_v2: 0.1225,
            wake_v2: 0.01,
            sleep_steps: 60,
            slop: 0.0005,
            baumgarte: 0.8,
            wake_pen: 0.02,
            island_split: false,
        }
    }
}

/// Cached normal impulse per pair (sorted), re-applied next step. Only `jn` persists.
#[derive(Clone, Copy, Debug, Default)]
struct Warm {
    a: usize,
    b: usize,
    axis: u8,
    jn: f32,
}

/// Single owner of the wake write: clear sleep and reset the timer.
/// Both [`Solver::wake`] and [`crate::substep::Schedule::wake`] call this.
pub(crate) fn wake_one(sleeping: &mut [bool], timers: &mut [u32], idx: usize) {
    if idx < sleeping.len() {
        sleeping[idx] = false;
        timers[idx] = 0;
    }
}

#[derive(Default)]
pub struct Solver {
    cfg: Config,
    timers: Vec<u32>,
    sleeping: Vec<bool>,
    inv_mass: Vec<f32>,
    warm: Vec<Warm>,
    // Start-of-step position snapshot, reused across steps (no per-step alloc).
    prev_xy: Vec<(f32, f32)>,
    // Per-substep scratch, reused across substeps and steps (no per-step alloc).
    // Capacities grow once, then `clear` keeps them; contents are identical
    // to the fresh `Vec` path, so the hash never moves.
    pairs: Vec<(usize, usize)>,
    order: Vec<usize>,
    acc: Vec<(f32, f32)>,
    axes: Vec<u8>,
    tuples: Vec<(f32, f32, f32, f32)>,
    mins: Vec<f32>,
    maxs: Vec<f32>,
    parent: Vec<usize>,
    roots: Vec<usize>,
}

impl Solver {
    /// Default solver: legacy single-island path, gravity on, islands awake.
    pub fn new() -> Self {
        Self::default()
    }
    /// Solver with explicit config (gravity, island split, sleep thresholds).
    pub fn with_config(cfg: Config) -> Self {
        Self {
            cfg,
            ..Self::default()
        }
    }
    /// Active config (copy; mutating it changes nothing until `with_config`).
    pub fn config(&self) -> Config {
        self.cfg
    }
    /// Jolt/Rapier-style island-split solve path. Default off.
    pub fn set_island_split(&mut self, on: bool) {
        self.cfg.island_split = on;
    }
    fn ensure(&mut self, n: usize) {
        if self.timers.len() < n {
            self.timers.resize(n, 0);
            self.sleeping.resize(n, false);
            self.inv_mass.resize(n, 1.0);
        } else {
            self.timers.truncate(n);
            self.sleeping.truncate(n);
            self.inv_mass.truncate(n);
        }
        self.warm.retain(|w| w.a < n && w.b < n);
    }
    pub fn set_static(&mut self, idx: usize, statik: bool) {
        self.ensure(idx + 1);
        self.inv_mass[idx] = if statik { 0.0 } else { 1.0 };
        if statik {
            self.sleeping[idx] = false;
            self.timers[idx] = 0;
        }
    }
    pub fn is_static(&self, idx: usize) -> bool {
        self.inv_mass.get(idx).copied().unwrap_or(1.0) == 0.0
    }
    pub fn is_sleeping(&self, idx: usize) -> bool {
        self.sleeping.get(idx).copied().unwrap_or(false)
    }
    pub fn sleeping_mask(&self) -> &[bool] {
        &self.sleeping
    }
    pub fn sleeping_count(&self) -> usize {
        self.sleeping.iter().filter(|s| **s).count()
    }
    pub fn wake(&mut self, idx: usize) {
        wake_one(&mut self.sleeping, &mut self.timers, idx);
    }
    pub fn wake_all(&mut self) {
        for (s, t) in self.sleeping.iter_mut().zip(self.timers.iter_mut()) {
            *s = false;
            *t = 0;
        }
    }
    /// Advance by `dt`, split into `substeps` (default 4) x `iterations` passes.
    pub fn step(&mut self, bodies: &mut [Body], dt: f32) {
        self.ensure(bodies.len());
        if bodies.is_empty() || dt.is_nan() || dt <= 0.0 {
            return;
        }
        // Rest keys on displacement, not speed: settled stacks hold residual velocity but never move.
        // Reused scratch, no per-step alloc (same rule the XPBD path already used).
        self.prev_xy.clear();
        self.prev_xy.extend(bodies.iter().map(|b| (b.x, b.y)));
        let dt = dt.min(0.25);
        let nsub = self.cfg.substeps.max(1);
        let h = dt / substeps_as_f32(nsub);
        let (g, rest, mu) = (
            self.cfg.gravity,
            self.cfg.restitution,
            self.cfg.friction.clamp(0.0, 1.0),
        );
        let (slop, baumgarte, iterations, sleep_v2, wake_pen, island_split) = (
            self.cfg.slop,
            self.cfg.baumgarte,
            self.cfg.iterations,
            self.cfg.sleep_v2,
            self.cfg.wake_pen,
            self.cfg.island_split,
        );
        for _ in 0..nsub {
            integrate_bodies(bodies, &self.sleeping, &self.inv_mass, h, g, rest, mu);
            contact_pairs_into(
                bodies,
                &mut self.pairs,
                &mut self.tuples,
                &mut self.mins,
                &mut self.maxs,
                island_split,
            );
            island_order_into(
                bodies.len(),
                &self.pairs,
                &mut self.order,
                &mut self.parent,
                &mut self.roots,
            );
            wake_from_contacts(
                bodies,
                &self.pairs,
                &self.order,
                &mut self.sleeping,
                &mut self.timers,
                sleep_v2,
                wake_pen,
            );
            warm_start_into(
                bodies,
                &self.sleeping,
                &self.inv_mass,
                &self.warm,
                slop,
                &self.pairs,
                &self.order,
                true,
                nsub,
                &mut self.acc,
                &mut self.axes,
            );
            solve_pass(
                bodies,
                &self.inv_mass,
                &self.pairs,
                &self.order,
                &mut self.acc,
                &self.sleeping,
                rest,
                mu,
                slop,
                baumgarte,
                iterations,
                &self.roots,
                island_split,
            );
            warm_store_into(
                &mut self.warm,
                &self.pairs,
                &self.order,
                &self.acc,
                &self.axes,
            );
        }
        sleep_update(
            bodies,
            &mut self.sleeping,
            &mut self.timers,
            &self.inv_mass,
            &self.cfg,
            &self.prev_xy,
        );
    }
    /// Advance with XPBD distance joints (steals.md A5): the same integrate
    /// and contact path as `step`, plus a joint pass per substep with the
    /// joint residual as the final word. Lambdas clear per step and build
    /// over substeps x iterations. Empty joints answer `step` bit-for-bit.
    pub fn step_with_xpbd(&mut self, bodies: &mut [Body], dt: f32, joints: &mut [XpbdJoint]) {
        self.ensure(bodies.len());
        if bodies.is_empty() || dt.is_nan() || dt <= 0.0 {
            return;
        }
        reset_lambdas(joints);
        // Rest keys on displacement, not speed: settled hangs hold residual
        // velocity but never move. Reused scratch, no per-step alloc.
        self.prev_xy.clear();
        self.prev_xy.extend(bodies.iter().map(|b| (b.x, b.y)));
        let dt = dt.min(0.25);
        let nsub = self.cfg.substeps.max(1);
        let h = dt / substeps_as_f32(nsub);
        let (g, rest, mu) = (
            self.cfg.gravity,
            self.cfg.restitution,
            self.cfg.friction.clamp(0.0, 1.0),
        );
        let (slop, baumgarte, iterations, sleep_v2, wake_pen, island_split) = (
            self.cfg.slop,
            self.cfg.baumgarte,
            self.cfg.iterations,
            self.cfg.sleep_v2,
            self.cfg.wake_pen,
            self.cfg.island_split,
        );
        for _ in 0..nsub {
            integrate_bodies(bodies, &self.sleeping, &self.inv_mass, h, g, rest, mu);
            contact_pairs_into(
                bodies,
                &mut self.pairs,
                &mut self.tuples,
                &mut self.mins,
                &mut self.maxs,
                island_split,
            );
            island_order_into(
                bodies.len(),
                &self.pairs,
                &mut self.order,
                &mut self.parent,
                &mut self.roots,
            );
            wake_from_contacts(
                bodies,
                &self.pairs,
                &self.order,
                &mut self.sleeping,
                &mut self.timers,
                sleep_v2,
                wake_pen,
            );
            warm_start_into(
                bodies,
                &self.sleeping,
                &self.inv_mass,
                &self.warm,
                slop,
                &self.pairs,
                &self.order,
                true,
                nsub,
                &mut self.acc,
                &mut self.axes,
            );
            // XPBD contacts stay on the legacy loop; joints have the final word.
            solve_legacy(
                bodies,
                &self.inv_mass,
                &self.pairs,
                &self.order,
                &mut self.acc,
                &self.sleeping,
                rest,
                mu,
                slop,
                baumgarte,
                iterations,
            );
            warm_store_into(
                &mut self.warm,
                &self.pairs,
                &self.order,
                &self.acc,
                &self.axes,
            );
            // Joints last each substep: their residual is the final word.
            self.solve_xpbd_joints(bodies, joints, h);
        }
        sleep_update(
            bodies,
            &mut self.sleeping,
            &mut self.timers,
            &self.inv_mass,
            &self.cfg,
            &self.prev_xy,
        );
    }
    /// XPBD joint passes for one substep, slice order (deterministic).
    /// Static, sleeping and grounded sides read as infinite mass, the same
    /// rule `resolve` uses.
    fn solve_xpbd_joints(&mut self, bodies: &mut [Body], joints: &mut [XpbdJoint], h: f32) {
        for _ in 0..self.cfg.iterations {
            for j in joints.iter_mut() {
                let (la, lb) = (j.a, j.b);
                if la == lb || la >= bodies.len() || lb >= bodies.len() {
                    continue;
                }
                if self.sleeping[la] && self.sleeping[lb] {
                    continue;
                }
                let mut wa = if self.sleeping[la] {
                    0.0
                } else {
                    self.inv_mass[la]
                };
                let mut wb = if self.sleeping[lb] {
                    0.0
                } else {
                    self.inv_mass[lb]
                };
                if bodies[la].y - bodies[la].hy <= self.cfg.slop {
                    wa = 0.0;
                }
                if bodies[lb].y - bodies[lb].hy <= self.cfg.slop {
                    wb = 0.0;
                }
                if wa + wb <= 0.0 {
                    continue;
                }
                let rx = f32::from(j.rest_x_mm) / XPBD_MM;
                let ry = f32::from(j.rest_y_mm) / XPBD_MM;
                let comp = j.compliance;
                let (ba, bb) = if la < lb {
                    let (head, tail) = bodies.split_at_mut(lb);
                    (&mut head[la], &mut tail[0])
                } else {
                    let (head, tail) = bodies.split_at_mut(la);
                    (&mut tail[0], &mut head[lb])
                };
                xpbd_axis(
                    &mut ba.x,
                    &mut ba.vx,
                    &mut bb.x,
                    &mut bb.vx,
                    wa,
                    wb,
                    rx,
                    comp,
                    h,
                    &mut j.lam_x,
                );
                xpbd_axis(
                    &mut ba.y,
                    &mut ba.vy,
                    &mut bb.y,
                    &mut bb.vy,
                    wa,
                    wb,
                    ry,
                    comp,
                    h,
                    &mut j.lam_y,
                );
            }
        }
    }
    /// Binary-search warm entry; `warm` stays sorted by (a, b) via `warm_store_into`.
    fn warm_lookup(&self, a: usize, b: usize) -> Option<Warm> {
        warm_lookup_in(&self.warm, a, b)
    }
    /// FNV-1a over body bits + sleep byte + len. Deterministic across runs.
    pub fn state_hash(&self, bodies: &[Body]) -> u64 {
        hash_bodies(bodies, &self.sleeping)
    }
}

pub fn hash_bodies(bodies: &[Body], sleeping: &[bool]) -> u64 {
    let mut h = Fnv64::new();
    for (i, b) in bodies.iter().enumerate() {
        for w in [b.x.to_bits(), b.y.to_bits(), b.vx.to_bits(), b.vy.to_bits()] {
            h.write_u32(w);
        }
        h.write_u8(u8::from(sleeping.get(i).copied().unwrap_or(false)));
    }
    // Length tail folds one whole word plus a prime step, exactly like before:
    // a byte-wise fold would move every golden, so the tail stays word-wise
    // with the prime owned by the shared hash module.
    let mut state = h.finish();
    state ^= len_as_u64(bodies.len());
    state = state.wrapping_mul(FNV64_PRIME);
    state
}

/// Substep count to float divisor with saturating semantics: every reachable
/// config fits in `u16`, so this matches the old conversion exactly; an
/// absurd count clamps to `u16::MAX` instead of rounding past it, keeping
/// the divisor finite and the step total.
fn substeps_as_f32(nsub: usize) -> f32 {
    f32::from(u16::try_from(nsub).unwrap_or(u16::MAX))
}

/// Body count to hash word with saturating semantics: every reachable count
/// fits, so this matches the old conversion exactly; an absurd count clamps
/// instead of wrapping, so a corrupt length can never alias a small stack.
fn len_as_u64(len: usize) -> u64 {
    u64::try_from(len).unwrap_or(u64::MAX)
}

/// Binary-search warm entry in a sorted slice; the single owner for lookups.
fn warm_lookup_in(warm: &[Warm], a: usize, b: usize) -> Option<Warm> {
    warm.binary_search_by_key(&(a, b), |w| (w.a, w.b))
        .ok()
        .map(|i| warm[i])
}

/// Re-apply last step's normal impulse into reused `acc`/`axes`.
/// Same ops and order as the old per-substep `warm_start`; only the
/// allocation moves into caller-owned scratch, so the hash never moves.
#[expect(
    clippy::too_many_arguments,
    reason = "wide signature kept until its inputs move into a params struct"
)]
fn warm_start_into(
    bodies: &mut [Body],
    sleeping: &[bool],
    inv_mass: &[f32],
    warm: &[Warm],
    slop: f32,
    pairs: &[(usize, usize)],
    order: &[usize],
    seed: bool,
    nsub: usize,
    acc: &mut Vec<(f32, f32)>,
    axes: &mut Vec<u8>,
) {
    acc.clear();
    axes.clear();
    acc.reserve(order.len().saturating_sub(acc.capacity().min(order.len())));
    axes.reserve(order.len().saturating_sub(axes.capacity().min(order.len())));
    for &pi in order {
        let (a, b) = pairs[pi];
        let axis = contact_axis(bodies, a, b);
        axes.push(axis);
        let mut j = (0.0, 0.0);
        if seed && !(sleeping[a] && sleeping[b]) {
            if let Some(w) = warm_lookup_in(warm, a, b) {
                if w.axis == axis {
                    let share = w.jn / substeps_as_f32(nsub.max(1));
                    apply_impulse(
                        bodies,
                        inv_mass,
                        a,
                        b,
                        axis,
                        share,
                        slop,
                        sleeping[a],
                        sleeping[b],
                    );
                    j = (w.jn, 0.0);
                }
            }
        }
        acc.push(j);
    }
}

/// Cache this substep's accumulated impulses into reused `warm`,
/// sorted for determinism. Same content as the old fresh-`Vec` path.
fn warm_store_into(
    warm: &mut Vec<Warm>,
    pairs: &[(usize, usize)],
    order: &[usize],
    acc: &[(f32, f32)],
    axes: &[u8],
) {
    warm.clear();
    warm.reserve(order.len().saturating_sub(warm.capacity().min(order.len())));
    for (k, &pi) in order.iter().enumerate() {
        let (a, b) = pairs[pi];
        warm.push(Warm {
            a,
            b,
            axis: axes[k],
            jn: acc[k].0,
        });
    }
    warm.sort_by_key(|e| (e.a, e.b));
}

// A5 XPBD distance joints (steals.md A5): per-substep h with a compliance
// lambda term, joints only. Rest gaps stay integer millimeters; the solve
// adds no sqrt, trig or pow, only + - * / on the two axes separately.
/// Millimeters per meter: XPBD joint rests stay integer millimeters.
pub const XPBD_MM: f32 = 1000.0;

/// One rigid-or-soft link between two bodies, solved per substep.
/// `compliance` is alpha (0 = rigid); lambdas clear each step and build
/// over substeps x iterations, so repeat solves converge, not fight.
/// Rests stay whole millimeters (`i16` covers +-32 m, lossless into `f32`).
#[derive(Clone, Copy, Debug)]
pub struct XpbdJoint {
    pub a: usize,
    pub b: usize,
    pub rest_x_mm: i16,
    pub rest_y_mm: i16,
    pub compliance: f32,
    lam_x: f32,
    lam_y: f32,
}

impl XpbdJoint {
    /// Rest gaps in whole millimeters, compliance alpha (0 = rigid).
    /// Negative alpha clamps to rigid; lambdas start cleared.
    pub fn new(a: usize, b: usize, rest_x_mm: i16, rest_y_mm: i16, compliance: f32) -> Self {
        Self {
            a,
            b,
            rest_x_mm,
            rest_y_mm,
            compliance: compliance.max(0.0),
            lam_x: 0.0,
            lam_y: 0.0,
        }
    }
}

/// Substep size for this step: the full clamped dt split into `nsub`.
pub fn xpbd_h(dt: f32, nsub: usize) -> f32 {
    dt.min(0.25) / substeps_as_f32(nsub.max(1))
}

/// XPBD lambda update for one axis: `-C / (wsum + alpha / h^2)`.
/// Unit gradients (axis-separated), so no squared-length term.
/// Answers 0 when there is no mass to move or no time to move it in.
pub fn xpbd_delta_lambda(c: f32, wsum: f32, compliance: f32, h: f32) -> f32 {
    if !c.is_finite() || wsum <= 0.0 || !h.is_finite() || h <= 0.0 {
        return 0.0;
    }
    let denom = wsum + compliance.max(0.0) / (h * h);
    if !denom.is_finite() || denom <= 0.0 {
        return 0.0;
    }
    -c / denom
}

/// One XPBD axis pass: positional D-lambda plus a velocity projection so
/// the link holds instead of buzzing. Static, sleeping and grounded sides
/// already read as zero weight by the caller.
#[expect(
    clippy::too_many_arguments,
    reason = "one axis solve kept flat until joints grow a params struct"
)]
fn xpbd_axis(
    xa: &mut f32,
    va: &mut f32,
    xb: &mut f32,
    vb: &mut f32,
    wa: f32,
    wb: f32,
    rest: f32,
    compliance: f32,
    h: f32,
    lam: &mut f32,
) {
    let wsum = wa + wb;
    if wsum <= 0.0 {
        return;
    }
    let dl = xpbd_delta_lambda((*xb - *xa) - rest, wsum, compliance, h);
    if dl != 0.0 {
        *lam += dl;
        *xa -= dl * wa;
        *xb += dl * wb;
    }
    let rvn = *vb - *va;
    if rvn != 0.0 {
        let jv = -rvn / wsum;
        *va -= jv * wa;
        *vb += jv * wb;
    }
}

/// Gravity integrate plus ground clamp over awake bodies, index order.
/// Shared by `step` and the XPBD path: same ops, same order, same hash.
fn integrate_bodies(
    bodies: &mut [Body],
    sleeping: &[bool],
    inv_mass: &[f32],
    h: f32,
    gravity: f32,
    restitution: f32,
    friction: f32,
) {
    for (i, b) in bodies.iter_mut().enumerate() {
        if sleeping[i] || inv_mass[i] == 0.0 {
            continue;
        }
        b.vy -= gravity * h;
        b.x += b.vx * h;
        b.y += b.vy * h;
        if b.y < b.hy {
            b.y = b.hy;
            kill_downward(b, restitution, friction);
        }
    }
}

/// Whole-step rest gate over displacement plus residual speed.
/// Shared by `step` and the XPBD path: same ops, same order, same hash.
fn sleep_update(
    bodies: &mut [Body],
    sleeping: &mut [bool],
    timers: &mut [u32],
    inv_mass: &[f32],
    cfg: &Config,
    prev: &[(f32, f32)],
) {
    for (i, b) in bodies.iter_mut().enumerate() {
        if inv_mass[i] == 0.0 {
            sleeping[i] = false;
            timers[i] = 0;
            continue;
        }
        let v2 = b.vx * b.vx + b.vy * b.vy;
        // Displacement gate catches true rest; velocity gate catches settled stacks.
        let dx = b.x - prev[i].0;
        let dy = b.y - prev[i].1;
        let moved2 = dx * dx + dy * dy;
        if sleeping[i] {
            // Sleepers skip integration; kicks and teleports are the only wake signals.
            if v2 >= cfg.wake_v2 || moved2 > WAKE_MOVE2 {
                sleeping[i] = false;
                timers[i] = 0;
            }
            continue;
        }
        if moved2 <= SLEEP_MOVE2 || v2 <= 1e-8 {
            // At most sub-perceptual jitter: count toward sleep.
            timers[i] = timers[i].saturating_add(1);
            if timers[i] >= cfg.sleep_steps {
                sleeping[i] = true;
                b.vx = 0.0;
                b.vy = 0.0;
            }
        } else {
            // Moving: stay awake, timer reset.
            timers[i] = 0;
        }
    }
}

/// Sorted overlapping AABB pairs (i < j), deterministic.
/// Live tick path uses [`contact_pairs_into`] for both grid and BVH
/// candidates; the wrappers below are test-only oracles with the same output.
#[cfg(test)]
fn contact_pairs(b: &[Body]) -> Vec<(usize, usize)> {
    contact_pairs_inner(b, false)
}
#[cfg(test)]
fn contact_pairs_inner(b: &[Body], use_bvh: bool) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut tuples = Vec::new();
    let mut mins = Vec::new();
    let mut maxs = Vec::new();
    contact_pairs_into(b, &mut out, &mut tuples, &mut mins, &mut maxs, use_bvh);
    out
}

/// Sorted overlapping AABB pairs into reused `out`; exact AABB gate over
/// grid or BVH candidates, deterministic. Caller-owned `pairs`/`tuples`/
/// `mins`/`maxs` scratch, so the hash never moves.
/// Note: `broadphase::candidate_pairs`/`bvh_pairs` still allocate their
/// internal keys/tree per call (owned by `broadphase.rs`, outside this
/// packet's scope); this reuses everything `solver.rs` owns.
fn contact_pairs_into(
    b: &[Body],
    out: &mut Vec<(usize, usize)>,
    tuples: &mut Vec<(f32, f32, f32, f32)>,
    mins: &mut Vec<f32>,
    maxs: &mut Vec<f32>,
    use_bvh: bool,
) {
    out.clear();
    if b.len() < 2 {
        return;
    }
    // Either candidate set is a superset; the exact gate restores equality.
    let candidates = if use_bvh {
        mins.clear();
        maxs.clear();
        if mins.capacity() < b.len() {
            mins.reserve(b.len() - mins.capacity());
        }
        if maxs.capacity() < b.len() {
            maxs.reserve(b.len() - maxs.capacity());
        }
        for body in b.iter() {
            mins.push(body.x - body.hx);
            maxs.push(body.x + body.hx);
        }
        crate::broadphase::bvh_pairs(mins, maxs)
    } else {
        // Cell covers the largest finite diameter so big bodies span few cells;
        // any positive cell keeps candidates a superset of true overlaps and the
        // exact gate below restores equality. Non-finite extents stay out of the
        // cell so they cannot blow the grid into millions of keys.
        let mut max_half = 1.0f32;
        for body in b {
            if body.hx.is_finite() && body.hy.is_finite() {
                max_half = max_half.max(body.hx).max(body.hy);
            }
        }
        let cell = (2.0f32 * max_half).max(2.0);
        tuples.clear();
        if tuples.capacity() < b.len() {
            tuples.reserve(b.len() - tuples.capacity());
        }
        for v in b.iter() {
            tuples.push((v.x, v.y, v.hx, v.hy));
        }
        crate::broadphase::candidate_pairs(tuples, cell)
    };
    if out.capacity() < candidates.len() {
        out.reserve(candidates.len() - out.capacity());
    }
    for (p, q) in candidates {
        let (a, c) = if p < q { (p, q) } else { (q, p) };
        if a >= b.len() || c >= b.len() {
            continue;
        }
        // Bool gate (not negated float compare): NaN never overlaps,
        // matching the brute oracle below.
        if (b[a].x - b[c].x).abs() < b[a].hx + b[c].hx
            && (b[a].y - b[c].y).abs() < b[a].hy + b[c].hy
        {
            out.push((a, c));
        }
    }
    out.sort_unstable();
    out.dedup();
}

/// Brute-force O(n^2) equality oracle for `contact_pairs`: the old path,
/// kept so tests prove the sweep answers the same pair set.
#[cfg(test)]
fn contact_pairs_brute(b: &[Body]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    for i in 0..b.len() {
        for j in (i + 1)..b.len() {
            if (b[i].x - b[j].x).abs() < b[i].hx + b[j].hx
                && (b[i].y - b[j].y).abs() < b[i].hy + b[j].hy
            {
                out.push((i, j));
            }
        }
    }
    out
}

/// Min-axis penetration depth, or -1 when separated.
fn pen_depth(b: &[Body], i: usize, j: usize) -> f32 {
    let ox = b[i].hx + b[j].hx - (b[i].x - b[j].x).abs();
    if ox <= 0.0 {
        return -1.0;
    }
    let oy = b[i].hy + b[j].hy - (b[i].y - b[j].y).abs();
    if oy <= 0.0 {
        return -1.0;
    }
    ox.min(oy)
}

/// Canonical pair order into reused `order`: pairs grouped by union-find
/// island root, pairs sorted by (root, a, b). Deterministic; caller-owned
/// `order`/`parent`/`roots` scratch, so the hash never moves.
fn island_order_into(
    n: usize,
    pairs: &[(usize, usize)],
    order: &mut Vec<usize>,
    parent: &mut Vec<usize>,
    roots: &mut Vec<usize>,
) {
    island_roots_into(n, pairs, parent, roots);
    order.clear();
    if order.capacity() < pairs.len() {
        order.reserve(pairs.len() - order.capacity());
    }
    order.extend(0..pairs.len());
    order.sort_by_key(|&k| (roots[pairs[k].0], pairs[k].0, pairs[k].1));
}

/// Union-find island roots into reused `parent`/`roots`: union by minimum
/// with path compression; `roots[i]` is its component minimum. Caller-owned
/// scratch.
fn island_roots_into(
    n: usize,
    pairs: &[(usize, usize)],
    parent: &mut Vec<usize>,
    roots: &mut Vec<usize>,
) {
    fn find(p: &mut [usize], mut x: usize) -> usize {
        while p[x] != x {
            p[x] = p[p[x]];
            x = p[x];
        }
        x
    }
    parent.clear();
    if parent.capacity() < n {
        parent.reserve(n - parent.capacity());
    }
    parent.extend(0..n);
    for &(a, b) in pairs {
        if a >= n || b >= n {
            continue;
        }
        let ra = find(parent, a);
        let rb = find(parent, b);
        if ra != rb {
            if ra < rb {
                parent[rb] = ra;
            } else {
                parent[ra] = rb;
            }
        }
    }
    roots.clear();
    if roots.capacity() < n {
        roots.reserve(n - roots.capacity());
    }
    roots.resize(n, 0);
    for i in 0..n {
        roots[i] = find(parent, i);
    }
}

/// Jolt/Rapier-style island split over precomputed `roots`: each island
/// runs all `iterations` before the next starts. `order` is already grouped
/// by island root, so each run is contiguous. Islands own disjoint bodies,
/// so this answers the legacy interleaved loop bit-for-bit on settled
/// stacks; caller-owned `roots` scratch, order stays deterministic.
#[expect(
    clippy::too_many_arguments,
    reason = "one island solve kept flat until contacts grow a params struct"
)]
fn solve_island_split_on_roots(
    bodies: &mut [Body],
    inv: &[f32],
    pairs: &[(usize, usize)],
    order: &[usize],
    acc: &mut [(f32, f32)],
    sleeping: &[bool],
    rest: f32,
    mu: f32,
    slop: f32,
    beta: f32,
    iterations: usize,
    roots: &[usize],
) {
    if order.is_empty() || iterations == 0 {
        return;
    }
    let mut start = 0;
    while start < order.len() {
        let root = roots[pairs[order[start]].0];
        let mut end = start + 1;
        while end < order.len() && roots[pairs[order[end]].0] == root {
            end += 1;
        }
        for _ in 0..iterations {
            for k in start..end {
                let (a, b) = pairs[order[k]];
                if sleeping[a] && sleeping[b] {
                    continue;
                }
                resolve(
                    bodies,
                    inv,
                    a,
                    b,
                    rest,
                    mu,
                    slop,
                    beta,
                    &mut acc[k],
                    sleeping[a],
                    sleeping[b],
                );
            }
        }
        start = end;
    }
}

/// Wake sleepers touched by a fast or deeply-penetrating partner.
/// Shared by `step` and the XPBD path: same ops and order, same hash.
fn wake_from_contacts(
    bodies: &[Body],
    pairs: &[(usize, usize)],
    order: &[usize],
    sleeping: &mut [bool],
    timers: &mut [u32],
    sleep_v2: f32,
    wake_pen: f32,
) {
    for &k in order {
        let (a, b) = pairs[k];
        if sleeping[a] == sleeping[b] {
            continue;
        }
        let (s, o) = if sleeping[a] { (a, b) } else { (b, a) };
        let v2 = bodies[o].vx * bodies[o].vx + bodies[o].vy * bodies[o].vy;
        if v2 > sleep_v2 || pen_depth(bodies, a, b) > wake_pen {
            sleeping[s] = false;
            timers[s] = 0;
        }
    }
}

/// One legacy-loop contact pass over `order`: all `iterations` interleaved.
/// Same branches and order as the inline loop it replaces, same hash.
#[expect(
    clippy::too_many_arguments,
    reason = "one solve pass kept flat until contacts grow a params struct"
)]
fn solve_legacy(
    bodies: &mut [Body],
    inv: &[f32],
    pairs: &[(usize, usize)],
    order: &[usize],
    acc: &mut [(f32, f32)],
    sleeping: &[bool],
    rest: f32,
    mu: f32,
    slop: f32,
    baumgarte: f32,
    iterations: usize,
) {
    for _ in 0..iterations {
        for (k, &pi) in order.iter().enumerate() {
            let (a, b) = pairs[pi];
            if sleeping[a] && sleeping[b] {
                continue;
            }
            resolve(
                bodies,
                inv,
                a,
                b,
                rest,
                mu,
                slop,
                baumgarte,
                &mut acc[k],
                sleeping[a],
                sleeping[b],
            );
        }
    }
}

/// One contact-solve pass: island-split runs or the legacy loop.
/// Shared by `step` so it stays under the fn-length ratchet: same
/// branches, same order, same hash.
#[expect(
    clippy::too_many_arguments,
    reason = "one solve pass kept flat until contacts grow a params struct"
)]
fn solve_pass(
    bodies: &mut [Body],
    inv: &[f32],
    pairs: &[(usize, usize)],
    order: &[usize],
    acc: &mut [(f32, f32)],
    sleeping: &[bool],
    rest: f32,
    mu: f32,
    slop: f32,
    baumgarte: f32,
    iterations: usize,
    roots: &[usize],
    island_split: bool,
) {
    if island_split {
        solve_island_split_on_roots(
            bodies, inv, pairs, order, acc, sleeping, rest, mu, slop, baumgarte, iterations, roots,
        );
    } else {
        solve_legacy(
            bodies, inv, pairs, order, acc, sleeping, rest, mu, slop, baumgarte, iterations,
        );
    }
}

/// Clear XPBD lambdas at step start; they rebuild over substeps x iterations.
fn reset_lambdas(joints: &mut [XpbdJoint]) {
    for j in joints.iter_mut() {
        (j.lam_x, j.lam_y) = (0.0, 0.0);
    }
}

/// Downward-velocity kill + rest friction. Shared by the integrate ground
/// clamp (one canon, never twice).
fn kill_downward(b: &mut Body, rest: f32, mu: f32) {
    if b.vy < 0.0 {
        b.vy = -b.vy * rest;
        if b.vy.abs() < 0.05 {
            b.vy = 0.0;
        }
    }
    if b.vy == 0.0 {
        b.vx *= 1.0 - mu;
        if b.vx.abs() < 0.001 {
            b.vx = 0.0;
        }
    }
}

/// Min-penetration axis: 0 = x, 1 = y. Mirrors `resolve`'s branch choice.
fn contact_axis(b: &[Body], a: usize, c: usize) -> u8 {
    let ox = b[a].hx + b[c].hx - (b[c].x - b[a].x).abs();
    let oy = b[a].hy + b[c].hy - (b[c].y - b[a].y).abs();
    u8::from(ox >= oy)
}

/// Apply a cached normal impulse along `axis`, with the same grounded-side
/// infinite mass `resolve` uses — otherwise warm-start would re-inject the
/// kick the effective masses just removed. Tangent is rebuilt by the
/// iterations each substep, never carried.
#[expect(
    clippy::too_many_arguments,
    reason = "wide signature kept until its inputs move into a params struct"
)]
fn apply_impulse(
    b: &mut [Body],
    inv: &[f32],
    a: usize,
    c: usize,
    axis: u8,
    jn: f32,
    slop: f32,
    sa: bool,
    sb: bool,
) {
    let (ia, ib) = (
        if sa || b[a].y - b[a].hy <= slop {
            0.0
        } else {
            inv[a]
        },
        if sb || b[c].y - b[c].hy <= slop {
            0.0
        } else {
            inv[c]
        },
    );
    if axis == 0 {
        let s = if b[c].x >= b[a].x { 1.0f32 } else { -1.0 };
        b[a].vx -= s * jn * ia;
        b[c].vx += s * jn * ib;
    } else {
        let s = if b[c].y >= b[a].y { 1.0f32 } else { -1.0 };
        b[a].vy -= s * jn * ia;
        b[c].vy += s * jn * ib;
    }
}

/// One sequential-impulse contact: Baumgarte positional split by inverse mass,
/// velocity kill along min-penetration axis + accumulated-clamp Coulomb friction.
/// `acc` carries the pair's impulse totals (seeded by warm-start).
#[expect(
    clippy::too_many_arguments,
    reason = "wide signature kept until its inputs move into a params struct"
)]
fn resolve(
    b: &mut [Body],
    inv: &[f32],
    a: usize,
    c: usize,
    rest: f32,
    mu: f32,
    slop: f32,
    beta: f32,
    acc: &mut (f32, f32),
    sa: bool,
    sb: bool,
) {
    let ia = if sa { 0.0 } else { inv[a] };
    let ib = if sb { 0.0 } else { inv[c] };
    let wsum = ia + ib;
    if wsum <= 0.0 {
        return;
    }
    let dx = b[c].x - b[a].x;
    let dy = b[c].y - b[a].y;
    let ox = b[a].hx + b[c].hx - dx.abs();
    if ox <= 0.0 {
        return;
    }
    let oy = b[a].hy + b[c].hy - dy.abs();
    if oy <= 0.0 {
        return;
    }
    // Grounded body counts as infinite mass: partner takes the full impulse, base never re-kicked.
    let va = if b[a].y - b[a].hy <= slop { 0.0 } else { ia };
    let vb = if b[c].y - b[c].hy <= slop { 0.0 } else { ib };
    let (va, vb, vdiv) = if va + vb > 0.0 {
        (va, vb, va + vb)
    } else {
        (ia, ib, wsum)
    };
    // Grounded body takes none of the split: partner separates fully, base never sinks.
    let ga = b[a].y - b[a].hy <= slop;
    let gc = b[c].y - b[c].hy <= slop;
    let (mut pa, mut pb) = match (ga, gc) {
        (true, false) => (0.0, 1.0),
        (false, true) => (1.0, 0.0),
        _ => (ia, ib),
    };
    if ia == 0.0 {
        pa = 0.0;
    }
    if ib == 0.0 {
        pb = 0.0;
    }
    let (pa, pb, pdiv) = if pa + pb > 0.0 {
        (pa, pb, pa + pb)
    } else {
        (ia, ib, wsum)
    };
    if ox < oy {
        let s = if dx >= 0.0 { 1.0f32 } else { -1.0 };
        let corr = (ox - slop).max(0.0) * beta;
        b[a].x -= s * corr * pa / pdiv;
        b[c].x += s * corr * pb / pdiv;
        let rvn = (b[c].vx - b[a].vx) * s;
        // Overshoot unwinds instead of persisting: the cache self-corrects.
        let jn_new = (acc.0 - (1.0 + rest) * rvn / vdiv).max(0.0);
        let dj = jn_new - acc.0;
        acc.0 = jn_new;
        b[a].vx -= s * dj * va;
        b[c].vx += s * dj * vb;
        if jn_new > 0.0 {
            let vt = b[c].vy - b[a].vy;
            let lim = mu * jn_new;
            let jt_new = (acc.1 - vt / vdiv).clamp(-lim, lim);
            let djt = jt_new - acc.1;
            b[a].vy -= djt * va;
            b[c].vy += djt * vb;
            acc.1 = jt_new;
        }
    } else {
        let s = if dy >= 0.0 { 1.0f32 } else { -1.0 };
        let corr = (oy - slop).max(0.0) * beta;
        b[a].y -= s * corr * pa / pdiv;
        b[c].y += s * corr * pb / pdiv;
        let rvn = (b[c].vy - b[a].vy) * s;
        // Same as x-branch: overshoot unwinds instead of persisting.
        let jn_new = (acc.0 - (1.0 + rest) * rvn / vdiv).max(0.0);
        let dj = jn_new - acc.0;
        acc.0 = jn_new;
        b[a].vy -= s * dj * va;
        b[c].vy += s * dj * vb;
        if jn_new > 0.0 {
            let vt = b[c].vx - b[a].vx;
            let lim = mu * jn_new;
            let jt_new = (acc.1 - vt / vdiv).clamp(-lim, lim);
            let djt = jt_new - acc.1;
            b[a].vx -= djt * va;
            b[c].vx += djt * vb;
            acc.1 = jt_new;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixture row index to `f32` with saturating semantics: every reachable
    /// stack index fits in `u16`, so this matches the old conversion exactly;
    /// an absurd index clamps to `u16::MAX` instead of rounding past it.
    fn fixture_index_as_f32(i: i32) -> f32 {
        f32::from(u16::try_from(i).unwrap_or(u16::MAX))
    }

    /// Low 16 bits of the deterministic stream with explicit truncation: the
    /// mask keeps the same bits the old narrowing kept, then the checked
    /// conversion cannot fail, so the scatter fixture stays bit-identical.
    fn stream_word_as_u16(word: u32) -> u16 {
        u16::try_from(word & 0xFFFF).unwrap_or(u16::MAX)
    }

    fn stack5() -> Vec<Body> {
        (0..5)
            .map(|i| Body {
                x: 0.0,
                y: 0.5 + fixture_index_as_f32(i) * 1.02,
                vx: 0.0,
                vy: 0.0,
                hx: 0.5,
                hy: 0.5,
            })
            .collect()
    }
    fn settle() -> (Vec<Body>, Solver) {
        let mut s = Solver::new();
        let mut b = stack5();
        for _ in 0..900 {
            s.step(&mut b, 1.0 / 60.0);
        }
        (b, s)
    }

    #[test]
    fn five_box_sleep_hash() {
        let (b1, s1) = settle();
        let (b2, s2) = settle();
        assert_eq!(s1.sleeping_count(), 5, "all boxes must sleep");
        assert_eq!(
            s1.state_hash(&b1),
            s2.state_hash(&b2),
            "hash identical x2 runs"
        );
        let h = s1.state_hash(&b1);
        let (mut b1, mut s1) = (b1, s1);
        for _ in 0..120 {
            s1.step(&mut b1, 1.0 / 60.0);
        }
        assert_eq!(s1.state_hash(&b1), h, "sleeping hash stable");
        for w in b1.windows(2) {
            assert!(w[1].y > w[0].y, "stack order kept");
        }
        for b in &b1 {
            // Resting pen <=1cm is equilibrium, not tunneling; tunnel/explosion still fails.
            assert!(
                b.y >= b.hy - 0.01 && b.x.abs() < 0.05,
                "no tunnel/explosion: {b:?}"
            );
        }
    }

    fn stack10() -> Vec<Body> {
        (0..10)
            .map(|i| Body {
                x: 0.0,
                y: 0.5 + fixture_index_as_f32(i) * 1.02,
                vx: 0.0,
                vy: 0.0,
                hx: 0.5,
                hy: 0.5,
            })
            .collect()
    }
    fn max_pen(b: &[Body]) -> f32 {
        let mut m = (b[0].hy - b[0].y).max(0.0);
        for w in b.windows(2) {
            let gap = (w[1].y - w[0].y) - (w[0].hy + w[1].hy);
            m = m.max(-gap);
        }
        m
    }

    #[test]
    fn ten_box_penetration_sub_mm() {
        let run = || {
            let mut s = Solver::new();
            let mut b = stack10();
            for _ in 0..1800 {
                s.step(&mut b, 1.0 / 60.0);
            }
            eprintln!(
                "EXP sleep={} pen={:.6} b0y={:.5} b9y={:.5}",
                s.sleeping_count(),
                max_pen(&b),
                b[0].y,
                b[9].y
            );
            let mv: f32 = b.iter().map(|x| x.vx.abs() + x.vy.abs()).sum();
            eprintln!("EXP sum|v|={:.6} vy0={:.6} vy9={:.6}", mv, b[0].vy, b[9].vy);
            (b, s)
        };
        let (b1, s1) = run();
        let (b2, s2) = run();
        assert_eq!(s1.sleeping_count(), 10, "all boxes must sleep");
        assert_eq!(
            s1.state_hash(&b1),
            s2.state_hash(&b2),
            "hash identical x2 runs"
        );
        for w in b1.windows(2) {
            assert!(w[1].y > w[0].y, "stack order kept");
        }
        for b in &b1 {
            assert!(b.x.abs() < 0.05, "no lateral drift: {b:?}");
        }
        let pen = max_pen(&b1);
        assert!(
            pen < 0.001,
            "C14 gate: 10-box max penetration {pen:.6}m < 1mm"
        );
    }

    #[test]
    fn overlap_killed_static_holds() {
        let mut s = Solver::new();
        s.set_static(1, true);
        let mut b = vec![
            Body {
                x: 0.0,
                y: 2.0,
                vx: 0.0,
                vy: 0.0,
                hx: 0.5,
                hy: 0.5,
            },
            Body {
                x: 0.0,
                y: 2.4,
                vx: 0.0,
                vy: 0.0,
                hx: 0.5,
                hy: 0.5,
            },
        ];
        for _ in 0..240 {
            s.step(&mut b, 1.0 / 60.0);
        }
        assert_eq!((b[1].x, b[1].y), (0.0, 2.4), "static never moves");
        assert!(
            (b[0].y - b[1].y).abs() >= 1.0 - 0.01,
            "overlap resolved: {:?}",
            b
        );
    }

    #[test]
    fn wake_on_kick_and_api() {
        let (mut b, mut s) = settle();
        assert_eq!(s.sleeping_count(), 5);
        b[4].vy = 3.0;
        s.step(&mut b, 1.0 / 60.0);
        assert!(!s.is_sleeping(4));
        s.wake_all();
        assert_eq!(s.sleeping_count(), 0);
        s.wake(0);
        assert!(!s.is_sleeping(0));
    }

    #[test]
    fn hash_matches_shared_fnv_bytes() {
        use engine_core::hash::{FNV64_OFFSET, FNV64_PRIME};
        fn legacy(bodies: &[Body], sleeping: &[bool]) -> u64 {
            let mut h: u64 = FNV64_OFFSET;
            for (i, b) in bodies.iter().enumerate() {
                for w in [b.x.to_bits(), b.y.to_bits(), b.vx.to_bits(), b.vy.to_bits()] {
                    for byte in w.to_le_bytes() {
                        h ^= u64::from(byte);
                        h = h.wrapping_mul(FNV64_PRIME);
                    }
                }
                let s = u8::from(sleeping.get(i).copied().unwrap_or(false));
                h ^= u64::from(s);
                h = h.wrapping_mul(FNV64_PRIME);
            }
            h ^= u64::try_from(bodies.len()).unwrap_or(u64::MAX);
            h = h.wrapping_mul(FNV64_PRIME);
            h
        }
        let one = vec![Body::new(0.0, 0.5, 0.0, 0.0, 0.5, 0.5)];
        // Short sleep slices read as awake; a 300-body stack pins the
        // multi-byte length tail (word-wise, not byte-wise).
        let many = vec![Body::new(0.0, 0.5, 0.0, 0.0, 0.5, 0.5); 300];
        for (bodies, sleep) in [
            (vec![], vec![]),
            (one.clone(), vec![true]),
            (one.clone(), vec![]),
            (many.clone(), vec![false; 300]),
        ] {
            assert_eq!(hash_bodies(&bodies, &sleep), legacy(&bodies, &sleep));
        }
        let mut moved = one.clone();
        moved[0].x = 0.001;
        assert_ne!(hash_bodies(&one, &[false]), hash_bodies(&moved, &[false]));
    }

    #[test]
    fn a5_xpbd_lambda_shape() {
        // Donor theory: DL = -C / (wsum + alpha / h^2) on unit gradients.
        let h = xpbd_h(1.0 / 60.0, 4);
        assert!((h - 1.0 / 240.0).abs() < 1e-7, "h = dt/nsub, got {h}");
        assert_eq!(xpbd_delta_lambda(0.0, 2.0, 0.0, h), 0.0);
        let rigid = xpbd_delta_lambda(0.5, 2.0, 0.0, h);
        assert!(
            (rigid + 0.25).abs() < 1e-6,
            "rigid DL = -C/wsum, got {rigid}"
        );
        let soft = xpbd_delta_lambda(0.5, 2.0, 1e-4, h);
        assert!(
            soft > rigid && soft < 0.0,
            "compliance softens the kick, got {soft}"
        );
        assert_eq!(xpbd_delta_lambda(0.5, 0.0, 0.0, h), 0.0, "no mass holds");
        assert_eq!(xpbd_delta_lambda(0.5, 2.0, 0.0, 0.0), 0.0, "no time holds");
    }

    // 2-bone hang: static anchor holds the bob 1.000 m below, integer mm.
    // The bob starts 10 cm low so the joint has work to do.
    fn pair() -> Vec<Body> {
        vec![
            Body {
                x: 0.0,
                y: 3.0,
                vx: 0.0,
                vy: 0.0,
                hx: 0.5,
                hy: 0.5,
            },
            Body {
                x: 0.0,
                y: 1.9,
                vx: 0.0,
                vy: 0.0,
                hx: 0.5,
                hy: 0.5,
            },
        ]
    }
    fn joint() -> Vec<XpbdJoint> {
        vec![XpbdJoint::new(0, 1, 0, -1000, 0.0)]
    }

    #[test]
    fn a5_xpbd_pendulum_residual_and_p99() {
        // Axis-separated gap from the 1 m hang: no sqrt in the gate.
        fn gap(b: &[Body]) -> f32 {
            (b[1].x - b[0].x).abs().max(((b[1].y - b[0].y) + 1.0).abs())
        }
        fn jitter(b: &[Body]) -> f32 {
            b.iter().map(|x| x.vx.abs() + x.vy.abs()).sum()
        }
        // Baseline: the same pair under the plain 4x10 solver, no joints.
        let mut base = pair();
        let mut bs = Solver::new();
        bs.set_static(0, true);
        for _ in 0..600 {
            bs.step(&mut base, 1.0 / 60.0);
        }
        let (base_res, base_jit) = (gap(&base), jitter(&base));
        // Joint run: XPBD reels the bob up to the 1 m mark and hangs it.
        let mut b = pair();
        let mut s = Solver::new();
        s.set_static(0, true);
        let mut js = joint();
        for _ in 0..600 {
            s.step_with_xpbd(&mut b, 1.0 / 60.0, &mut js);
        }
        let (res, jit) = (gap(&b), jitter(&b));
        eprintln!("A5 hang res={res:.6} base={base_res:.6} jit={jit:.6} base_jit={base_jit:.6}");
        assert!(res <= 0.001, "joint residual {res:.6} m over 1 mm");
        assert!(
            res <= base_res + 0.001,
            "joint {res:.6} over baseline {base_res:.6} + 1 mm"
        );
        assert!(jit <= base_jit + 0.05, "joint buzz {jit:.6} over baseline");
        // Floor fixture beside the hang: one dropped body rests sub-mm.
        let mut f = vec![Body {
            x: 0.0,
            y: 5.0,
            vx: 0.0,
            vy: 0.0,
            hx: 0.5,
            hy: 0.5,
        }];
        let mut fs = Solver::new();
        for _ in 0..600 {
            fs.step(&mut f, 1.0 / 60.0);
        }
        let pen = (f[0].hy - f[0].y).max(0.0);
        assert!(pen <= 0.001, "floor pen {pen:.6} m over 1 mm");
        // Empty joints answer the plain step bit-for-bit.
        let mut e1 = vec![Body {
            x: 0.0,
            y: 5.0,
            vx: 0.0,
            vy: 0.0,
            hx: 0.5,
            hy: 0.5,
        }];
        let mut e2 = e1.clone();
        let (mut se1, mut se2) = (Solver::new(), Solver::new());
        let mut no: Vec<XpbdJoint> = vec![];
        for _ in 0..600 {
            se1.step(&mut e1, 1.0 / 60.0);
            se2.step_with_xpbd(&mut e2, 1.0 / 60.0, &mut no);
        }
        assert_eq!(
            se1.state_hash(&e1),
            se2.state_hash(&e2),
            "empty-joint run must match plain step"
        );
        // Same seed + steps = same hash; the settled hang stays bit still.
        let h1 = s.state_hash(&b);
        let mut b2 = pair();
        let mut s2 = Solver::new();
        s2.set_static(0, true);
        let mut j2 = joint();
        for _ in 0..600 {
            s2.step_with_xpbd(&mut b2, 1.0 / 60.0, &mut j2);
        }
        assert_eq!(h1, s2.state_hash(&b2), "joint hash x2");
        for _ in 0..120 {
            s.step_with_xpbd(&mut b, 1.0 / 60.0, &mut js);
        }
        assert_eq!(h1, s.state_hash(&b), "settled hang holds still");
    }

    #[test]
    #[ignore = "timing: run under proof --perf"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn a5_xpbd_joint_step_p99_under_perf() {
        // Worst-step gate on the settled hang: 20 warm, 200 timed.
        let mut t = pair();
        let mut ts = Solver::new();
        ts.set_static(0, true);
        let mut tj = joint();
        for _ in 0..20 {
            ts.step_with_xpbd(&mut t, 1.0 / 60.0, &mut tj);
        }
        let mut ns: Vec<u128> = Vec::with_capacity(200);
        for _ in 0..200 {
            let now = std::time::Instant::now();
            ts.step_with_xpbd(&mut t, 1.0 / 60.0, &mut tj);
            ns.push(now.elapsed().as_nanos());
        }
        ns.sort_unstable();
        let p99 = ns[ns.len() * 99 / 100];
        eprintln!("A5 joint step p50={}ns p99={}ns", ns[100], p99);
        assert!(p99 <= 150_000, "joint p99 {p99} ns over 150 us");
    }

    #[test]
    fn conversions_match_reachable_saturate_absurd() {
        for (n, want_f, want_u) in [
            (1usize, 1.0f32, 1u64),
            (4, 4.0, 4),
            (60, 60.0, 60),
            (60_000, 60_000.0, 60_000),
        ] {
            assert_eq!(substeps_as_f32(n), want_f);
            assert_eq!(len_as_u64(n), want_u);
        }
        assert_eq!(len_as_u64(1_000_000), 1_000_000u64);
        // Above `u16::MAX` the divisor clamps instead of rounding on.
        assert_eq!(substeps_as_f32(1_000_000), f32::from(u16::MAX));
        assert_eq!(len_as_u64(usize::MAX), u64::MAX);
        assert_eq!(substeps_as_f32(usize::MAX), f32::from(u16::MAX));
        assert!(substeps_as_f32(usize::MAX).is_finite());
        // Fixture helpers match the old conversion on every reachable input
        // and clamp or truncate explicitly past it, keeping the tick bit-identical.
        for (i, want) in [(0i32, 0.0f32), (1, 1.0), (4, 4.0), (9, 9.0)] {
            assert_eq!(fixture_index_as_f32(i), want, "fixture index {i}");
        }
        assert_eq!(
            fixture_index_as_f32(i32::MAX),
            f32::from(u16::MAX),
            "absurd index clamps"
        );
        assert_eq!(
            fixture_index_as_f32(-1),
            f32::from(u16::MAX),
            "negative index clamps"
        );
        assert_eq!(stream_word_as_u16(0), 0, "zero stream word");
        assert_eq!(stream_word_as_u16(0x1234_5678), 0x5678, "low 16 bits kept");
        assert_eq!(
            stream_word_as_u16(0x1234_5678 >> 8),
            0x3456,
            "shifted stream word"
        );
        assert_eq!(
            stream_word_as_u16(u32::MAX),
            u16::MAX,
            "all-ones truncates to max"
        );
    }

    #[test]
    fn island_split_off_by_default_matches_hash() {
        assert!(!Config::default().island_split, "flag defaults off");
        // Two far-apart stacks = two islands, no shared bodies.
        fn two_stacks() -> Vec<Body> {
            let mut v = Vec::with_capacity(6);
            for side in [-5.0f32, 5.0] {
                for i in 0..3 {
                    v.push(Body {
                        x: side,
                        y: 0.5 + fixture_index_as_f32(i) * 1.02,
                        vx: 0.0,
                        vy: 0.0,
                        hx: 0.5,
                        hy: 0.5,
                    });
                }
            }
            v
        }
        fn run(split: bool) -> (Vec<Body>, Solver) {
            let mut cfg = Config::default();
            cfg.island_split = split;
            let mut s = Solver::with_config(cfg);
            let mut b = two_stacks();
            for _ in 0..240 {
                s.step(&mut b, 1.0 / 60.0);
            }
            (b, s)
        }
        let (b0, s0) = run(false);
        let (b1, s1) = run(true);
        assert_eq!(
            s0.state_hash(&b0),
            s1.state_hash(&b1),
            "split path must match legacy hash"
        );
    }

    fn scatter_bodies(n: usize) -> Vec<Body> {
        let mut s: u32 = 0x2040;
        let mut next = || {
            s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            f32::from(stream_word_as_u16(s >> 8)) / f32::from(u16::MAX)
        };
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            out.push(Body {
                x: next() * 60.0,
                y: next() * 60.0,
                vx: 0.0,
                vy: 0.0,
                hx: 0.5,
                hy: 0.5,
            });
        }
        out
    }

    #[test]
    fn sweep_pairs_match_brute_oracle() {
        for n in [0, 1, 2, 10, 60] {
            let b = scatter_bodies(n);
            assert_eq!(contact_pairs(&b), contact_pairs_brute(&b), "n={n}");
        }
        // Dense stack plus a NaN body: NaN never overlaps, both paths agree.
        let mut b = stack10();
        b.push(Body {
            x: f32::NAN,
            y: f32::NAN,
            vx: 0.0,
            vy: 0.0,
            hx: 0.5,
            hy: 0.5,
        });
        assert_eq!(contact_pairs(&b), contact_pairs_brute(&b));
        // Stepping with the sweep still answers the same hash x2.
        let run = || {
            let mut s = Solver::new();
            let mut bodies = stack10();
            for _ in 0..240 {
                s.step(&mut bodies, 1.0 / 60.0);
            }
            (bodies, s)
        };
        let (b1, s1) = run();
        let (b2, s2) = run();
        assert_eq!(s1.state_hash(&b1), s2.state_hash(&b2));
    }

    #[test]
    fn broadphase_grid_matches_brute_oracle() {
        // Behavior: the built uniform-grid has a live tick caller; its filtered
        // set equals the brute oracle on scattered, stacked and NaN fixtures.
        for n in [0, 1, 2, 10, 60] {
            let b = scatter_bodies(n);
            assert_eq!(contact_pairs(&b), contact_pairs_brute(&b), "n={n}");
        }
        let mut b = stack10();
        b.push(Body {
            x: f32::NAN,
            y: f32::NAN,
            vx: 0.0,
            vy: 0.0,
            hx: 0.5,
            hy: 0.5,
        });
        assert_eq!(contact_pairs(&b), contact_pairs_brute(&b));
    }

    #[test]
    fn broadphase_grid_never_panics_on_nonfinite() {
        // Failure: non-finite centres, extents and huge boxes never panic and
        // never report a NaN pair; output still equals the brute oracle.
        let b = vec![
            Body {
                x: f32::NAN,
                y: 0.0,
                vx: 0.0,
                vy: 0.0,
                hx: 0.5,
                hy: 0.5,
            },
            Body {
                x: f32::INFINITY,
                y: 0.0,
                vx: 0.0,
                vy: 0.0,
                hx: 0.5,
                hy: 0.5,
            },
            Body {
                x: 0.0,
                y: 0.0,
                vx: 0.0,
                vy: 0.0,
                hx: 1.0e20,
                hy: 0.5,
            },
            Body {
                x: 0.4,
                y: 0.0,
                vx: 0.0,
                vy: 0.0,
                hx: 0.5,
                hy: 0.5,
            },
        ];
        let got = contact_pairs(&b);
        assert_eq!(got, contact_pairs_brute(&b));
        for (a, c) in &got {
            assert!(
                b[*a].x.is_finite() && b[*c].x.is_finite(),
                "no NaN pair {got:?}"
            );
        }
    }

    #[test]
    fn bvh_candidates_match_grid_and_brute() {
        for n in [0, 2, 10, 60] {
            let b = scatter_bodies(n);
            let (bv, grid, brute) = (
                contact_pairs_inner(&b, true),
                contact_pairs(&b),
                contact_pairs_brute(&b),
            );
            assert_eq!(bv, brute, "bvh n={n}");
            assert_eq!(bv, grid, "bvh matches grid n={n}");
        }
        let mut b = stack10();
        b.push(Body {
            x: f32::NAN,
            y: f32::NAN,
            vx: 0.0,
            vy: 0.0,
            hx: 0.5,
            hy: 0.5,
        });
        assert_eq!(contact_pairs_inner(&b, true), contact_pairs_brute(&b));
        assert_eq!(contact_pairs_inner(&b, true), contact_pairs(&b));
    }

    #[test]
    fn bvh_never_panics_on_nonfinite() {
        fn body(x: f32, hx: f32) -> Body {
            Body {
                x,
                y: 0.0,
                vx: 0.0,
                vy: 0.0,
                hx,
                hy: 0.5,
            }
        }
        let b = vec![
            body(f32::NAN, 0.5),
            body(f32::INFINITY, 0.5),
            body(0.0, 1.0e20),
        ];
        let got = contact_pairs_inner(&b, true);
        assert_eq!(got, contact_pairs_brute(&b));
        assert_eq!(got, contact_pairs(&b));
    }

    #[test]
    fn warm_binary_search_matches_linear() {
        let (mut b, mut s) = settle();
        for _ in 0..30 {
            s.step(&mut b, 1.0 / 60.0);
        }
        for a in 0..b.len() {
            for c in (a + 1)..b.len() {
                let linear = s.warm.iter().find(|w| w.a == a && w.b == c);
                let cached = s.warm_lookup(a, c);
                assert_eq!(
                    linear.map(|w| (w.a, w.b, w.axis, w.jn)),
                    cached.map(|w| (w.a, w.b, w.axis, w.jn)),
                    "pair {a},{c}"
                );
            }
        }
        // Missing pair answers None on both paths.
        assert!(s.warm_lookup(b.len() + 9, b.len() + 10).is_none());
    }

    #[test]
    fn pair_discovery_sweep_matches_brute() {
        // Crowded uniform clouds: the single-pass sweep must find exactly the
        // brute-force pair set at both sizes.
        for n in [1200usize, 4000usize] {
            let b = scatter_bodies(n);
            let brute = contact_pairs_brute(&b);
            let fast = contact_pairs(&b);
            assert_eq!(fast, brute, "sweep must match brute set n={n}");
        }
    }

    #[test]
    #[ignore = "timing: run under proof --perf"]
    #[expect(
        clippy::disallowed_types,
        clippy::disallowed_methods,
        reason = "test-only timing, never the tick"
    )]
    fn pair_discovery_bench_prints_lower() {
        // Brute force pays O(n^2) pair gates while the single-pass sweep pays
        // one order sort plus the x-window with the y gate inline, so the
        // sweep must win at both sizes.
        for n in [1200usize, 4000usize] {
            let b = scatter_bodies(n);
            let t0 = std::time::Instant::now();
            let brute = contact_pairs_brute(&b);
            let brute_ns = t0.elapsed().as_nanos();
            let t1 = std::time::Instant::now();
            let fast = contact_pairs(&b);
            let fast_ns = t1.elapsed().as_nanos();
            eprintln!(
                "PAIR n={n} brute={brute_ns}ns sweep={fast_ns}ns pairs={}",
                fast.len()
            );
            assert_eq!(fast.len(), brute.len(), "sweep pair count n={n}");
            assert!(
                fast_ns <= brute_ns,
                "sweep {fast_ns}ns must not exceed brute {brute_ns}ns n={n}"
            );
        }
    }
}
