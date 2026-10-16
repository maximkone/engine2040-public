//! shape_match: shape matching for bonded debris, so a cluster of chunks held
//! by bonds keeps its form under torque instead of drifting apart.
//!
//! Consumer: the arena's bonded-chunk step may call [`ShapeMatcher::step`]
//! after it integrates a cluster; it is opt-in and never part of
//! `BondedCollection::step`, so the tick hash of plain props does not move.
//! Data owner: `engine-destruction`. Units: meters, seconds, mass in the
//! caller's units. Determinism scope: `+ - * /` and one `sqrt` per
//! normalization, fixed iteration count, index-order sums, no trig, no
//! `mul_add`, so equal inputs give equal bits on every PC; the state hash
//! folds the warm rotation bits. Errors: [`ShapeError`] for bad clusters,
//! springs and steps; nothing panics on user data. Budget: `O(n)` per step
//! plus at most [`MAX_ITERS`] rotation iterations over a 3x3 matrix, so a
//! 64-chunk cluster costs a few microseconds.
//!
//! PORT: shape-matching rotation extraction and the dt-independent spring
//! discipline follow dimforge/rapier `soft_body_shape_matching.rs`
//! (`extract_rotation`) and `IntegrationParameters::erp_inv_dt`, Apache-2.0
//! (rapier3d 0.35/0.36 are workspace dependencies); the first-order
//! quaternion update stands in for the library's exact sin/cos step, and
//! the volume proxy is this crate's own. Idea source: Mueller et al. 2005
//! "Meshless deformations based on shape matching", and Mueller, Bender,
//! Chentanez, Macklin 2016 "A Robust Method to Extract the Rotational Part
//! of Deformations".

use engine_core::Fnv64;

/// Most chunks one cluster holds.
pub const MAX_CLUSTER: usize = 64;
/// Rotation iterations per step at most; warm starts need one or two.
pub const MAX_ITERS: usize = 8;
/// Largest accepted step, seconds.
pub const MAX_DT: f32 = 0.25;
/// Largest accepted natural frequency, Hz.
pub const MAX_FREQUENCY_HZ: f32 = 120.0;
/// `2 * pi` as `f32`, exact to the type.
const TWO_PI: f32 = 6.283_185_5;
/// Stops the rotation iteration once the correction is this small.
const ROT_EPS: f32 = 1e-7;

/// Why a cluster, spring or step was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShapeError {
    /// Fewer than two or more than [`MAX_CLUSTER`] chunks, or the slices
    /// disagree in length.
    BadCount,
    /// A position, velocity or rest point is not finite.
    NotFinite,
    /// A mass is not finite and positive.
    BadMass,
    /// Frequency or damping ratio is out of range.
    BadSpring,
    /// The step is non-finite, non-positive or past [`MAX_DT`].
    BadStep,
    /// All rest points coincide, so no shape exists.
    DegenerateShape,
}

impl core::fmt::Display for ShapeError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadCount => write!(f, "shape cluster size is out of bounds"),
            Self::NotFinite => write!(f, "shape cluster holds a non-finite value"),
            Self::BadMass => write!(f, "shape cluster mass is out of bounds"),
            Self::BadSpring => write!(f, "shape spring is out of bounds"),
            Self::BadStep => write!(f, "shape step is out of bounds"),
            Self::DegenerateShape => write!(f, "shape rest points coincide"),
        }
    }
}

impl std::error::Error for ShapeError {}

/// A spring by natural frequency and damping ratio, so its stiffness does
/// not depend on the step length (the `erp_inv_dt` discipline).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpringCoefficients {
    /// Natural frequency, Hz; zero turns the match off.
    pub natural_frequency: f32,
    /// Damping ratio; `1.0` is critical, larger is stiffer per step.
    pub damping_ratio: f32,
}

impl SpringCoefficients {
    /// Range-checks the spring.
    pub fn validate(&self) -> Result<(), ShapeError> {
        let freq_ok = self.natural_frequency.is_finite()
            && (0.0..=MAX_FREQUENCY_HZ).contains(&self.natural_frequency);
        let damp_ok = self.damping_ratio.is_finite() && self.damping_ratio > 0.0;
        if freq_ok && damp_ok && self.damping_ratio <= 100.0 {
            Ok(())
        } else {
            Err(ShapeError::BadSpring)
        }
    }

    /// Error reduction per second: `w / (dt * w + 2 * zeta)`.
    #[must_use]
    pub fn erp_inv_dt(&self, dt: f32) -> f32 {
        let w = self.natural_frequency * TWO_PI;
        w / (dt * w + 2.0 * self.damping_ratio)
    }

    /// Fraction of the error removed in one step of `dt`, in `[0, 1)`.
    #[must_use]
    pub fn erp(&self, dt: f32) -> f32 {
        dt * self.erp_inv_dt(dt)
    }
}

/// Rest shape of one cluster plus the warm-started rotation.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeMatcher {
    /// Rest points relative to the rest center of mass.
    rest: Vec<[f32; 3]>,
    mass: Vec<f32>,
    total_mass: f32,
    /// Mean rest distance from the center of mass, the volume proxy.
    rest_radius: f32,
    spring: SpringCoefficients,
    /// Warm rotation as `[x, y, z, w]`, unit length.
    rot: [f32; 4],
}

impl ShapeMatcher {
    /// Builds a matcher from the rest positions and masses of a cluster.
    pub fn new(
        rest: &[[f32; 3]],
        mass: &[f32],
        spring: SpringCoefficients,
    ) -> Result<Self, ShapeError> {
        spring.validate()?;
        if rest.len() < 2 || rest.len() > MAX_CLUSTER || rest.len() != mass.len() {
            return Err(ShapeError::BadCount);
        }
        if rest.iter().any(|p| p.iter().any(|f| !f.is_finite())) {
            return Err(ShapeError::NotFinite);
        }
        if mass.iter().any(|m| !m.is_finite() || *m <= 0.0) {
            return Err(ShapeError::BadMass);
        }
        let total_mass: f32 = mass.iter().sum();
        let com = center_of_mass(rest, mass, total_mass);
        let rel: Vec<[f32; 3]> = rest.iter().map(|p| sub(*p, com)).collect();
        let mut radius = 0.0f32;
        for (p, m) in rel.iter().zip(mass) {
            radius += m * length(*p);
        }
        let rest_radius = radius / total_mass;
        if !total_mass.is_finite() || rest_radius.is_nan() || rest_radius <= 1e-6 {
            return Err(ShapeError::DegenerateShape);
        }
        Ok(Self {
            rest: rel,
            mass: mass.to_vec(),
            total_mass,
            rest_radius,
            spring,
            rot: [0.0, 0.0, 0.0, 1.0],
        })
    }

    /// Chunks in the cluster.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rest.len()
    }

    /// True when the cluster is empty (never after a successful build).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rest.is_empty()
    }

    /// The warm rotation as `[x, y, z, w]`.
    #[must_use]
    pub fn rotation(&self) -> [f32; 4] {
        self.rot
    }

    /// Goal positions for the current cloud: `com + R * rest`, after the
    /// rotation extraction. Does not move anything.
    pub fn goals(&mut self, pos: &[[f32; 3]], out: &mut [[f32; 3]]) -> Result<(), ShapeError> {
        self.check(pos)?;
        if out.len() != pos.len() {
            return Err(ShapeError::BadCount);
        }
        let com = center_of_mass(pos, &self.mass, self.total_mass);
        self.rot = extract_rotation(&self.covariance(pos, com), self.rot);
        let r = matrix_of(self.rot);
        for (g, q) in out.iter_mut().zip(&self.rest) {
            *g = add(com, mul_vec(&r, *q));
        }
        Ok(())
    }

    /// One shape-matching step after the caller integrated `pos` by `vel`
    /// over `dt`: each chunk moves a spring fraction toward its goal and
    /// its velocity takes the matching share, shape first, then the volume
    /// proxy (mean radius back toward rest) with the same fraction.
    pub fn step(
        &mut self,
        pos: &mut [[f32; 3]],
        vel: &mut [[f32; 3]],
        dt: f32,
    ) -> Result<(), ShapeError> {
        if !dt.is_finite() || dt <= 0.0 || dt > MAX_DT {
            return Err(ShapeError::BadStep);
        }
        self.check(pos)?;
        if vel.len() != pos.len() || vel.iter().any(|v| v.iter().any(|f| !f.is_finite())) {
            return Err(ShapeError::BadCount);
        }
        let alpha = self.spring.erp(dt);
        let com = center_of_mass(pos, &self.mass, self.total_mass);
        self.rot = extract_rotation(&self.covariance(pos, com), self.rot);
        let r = matrix_of(self.rot);
        // Constraint order: shape first (goal deltas leave the center of
        // mass where it is), then the volume proxy on the shaped cloud.
        let mut shaped = [[0.0f32; 3]; MAX_CLUSTER];
        let mut radius = 0.0f32;
        for i in 0..pos.len() {
            let goal = add(com, mul_vec(&r, self.rest[i]));
            shaped[i] = add(pos[i], scale3(sub(goal, pos[i]), alpha));
            radius += self.mass[i] * length(sub(shaped[i], com));
        }
        radius /= self.total_mass;
        let scale = if radius > 1e-6 {
            self.rest_radius / radius
        } else {
            1.0
        };
        for i in 0..pos.len() {
            let radial = sub(shaped[i], com);
            let next = add(shaped[i], scale3(radial, (scale - 1.0) * alpha));
            let delta = sub(next, pos[i]);
            pos[i] = next;
            vel[i] = add(vel[i], scale3(delta, 1.0 / dt));
        }
        Ok(())
    }

    /// Largest distance from a chunk to its goal under the best-fit
    /// rotation: how far the cluster is from holding its shape.
    pub fn shape_error(&mut self, pos: &[[f32; 3]]) -> Result<f32, ShapeError> {
        let mut goals = vec![[0.0f32; 3]; pos.len()];
        self.goals(pos, &mut goals)?;
        let mut worst = 0.0f32;
        for (p, g) in pos.iter().zip(&goals) {
            worst = worst.max(length(sub(*p, *g)));
        }
        Ok(worst)
    }

    /// FNV-1a over the warm rotation bits.
    #[must_use]
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv64::new();
        for f in self.rot {
            h.write_u32(f.to_bits());
        }
        h.finish()
    }

    fn check(&self, pos: &[[f32; 3]]) -> Result<(), ShapeError> {
        if pos.len() != self.rest.len() {
            return Err(ShapeError::BadCount);
        }
        if pos.iter().any(|p| p.iter().any(|f| !f.is_finite())) {
            return Err(ShapeError::NotFinite);
        }
        Ok(())
    }

    /// Mass-weighted covariance `A = sum m (p - com) q^T`, rows over the
    /// current offset, columns over the rest offset.
    fn covariance(&self, pos: &[[f32; 3]], com: [f32; 3]) -> [[f32; 3]; 3] {
        let mut a = [[0.0f32; 3]; 3];
        for ((p, q), m) in pos.iter().zip(&self.rest).zip(&self.mass) {
            let d = sub(*p, com);
            for (row, dr) in a.iter_mut().zip(d) {
                for (cell, qc) in row.iter_mut().zip(q) {
                    *cell += m * dr * qc;
                }
            }
        }
        a
    }
}

/// Rotation of the polar decomposition of `a`, warm-started from `warm`:
/// iterates `q <- normalize(q + 0.5 * omega * q)` with
/// `omega = sum(r_j x a_j) / (|sum(r_j . a_j)| + eps)` over the columns.
fn extract_rotation(a: &[[f32; 3]; 3], warm: [f32; 4]) -> [f32; 4] {
    let mut q = warm;
    for _ in 0..MAX_ITERS {
        let r = matrix_of(q);
        let mut omega = [0.0f32; 3];
        let mut denom = 0.0f32;
        for c in 0..3 {
            let rc = [r[0][c], r[1][c], r[2][c]];
            let ac = [a[0][c], a[1][c], a[2][c]];
            // r_j x a_j, component by component.
            omega = add(
                omega,
                [
                    rc[1] * ac[2] - rc[2] * ac[1],
                    rc[2] * ac[0] - rc[0] * ac[2],
                    rc[0] * ac[1] - rc[1] * ac[0],
                ],
            );
            denom += dot(rc, ac);
        }
        let scale = 1.0 / (denom.abs() + 1e-9);
        omega = scale3(omega, scale);
        let w = length(omega);
        if w < ROT_EPS {
            break;
        }
        // Quaternion product (omega/2, 0) * q, added to q, renormalized.
        let h = scale3(omega, 0.5);
        let dq = [
            h[0] * q[3] + h[1] * q[2] - h[2] * q[1],
            h[1] * q[3] + h[2] * q[0] - h[0] * q[2],
            h[2] * q[3] + h[0] * q[1] - h[1] * q[0],
            -(h[0] * q[0] + h[1] * q[1] + h[2] * q[2]),
        ];
        let next = [q[0] + dq[0], q[1] + dq[1], q[2] + dq[2], q[3] + dq[3]];
        let len =
            (next[0] * next[0] + next[1] * next[1] + next[2] * next[2] + next[3] * next[3]).sqrt();
        if len.is_nan() || len <= 1e-12 {
            break;
        }
        q = [next[0] / len, next[1] / len, next[2] / len, next[3] / len];
    }
    q
}

/// Rotation matrix of a unit quaternion `[x, y, z, w]`, rows.
fn matrix_of(q: [f32; 4]) -> [[f32; 3]; 3] {
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let (xx, yy, zz) = (x * x, y * y, z * z);
    let (xy, xz, yz) = (x * y, x * z, y * z);
    let (wx, wy, wz) = (w * x, w * y, w * z);
    [
        [1.0 - 2.0 * (yy + zz), 2.0 * (xy - wz), 2.0 * (xz + wy)],
        [2.0 * (xy + wz), 1.0 - 2.0 * (xx + zz), 2.0 * (yz - wx)],
        [2.0 * (xz - wy), 2.0 * (yz + wx), 1.0 - 2.0 * (xx + yy)],
    ]
}

fn center_of_mass(pos: &[[f32; 3]], mass: &[f32], total: f32) -> [f32; 3] {
    let mut c = [0.0f32; 3];
    for (p, m) in pos.iter().zip(mass) {
        c = add(c, scale3(*p, *m));
    }
    scale3(c, 1.0 / total)
}

fn mul_vec(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale3(a: [f32; 3], s: f32) -> [f32; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn length(a: [f32; 3]) -> f32 {
    dot(a, a).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn cube() -> Vec<[f32; 3]> {
        let mut pts = Vec::new();
        for x in [0.0f32, 1.0] {
            for y in [0.0f32, 1.0] {
                for z in [0.0f32, 1.0] {
                    pts.push([x, y, z]);
                }
            }
        }
        pts
    }

    fn spring(freq: f32) -> SpringCoefficients {
        SpringCoefficients {
            natural_frequency: freq,
            damping_ratio: 1.0,
        }
    }

    /// Rotates a point by the unit quaternion of `(axis * k, 1)`: Cayley
    /// style, no trig, so the test stays inside the sim math rules.
    fn rotate_by(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
        let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
        let m = matrix_of([q[0] / n, q[1] / n, q[2] / n, q[3] / n]);
        mul_vec(&m, v)
    }

    /// One tick of a free cluster: integrate, then shape-match.
    fn tick(m: &mut ShapeMatcher, pos: &mut [[f32; 3]], vel: &mut [[f32; 3]]) {
        for (p, v) in pos.iter_mut().zip(vel.iter()) {
            *p = add(*p, scale3(*v, DT));
        }
        m.step(pos, vel, DT).expect("steps");
    }

    /// The torque fixture: chunk 0 is kicked up and sideways for five
    /// ticks, then the cluster runs on for 180 ticks; returns the worst
    /// shape error after tick 20 and the final hash of positions.
    fn torque_run(freq: f32) -> (f32, u64) {
        let rest = cube();
        let mut m = ShapeMatcher::new(&rest, &[1.0; 8], spring(freq)).expect("cluster");
        let mut pos = rest.clone();
        let mut vel = vec![[0.0f32; 3]; 8];
        let mut worst = 0.0f32;
        for t in 0..185 {
            if t < 5 {
                vel[0] = add(vel[0], [0.0, 4.0, 2.0]);
            }
            tick(&mut m, &mut pos, &mut vel);
            if t >= 20 {
                worst = worst.max(m.shape_error(&pos).expect("error"));
            }
        }
        let mut h = Fnv64::new();
        for p in &pos {
            for f in p {
                h.write_u32(f.to_bits());
            }
        }
        h.write_u64(m.state_hash());
        (worst, h.finish())
    }

    #[test]
    fn shape_match_holds_the_shape_under_the_torque_fixture() {
        let (held, hash_a) = torque_run(6.0);
        let (_, hash_b) = torque_run(6.0);
        assert_eq!(hash_a, hash_b, "run x2 identical hashes");
        let (loose, _) = torque_run(0.0);
        assert!(held < 0.1, "matched cluster error {held}");
        assert!(loose > 1.0, "free cluster error {loose}");
        println!(
            "shape_match torque fixture: held {held:.4} m, free {loose:.4} m, hash {hash_a:016x}"
        );
    }

    #[test]
    fn a_cluster_at_rest_is_a_fixed_point() {
        let rest = cube();
        let mut m = ShapeMatcher::new(&rest, &[1.0; 8], spring(6.0)).expect("cluster");
        let mut pos: Vec<[f32; 3]> = rest.iter().map(|p| add(*p, [5.0, 2.0, -1.0])).collect();
        let mut vel = vec![[0.0f32; 3]; 8];
        let before = pos.clone();
        m.step(&mut pos, &mut vel, DT).expect("steps");
        for (a, b) in pos.iter().zip(&before) {
            assert!(length(sub(*a, *b)) < 1e-5);
        }
        assert!(vel.iter().all(|v| length(*v) < 1e-3));
        assert!(m.shape_error(&pos).expect("error") < 1e-5);
        let q = m.rotation();
        assert!((q[3].abs() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn the_rotation_extraction_recovers_a_known_rotation() {
        let rest = cube();
        let mut m = ShapeMatcher::new(&rest, &[1.0; 8], spring(6.0)).expect("cluster");
        let q = [0.2f32, -0.3, 0.1, 1.0];
        let com = [0.5f32, 0.5, 0.5];
        let pos: Vec<[f32; 3]> = rest
            .iter()
            .map(|p| add(com, rotate_by(q, sub(*p, com))))
            .collect();
        assert!(m.shape_error(&pos).expect("error") < 1e-4);
    }

    #[test]
    fn the_spring_is_dt_independent_by_its_frequency() {
        let s = spring(6.0);
        // Half the step twice removes about the same error as one step.
        let one = s.erp(1.0 / 60.0);
        let two = 1.0 - (1.0 - s.erp(1.0 / 120.0)) * (1.0 - s.erp(1.0 / 120.0));
        assert!((one - two).abs() < 0.05, "one {one} vs two {two}");
        assert!(one > 0.0 && one < 1.0);
        assert_eq!(spring(0.0).erp(DT), 0.0);
    }

    #[test]
    fn bad_clusters_springs_and_steps_refuse() {
        let rest = cube();
        let ok = spring(6.0);
        assert_eq!(
            ShapeMatcher::new(&rest[..1], &[1.0], ok).unwrap_err(),
            ShapeError::BadCount
        );
        assert_eq!(
            ShapeMatcher::new(&rest, &[1.0; 7], ok).unwrap_err(),
            ShapeError::BadCount
        );
        assert_eq!(
            ShapeMatcher::new(&rest, &[0.0; 8], ok).unwrap_err(),
            ShapeError::BadMass
        );
        let mut nan = rest.clone();
        nan[3][1] = f32::NAN;
        assert_eq!(
            ShapeMatcher::new(&nan, &[1.0; 8], ok).unwrap_err(),
            ShapeError::NotFinite
        );
        assert_eq!(
            ShapeMatcher::new(&[[1.0; 3], [1.0; 3]], &[1.0; 2], ok).unwrap_err(),
            ShapeError::DegenerateShape
        );
        for bad in [-1.0, f32::NAN, MAX_FREQUENCY_HZ + 1.0] {
            assert_eq!(
                ShapeMatcher::new(&rest, &[1.0; 8], spring(bad)).unwrap_err(),
                ShapeError::BadSpring
            );
        }
        let mut m = ShapeMatcher::new(&rest, &[1.0; 8], ok).expect("cluster");
        let mut pos = rest.clone();
        let mut vel = vec![[0.0f32; 3]; 8];
        for dt in [0.0, -1.0, f32::NAN, MAX_DT + 1.0] {
            assert_eq!(m.step(&mut pos, &mut vel, dt), Err(ShapeError::BadStep));
        }
        assert_eq!(
            m.step(&mut pos[..4], &mut vel, DT),
            Err(ShapeError::BadCount)
        );
        pos[2][0] = f32::INFINITY;
        assert_eq!(m.step(&mut pos, &mut vel, DT), Err(ShapeError::NotFinite));
    }

    #[test]
    fn a_squashed_cluster_recovers_its_size() {
        let rest = cube();
        let mut m = ShapeMatcher::new(&rest, &[1.0; 8], spring(6.0)).expect("cluster");
        let com = [0.5f32, 0.5, 0.5];
        // Uniformly squashed to 60 percent: pure rotation fits it, volume must grow it.
        let mut pos: Vec<[f32; 3]> = rest
            .iter()
            .map(|p| add(com, scale3(sub(*p, com), 0.6)))
            .collect();
        let mut vel = vec![[0.0f32; 3]; 8];
        let radius = |pos: &[[f32; 3]]| {
            let mut r = 0.0f32;
            for p in pos {
                r += length(sub(*p, com));
            }
            r / 8.0
        };
        let before = radius(&pos);
        for _ in 0..30 {
            tick(&mut m, &mut pos, &mut vel);
        }
        assert!(radius(&pos) > before * 1.2, "squash recovers toward rest");
    }
}
