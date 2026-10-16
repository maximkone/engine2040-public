//! substep: explicit deterministic solver schedule.
//!
//! Consumer: `tests/contacts.rs` drives the arena duel bodies
//! through this schedule instead of the implicit loop in [`crate::solver`].
//! Owner: this module owns [`Schedule`], [`ScheduleConfig`], [`Phase`] and
//! [`ScheduleError`]. Units: meters, seconds, 60 Hz ticks. Determinism:
//! fixed substep count with the phase order [`PHASE_ORDER`]; `+ - * /` and
//! `sqrt` only, index-order iteration, no clock. Errors: [`ScheduleError`],
//! never a panic on caller data. Budget: the tick path reuses caller-visible
//! scratch inside [`Schedule`] and allocates nothing once body capacity is
//! warm; see `substep_tick_stays_alloc_free`.
//!
//! Shape: PORT of the Avian `SolverSchedulePlugin` + `XpbdSolverPlugin`
//! ordering idea (prepare, then per-substep integrate, solve, sleep),
//! reimplemented integer-friendly and std-only; zero donor bytes.
//! Joints solve in XPBD shape (arXiv 1606.07304 ideas only): one lambda
//! per joint with a compliance/`h^2` term; `+ - * /` and `sqrt` only,
//! never `fma`, so the hash never moves between runs.
//! See `docs/plan/steals.md` for the license note (Apache-2.0/MIT dual).
#![forbid(unsafe_code)]

use crate::Body;
use engine_core::Fnv64;
use std::fmt::{Display, Formatter};

/// Fixed substeps per tick when the caller uses [`ScheduleConfig::default`].
pub const DEFAULT_SUBSTEPS: usize = 4;
/// Fixed solver iterations per substep for contacts and joints.
pub const DEFAULT_ITERATIONS: usize = 10;
/// Gravity, m/s^2, positive pulls `-y`.
pub const DEFAULT_GRAVITY: f32 = 9.8;
/// Largest accepted substep count; more would shrink `h` past usefulness.
pub const MAX_SUBSTEPS: usize = 16;
/// Largest accepted iteration count per substep.
pub const MAX_ITERATIONS: usize = 64;
/// Gravity bound, m/s^2; past this the caller mis-scaled.
pub const MAX_GRAVITY: f32 = 50.0;
/// Joint rest-length bound, meters.
pub const MAX_REST_M: f32 = 2000.0;
/// Sleep speed gate, squared velocity; bodies under this count toward rest.
pub const SLEEP_V2: f32 = 1.0e-8;
/// Ticks of stillness before a body sleeps.
pub const SLEEP_STEPS: u32 = 60;
/// Wake speed gate, squared velocity; kicks wake sleepers.
pub const WAKE_V2: f32 = 0.01;
/// Positional slop, meters; penetration under this is equilibrium.
pub const CONTACT_SLOP: f32 = 0.0005;
/// Baumgarte split factor for contact correction.
pub const CONTACT_BAUMGARTE: f32 = 0.8;

/// One solver phase. The schedule runs every substep in [`PHASE_ORDER`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Semi-implicit Euler integrate plus the ground clamp.
    Integrate,
    /// AABB contact solve, Baumgarte split plus velocity kill.
    SolveContacts,
    /// Distance-joint projection in canonical `(a, b)` order.
    SolveJoints,
    /// Sleep bookkeeping from squared speed with hysteresis.
    UpdateSleep,
}

impl Phase {
    /// Stable hash code: integrate reads 0 through sleep reads 3.
    const fn code(self) -> u8 {
        match self {
            Phase::Integrate => 0,
            Phase::SolveContacts => 1,
            Phase::SolveJoints => 2,
            Phase::UpdateSleep => 3,
        }
    }
}

/// Explicit phase order. Every substep runs these four in this order;
/// the order plus the substep count enter the state hash.
pub const PHASE_ORDER: [Phase; 4] = [
    Phase::Integrate,
    Phase::SolveContacts,
    Phase::SolveJoints,
    Phase::UpdateSleep,
];

/// Schedule tuning: explicit substep count, iterations and gravity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScheduleConfig {
    /// Substeps per `step` tick; explicit, hashed.
    pub substeps: usize,
    /// Contact/joint iterations per substep.
    pub iterations: usize,
    /// Gravity acceleration, m/s^2.
    pub gravity: f32,
}

impl Default for ScheduleConfig {
    fn default() -> Self {
        Self {
            substeps: DEFAULT_SUBSTEPS,
            iterations: DEFAULT_ITERATIONS,
            gravity: DEFAULT_GRAVITY,
        }
    }
}

impl ScheduleConfig {
    /// Build with validation: substeps in `1..=MAX_SUBSTEPS`, iterations in
    /// `1..=MAX_ITERATIONS`, gravity finite within `±MAX_GRAVITY`.
    pub fn new(substeps: usize, iterations: usize, gravity: f32) -> Result<Self, ScheduleError> {
        if !(1..=MAX_SUBSTEPS).contains(&substeps) {
            return Err(ScheduleError::BadSubsteps(substeps));
        }
        if !(1..=MAX_ITERATIONS).contains(&iterations) {
            return Err(ScheduleError::BadIterations(iterations));
        }
        if !gravity.is_finite() || gravity.abs() > MAX_GRAVITY {
            return Err(ScheduleError::BadGravity(gravity));
        }
        Ok(Self {
            substeps,
            iterations,
            gravity,
        })
    }
}

/// Distance joint between two bodies: rest length in meters.
/// `compliance` is the XPBD alpha (`0.0` = rigid); softer joints carry a
/// larger value and give more under the same substep size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Joint {
    /// First body index.
    pub a: usize,
    /// Second body index.
    pub b: usize,
    /// Rest length, meters.
    pub rest: f32,
    /// XPBD compliance alpha; `0.0` is rigid.
    pub compliance: f32,
}

impl Joint {
    /// Build with validation: distinct indices, rest finite in `(0, MAX_REST_M]`.
    /// Rigid: compliance reads `0.0`.
    pub fn new(a: usize, b: usize, rest: f32) -> Result<Self, ScheduleError> {
        Self::with_compliance(a, b, rest, 0.0)
    }

    /// Build with validation plus XPBD compliance: distinct indices, rest
    /// finite in `(0, MAX_REST_M]`; compliance sanitizes to rigid (`0.0`)
    /// when negative or non-finite, so bad softness never refuses a joint.
    pub fn with_compliance(
        a: usize,
        b: usize,
        rest: f32,
        compliance: f32,
    ) -> Result<Self, ScheduleError> {
        if a == b {
            return Err(ScheduleError::BadJoint(a, b));
        }
        if !rest.is_finite() || rest <= 0.0 || rest > MAX_REST_M {
            return Err(ScheduleError::BadRest(rest));
        }
        let clean = if compliance.is_finite() {
            compliance.max(0.0)
        } else {
            0.0
        };
        Ok(Self {
            a,
            b,
            rest,
            compliance: clean,
        })
    }
}

/// What refused a schedule call. Refusals change nothing.
#[derive(Clone, Debug, PartialEq)]
pub enum ScheduleError {
    /// Substep count outside `1..=MAX_SUBSTEPS`; the culprit is carried.
    BadSubsteps(usize),
    /// Iteration count outside `1..=MAX_ITERATIONS`; the culprit is carried.
    BadIterations(usize),
    /// Gravity non-finite or past the bound; the culprit is carried.
    BadGravity(f32),
    /// Joint endpoints equal or out of order context; both are carried.
    BadJoint(usize, usize),
    /// Joint rest length non-finite, non-positive or past the bound.
    BadRest(f32),
}

impl Display for ScheduleError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            ScheduleError::BadSubsteps(n) => {
                write!(f, "substep: substeps {n} outside 1..={MAX_SUBSTEPS}")
            }
            ScheduleError::BadIterations(n) => {
                write!(f, "substep: iterations {n} outside 1..={MAX_ITERATIONS}")
            }
            ScheduleError::BadGravity(g) => {
                write!(f, "substep: gravity {g} non-finite or past {MAX_GRAVITY}")
            }
            ScheduleError::BadJoint(a, b) => {
                write!(f, "substep: joint endpoints equal ({a}, {b})")
            }
            ScheduleError::BadRest(r) => {
                write!(f, "substep: rest {r} outside (0, {MAX_REST_M}]")
            }
        }
    }
}

impl std::error::Error for ScheduleError {}

/// Explicit substepped solver schedule over [`Body`] slices.
///
/// Joints stay sorted by `(a, b)` so insertion order never moves the hash.
/// Scratch buffers (`pairs`, `order`, sleep state) live inside the schedule
/// and are reused every tick, so a warm schedule allocates nothing per tick.
pub struct Schedule {
    cfg: ScheduleConfig,
    joints: Vec<Joint>,
    /// One XPBD lambda per joint: cleared each `step`, built over substeps
    /// x iterations, reused so a warm tick allocates nothing.
    lambdas: Vec<f32>,
    sleeping: Vec<bool>,
    timers: Vec<u32>,
    pairs: Vec<(usize, usize)>,
    order: Vec<usize>,
}

impl Schedule {
    /// Fresh schedule with default tuning and no joints.
    pub fn new() -> Self {
        Self::with_config(ScheduleConfig::default())
    }

    /// Fresh schedule with explicit tuning.
    pub fn with_config(cfg: ScheduleConfig) -> Self {
        Self {
            cfg,
            joints: Vec::new(),
            lambdas: Vec::new(),
            sleeping: Vec::new(),
            timers: Vec::new(),
            pairs: Vec::new(),
            order: Vec::new(),
        }
    }

    /// Current tuning.
    pub fn config(&self) -> ScheduleConfig {
        self.cfg
    }

    /// Explicit substep count.
    pub fn substeps(&self) -> usize {
        self.cfg.substeps
    }

    /// Joints in canonical order.
    pub fn joints(&self) -> &[Joint] {
        &self.joints
    }

    /// Sleeping mask, one entry per body seen.
    pub fn sleeping_mask(&self) -> &[bool] {
        &self.sleeping
    }

    /// Count of sleeping bodies.
    pub fn sleeping_count(&self) -> usize {
        self.sleeping.iter().filter(|s| **s).count()
    }

    /// Add a distance joint; the joint list stays sorted by `(a, b)`.
    pub fn add_joint(&mut self, joint: Joint) -> Result<(), ScheduleError> {
        if self.joints.len() >= 4096 {
            return Err(ScheduleError::BadJoint(joint.a, joint.b));
        }
        self.joints.push(joint);
        self.joints.sort_by_key(|j| (j.a, j.b));
        Ok(())
    }

    /// Drop all joints.
    pub fn clear_joints(&mut self) {
        self.joints.clear();
    }

    /// Wake one body.
    pub fn wake(&mut self, idx: usize) {
        crate::solver::wake_one(&mut self.sleeping, &mut self.timers, idx);
    }

    /// Schedule fingerprint: phase codes in [`PHASE_ORDER`] plus the substep
    /// and iteration counts. A reordered phase or a new count hashes different.
    pub fn schedule_hash(&self) -> u64 {
        let mut h = Fnv64::new();
        for phase in PHASE_ORDER {
            h.write_u8(phase.code());
        }
        h.write_u64(count_as_u64(self.cfg.substeps));
        h.write_u64(count_as_u64(self.cfg.iterations));
        h.finish()
    }

    /// Deterministic state hash: body bits plus sleep bytes plus the
    /// schedule fingerprint, so phase order and substep count are recorded.
    pub fn state_hash(&self, bodies: &[Body]) -> u64 {
        let mut h = Fnv64::new();
        for (i, b) in bodies.iter().enumerate() {
            for w in [b.x.to_bits(), b.y.to_bits(), b.vx.to_bits(), b.vy.to_bits()] {
                h.write_u32(w);
            }
            h.write_bool(self.sleeping.get(i).copied().unwrap_or(false));
        }
        h.write_u64(count_as_u64(bodies.len()));
        h.write_u64(self.schedule_hash());
        h.finish()
    }

    /// Worst joint drift, meters: max `|dist - rest|` over in-range joints,
    /// zero when no joint applies. Pure, no allocation.
    pub fn joint_drift(&self, bodies: &[Body]) -> f32 {
        let mut worst = 0.0f32;
        for j in &self.joints {
            if j.a >= bodies.len() || j.b >= bodies.len() {
                continue;
            }
            let dx = bodies[j.b].x - bodies[j.a].x;
            let dy = bodies[j.b].y - bodies[j.a].y;
            let dist = (dx * dx + dy * dy).sqrt();
            if !dist.is_finite() {
                continue;
            }
            let drift = (dist - j.rest).abs();
            if drift > worst {
                worst = drift;
            }
        }
        worst
    }

    /// Advance bodies by `dt` seconds through the explicit schedule: every
    /// substep runs integrate, contacts, joints, sleep in [`PHASE_ORDER`].
    /// Non-finite or non-positive `dt` steps nothing; out-of-range joints
    /// are skipped. Only `+ - * /` and `sqrt`, index order.
    pub fn step(&mut self, bodies: &mut [Body], dt: f32) {
        self.ensure(bodies.len());
        if bodies.is_empty() || !dt.is_finite() || dt <= 0.0 {
            return;
        }
        let dt = dt.min(0.25);
        let nsub = self.cfg.substeps.max(1);
        let h = dt / substeps_as_f32(nsub);
        if self.lambdas.len() < self.joints.len() {
            self.lambdas.resize(self.joints.len(), 0.0);
        }
        for l in &mut self.lambdas {
            *l = 0.0;
        }
        for _ in 0..nsub {
            for phase in PHASE_ORDER {
                match phase {
                    Phase::Integrate => integrate(bodies, &self.sleeping, self.cfg.gravity, h),
                    Phase::SolveContacts => self.solve_contacts(bodies),
                    Phase::SolveJoints => self.solve_joints(bodies, h),
                    Phase::UpdateSleep => self.update_sleep(bodies),
                }
            }
        }
    }

    fn ensure(&mut self, n: usize) {
        if self.sleeping.len() < n {
            self.sleeping.resize(n, false);
            self.timers.resize(n, 0);
        } else {
            self.sleeping.truncate(n);
            self.timers.truncate(n);
        }
    }

    fn solve_contacts(&mut self, bodies: &mut [Body]) {
        self.pairs.clear();
        for i in 0..bodies.len() {
            for j in (i + 1)..bodies.len() {
                if (bodies[i].x - bodies[j].x).abs() < bodies[i].hx + bodies[j].hx
                    && (bodies[i].y - bodies[j].y).abs() < bodies[i].hy + bodies[j].hy
                {
                    self.pairs.push((i, j));
                }
            }
        }
        self.order.clear();
        self.order.extend(0..self.pairs.len());
        self.order.sort_by_key(|&k| {
            let (a, b) = self.pairs[k];
            (a, b)
        });
        for _ in 0..self.cfg.iterations {
            for &pi in self.order.iter() {
                let (a, b) = self.pairs[pi];
                resolve_contact(
                    bodies,
                    &self.sleeping,
                    a,
                    b,
                    CONTACT_SLOP,
                    CONTACT_BAUMGARTE,
                );
            }
        }
    }

    /// XPBD distance solve in the paper's shape: `C = dist - rest` with the
    /// unit gradient along `(dx, dy)`, one lambda per joint that builds over
    /// substeps x iterations, and a compliance term scaled by the substep
    /// size (`alpha / h^2`; rigid joints read `0.0`). Canonical joint order,
    /// `+ - * /` and `sqrt` only, never `fma`.
    fn solve_joints(&mut self, bodies: &mut [Body], h: f32) {
        if !h.is_finite() || h <= 0.0 {
            return;
        }
        let hh = h * h;
        for _ in 0..self.cfg.iterations {
            for j in 0..self.joints.len() {
                let (a, b, rest, compliance) = (
                    self.joints[j].a,
                    self.joints[j].b,
                    self.joints[j].rest,
                    self.joints[j].compliance,
                );
                if a >= bodies.len() || b >= bodies.len() {
                    continue;
                }
                let sa = self.sleeping[a];
                let sb = self.sleeping[b];
                if sa && sb {
                    continue;
                }
                let dx = bodies[b].x - bodies[a].x;
                let dy = bodies[b].y - bodies[a].y;
                let dist_sq = dx * dx + dy * dy;
                if !dist_sq.is_finite() || dist_sq <= 1.0e-12 {
                    continue;
                }
                let dist = dist_sq.sqrt();
                if !dist.is_finite() || dist <= 0.0 {
                    continue;
                }
                let (wa, wb) = match (sa, sb) {
                    (true, false) => (0.0, 1.0),
                    (false, true) => (1.0, 0.0),
                    _ => (0.5, 0.5),
                };
                let wsum = wa + wb;
                if wsum <= 0.0 {
                    continue;
                }
                let soft = compliance.max(0.0) / hh;
                let denom = wsum + soft;
                if !denom.is_finite() || denom <= 0.0 {
                    continue;
                }
                let c = dist - rest;
                let dl = (-c - soft * self.lambdas[j]) / denom;
                if dl == 0.0 {
                    continue;
                }
                self.lambdas[j] += dl;
                let nx = dx / dist;
                let ny = dy / dist;
                bodies[a].x -= nx * dl * wa;
                bodies[a].y -= ny * dl * wa;
                bodies[b].x += nx * dl * wb;
                bodies[b].y += ny * dl * wb;
            }
        }
    }

    fn update_sleep(&mut self, bodies: &mut [Body]) {
        for (i, b) in bodies.iter_mut().enumerate() {
            let v2 = b.vx * b.vx + b.vy * b.vy;
            if !v2.is_finite() {
                self.sleeping[i] = false;
                self.timers[i] = 0;
                continue;
            }
            if self.sleeping[i] {
                if v2 >= WAKE_V2 {
                    self.sleeping[i] = false;
                    self.timers[i] = 0;
                }
                continue;
            }
            if v2 <= SLEEP_V2 {
                self.timers[i] = self.timers[i].saturating_add(1);
                if self.timers[i] >= SLEEP_STEPS {
                    self.sleeping[i] = true;
                    b.vx = 0.0;
                    b.vy = 0.0;
                }
            } else {
                self.timers[i] = 0;
            }
        }
    }
}

impl Default for Schedule {
    fn default() -> Self {
        Self::new()
    }
}

/// Semi-implicit Euler integrate plus the ground clamp, skipping sleepers.
fn integrate(bodies: &mut [Body], sleeping: &[bool], gravity: f32, h: f32) {
    for (i, b) in bodies.iter_mut().enumerate() {
        if sleeping.get(i).copied().unwrap_or(false) {
            continue;
        }
        b.vy -= gravity * h;
        b.x += b.vx * h;
        b.y += b.vy * h;
        if !b.x.is_finite() {
            b.x = 0.0;
            b.vx = 0.0;
        }
        if !b.y.is_finite() || !b.vy.is_finite() {
            b.y = b.hy;
            b.vy = 0.0;
            continue;
        }
        if b.y < b.hy {
            b.y = b.hy;
            if b.vy < 0.0 {
                b.vy = 0.0;
            }
            b.vx *= 0.9;
            if b.vx.abs() < 0.001 {
                b.vx = 0.0;
            }
        }
    }
}

/// One AABB contact: Baumgarte split by sleep-aware mass plus velocity kill
/// along the min-penetration axis. Deterministic, no allocation.
fn resolve_contact(
    bodies: &mut [Body],
    sleeping: &[bool],
    a: usize,
    b: usize,
    slop: f32,
    beta: f32,
) {
    if a >= bodies.len() || b >= bodies.len() || a == b {
        return;
    }
    if sleeping.get(a).copied().unwrap_or(false) && sleeping.get(b).copied().unwrap_or(false) {
        return;
    }
    let dx = bodies[b].x - bodies[a].x;
    let dy = bodies[b].y - bodies[a].y;
    let ox = bodies[a].hx + bodies[b].hx - dx.abs();
    if ox <= 0.0 {
        return;
    }
    let oy = bodies[a].hy + bodies[b].hy - dy.abs();
    if oy <= 0.0 {
        return;
    }
    let (wa, wb) = match (
        sleeping.get(a).copied().unwrap_or(false),
        sleeping.get(b).copied().unwrap_or(false),
    ) {
        (true, false) => (0.0, 1.0),
        (false, true) => (1.0, 0.0),
        _ => (0.5, 0.5),
    };
    if ox < oy {
        let s = if dx >= 0.0 { 1.0f32 } else { -1.0 };
        let corr = (ox - slop).max(0.0) * beta;
        bodies[a].x -= s * corr * wa * 2.0 * 0.5;
        bodies[b].x += s * corr * wb * 2.0 * 0.5;
        let rvn = (bodies[b].vx - bodies[a].vx) * s;
        if rvn < 0.0 {
            bodies[a].vx += s * rvn * wa;
            bodies[b].vx -= s * rvn * wb;
        }
    } else {
        let s = if dy >= 0.0 { 1.0f32 } else { -1.0 };
        let corr = (oy - slop).max(0.0) * beta;
        bodies[a].y -= s * corr * wa * 2.0 * 0.5;
        bodies[b].y += s * corr * wb * 2.0 * 0.5;
        let rvn = (bodies[b].vy - bodies[a].vy) * s;
        if rvn < 0.0 {
            bodies[a].vy += s * rvn * wa;
            bodies[b].vy -= s * rvn * wb;
        }
    }
}

/// Substep count to float divisor with saturating semantics.
fn substeps_as_f32(nsub: usize) -> f32 {
    f32::from(u16::try_from(nsub).unwrap_or(u16::MAX))
}

/// Body count to hash word with saturating semantics.
fn count_as_u64(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rigid::FIXED_DT;

    fn two_link() -> (Vec<Body>, Schedule) {
        let bodies = vec![
            Body::new(0.0, 3.0, 0.0, 0.0, 0.25, 0.25),
            Body::new(1.0, 3.0, 0.0, 0.0, 0.25, 0.25),
        ];
        let mut schedule = Schedule::new();
        schedule
            .add_joint(Joint::new(0, 1, 1.0).expect("valid joint"))
            .expect("add joint");
        (bodies, schedule)
    }

    fn run_tape(substeps: usize, ticks: usize) -> (Vec<Body>, Schedule) {
        let cfg =
            ScheduleConfig::new(substeps, DEFAULT_ITERATIONS, DEFAULT_GRAVITY).expect("valid cfg");
        let mut schedule = Schedule::with_config(cfg);
        schedule
            .add_joint(Joint::new(0, 1, 1.0).expect("valid joint"))
            .expect("add joint");
        let mut bodies = vec![
            Body::new(0.0, 3.0, 0.4, 0.0, 0.25, 0.25),
            Body::new(1.0, 3.2, -0.2, 0.0, 0.25, 0.25),
        ];
        for _ in 0..ticks {
            schedule.step(&mut bodies, FIXED_DT);
        }
        (bodies, schedule)
    }

    #[test]
    fn substep_order() {
        assert_eq!(
            PHASE_ORDER,
            [
                Phase::Integrate,
                Phase::SolveContacts,
                Phase::SolveJoints,
                Phase::UpdateSleep,
            ],
            "phase order is explicit: integrate, contacts, joints, sleep"
        );
        let a = Schedule::new();
        assert_eq!(a.substeps(), DEFAULT_SUBSTEPS);
        let four = run_tape(4, 600);
        let four_again = run_tape(4, 600);
        assert_eq!(
            four.1.state_hash(&four.0),
            four_again.1.state_hash(&four_again.0),
            "600-tick tape hashes twice"
        );
        assert_eq!(
            four.1.schedule_hash(),
            four_again.1.schedule_hash(),
            "schedule fingerprint stable"
        );
        let two = run_tape(2, 600);
        assert_ne!(
            four.1.state_hash(&four.0),
            two.1.state_hash(&two.0),
            "substep count is recorded in the hash"
        );
        assert_ne!(
            four.1.schedule_hash(),
            two.1.schedule_hash(),
            "schedule fingerprint moves with the count"
        );
        let drift = four.1.joint_drift(&four.0);
        assert!(
            drift < 0.005,
            "joint drift within 5 mm after 600 ticks, got {drift:.6} m"
        );
        let (bodies, schedule) = two_link();
        let _ = (bodies, schedule);
    }

    #[test]
    fn substep_joint_xpbd_lambda_shape() {
        fn tape(compliance: f32, ticks: usize) -> (Vec<Body>, Schedule) {
            let cfg =
                ScheduleConfig::new(4, DEFAULT_ITERATIONS, DEFAULT_GRAVITY).expect("valid cfg");
            let mut schedule = Schedule::with_config(cfg);
            schedule
                .add_joint(Joint::with_compliance(0, 1, 1.0, compliance).expect("valid joint"))
                .expect("add joint");
            let mut bodies = vec![
                Body::new(0.0, 3.0, 0.0, 0.0, 0.25, 0.25),
                Body::new(1.2, 3.0, 0.0, 0.0, 0.25, 0.25),
            ];
            for _ in 0..ticks {
                schedule.step(&mut bodies, FIXED_DT);
            }
            (bodies, schedule)
        }
        let (rigid_bodies, rigid) = tape(0.0, 120);
        let (soft_bodies, soft) = tape(0.5, 120);
        let rigid_drift = rigid.joint_drift(&rigid_bodies);
        let soft_drift = soft.joint_drift(&soft_bodies);
        assert!(
            rigid_drift < 0.005,
            "rigid link closes a 0.2 m stretch, got {rigid_drift:.6} m"
        );
        assert!(
            soft_drift > rigid_drift,
            "compliance softens: soft {soft_drift:.6} m vs rigid {rigid_drift:.6} m"
        );
        let (again_bodies, again) = tape(0.0, 120);
        assert_eq!(
            rigid.state_hash(&rigid_bodies),
            again.state_hash(&again_bodies),
            "repeat tape hashes equal"
        );
    }

    #[test]
    fn substep_rejects_bad_input() {
        assert_eq!(
            ScheduleConfig::new(0, 10, 9.8).unwrap_err(),
            ScheduleError::BadSubsteps(0)
        );
        assert_eq!(
            ScheduleConfig::new(4, 0, 9.8).unwrap_err(),
            ScheduleError::BadIterations(0)
        );
        assert!(
            matches!(
                ScheduleConfig::new(4, 10, f32::NAN).unwrap_err(),
                ScheduleError::BadGravity(_)
            ),
            "NaN gravity is refused"
        );
        assert_eq!(
            Joint::new(1, 1, 1.0).unwrap_err(),
            ScheduleError::BadJoint(1, 1)
        );
        assert_eq!(
            Joint::new(0, 1, 0.0).unwrap_err(),
            ScheduleError::BadRest(0.0)
        );
        let mut schedule = Schedule::new();
        let before = schedule.schedule_hash();
        schedule.step(&mut [], FIXED_DT);
        schedule.step(&mut [Body::new(0.0, 1.0, 0.0, 0.0, 0.5, 0.5)], f32::NAN);
        assert_eq!(schedule.schedule_hash(), before);
        assert_eq!(schedule.sleeping_count(), 0);
    }

    #[test]
    fn substep_tick_stays_alloc_free() {
        use engine_core::alloc_probe;
        let (mut bodies, mut schedule) = two_link();
        for _ in 0..70 {
            schedule.step(&mut bodies, FIXED_DT);
        }
        let ((), allocs) = alloc_probe::measure(|| {
            for _ in 0..60 {
                schedule.step(&mut bodies, FIXED_DT);
            }
            let _ = schedule.state_hash(&bodies);
        });
        assert_eq!(allocs.calls, 0, "warm tick path allocates nothing");
    }
}
