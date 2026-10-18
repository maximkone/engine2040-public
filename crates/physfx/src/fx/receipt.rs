//! receipt: what one ragdoll spawn measured, and the contacts it turns off.
//!
//! The solver blasts two capsules apart when they start inside each other. The spawn therefore
//! counts the bone pairs that overlap in the standing pose and have no joint between them.
//! A grandparent pair (a forearm inside the chest, a hand beside the head) is switched off for
//! this ragdoll alone: a joint that locks nothing and turns contacts off. Collision groups would
//! do it too, but 32 bits cannot name 15 bones per ragdoll, and a bone's bit would let it pass
//! through the same bone of every other ragdoll. Any other overlapping pair is only counted, so
//! the receipt shows what the solver will push.

use super::PhysFx;
use crate::rig::Rig;
use crate::types::Transform;
use rapier3d::dynamics::{GenericJointBuilder, JointAxesMask, RigidBodyHandle};
use rapier3d::math::{Pose, Rotation, Vector};
use rapier3d::parry::query::contact;
use rapier3d::parry::shape::Capsule;

/// What a ragdoll spawn measured, for the page and the window to print.
#[derive(Clone, Debug, PartialEq)]
pub struct RagdollReceipt {
    /// Mass of every bone, kilograms, skeleton order.
    pub mass_kg: Vec<f32>,
    /// `child mass / parent mass` of every joint, in bone order; the root has no joint.
    pub mass_ratio: Vec<f32>,
    /// Bone pairs with no joint between them whose capsules overlapped at spawn.
    pub overlaps: usize,
    /// Of those, the grandparent pairs whose contacts were switched off.
    pub filtered: usize,
}

impl RagdollReceipt {
    /// The one line the page and the window print for a ragdoll spawn:
    /// `ragdoll: bones=.. overlaps=.. filtered=.. mass_ratio=min..max`.
    pub fn receipt_line(&self) -> String {
        let (lo, hi) = self
            .mass_ratio
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), r| (lo.min(*r), hi.max(*r)));
        format!(
            "ragdoll: bones={} overlaps={} filtered={} mass_ratio={lo:.2}..{hi:.2}",
            self.mass_kg.len(),
            self.overlaps,
            self.filtered
        )
    }
}

/// The bone pairs, lower index first, that have no joint between them and whose capsules
/// overlap in `placed`.
pub(super) fn unjoined_overlaps(rig: &Rig, placed: &[Transform]) -> Vec<(usize, usize)> {
    let capsules: Vec<(Capsule, Pose)> = rig
        .shapes()
        .iter()
        .zip(placed)
        .map(|(shape, at)| {
            let pos = Vector::new(at.pos[0], at.pos[1], at.pos[2]);
            let rot = Rotation::from_xyzw(at.rot[0], at.rot[1], at.rot[2], at.rot[3]);
            (
                Capsule::new_y(shape.half_height, shape.radius),
                Pose::from_parts(pos, rot),
            )
        })
        .collect();
    let mut pairs = Vec::new();
    for b in 1..capsules.len() {
        for a in 0..b {
            if rig.parent(a) == Some(b) || rig.parent(b) == Some(a) {
                continue;
            }
            let (ca, pa) = &capsules[a];
            let (cb, pb) = &capsules[b];
            if matches!(contact(pa, ca, pb, cb, 0.0), Ok(Some(c)) if c.dist < 0.0) {
                pairs.push((a, b));
            }
        }
    }
    pairs
}

/// True when one bone is the other's grandparent.
fn grandparents(rig: &Rig, a: usize, b: usize) -> bool {
    let up2 = |bone: usize| rig.parent(bone).and_then(|p| rig.parent(p));
    up2(a) == Some(b) || up2(b) == Some(a)
}

impl PhysFx {
    /// Switches off the contacts of the grandparent pairs among `overlaps` and returns how many.
    pub(super) fn switch_off_overlaps(
        &mut self,
        rig: &Rig,
        overlaps: &[(usize, usize)],
        handles: &[RigidBodyHandle],
    ) -> usize {
        let mut off = 0;
        for (a, b) in overlaps {
            if !grandparents(rig, *a, *b) {
                continue;
            }
            let joint = GenericJointBuilder::new(JointAxesMask::empty())
                .contacts_enabled(false)
                .build();
            self.world
                .impulse_joints
                .insert(handles[*a], handles[*b], joint, true);
            off += 1;
        }
        off
    }

    /// The receipt of a ragdoll just built from `rig`: masses as Rapier computed them.
    pub(super) fn measure_ragdoll(
        &self,
        rig: &Rig,
        handles: &[RigidBodyHandle],
        overlaps: usize,
        filtered: usize,
    ) -> RagdollReceipt {
        let mass_kg: Vec<f32> = handles
            .iter()
            .map(|h| self.world.bodies.get(*h).map_or(0.0, |body| body.mass()))
            .collect();
        let mass_ratio = (1..mass_kg.len())
            .filter_map(|bone| Some(mass_kg[bone] / mass_kg[rig.parent(bone)?]))
            .collect();
        RagdollReceipt {
            mass_kg,
            mass_ratio,
            overlaps,
            filtered,
        }
    }

    /// What the ragdoll in `slot` measured at its spawn, or `None` when the slot has none.
    pub fn ragdoll_receipt(&self, slot: u8) -> Option<&RagdollReceipt> {
        self.groups
            .iter()
            .find(|g| g.id == super::GroupId::Ragdoll(slot))
            .and_then(|g| g.receipt.as_ref())
    }
}
