//! follow: the joints drive a ragdoll toward the clip pose, carrying the clip's own speed.
//!
//! Consumer: the look side calls [`PhysFx::drive_pose`] once per tick with the pose the clip
//! showed last tick and the pose it shows now. Each turning axis of a ball joint then gets the
//! new pose's angle as its target **and** the angular velocity that gets from the old pose to the
//! new one in `dt`, so a shoved arm catches up with the animation instead of lagging a tick behind
//! it and snapping back. Ported from Jolt's `Ragdoll::DriveToPoseUsingMotors(prev_pose, pose, dt)`
//! (JoltPhysics, `Jolt/Physics/Ragdoll/Ragdoll.cpp`): Jolt drives the joint in
//! position-and-velocity mode with `(rot * prev_rot.conjunct()).GetAngularVelocity(dt)` as the
//! target angular velocity. Our motor is Rapier's spring-damper, so the same pair is
//! `set_motor(axis, target_angle, target_speed, stiffness, damping)`.
//!
//! This is the picture layer: floats only, never hashed, never read back by the sim. Weight 0
//! writes nothing (it hands the joints back to their resting friction only if a drive of ours was
//! left on them, and never touches a live hit brace), so a caller that never drives a body gets
//! exactly the bytes it got before this module existed.
//! Limits: a hinge (knee, elbow) keeps the brace/tone path — its motor reads the angle in its own
//! joint frame, which the pose delta alone does not give, so [`PhysFx::drive_pose`] drives ball
//! joints only.
#![forbid(unsafe_code)]

use super::hit::{drive, spawn_angles, DAMPING, STIFFNESS, TURNS};
use super::{GroupId, PhysFx};
use crate::check;
use crate::error::{Error, Field};
use crate::types::Transform;
use rapier3d::math::Vector;

/// Target angular velocity ceiling, rad/s: the drive reads the pose delta over one tick, so a clip
/// that teleports between two ticks would ask for an unbounded speed and whip the body. The target
/// velocity saturates here and the position spring closes the rest over the next ticks, which is
/// the same role Jolt's `DriveToPoseUsingMotors(prev, pose, dt)` gives a bounded `dt`: a jump in
/// the clip is not a pose the bones can reach in one step.
pub const MAX_DRIVE_OMEGA: f32 = 15.0;

/// Pose target step ceiling, radians of joint angle per call: the position target moves at most
/// this far from the angle the previous pose asks for, so a clip that teleports a limb (a respawn,
/// a cut take) is followed over several ticks at a bounded rate instead of snapping the motor to an
/// unreachable angle in one. Any real 60 Hz clip turns far less than this per tick.
pub const MAX_DRIVE_ANGLE: f32 = 0.5;

impl PhysFx {
    /// Drives the ragdoll in `slot` toward the clip pose `now`, given `prev` (the clip pose one
    /// tick ago), `dt` seconds and a `weight` in `0..=1`: every ball joint's motor gets the new
    /// relative angle as its target and the pose delta's angular velocity as its target speed,
    /// with the gain scaled by the body's muscle, so a limp body gets no drive at all. Weight 0
    /// leaves the world exactly as it was. Returns how many joints were driven; 0 when the slot
    /// has no ragdoll or the weight or muscle is zero. A bad slice, a bad transform or a bad `dt`
    /// is refused with nothing changed.
    ///
    /// Order matters for one tick only: call it after the step whose pose you measured (or any
    /// time before the next [`PhysFx::step`]), because the step starts by fading hit braces and a
    /// live pose drive supersedes that fade for the coming step.
    pub fn drive_pose(
        &mut self,
        slot: u8,
        prev: &[Transform],
        now: &[Transform],
        dt: f32,
        weight: f32,
    ) -> Result<usize, Error> {
        self.follow_pose(slot, prev, now, dt, weight, true)
    }

    /// The position-only twin of [`PhysFx::drive_pose`]: the same targets and gain with no
    /// velocity term. Test-only, so the lag proof can weigh the two against each other.
    #[cfg(test)]
    pub(crate) fn drive_pose_position_only(
        &mut self,
        slot: u8,
        prev: &[Transform],
        now: &[Transform],
        dt: f32,
        weight: f32,
    ) -> Result<usize, Error> {
        self.follow_pose(slot, prev, now, dt, weight, false)
    }

    /// The one drive path: validate, then either write the pose motors or hand the joints back.
    fn follow_pose(
        &mut self,
        slot: u8,
        prev: &[Transform],
        now: &[Transform],
        dt: f32,
        weight: f32,
        speed: bool,
    ) -> Result<usize, Error> {
        if !dt.is_finite() {
            return Err(Error::NonFinite(Field::Ticks));
        }
        if dt <= 0.0 {
            return Err(Error::OutOfRange(Field::Ticks));
        }
        if !weight.is_finite() {
            return Err(Error::NonFinite(Field::Muscle));
        }
        let Some(at) = self
            .groups
            .iter()
            .position(|g| g.id == GroupId::Ragdoll(slot))
        else {
            return Ok(0);
        };
        let bones = self.groups[at].parents.len();
        if prev.len() != bones || now.len() != bones {
            return Err(Error::PoseLength {
                expected: bones,
                got: prev.len().min(now.len()),
            });
        }
        let (from, to) = Self::checked_poses(prev, now)?;
        let muscle = f32::from(self.groups[at].muscle_permille) / 1000.0;
        let gain = if weight > 0.0 {
            weight.min(1.0) * muscle
        } else {
            0.0
        };
        if gain <= 0.0 {
            self.rest_pose_drive(at);
            return Ok(0);
        }
        let driven = self.write_pose_motors(at, &from, &to, dt, gain, speed);
        // A pose drive supersedes this frame's hit brace: leave the strength at the tone so the
        // next step's fade keeps these motors instead of writing the spawn angles back.
        if let Some(motors) = self.groups[at].drive.as_mut() {
            motors.strength = motors.tone;
        }
        Ok(driven)
    }

    /// Writes the pose motors of group `at`: for every ball joint, the rate-limited target angle
    /// from `to` and the pose delta's target speed, at `gain` of the full stiffness. Returns how
    /// many joints were driven.
    fn write_pose_motors(
        &mut self,
        at: usize,
        from: &[Transform],
        to: &[Transform],
        dt: f32,
        gain: f32,
        speed: bool,
    ) -> usize {
        let stiffness = STIFFNESS * gain;
        let damping = DAMPING * gain.sqrt();
        let (parents, joints) = {
            let group = &self.groups[at];
            (&group.parents, &group.drive)
        };
        let Some(motors) = joints else {
            return 0;
        };
        let set = &mut self.world.impulse_joints;
        let mut driven = 0usize;
        for ((handle, _hold), (bone, parent)) in motors.joints.iter().zip(
            parents
                .iter()
                .enumerate()
                .filter_map(|(b, p)| p.map(|p| (b, p))),
        ) {
            let target = spawn_angles(&to[parent], &to[bone]);
            let was = spawn_angles(&from[parent], &from[bone]);
            let turn = delta_angular_velocity(from, to, parent, bone, dt);
            let Some(joint) = set.get_mut(*handle, false) else {
                continue;
            };
            // A hinge keeps the brace/tone path: its motor reads one angle in its own frame.
            let free = TURNS
                .iter()
                .filter(|axis| !joint.data.locked_axes.contains((**axis).into()))
                .count();
            if free < 2 {
                continue;
            }
            for (lane, (axis, (angle, omega))) in
                TURNS.into_iter().zip(target.iter().zip(turn)).enumerate()
            {
                if joint.data.locked_axes.contains(axis.into()) {
                    continue;
                }
                // The target angle walks at most [`MAX_DRIVE_ANGLE`] from the previous pose's
                // angle: a teleport in the clip is followed over several ticks, never in one.
                let hold = was[lane] + (angle - was[lane]).clamp(-MAX_DRIVE_ANGLE, MAX_DRIVE_ANGLE);
                let rate = if speed {
                    omega.clamp(-MAX_DRIVE_OMEGA, MAX_DRIVE_OMEGA)
                } else {
                    0.0
                };
                joint.data.set_motor(axis, hold, rate, stiffness, damping);
            }
            driven += 1;
        }
        driven
    }

    /// The two pose slices as checked, normalized transforms, in skeleton order: every position and
    /// rotation is validated before the world is touched.
    fn checked_poses(
        prev: &[Transform],
        now: &[Transform],
    ) -> Result<(Vec<Transform>, Vec<Transform>), Error> {
        let mut from = Vec::with_capacity(prev.len());
        let mut to = Vec::with_capacity(now.len());
        for (was, is) in prev.iter().zip(now) {
            from.push(check::transform(*was)?);
            to.push(check::transform(*is)?);
        }
        Ok((from, to))
    }

    /// Hands the joints of group `at` back to the resting friction, which is the state a body that
    /// was never driven has. A live hit brace (strength above the tone) is left alone: its own
    /// fade owns those motors, so a weight-0 call writes no byte.
    fn rest_pose_drive(&mut self, at: usize) {
        let Some(motors) = self.groups[at].drive.as_mut() else {
            return;
        };
        if motors.strength > motors.tone {
            return;
        }
        let (joints, tone) = (&motors.joints, motors.tone);
        drive(&mut self.world.impulse_joints, joints, tone, tone);
    }
}

/// The angular velocity, rad/s, that turns `prev[parent] -> prev[bone]` into
/// `now[parent] -> now[bone]` in one `dt`: Jolt's
/// `(rot * prev_rot.conjunct()).GetAngularVelocity(dt)` on the relative rotation, as the per-axis
/// speeds Rapier's motor reads. A rotation too small to name an axis reads zero speed.
fn delta_angular_velocity(
    prev: &[Transform],
    now: &[Transform],
    parent: usize,
    bone: usize,
    dt: f32,
) -> [f32; 3] {
    use super::joints::quat;
    let was = quat(&prev[parent]).inverse() * quat(&prev[bone]);
    let is = quat(&now[parent]).inverse() * quat(&now[bone]);
    let (axis, angle) = (is * was.inverse()).to_axis_angle();
    if !angle.is_finite() || angle <= 1.0e-6 {
        return [0.0; 3];
    }
    let speed = angle / dt;
    let v = Vector::new(axis.x, axis.y, axis.z) * speed;
    if v.x.is_finite() && v.y.is_finite() && v.z.is_finite() {
        [v.x, v.y, v.z]
    } else {
        [0.0; 3]
    }
}

#[cfg(test)]
impl PhysFx {
    /// The fastest target angular velocity and the widest target angle any motor of the world
    /// asks for, as `(rad/s, rad)`: the proof that the two caps hold reads the motors, which live
    /// in a field no test module can see.
    pub(crate) fn follow_drive_state(&self) -> (f32, f32) {
        let (mut fast, mut wide) = (0.0f32, 0.0f32);
        for (_, joint) in self.world.impulse_joints.iter() {
            for motor in joint.data.motors.iter() {
                fast = fast.max(motor.target_vel.abs());
                wide = wide.max(motor.target_pos.abs());
            }
        }
        (fast, wide)
    }
}
