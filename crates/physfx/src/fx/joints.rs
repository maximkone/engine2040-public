//! joints: the knees and elbows of a ragdoll are hinges.
//!
//! `insert_joints` joins every bone to its parent with a ball and a swing cone that gets
//! narrower with depth, so a knee only bends 34 degrees either way and a folded leg is thrown
//! straight. The bones a rig names as hinges get a hinge instead: one axis, the pelvis side
//! axis, and a flexion range counted from the straight limb.

use super::PhysFx;
use crate::rig::Rig;
use crate::types::Transform;
use rapier3d::dynamics::{
    GenericJoint, GenericJointBuilder, JointAxesMask, JointAxis, RigidBodyHandle,
};
use rapier3d::math::{Pose, Rotation, Vector};

pub(super) fn quat(at: &Transform) -> Rotation {
    Rotation::from_xyzw(at.rot[0], at.rot[1], at.rot[2], at.rot[3])
}

fn vector(at: &Transform) -> Vector {
    Vector::new(at.pos[0], at.pos[1], at.pos[2])
}

/// A hinge between `parent` and `child` about `axis` (world), anchored at the midpoint of the
/// two bodies like the ball joints are. `range` is the flexion from the straight limb. The joint
/// frames coincide at the spawn pose, so the joint angle starts at zero: the fold the pose
/// already has shifts the range, and the range always holds the spawn pose.
fn hinge_joint(
    parent: &Transform,
    child: &Transform,
    axis: Vector,
    range: [f32; 2],
) -> GenericJoint {
    let (qp, qc) = (quat(parent), quat(child));
    let (yp, yc) = (qp * Vector::Y, qc * Vector::Y);
    let flexion = yp.cross(yc).dot(axis).atan2(yp.dot(yc));
    let low = (range[0] - flexion).min(0.0);
    let high = (range[1] - flexion).max(0.0);
    let mid = (vector(parent) + vector(child)) * 0.5;
    let world_frame = Rotation::from_rotation_arc(Vector::X, axis);
    GenericJointBuilder::new(JointAxesMask::LOCKED_REVOLUTE_AXES)
        .local_frame1(Pose::from_parts(
            qp.inverse() * (mid - vector(parent)),
            qp.inverse() * world_frame,
        ))
        .local_frame2(Pose::from_parts(
            qc.inverse() * (mid - vector(child)),
            qc.inverse() * world_frame,
        ))
        .limits(JointAxis::AngX, [low, high])
        .contacts_enabled(false)
        .build()
}

impl PhysFx {
    /// Swaps the ball joint of every hinge of `rig` for a hinge. `placed` is the spawn pose of
    /// every bone and `handles` the bodies, both in bone order.
    pub(super) fn make_hinges(
        &mut self,
        rig: &Rig,
        placed: &[Transform],
        handles: &[RigidBodyHandle],
    ) {
        let axis = quat(&placed[0]) * Vector::X;
        for hinge in rig.hinges() {
            let Some(parent) = rig.parent(hinge.bone) else {
                continue;
            };
            let data = hinge_joint(
                &placed[parent],
                &placed[hinge.bone],
                axis,
                [hinge.min, hinge.max],
            );
            let balls: Vec<_> = self
                .world
                .impulse_joints
                .joints_between(handles[parent], handles[hinge.bone])
                .map(|(handle, _)| handle)
                .collect();
            for handle in balls {
                if let Some(joint) = self.world.impulse_joints.get_mut(handle, false) {
                    joint.data = data;
                }
            }
        }
    }
}
