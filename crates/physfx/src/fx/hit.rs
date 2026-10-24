//! hit: a blow on a ragdoll spreads over the bones near it, and the joints stiffen for a moment.
//!
//! A blow used to land whole on the root bone and travel up the joints, so a hit on the hand
//! moved the pelvis first. Now its total impulse is split over the bones inside the reach, the
//! nearer ones taking more, so the momentum added is exactly the blow. Every joint also holds the
//! angle it had at the spawn with a PD motor whose strength jumps to the body's muscle on a hit and
//! falls back in [`HIT_FADE_S`] to a resting tone, [`RAGDOLL_TONE`] of the muscle: the body
//! braces against the blow, then its joints keep only friction (no pose), so it never whips
//! like a sack yet still folds, settles and sleeps. The brace's damping is critical at every
//! strength, so a joint never rings as it fades.

use super::joints::quat;
use super::{GroupId, PhysFx};
use crate::check::{self, MAX_COORD, MAX_IMPULSE};
use crate::error::{Error, Field};
use crate::rig::Rig;
use crate::types::Transform;
use engine_physics::rapier::FIXED_DT;
use rapier3d::dynamics::{
    ImpulseJointHandle, ImpulseJointSet, JointAxesMask, JointAxis, RigidBodyHandle,
};
use rapier3d::math::Vector;

/// Seconds the stiffness a hit gives takes to fall from full strength to nothing.
pub const HIT_FADE_S: f32 = 0.3;
/// Motor stiffness at full strength, per second squared. Measured on a shove at the head with four
/// solver iterations: 400 held back a fifth of the swing in five ticks, 10 000 holds back more
/// than half of it, 50 000 gains nothing more per joint than the solver can deliver.
pub(super) const STIFFNESS: f32 = 10_000.0;
/// Motor damping at full strength, per second: critical for [`STIFFNESS`] (twice its root). A
/// motor at strength `s` gets `DAMPING * sqrt(s)`, critical for its `STIFFNESS * s`.
pub(super) const DAMPING: f32 = 200.0;
/// Resting muscle tone of a ragdoll as a share of its body's muscle: the strength the motors rest
/// at when nothing hit it, and what a brace fades back to. At rest the motors pull toward no
/// pose (a pose spring would fight the floor forever and never let the body sleep): they are
/// joint friction, a velocity motor aimed at zero that soaks up swing. So a fallen or towed body
/// stops whipping like a sack, folds slowly under its weight, settles and sleeps. Zero muscle
/// keeps zero tone, so a body file can still ask for a limp one.
pub const RAGDOLL_TONE: f32 = 0.2;
/// Joint friction at the resting tone of a full muscle, per second: the velocity-motor factor
/// each free turning axis gets at rest (scaled by the body's muscle).
const TONE_DAMPING: f32 = 40.0;
/// The turning axes of a joint, which a motor can drive when the joint leaves them free.
pub(super) const TURNS: [JointAxis; 3] = [JointAxis::AngX, JointAxis::AngY, JointAxis::AngZ];

/// The motors of one ragdoll.
pub(super) struct Drive {
    /// The joint of every bone that has a parent, in bone order, with the angle of each turning
    /// axis at the spawn: what the motor holds.
    pub joints: Vec<(ImpulseJointHandle, [f32; 3])>,
    /// The body's muscle right after a hit, falling to `tone`; the motors are off at 0.
    pub strength: f32,
    /// The resting strength: [`RAGDOLL_TONE`] of the body's muscle.
    pub tone: f32,
}

/// Drives every free turning axis of `joints`: above the resting `tone`, a brace that points the
/// axis at the angle it held at the spawn with `strength` of the full gain; at the tone, joint
/// friction only (no pose); at zero, the motors are off.
pub(super) fn drive(
    set: &mut ImpulseJointSet,
    joints: &[(ImpulseJointHandle, [f32; 3])],
    strength: f32,
    tone: f32,
) {
    for (handle, hold) in joints {
        let Some(joint) = set.get_mut(*handle, false) else {
            continue;
        };
        if strength <= 0.0 {
            joint.data.motor_axes = JointAxesMask::empty();
            continue;
        }
        let braced = strength > tone;
        let free = TURNS
            .iter()
            .filter(|axis| !joint.data.locked_axes.contains((**axis).into()))
            .count();
        if !braced && free < 2 {
            // A hinge (knee, elbow) has one free axis and its limit holds it. Friction on it
            // fought the limit in the solver and let an elbow fold past it, so a resting hinge
            // keeps no motor; the ball joints carry the tone.
            joint.data.motor_axes = JointAxesMask::empty();
            continue;
        }
        for (axis, angle) in TURNS.into_iter().zip(hold) {
            if joint.data.locked_axes.contains(axis.into()) {
                continue;
            }
            if braced {
                joint.data.set_motor_position(
                    axis,
                    *angle,
                    STIFFNESS * strength,
                    DAMPING * strength.sqrt(),
                );
            } else {
                joint
                    .data
                    .set_motor_velocity(axis, 0.0, TONE_DAMPING * strength / RAGDOLL_TONE);
            }
        }
    }
}

/// The angle on each turning axis of a ball joint between `parent` and `child` at the spawn, as
/// Rapier's motor reads it: twice the arcsine of the relative rotation's vector part, in the
/// parent's frame. The ball joints have no frame of their own, so this is not zero in general.
pub(super) fn spawn_angles(parent: &Transform, child: &Transform) -> [f32; 3] {
    let mut turn = quat(parent).inverse() * quat(child);
    if turn.w < 0.0 {
        turn = -turn;
    }
    [turn.x, turn.y, turn.z].map(|c| 2.0 * c.clamp(-1.0, 1.0).asin())
}

impl PhysFx {
    /// Lands a blow of `impulse` (newton-seconds) at `point` on the ragdoll in `slot`: the impulse
    /// is split over the bones whose centers lie within `radius` meters of the point, weighted
    /// `1 - distance / radius`, and the bones' impulses add up to `impulse`. A point outside
    /// every bone's reach gives the whole blow to the nearest bone. Every joint then braces:
    /// motor strength jumps to the body's muscle and falls to the resting tone over
    /// [`HIT_FADE_S`]. Returns how
    /// many bones took part; 0 when the slot has no ragdoll. A blow that would throw a bone past
    /// the speed cap, or a bad number, is refused and changes nothing.
    pub fn hit_ragdoll(
        &mut self,
        slot: u8,
        point: [f32; 3],
        radius: f32,
        impulse: [f32; 3],
    ) -> Result<usize, Error> {
        check::vec3(point, MAX_COORD, Field::Position)?;
        check::extent(radius)?;
        check::vec3(impulse, MAX_IMPULSE, Field::Impulse)?;
        let Some(at) = self
            .groups
            .iter()
            .position(|g| g.id == GroupId::Ragdoll(slot))
        else {
            return Ok(0);
        };
        // (body, distance from the point, mass) of every bone.
        let bones: Vec<(RigidBodyHandle, f32, f32)> = self.groups[at]
            .bodies
            .iter()
            .filter_map(|index| {
                let handle = self
                    .records
                    .get(usize::try_from(*index).ok()?)?
                    .as_ref()?
                    .handle;
                let body = self.world.bodies.get(handle)?;
                let c = body.translation();
                let d = [c.x - point[0], c.y - point[1], c.z - point[2]];
                let distance = d.iter().map(|v| v * v).sum::<f32>().sqrt();
                Some((handle, distance, body.mass()))
            })
            .collect();
        let mut weights: Vec<f32> = bones
            .iter()
            .map(|(_, distance, _)| (1.0 - distance / radius).max(0.0))
            .collect();
        if weights.iter().all(|w| *w == 0.0) {
            if let Some(nearest) = bones
                .iter()
                .enumerate()
                .min_by(|a, b| a.1 .1.total_cmp(&b.1 .1))
                .map(|(i, _)| i)
            {
                weights[nearest] = 1.0;
            }
        }
        let total: f32 = weights.iter().sum();
        let shares: Vec<[f32; 3]> = weights
            .iter()
            .map(|w| impulse.map(|c| c * w / total))
            .collect();
        for (share, (_, _, mass)) in shares.iter().zip(&bones) {
            check::velocity_gain(*share, *mass)?;
        }
        let mut pushed = 0;
        for ((handle, _, _), share) in bones.iter().zip(&shares) {
            if *share == [0.0; 3] {
                continue;
            }
            if let Some(body) = self.world.bodies.get_mut(*handle) {
                body.apply_impulse_at_point(
                    Vector::new(share[0], share[1], share[2]),
                    Vector::new(point[0], point[1], point[2]),
                    true,
                );
                pushed += 1;
            }
        }
        // The body's muscles decide how hard the joints brace: a body with no
        // muscle gives way at once, a body with full muscle braces as hard as
        // the motors can.
        let muscle = f32::from(self.groups[at].muscle_permille) / 1000.0;
        if let Some(drive_of) = self.groups[at].drive.as_mut() {
            drive_of.strength = muscle.max(drive_of.tone);
            drive(
                &mut self.world.impulse_joints,
                &drive_of.joints,
                drive_of.strength,
                drive_of.tone,
            );
        }
        Ok(pushed)
    }

    /// Holds the ragdoll in `slot` braced at least at `strength` of the full gain this tick (the
    /// PuppetMaster pin weight): a body that is still fighting, towed on a rope or thrown, keeps
    /// its shape while the caller asks, then fades back to its tone over [`HIT_FADE_S`] once the
    /// calls stop. Capped at the body's muscle, so a limp body file stays limp. Returns the
    /// strength now, `None` when the slot has no ragdoll.
    pub fn brace_ragdoll(&mut self, slot: u8, strength: f32) -> Option<f32> {
        let at = self
            .groups
            .iter()
            .position(|g| g.id == GroupId::Ragdoll(slot))?;
        let muscle = f32::from(self.groups[at].muscle_permille) / 1000.0;
        let motors = self.groups[at].drive.as_mut()?;
        let want = if strength.is_finite() {
            strength.clamp(0.0, 1.0).min(muscle)
        } else {
            0.0
        };
        if want > motors.strength {
            motors.strength = want;
            drive(
                &mut self.world.impulse_joints,
                &motors.joints,
                motors.strength,
                motors.tone,
            );
        }
        Some(motors.strength)
    }

    /// How hard the joints of the ragdoll in `slot` brace now: the body's muscle right after a
    /// hit, falling to the resting tone ([`RAGDOLL_TONE`] of the muscle), 0 when the motors are
    /// off. `None` when the slot has no ragdoll.
    pub fn ragdoll_strength(&self, slot: u8) -> Option<f32> {
        self.groups
            .iter()
            .find(|g| g.id == GroupId::Ragdoll(slot))?
            .drive
            .as_ref()
            .map(|d| d.strength)
    }

    /// The motors of a ragdoll just built, switched on at the resting tone: the joint that ties
    /// each bone with a parent to it, in bone order, with the angles to hold. A hinge was framed
    /// to read zero at the spawn.
    pub(super) fn spawn_drive(
        &mut self,
        rig: &Rig,
        placed: &[Transform],
        handles: &[RigidBodyHandle],
    ) -> Drive {
        let joints = (0..handles.len())
            .filter_map(|bone| {
                let parent = rig.parent(bone)?;
                let (handle, _) = self
                    .world
                    .impulse_joints
                    .joints_between(handles[parent], handles[bone])
                    .next()?;
                let hinge = rig.hinges().iter().any(|h| h.bone == bone);
                let hold = if hinge {
                    [0.0; 3]
                } else {
                    spawn_angles(&placed[parent], &placed[bone])
                };
                Some((handle, hold))
            })
            .collect();
        let tone = f32::from(rig.muscle_permille()) / 1000.0 * RAGDOLL_TONE;
        let motors = Drive {
            joints,
            strength: tone,
            tone,
        };
        drive(&mut self.world.impulse_joints, &motors.joints, tone, tone);
        motors
    }

    /// Takes one tick off every bracing ragdoll's strength; call it before the world steps.
    pub(super) fn fade_drives(&mut self) {
        let fall = FIXED_DT / HIT_FADE_S;
        for group in &mut self.groups {
            let Some(motors) = group.drive.as_mut().filter(|d| d.strength > d.tone) else {
                continue;
            };
            motors.strength = (motors.strength - fall).max(motors.tone);
            drive(
                &mut self.world.impulse_joints,
                &motors.joints,
                motors.strength,
                motors.tone,
            );
        }
    }

    /// The summed momentum (mass times velocity) of the ragdoll in `slot`, kg m/s.
    #[cfg(test)]
    pub(crate) fn ragdoll_momentum(&self, slot: u8) -> [f32; 3] {
        let mut sum = [0.0; 3];
        let group = self.groups.iter().find(|g| g.id == GroupId::Ragdoll(slot));
        for index in group.iter().flat_map(|g| &g.bodies) {
            let Some(Some(rec)) = usize::try_from(*index)
                .ok()
                .and_then(|at| self.records.get(at))
            else {
                continue;
            };
            if let Some(body) = self.world.bodies.get(rec.handle) {
                let (m, v) = (body.mass(), body.linvel());
                sum[0] += m * v.x;
                sum[1] += m * v.y;
                sum[2] += m * v.z;
            }
        }
        sum
    }
}
