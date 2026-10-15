//! rapier: Rapier adapter behind the opt-in `rapier` feature.
//!
//! Capsules plus static boxes on a fixed step, with a canonical state hash
//! and owned snapshot/restore for rollback. The headless default build stays
//! std-only; this module only exists with `features = ["rapier"]`.
//!
//! Determinism scope (HASH-LAW tier note): the world runs rapier3d with
//! `enhanced-determinism` and without SIMD/parallel features, one fixed `dt`
//! per [`World::step`] call, bodies iterated in spawn order. The glue in this
//! module uses only comparisons, `+ - * /`, `round` and integer math: no
//! formatting of floats, no `sin`/`cos`/`exp`/`powf` family calls. Rapier
//! internals use `+ - * /` and `sqrt` under their `enhanced-determinism`
//! guarantee; that internal float use is the arena integration's scope, not
//! part of this glue. Character controllers, arenas and network rollback
//! consume this API;
//! they own any further determinism claims.
#![forbid(unsafe_code)]

use crate::islands::{self, SnapshotError, STATE_FLOATS};
use engine_core::Fnv64;
use rapier3d::dynamics::{
    CCDSolver, ImpulseJointSet, IntegrationParameters, IslandManager, MultibodyJointSet,
    RigidBodyBuilder, RigidBodyHandle, RigidBodySet, SoftBodySet,
};
use rapier3d::geometry::{BroadPhaseBvh, ColliderBuilder, ColliderSet, NarrowPhase};
use rapier3d::math::{Rotation, Vector};
use rapier3d::pipeline::PhysicsPipeline;
use std::fmt::{Display, Formatter};

/// Fixed step, 60 Hz. Callers pass this to [`World::step`].
pub const FIXED_DT: f32 = 1.0 / 60.0;
/// Gravity, m/s^2, -Y.
pub const GRAVITY_Y: f32 = -9.81;
/// Hash quantization: state floats are scaled by this and rounded to `i64`.
pub const HASH_SCALE: f64 = 1_000_000.0;
/// Upper bound for accepted lengths (radii, heights, half-extents, positions).
pub(crate) const MAX_LEN: f32 = 1_000.0;

/// Typed errors for the Rapier adapter. User input never panics.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// `step` needs a finite `dt` greater than zero.
    InvalidDt,
    /// Capsule radius must be finite and in `(0, MAX_LEN]`.
    InvalidRadius,
    /// Capsule cylinder height must be finite and in `[0, MAX_LEN]`.
    InvalidHeight,
    /// Static-box half-extents must be finite and each in `(0, MAX_LEN]`.
    InvalidHalfExtents,
    /// A position component was non-finite or out of range.
    NonFinitePosition,
    /// Snapshot body count does not match the world's dynamic body count.
    SnapshotMismatch,
    /// Island snapshot bytes were refused; the world is unchanged.
    SnapshotBytes(SnapshotError),
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Error::InvalidDt => "invalid dt: need finite dt > 0",
            Error::InvalidRadius => "invalid radius: need finite radius in (0, 1000]",
            Error::InvalidHeight => "invalid height: need finite height in [0, 1000]",
            Error::InvalidHalfExtents => "invalid half-extents: need finite values in (0, 1000]",
            Error::NonFinitePosition => "invalid position: need finite values within +-1000",
            Error::SnapshotMismatch => "snapshot body count does not match world",
            Error::SnapshotBytes(e) => return write!(f, "island snapshot bytes refused: {e}"),
        };
        f.write_str(s)
    }
}

impl std::error::Error for Error {}

/// Kinematic state of one dynamic body. Plain owned data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyState {
    /// Center position, meters.
    pub pos: [f32; 3],
    /// Rotation quaternion `(x, y, z, w)`.
    pub rot: [f32; 4],
    /// Linear velocity, m/s.
    pub linvel: [f32; 3],
    /// Angular velocity, rad/s.
    pub angvel: [f32; 3],
}

impl BodyState {
    /// The state as the [`STATE_FLOATS`] words the island snapshot carries.
    fn words(&self) -> [f32; STATE_FLOATS] {
        let [px, py, pz] = self.pos;
        let [rx, ry, rz, rw] = self.rot;
        let [lx, ly, lz] = self.linvel;
        let [ax, ay, az] = self.angvel;
        [px, py, pz, rx, ry, rz, rw, lx, ly, lz, ax, ay, az]
    }

    /// Inverse of [`Self::words`].
    fn from_words(w: &[f32; STATE_FLOATS]) -> Self {
        Self {
            pos: [w[0], w[1], w[2]],
            rot: [w[3], w[4], w[5], w[6]],
            linvel: [w[7], w[8], w[9]],
            angvel: [w[10], w[11], w[12]],
        }
    }
}

/// Owned snapshot of every dynamic body in spawn order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    /// One entry per dynamic body, spawn order.
    pub bodies: Vec<BodyState>,
}

/// One accepted spawn, kept so a reset can rebuild the world in the same order.
#[derive(Clone, Copy, Debug)]
enum Spawn {
    Capsule {
        pos: [f32; 3],
        radius: f32,
        height: f32,
    },
    Static {
        half: [f32; 3],
        pos: [f32; 3],
    },
}

/// Rapier world: dynamic capsules plus static boxes, fixed-stepped.
pub struct World {
    pipeline: PhysicsPipeline,
    gravity: Vector,
    params: IntegrationParameters,
    islands: IslandManager,
    broad: BroadPhaseBvh,
    narrow: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    soft_bodies: SoftBodySet,
    ccd: CCDSolver,
    /// Dynamic body handles in spawn order; fixes iteration/hash order.
    order: Vec<RigidBodyHandle>,
    /// Every accepted spawn in order, statics included, to rebuild from.
    spawns: Vec<Spawn>,
}

impl World {
    /// New empty world with Earth gravity and default integration parameters.
    pub fn new() -> Self {
        let params = IntegrationParameters {
            dt: FIXED_DT,
            ..Default::default()
        };
        Self {
            pipeline: PhysicsPipeline::new(),
            gravity: Vector::new(0.0, GRAVITY_Y, 0.0),
            params,
            islands: IslandManager::new(),
            broad: BroadPhaseBvh::new(),
            narrow: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            soft_bodies: SoftBodySet::new(),
            ccd: CCDSolver::new(),
            order: Vec::new(),
            spawns: Vec::new(),
        }
    }

    /// Spawn a Y-up dynamic capsule. `height` is the cylinder mid-section
    /// height (total height is `height + 2 * radius`). Returns the body index.
    pub fn spawn_capsule(
        &mut self,
        pos: [f32; 3],
        radius: f32,
        height: f32,
    ) -> Result<usize, Error> {
        check_pos(pos)?;
        if !radius.is_finite() || radius == 0.0 || !(0.0..=MAX_LEN).contains(&radius) {
            return Err(Error::InvalidRadius);
        }
        if !height.is_finite() || !(0.0..=MAX_LEN).contains(&height) {
            return Err(Error::InvalidHeight);
        }
        self.spawns.push(Spawn::Capsule {
            pos,
            radius,
            height,
        });
        Ok(self.insert_capsule(pos, radius, height))
    }

    /// Insert a validated dynamic capsule; returns its body index.
    fn insert_capsule(&mut self, pos: [f32; 3], radius: f32, height: f32) -> usize {
        let body = RigidBodyBuilder::dynamic()
            .translation(Vector::new(pos[0], pos[1], pos[2]))
            // Sleeping is disabled so kinematic state (pos/rot/vel) is the
            // complete future-affecting state and restore is bit-identical.
            .can_sleep(false)
            .build();
        let handle = self.bodies.insert(body);
        let collider = ColliderBuilder::capsule_y(height * 0.5, radius)
            .density(1.0)
            .friction(0.8)
            .restitution(0.0)
            .build();
        self.colliders
            .insert_with_parent(collider, handle, &mut self.bodies);
        self.order.push(handle);
        self.order.len() - 1
    }

    /// Add a static box. Returns the static index (not hashed, never moves).
    pub fn add_static_box(
        &mut self,
        half_extents: [f32; 3],
        pos: [f32; 3],
    ) -> Result<usize, Error> {
        check_pos(pos)?;
        for h in half_extents {
            if !h.is_finite() || h == 0.0 || !(0.0..=MAX_LEN).contains(&h) {
                return Err(Error::InvalidHalfExtents);
            }
        }
        self.spawns.push(Spawn::Static {
            half: half_extents,
            pos,
        });
        Ok(self.insert_static(half_extents, pos))
    }

    /// Insert a validated static box; returns its static index.
    fn insert_static(&mut self, half_extents: [f32; 3], pos: [f32; 3]) -> usize {
        let body = RigidBodyBuilder::fixed()
            .translation(Vector::new(pos[0], pos[1], pos[2]))
            .build();
        let handle = self.bodies.insert(body);
        let collider = ColliderBuilder::cuboid(half_extents[0], half_extents[1], half_extents[2])
            .friction(0.8)
            .restitution(0.0)
            .build();
        self.colliders
            .insert_with_parent(collider, handle, &mut self.bodies);
        self.order.len()
    }

    /// Advance the world by one fixed step. Only finite `dt > 0` is accepted;
    /// callers use [`FIXED_DT`].
    pub fn step(&mut self, dt: f32) -> Result<(), Error> {
        if !dt.is_finite() || dt <= 0.0 {
            return Err(Error::InvalidDt);
        }
        self.params.dt = dt;
        let hooks = ();
        let events = ();
        self.pipeline.step(
            self.gravity,
            &self.params,
            &mut self.islands,
            &mut self.broad,
            &mut self.narrow,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.soft_bodies,
            &mut self.ccd,
            &hooks,
            &events,
        );
        Ok(())
    }

    /// Canonical hash over dynamic bodies in spawn order. Each float is
    /// scaled by [`HASH_SCALE`], rounded to `i64` and folded as little-endian
    /// bytes (FNV-1a). No float formatting, no transcendental calls.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv64::new();
        for handle in &self.order {
            let Some(body) = self.bodies.get(*handle) else {
                continue;
            };
            let t = body.translation();
            let r = body.rotation();
            let lv = body.linvel();
            let av = body.angvel();
            for v in [
                t.x, t.y, t.z, r.x, r.y, r.z, r.w, lv.x, lv.y, lv.z, av.x, av.y, av.z,
            ] {
                h.write_i64(quantize(v));
            }
        }
        h.finish()
    }

    /// Copy out every dynamic body's kinematic state in spawn order.
    pub fn snapshot(&self) -> Snapshot {
        let mut bodies = Vec::with_capacity(self.order.len());
        for handle in &self.order {
            if let Some(body) = self.bodies.get(*handle) {
                let t = body.translation();
                let r = body.rotation();
                let lv = body.linvel();
                let av = body.angvel();
                bodies.push(BodyState {
                    pos: [t.x, t.y, t.z],
                    rot: [r.x, r.y, r.z, r.w],
                    linvel: [lv.x, lv.y, lv.z],
                    angvel: [av.x, av.y, av.z],
                });
            }
        }
        Snapshot { bodies }
    }

    /// Restore a snapshot. Counts must match; resuming with the same fixed
    /// `dt` sequence is bit-identical to the recorded run.
    pub fn restore(&mut self, snapshot: Snapshot) -> Result<(), Error> {
        if snapshot.bodies.len() != self.order.len() {
            return Err(Error::SnapshotMismatch);
        }
        for (handle, s) in self.order.iter().zip(snapshot.bodies.iter()) {
            let Some(body) = self.bodies.get_mut(*handle) else {
                return Err(Error::SnapshotMismatch);
            };
            body.set_translation(Vector::new(s.pos[0], s.pos[1], s.pos[2]), false);
            // Rapier snapshots are unit quaternions up to float rounding.
            // Re-normalizing those would move the last bit and break the
            // byte-equal rollback, so only hand-built snapshots that are far
            // off unit length (one `sqrt`, allowed sim math) get normalized.
            // Non-finite components fall back to identity.
            let q = if s.rot.iter().all(|c| c.is_finite()) {
                let raw = Rotation::from_xyzw(s.rot[0], s.rot[1], s.rot[2], s.rot[3]);
                let len2 = s.rot.iter().map(|c| c * c).sum::<f32>();
                if (len2 - 1.0).abs() <= 1e-6 {
                    raw
                } else {
                    raw.normalize()
                }
            } else {
                Rotation::IDENTITY
            };
            body.set_rotation(q, false);
            let lv = sanitize3(s.linvel);
            let av = sanitize3(s.angvel);
            body.set_linvel(Vector::new(lv[0], lv[1], lv[2]), false);
            body.set_angvel(Vector::new(av[0], av[1], av[2]), false);
            body.reset_forces(false);
            body.wake_up(true);
        }
        Ok(())
    }

    /// Snapshot bytes in canonical island order (see
    /// [`islands::island_snapshot`]): `coupled` lists the body pairs in
    /// contact, any order. The length is [`islands::snapshot_budget`], at
    /// most [`islands::FIGHTER_SNAPSHOT_BYTES`] per fighter plus the header.
    ///
    /// A snapshot is a sync point: the world is rebuilt around the same body
    /// states (see [`Self::reset_to`]), so the live run and a restore of these
    /// bytes step identically. Take snapshots at the same ticks on every peer.
    pub fn island_snapshot(&mut self, coupled: &[(usize, usize)]) -> Vec<u8> {
        let states = self.snapshot();
        let words: Vec<[f32; STATE_FLOATS]> = states.bodies.iter().map(BodyState::words).collect();
        let bytes = islands::island_snapshot(&words, coupled);
        // The body count matches by construction, so the restore cannot fail.
        let _ = self.reset_to(states);
        bytes
    }

    /// Roll every dynamic body back to [`Self::island_snapshot`] bytes, then
    /// rebuild around them like the snapshot did. Atomic: the bytes are fully
    /// parsed and validated first, so a refused snapshot leaves the world as
    /// it was.
    pub fn restore_island_snapshot(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let words = islands::parse_island_snapshot(bytes, self.order.len())
            .map_err(Error::SnapshotBytes)?;
        let bodies = words.iter().map(BodyState::from_words).collect();
        self.reset_to(Snapshot { bodies })
    }

    /// Rebuild the Rapier world from its spawn list, then put the dynamic
    /// bodies at `states`. Rapier keeps state between steps that a snapshot
    /// cannot hold: contact warm-start impulses and frozen lever arms, the
    /// broad-phase pair cache, island ids and active-set membership, CCD
    /// bookkeeping. A rebuilt world holds none of it, so both sides of a
    /// rollback step from the same place.
    fn reset_to(&mut self, states: Snapshot) -> Result<(), Error> {
        let spawns = std::mem::take(&mut self.spawns);
        *self = World::new();
        for spawn in &spawns {
            match *spawn {
                Spawn::Capsule {
                    pos,
                    radius,
                    height,
                } => {
                    self.insert_capsule(pos, radius, height);
                }
                Spawn::Static { half, pos } => {
                    self.insert_static(half, pos);
                }
            }
        }
        self.spawns = spawns;
        self.restore(states)
    }

    /// Number of dynamic bodies.
    pub fn body_count(&self) -> usize {
        self.order.len()
    }

    /// Center position of a dynamic body by index.
    pub fn body_position(&self, id: usize) -> Option<[f32; 3]> {
        ordered_translation(&self.bodies, &self.order, id)
    }

    /// Linear velocity of a dynamic body by index.
    pub fn body_linvel(&self, id: usize) -> Option<[f32; 3]> {
        ordered_linvel(&self.bodies, &self.order, id)
    }

    /// Teleport a dynamic body's center. The character controller applies its
    /// once-per-tick resolved displacement through here so the Rapier mirror
    /// stays in lockstep with the kinematic state. Returns false for an
    /// unknown body or an invalid position; the world is unchanged then.
    pub fn set_body_translation(&mut self, id: usize, pos: [f32; 3]) -> bool {
        if check_pos(pos).is_err() {
            return false;
        }
        let Some(handle) = self.order.get(id).copied() else {
            return false;
        };
        let Some(body) = self.bodies.get_mut(handle) else {
            return false;
        };
        body.set_translation(Vector::new(pos[0], pos[1], pos[2]), true);
        body.reset_forces(true);
        body.wake_up(true);
        true
    }
}

impl Default for World {
    fn default() -> Self {
        Self::new()
    }
}

/// Scale-round a float to `i64`; non-finite maps to a fixed sentinel.
pub(crate) fn quantize(v: f32) -> i64 {
    if !v.is_finite() {
        return i64::MIN;
    }
    let s = f64::from(v) * HASH_SCALE;
    if s >= 9.0e18 {
        i64::MAX
    } else if s <= -9.0e18 {
        i64::MIN + 1
    } else {
        s.round() as i64
    }
}

/// Replace non-finite components with zero so restore never injects NaN.
pub(crate) fn sanitize3(v: [f32; 3]) -> [f32; 3] {
    [
        if v[0].is_finite() { v[0] } else { 0.0 },
        if v[1].is_finite() { v[1] } else { 0.0 },
        if v[2].is_finite() { v[2] } else { 0.0 },
    ]
}

/// True when every component is finite and within [`MAX_LEN`]; the one
/// position gate every adapter in this crate validates through.
pub(crate) fn position_in_bounds(pos: [f32; 3]) -> bool {
    pos.iter().all(|c| c.is_finite() && c.abs() <= MAX_LEN)
}

/// Center of the body at `index` in insertion `order`: the one reader behind
/// every adapter's per-body position query. `None` past the end of `order`
/// or for a handle the set no longer holds.
pub(crate) fn ordered_translation(
    bodies: &RigidBodySet,
    order: &[RigidBodyHandle],
    index: usize,
) -> Option<[f32; 3]> {
    let t = bodies.get(*order.get(index)?)?.translation();
    Some([t.x, t.y, t.z])
}

/// Linear velocity of the body at `index` in insertion `order`, with the same
/// `None` cases as [`ordered_translation`].
pub(crate) fn ordered_linvel(
    bodies: &RigidBodySet,
    order: &[RigidBodyHandle],
    index: usize,
) -> Option<[f32; 3]> {
    let v = bodies.get(*order.get(index)?)?.linvel();
    Some([v.x, v.y, v.z])
}

fn check_pos(pos: [f32; 3]) -> Result<(), Error> {
    if position_in_bounds(pos) {
        Ok(())
    } else {
        Err(Error::NonFinitePosition)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arena_with_capsule() -> World {
        let mut w = World::new();
        w.add_static_box([10.0, 0.5, 10.0], [0.0, -0.5, 0.0])
            .expect("floor");
        w.spawn_capsule([0.0, 4.0, 0.0], 0.4, 1.0).expect("capsule");
        w
    }

    fn step_n(w: &mut World, n: usize) {
        for _ in 0..n {
            w.step(FIXED_DT).expect("fixed step");
        }
    }

    #[test]
    fn hash_twice_identical() {
        let mut a = arena_with_capsule();
        let mut b = arena_with_capsule();
        step_n(&mut a, 120);
        step_n(&mut b, 120);
        assert_eq!(a.state_hash(), b.state_hash());
        // Same world hashes stably without stepping.
        assert_eq!(a.state_hash(), a.state_hash());
    }

    #[test]
    fn hash_matches_shared_fnv_bytes() {
        use engine_core::hash::{FNV64_OFFSET, FNV64_PRIME};
        fn legacy(w: &World) -> u64 {
            let mut h: u64 = FNV64_OFFSET;
            for handle in &w.order {
                let Some(body) = w.bodies.get(*handle) else {
                    continue;
                };
                let t = body.translation();
                let r = body.rotation();
                let lv = body.linvel();
                let av = body.angvel();
                for v in [
                    t.x, t.y, t.z, r.x, r.y, r.z, r.w, lv.x, lv.y, lv.z, av.x, av.y, av.z,
                ] {
                    for b in quantize(v).to_le_bytes() {
                        h ^= u64::from(b);
                        h = h.wrapping_mul(FNV64_PRIME);
                    }
                }
            }
            h
        }
        let mut w = arena_with_capsule();
        assert_eq!(w.state_hash(), legacy(&w));
        step_n(&mut w, 120);
        assert_eq!(w.state_hash(), legacy(&w));
    }

    #[test]
    fn snapshot_restore_resimulate_identical() {
        let mut w = arena_with_capsule();
        step_n(&mut w, 60);
        let snap = w.snapshot();
        step_n(&mut w, 90);
        let after = w.state_hash();
        w.restore(snap.clone()).expect("restore");
        // Restored state matches the snapshot exactly.
        assert_eq!(w.snapshot(), snap);
        step_n(&mut w, 90);
        assert_eq!(w.state_hash(), after);
    }

    #[test]
    fn capsule_falls_to_floor_and_rests() {
        let mut w = arena_with_capsule();
        step_n(&mut w, 600);
        let pos = w.body_position(0).expect("body 0");
        let vel = w.body_linvel(0).expect("vel 0");
        // Floor top is y=0; capsule half-height is 0.5 (cylinder) + 0.4 (radius).
        let rest_y = 0.5 + 0.4;
        assert!(
            (pos[1] - rest_y).abs() < 0.1,
            "capsule y={} want {rest_y}",
            pos[1]
        );
        assert!(pos[0].abs() < 0.1 && pos[2].abs() < 0.1, "{pos:?}");
        let speed = vel[0].abs() + vel[1].abs() + vel[2].abs();
        assert!(speed < 0.2, "{vel:?}");
        assert!(pos.iter().all(|c| c.is_finite()));
    }

    #[test]
    fn invalid_dt_rejected() {
        let mut w = arena_with_capsule();
        for bad in [0.0, -FIXED_DT, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert_eq!(w.step(bad), Err(Error::InvalidDt), "dt={bad}");
        }
        assert!(w.step(FIXED_DT).is_ok());
    }

    #[test]
    fn invalid_spawn_input_rejected_without_panic() {
        let mut w = World::new();
        assert_eq!(
            w.spawn_capsule([0.0, 1.0, 0.0], 0.0, 1.0),
            Err(Error::InvalidRadius)
        );
        assert_eq!(
            w.spawn_capsule([0.0, 1.0, 0.0], -0.5, 1.0),
            Err(Error::InvalidRadius)
        );
        assert_eq!(
            w.spawn_capsule([0.0, 1.0, 0.0], f32::NAN, 1.0),
            Err(Error::InvalidRadius)
        );
        assert_eq!(
            w.spawn_capsule([0.0, 1.0, 0.0], 0.4, -1.0),
            Err(Error::InvalidHeight)
        );
        assert_eq!(
            w.spawn_capsule([0.0, 1.0, 0.0], 0.4, f32::INFINITY),
            Err(Error::InvalidHeight)
        );
        assert_eq!(
            w.spawn_capsule([0.0, f32::NAN, 0.0], 0.4, 1.0),
            Err(Error::NonFinitePosition)
        );
        assert_eq!(
            w.add_static_box([0.0, 0.5, 0.5], [0.0, 0.0, 0.0]),
            Err(Error::InvalidHalfExtents)
        );
        assert_eq!(
            w.add_static_box([f32::NAN, 0.5, 0.5], [0.0, 0.0, 0.0]),
            Err(Error::InvalidHalfExtents)
        );
        assert_eq!(w.body_count(), 0);
        // Mismatched snapshot restores nothing and reports the mismatch.
        assert_eq!(w.restore(Snapshot::default()), Ok(()));
        w.spawn_capsule([0.0, 2.0, 0.0], 0.4, 1.0).expect("capsule");
        assert_eq!(w.restore(Snapshot::default()), Err(Error::SnapshotMismatch));
        assert_eq!(w.body_position(99), None);
    }

    /// 64 fighters in 32 overlapping pairs on a floor: each pair pushes
    /// itself apart and spins, so rotations and velocities are all live.
    fn sixty_four_fighters() -> World {
        let mut w = World::new();
        w.add_static_box([40.0, 0.5, 40.0], [0.0, -0.5, 0.0])
            .expect("floor");
        for k in 0..32u8 {
            let (col, row) = (f32::from(k % 8), f32::from(k / 8));
            let (x, z) = (4.0 * col - 14.0, 4.0 * row - 6.0);
            w.spawn_capsule([x, 1.2, z], 0.4, 1.0).expect("left");
            let lift = 0.1 * f32::from(k % 3);
            w.spawn_capsule([x + 0.6, 1.2 + lift, z], 0.4, 1.0)
                .expect("right");
        }
        w
    }

    /// Fighters whose centers lie within 1.1 m of each other, ascending.
    fn clinching(w: &World) -> Vec<(usize, usize)> {
        let at: Vec<[f32; 3]> = (0..w.body_count())
            .map(|i| w.body_position(i).expect("body"))
            .collect();
        let mut pairs = Vec::new();
        for (i, a) in at.iter().enumerate() {
            for (j, b) in at.iter().enumerate().skip(i + 1) {
                let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
                if d[0] * d[0] + d[1] * d[1] + d[2] * d[2] < 1.1 * 1.1 {
                    pairs.push((i, j));
                }
            }
        }
        pairs
    }

    #[test]
    fn snapshot_64_fighters_rollback_is_byte_equal_x2() {
        let mut w = sixty_four_fighters();
        step_n(&mut w, 40);
        let pairs = clinching(&w);
        assert!(pairs.len() >= 32, "pairs clinch: {}", pairs.len());
        let checkpoint = w.island_snapshot(&pairs);
        let islands = crate::islands::build_islands(64, &pairs);
        assert_eq!(
            checkpoint.len(),
            crate::islands::snapshot_budget(64, islands.len())
        );
        assert!(
            checkpoint.len()
                <= crate::islands::SNAPSHOT_HEADER_BYTES
                    + 64 * crate::islands::FIGHTER_SNAPSHOT_BYTES,
            "{} bytes over the budget",
            checkpoint.len()
        );
        step_n(&mut w, 90);
        let (end_hash, end_bytes) = (w.state_hash(), w.island_snapshot(&pairs));
        assert_ne!(end_bytes, checkpoint, "the fighters moved");
        for round in 0..2 {
            w.restore_island_snapshot(&checkpoint).expect("restores");
            assert_eq!(
                w.island_snapshot(&pairs),
                checkpoint,
                "restore then snapshot, round {round}"
            );
            step_n(&mut w, 90);
            assert_eq!(w.state_hash(), end_hash, "resim hash, round {round}");
            assert_eq!(
                w.island_snapshot(&pairs),
                end_bytes,
                "resim bytes, round {round}"
            );
        }
    }

    #[test]
    fn snapshot_island_bytes_refused_leave_the_world_unchanged() {
        let mut w = sixty_four_fighters();
        step_n(&mut w, 30);
        let pairs = clinching(&w);
        let good = w.island_snapshot(&pairs);
        step_n(&mut w, 10);
        let (hash, live) = (w.state_hash(), w.island_snapshot(&pairs));
        let mut nan = good.clone();
        let last = nan.len() - 4;
        nan[last..].copy_from_slice(&f32::NAN.to_bits().to_le_bytes());
        let mut trailing = good.clone();
        trailing.push(7);
        let mut wrong_world = good.clone();
        wrong_world[4] = 63;
        for (what, bytes) in [
            ("truncated", good[..good.len() - 1].to_vec()),
            ("empty", Vec::new()),
            ("trailing", trailing),
            ("nan", nan),
            ("body count", wrong_world),
        ] {
            let result = w.restore_island_snapshot(&bytes);
            assert!(
                matches!(result, Err(Error::SnapshotBytes(_))),
                "{what}: {result:?}"
            );
            assert_eq!(w.state_hash(), hash, "{what} changed the world");
            assert_eq!(w.island_snapshot(&pairs), live, "{what} changed the bytes");
        }
        assert!(w.restore_island_snapshot(&good).is_ok());
        assert!(Error::SnapshotBytes(SnapshotError::Order)
            .to_string()
            .contains("refused"));
    }
}
