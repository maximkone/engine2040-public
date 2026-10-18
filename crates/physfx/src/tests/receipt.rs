//! receipt: what a ragdoll spawn measures (QR4 mass ratios, QR5 overlaps) and the grandparent
//! pairs it turns off (QR2).

use super::*;
use std::f32::consts::PI;

/// Mass of one test bone in kilograms, from the flesh density and the capsule volume.
fn capsule_kg(bone: usize) -> f32 {
    let (radius, half) = BONES[bone].3;
    let volume = PI * radius * radius * 2.0 * half + 4.0 / 3.0 * PI * radius * radius * radius;
    Material::Flesh.props().density() * volume
}

/// A ragdoll of the humanoid in `pose`, three meters up so only its own bones can touch.
fn spawned(pose: &LocalPose) -> PhysFx {
    let (rig, _) = humanoid();
    let mut fx = arena(Tier::Comfort);
    fx.spawn_ragdoll(0, &rig, &moved(pose, [0.0, 3.0, 0.0]), [0.0; 3], [0.0; 3])
        .expect("ragdoll");
    fx
}

/// The lower left arm folded into the chest: it overlaps the chest, its grandparent, and nothing else.
fn forearm_in_chest() -> LocalPose {
    let (_, mut pose) = humanoid();
    pose.positions[bone("lower_arm_l")] = [0.13, 0.15, 0.0];
    pose
}

/// The legs through each other: the two thighs and the limbs below them overlap.
fn legs_crossed() -> LocalPose {
    let (_, mut pose) = humanoid();
    pose.positions[bone("thigh_l")][0] = -0.02;
    pose.positions[bone("thigh_r")][0] = 0.02;
    pose
}

#[test]
fn receipt_has_one_mass_ratio_per_joint() {
    let (_, pose) = humanoid();
    let fx = spawned(&pose);
    let receipt = fx.ragdoll_receipt(0).expect("receipt");
    assert_eq!(receipt.mass_kg.len(), BONES.len());
    assert_eq!(receipt.mass_ratio.len(), BONES.len() - 1, "one per joint");
    for (index, ratio) in receipt.mass_ratio.iter().enumerate() {
        let child = index + 1;
        let parent = BONES[child].1.expect("a joint has a parent");
        let want = capsule_kg(child) / capsule_kg(parent);
        assert!(
            (ratio - want).abs() < 1.0e-3 * want,
            "{}: ratio {ratio} against {want}",
            BONES[child].0
        );
    }
    println!("{}", receipt.receipt_line());
}

#[test]
fn a_standing_pose_has_no_overlap_and_a_respawn_replaces_the_receipt() {
    let (rig, pose) = humanoid();
    let mut fx = spawned(&pose);
    assert_eq!(fx.ragdoll_receipt(0).map(|r| r.overlaps), Some(0));
    assert!(fx.ragdoll_receipt(1).is_none(), "no ragdoll in slot 1");
    fx.spawn_ragdoll(
        0,
        &rig,
        &moved(&legs_crossed(), [0.0, 3.0, 0.0]),
        [0.0; 3],
        [0.0; 3],
    )
    .expect("respawn");
    assert!(fx.ragdoll_receipt(0).is_some_and(|r| r.overlaps > 0));
}

#[test]
fn legs_through_each_other_log_overlaps_and_the_solver_still_pushes_them() {
    let mut fx = spawned(&legs_crossed());
    let receipt = fx.ragdoll_receipt(0).expect("receipt");
    println!("{}", receipt.receipt_line());
    assert!(receipt.overlaps > 0, "the crossed legs overlap at spawn");
    assert_eq!(receipt.filtered, 0, "siblings are not grandparent pairs");
    fx.step();
    assert!(
        fx.stats().contacts > 0,
        "an overlap that is not filtered is a live contact"
    );
}

#[test]
fn a_grandparent_overlap_is_counted_and_makes_no_contact() {
    let mut fx = spawned(&forearm_in_chest());
    let receipt = fx.ragdoll_receipt(0).expect("receipt");
    println!("{}", receipt.receipt_line());
    assert_eq!((receipt.overlaps, receipt.filtered), (1, 1));
    assert!(receipt.receipt_line().contains("overlaps=1 filtered=1"));
    fx.step();
    assert_eq!(
        fx.stats().contacts,
        0,
        "the forearm and the chest do not push each other"
    );
}
