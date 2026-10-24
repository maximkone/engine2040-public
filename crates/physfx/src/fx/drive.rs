//! drive: how a posed body meets geometry - keyframed, not free.
//!
//! Consumer: the look/tether side poses a hand or a spear grip that must follow
//! the clip exactly (SW161). A free rigid body there fights the solver against
//! gravity and pays jitter where the pose meets a wall, so a limb that is
//! animated, not simulated, is driven as a kinematic body for the tick and its
//! gravity is cancelled after the step. The mode is opt-in: no shipped caller
//! drives a body this way yet, so every existing body keeps the dynamic path
//! and the golden holds.
//!
//! PORT (ideas reimplemented, zero bytes vendored): JoltPhysics
//! `Samples/Tests/Rig/SoftKeyframedRigTest.cpp` (MIT): `DriveToPoseUsingKinematics(pose,
//! dt)` moves the body to the pose, then `AddLinearVelocity(GetGravity() * dt)`
//! "cancels the gravity that will be applied in the next step", and
//! `bcs.mMaxLinearVelocity = 10.0f` caps the drive speed.
//!
//! Units: metres, seconds, quaternions. Determinism: this is the picture layer
//! (floats, never hashed, never read back into the sim), so the same drive call
//! on the same body gives the same pose, and the mode changes nothing unless a
//! caller asks for it. Errors: total functions; a non-positive `dt` writes
//! nothing and reports a zero drive. Budget: O(1), no allocation.

use engine_physics::rapier::{FIXED_DT, GRAVITY_Y};
use rapier3d::dynamics::RigidBody;
use rapier3d::math::{Rotation, Vector};

/// Fastest a keyframed body may be driven, m/s (Jolt's `mMaxLinearVelocity`).
pub const MAX_KEYFRAMED_SPEED_M_S: f32 = 10.0;

/// How a body follows its pose for one tick.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drive {
    /// The solver owns the body: gravity, contacts and impulses all apply.
    Dynamic,
    /// The animation owns the pose: the body is kinematic for the tick, moving
    /// at most `speed_m_s` toward the pose and never faster than
    /// [`MAX_KEYFRAMED_SPEED_M_S`]. `key` is the caller's own handle on the
    /// clip or bone that poses it.
    Keyframed { key: u32, speed_m_s: f32 },
}

impl Drive {
    /// True when the pose, not the solver, owns the body.
    pub fn is_keyframed(self) -> bool {
        matches!(self, Self::Keyframed { .. })
    }

    /// The caller's key for a keyframed body, `None` for a dynamic one.
    pub fn key(self) -> Option<u32> {
        match self {
            Self::Dynamic => None,
            Self::Keyframed { key, .. } => Some(key),
        }
    }

    /// Drive speed after the cap, m/s; 0 for a dynamic body.
    pub fn speed_cap_m_s(self) -> f32 {
        match self {
            Self::Dynamic => 0.0,
            Self::Keyframed { speed_m_s, .. } => speed_m_s.clamp(0.0, MAX_KEYFRAMED_SPEED_M_S),
        }
    }
}

/// The longest distance one tick may drive a body toward its pose, metres.
fn step_budget(speed_m_s: f32, dt: f32) -> f32 {
    let dt = if dt.is_finite() && dt > 0.0 { dt } else { 0.0 };
    speed_m_s.clamp(0.0, MAX_KEYFRAMED_SPEED_M_S) * dt
}

/// Where the body goes this tick: the pose when it is within reach, otherwise
/// one capped step toward it.
fn step_toward(now: Vector, target: Vector, budget: f32) -> Vector {
    let delta = target - now;
    let distance = delta.length();
    if !distance.is_finite() || distance <= budget || budget <= 0.0 {
        return if distance.is_finite() && budget > 0.0 {
            target
        } else {
            now
        };
    }
    now + delta * (budget / distance)
}

/// Drive one body toward `target` for this tick and return the distance driven,
/// metres. Call it before the step: the body is kinematic, so the step moves it
/// to the driven pose instead of the solver pulling it there.
pub fn drive_keyframed(body: &mut RigidBody, target: Vector, dt: f32, speed_m_s: f32) -> f32 {
    let now = body.translation();
    let next = step_toward(now, target, step_budget(speed_m_s, dt));
    body.set_next_kinematic_translation(next);
    (next - now).length()
}

/// Drive translation and rotation in one tick: the same capped step for the
/// position, and the target orientation held for the tick. Returns the driven
/// distance, metres.
pub fn drive_keyframed_pose(
    body: &mut RigidBody,
    target: Vector,
    rotation: Rotation,
    dt: f32,
    speed_m_s: f32,
) -> f32 {
    if rotation.is_finite() {
        body.set_next_kinematic_rotation(rotation);
    }
    drive_keyframed(body, target, dt, speed_m_s)
}

/// Cancel the gravity the solver will apply on the next step (Jolt's
/// `AddLinearVelocity(gravity * dt)`): call it after the step, on the same
/// keyframed body. A non-positive `dt` writes nothing.
pub fn cancel_gravity(body: &mut RigidBody, dt: f32) {
    if !dt.is_finite() || dt <= 0.0 {
        return;
    }
    let before = body.linvel();
    let lifted = before + Vector::new(0.0, -GRAVITY_Y, 0.0) * dt;
    body.set_linvel(lifted, true);
}

/// The fixed step, re-exported so a caller drives on the same tick the solver
/// integrates.
pub const DRIVE_DT: f32 = FIXED_DT;

#[cfg(test)]
mod tests {
    use super::*;
    use rapier3d::dynamics::{RigidBodyBuilder, RigidBodyHandle};
    use rapier3d::geometry::ColliderBuilder;
    use rapier3d::pipeline::PhysicsWorld;

    /// Distance one tick drives at the cap: 2 mm.
    const TWO_MM: f32 = 0.002;

    /// A world on the fixed step: a floor, a 10 cm wall standing on it, a
    /// dynamic neighbour ball resting beside the hand's path, and the
    /// keyframed hand itself.
    fn world_with_wall() -> (PhysicsWorld, RigidBodyHandle, RigidBodyHandle) {
        let mut world = PhysicsWorld::new();
        world.gravity = Vector::new(0.0, GRAVITY_Y, 0.0);
        world.integration_parameters.dt = DRIVE_DT;
        let floor = world.bodies.insert(
            RigidBodyBuilder::fixed()
                .translation(Vector::new(0.0, -0.05, 0.0))
                .build(),
        );
        world.colliders.insert_with_parent(
            ColliderBuilder::cuboid(5.0, 0.05, 5.0).build(),
            floor,
            &mut world.bodies,
        );
        let wall = world.bodies.insert(
            RigidBodyBuilder::fixed()
                .translation(Vector::new(0.5, 0.5, 0.0))
                .build(),
        );
        world.colliders.insert_with_parent(
            ColliderBuilder::cuboid(0.05, 0.5, 0.5).build(),
            wall,
            &mut world.bodies,
        );
        let neighbour = world.bodies.insert(
            RigidBodyBuilder::dynamic()
                .translation(Vector::new(0.36, 0.05, 0.06))
                .build(),
        );
        world.colliders.insert_with_parent(
            ColliderBuilder::ball(0.05).density(900.0).build(),
            neighbour,
            &mut world.bodies,
        );
        let hand = world.bodies.insert(
            RigidBodyBuilder::kinematic_position_based()
                .translation(Vector::new(0.3, 0.05, 0.0))
                .build(),
        );
        world.colliders.insert_with_parent(
            ColliderBuilder::ball(0.03).build(),
            hand,
            &mut world.bodies,
        );
        (world, hand, neighbour)
    }

    /// SW161 F2P: a keyframed hand holds 2 mm a tick straight through a wall
    /// pass, and the dynamic neighbour it shoves is never launched.
    #[test]
    fn a_keyframed_hand_holds_its_pose_and_meets_a_wall_without_a_spike() {
        let (mut world, hand, neighbour) = world_with_wall();
        let start = world.bodies[hand].translation();
        let mut driven_total = 0.0f32;
        for tick in 1..=100u32 {
            let target = Vector::new(
                start.x + TWO_MM * f32::from(u16::try_from(tick).expect("100 ticks fit")),
                start.y,
                start.z,
            );
            let body = world.bodies.get_mut(hand).expect("the hand lives");
            let driven = drive_keyframed(body, target, DRIVE_DT, MAX_KEYFRAMED_SPEED_M_S);
            assert!(
                (driven - TWO_MM).abs() < 1e-5,
                "tick {tick} drove {driven} m, not 2 mm"
            );
            driven_total += driven;
            world.step();
            let body = world.bodies.get_mut(hand).expect("the hand lives");
            cancel_gravity(body, DRIVE_DT);
            let speed = world.bodies[neighbour].linvel().length();
            assert!(
                speed < 3.0,
                "tick {tick} launched the neighbour at {speed} m/s"
            );
        }
        let end = world.bodies[hand].translation();
        assert!(
            (end.x - (start.x + driven_total)).abs() < 1e-4,
            "the hand holds its pose tick for tick: {} vs {}",
            end.x,
            start.x + driven_total
        );
        assert!(
            end.x > 0.45,
            "the hand passed the wall's near face instead of being stopped at it"
        );
    }

    /// The mode caps the drive at 10 m/s, reports its key, and cancels exactly
    /// the gravity one step would add.
    #[test]
    fn keyframed_drive_clamps_its_speed_and_cancels_gravity() {
        let fast = Drive::Keyframed {
            key: 7,
            speed_m_s: 40.0,
        };
        assert!(fast.is_keyframed());
        assert_eq!(fast.key(), Some(7));
        assert_eq!(fast.speed_cap_m_s(), MAX_KEYFRAMED_SPEED_M_S);
        assert_eq!(Drive::Dynamic.key(), None);
        assert_eq!(Drive::Dynamic.speed_cap_m_s(), 0.0);

        let mut world = PhysicsWorld::new();
        world.integration_parameters.dt = DRIVE_DT;
        let handle = world.bodies.insert(
            RigidBodyBuilder::kinematic_position_based()
                .translation(Vector::new(0.0, 0.0, 0.0))
                .build(),
        );
        world.colliders.insert_with_parent(
            ColliderBuilder::ball(0.03).build(),
            handle,
            &mut world.bodies,
        );
        // A target far beyond one tick's reach is cut to the cap, not jumped.
        let body = world.bodies.get_mut(handle).expect("body lives");
        let driven = drive_keyframed_pose(
            body,
            Vector::new(5.0, 0.0, 0.0),
            Rotation::IDENTITY,
            DRIVE_DT,
            fast.speed_cap_m_s(),
        );
        let budget = MAX_KEYFRAMED_SPEED_M_S * DRIVE_DT;
        assert!(
            (driven - budget).abs() < 1e-5,
            "a far pose drives one capped step: {driven} vs {budget}"
        );
        // A non-positive step drives nothing at all.
        let body = world.bodies.get_mut(handle).expect("body lives");
        assert_eq!(
            drive_keyframed(body, Vector::new(5.0, 0.0, 0.0), 0.0, 10.0),
            0.0
        );

        // Cancelling gravity leaves the velocity the next step's gravity adds
        // exactly cancelled, and is idempotent in effect for a zero step.
        let dynamic = world.bodies.insert(
            RigidBodyBuilder::dynamic()
                .translation(Vector::new(0.0, 1.0, 0.0))
                .build(),
        );
        world.colliders.insert_with_parent(
            ColliderBuilder::ball(0.05).build(),
            dynamic,
            &mut world.bodies,
        );
        world.bodies[dynamic].set_linvel(Vector::new(0.0, 0.0, 0.0), true);
        let body = world.bodies.get_mut(dynamic).expect("body lives");
        cancel_gravity(body, DRIVE_DT);
        let lifted = world.bodies[dynamic].linvel().y;
        assert!(
            (lifted - (-GRAVITY_Y * DRIVE_DT)).abs() < 1e-6,
            "the gravity of one step is cancelled: {lifted}"
        );
        let body = world.bodies.get_mut(dynamic).expect("body lives");
        cancel_gravity(body, 0.0);
        assert_eq!(
            world.bodies[dynamic].linvel().y,
            lifted,
            "a zero step writes nothing"
        );
    }
}
