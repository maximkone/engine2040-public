//! The look's side of the frame audit (`view_truth::audit`): fold each posed
//! part and the palm and socket joints into the body's truth while a capture
//! is armed. View only: nothing here reaches the simulation or its hash.
//!
//! Consumer: `FighterLook::pose_frame`. The switch (`view_truth::audit::set_audit`)
//! is process wide because the window arms it once at capture setup and the
//! look has no config struct to carry it; with it off every call returns at
//! once, so a plain run pays one relaxed load per part. Data owner: this
//! module owns only the joint lookup; the bodies own the numbers.

use engine_anim::contract::Skeleton;
use engine_render::v1_skin::SkinnedFrame;

use super::{Rig, RigPart};
use crate::stage_bodies::place_joint;
#[cfg(test)]
use crate::view_truth::audit::PART_SAMPLES;
use crate::view_truth::audit::{audit_on, PartMaterial, PartTruth, ANCHOR_NAMES};
use crate::view_truth::{BodyTruth, ANCHORS};

/// The joint index of each audit anchor in `skeleton` (`palm_r`, the weapon
/// sockets), `None` when the rig has no joint of that name.
pub(super) fn anchor_joints(skeleton: &Skeleton) -> [Option<usize>; ANCHORS] {
    ANCHOR_NAMES.map(|name| skeleton.joints.iter().position(|joint| joint.name == name))
}

impl RigPart {
    /// This part's audit truth over `positions` (posed world vertices, or the
    /// bind mesh for a content check). `None` when no vertex is finite.
    fn truth(&self, index: usize, positions: &[[f32; 3]]) -> Option<PartTruth> {
        let material = PartMaterial {
            textured: self.texture.is_some(),
            base: self.material.base_color,
            metallic: self.material.metallic,
            roughness: self.material.roughness,
        };
        let index = u16::try_from(index).unwrap_or(u16::MAX);
        PartTruth::fold(
            index,
            std::sync::Arc::clone(&self.name),
            positions,
            material,
        )
    }
}

impl Rig {
    /// Fold the posed parts and place the audit anchors through the root, both
    /// lifted by the sand rest, so parts and anchors share the world the body
    /// bounds use. Does nothing while the audit is off.
    pub(super) fn audit_fold(
        &self,
        body: &mut BodyTruth,
        frames: &[SkinnedFrame],
        lift: f32,
        world: &[[[f32; 4]; 4]],
        root: &[[f32; 4]; 4],
    ) {
        if !audit_on() {
            return;
        }
        for (index, (part, frame)) in self.parts.iter().zip(frames).enumerate() {
            if let Some(mut truth) = part.truth(index, &frame.positions) {
                truth.lift(lift);
                body.parts.push(truth);
            }
        }
        for (anchor, joint) in body.anchors.iter_mut().zip(self.anchors) {
            *anchor = joint.and_then(|j| world.get(j)).map(|m| {
                let mut at = place_joint([m[3][0], m[3][1], m[3][2]], root);
                at[1] += lift;
                at
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::FighterLook;
    use super::*;

    /// The shipped realistic body carries what the frame audit judges by:
    /// its weapon and shield parts by name, the hand and socket joints, and a
    /// blade and shield in the kit's real size beside the body.
    #[test]
    fn the_legionary_carries_named_kit_parts_and_hand_joints() {
        let dir =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/fighters/rocketbox");
        let look = FighterLook::load_rocketbox(&dir).expect("the Rocketbox family loads");
        let rig = look
            .rigs
            .iter()
            .find(|rig| rig.variant == "rb_legionary")
            .expect("the legionary is in the family");
        assert!(
            rig.anchors.iter().all(Option::is_some),
            "palm_r, wep_pos_r, palm_l, wep_pos_l are all found: {:?}",
            rig.anchors
        );
        let truths: Vec<PartTruth> = rig
            .parts
            .iter()
            .enumerate()
            .filter_map(|(i, part)| part.truth(i, &part.mesh.positions))
            .collect();
        let named = |name: &str| truths.iter().find(|p| &*p.name == name);
        let blade = named("gladius_blade").expect("a gladius_blade part");
        let shield = named("scutum_board").expect("a scutum_board part");
        let body = named("sm002_body").expect("the body part");
        assert!(
            (0.35..=0.85).contains(&blade.len_m),
            "blade {} m",
            blade.len_m
        );
        assert!(
            (0.7..=1.5).contains(&shield.len_m),
            "shield {} m",
            shield.len_m
        );
        assert!((1.4..=2.2).contains(&body.len_m), "body {} m", body.len_m);
        assert!(blade.samples.len() == PART_SAMPLES && blade.textured);
    }
}
