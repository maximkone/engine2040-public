//! PX48: every ragdoll teleport clears the CCD solver's cached fixed-target list. A restore and
//! a parted bone are the two the arena hits, so both are covered here; the test lives beside
//! `ragdoll.rs` because that file is past the ratchet's 3000-line pin.

use super::*;
use engine_anim::contract::Joint;

/// Two-bone vertical chain above the floor: root at y = 3, child offset +0.5 in Y.
fn chain() -> (Skeleton, LocalPose, Vec<BoneShape>) {
    let skeleton = Skeleton {
        joints: vec![
            Joint {
                name: "root".into(),
                parent: None,
            },
            Joint {
                name: "tip".into(),
                parent: Some(0),
            },
        ],
    };
    let pose = LocalPose {
        positions: vec![[0.0, 3.0, 0.0], [0.0, 0.5, 0.0]],
        rotations: vec![[0.0, 0.0, 0.0, 1.0]; 2],
    };
    let shapes = vec![
        BoneShape {
            radius: 0.15,
            half_height: 0.2,
        };
        2
    ];
    (skeleton, pose, shapes)
}

fn ragdoll_on_floor() -> Ragdoll {
    let (skeleton, pose, shapes) = chain();
    let mut r = Ragdoll::spawn(&skeleton, &pose, &shapes).expect("spawn chain");
    r.add_static_box([10.0, 0.5, 10.0], [0.0, -0.5, 0.0])
        .expect("floor");
    r
}

/// A restore is a teleport to the solver: it clears the cache once and hashes back to the saved
/// pose; a parted bone clears it too.
#[test]
fn restore_and_part_bone_clear_ccd_targets() {
    let mut r = ragdoll_on_floor();
    for _ in 0..20 {
        r.step(FIXED_DT).expect("fixed step");
    }
    let saved = r.snapshot();
    let hash = r.state_hash();
    let clears = r.ccd_clears();
    r.restore(saved).expect("restore");
    assert_eq!(r.ccd_clears(), clears + 1, "restore kept the CCD cache");
    assert_eq!(r.state_hash(), hash, "restore moved the pose");

    for _ in 0..10 {
        r.step(FIXED_DT).expect("fixed step");
    }
    let before = r.ccd_clears();
    r.part_bone(0, [0.05, 0.0, 0.0], [0.0, 0.0, 0.0])
        .expect("part bone");
    assert_eq!(r.ccd_clears(), before + 1, "part_bone kept the CCD cache");
}
