//! sever: one bone of a ragdoll leaves the body as a limb group of its own (QR12).
//!
//! The rig here is the fifteen-bone CT7 shape the look draws, one bone more than `humanoid`
//! (the left palm), so the accepted count is exact: a sever leaves the parent fourteen.

use super::*;

/// The fifteen physical bones of the CT7 body, parent first, with the capsule of each.
const CT7: [BoneRow; 15] = [
    ("pelvis", None, [0.0, 1.1, 0.0], (0.09, 0.0)),
    ("torso", Some(0), [0.0, 0.17, 0.0], (0.115, 0.13)),
    ("head", Some(1), [0.0, 0.5, 0.0], (0.09, 0.03)),
    ("leg_upper_l", Some(0), [-0.1, -0.25, 0.0], (0.07, 0.148)),
    ("leg_lower_l", Some(3), [0.0, -0.4, 0.0], (0.055, 0.164)),
    ("foot_l", Some(4), [0.0, -0.3, 0.0], (0.04, 0.05)),
    ("leg_upper_r", Some(0), [0.1, -0.25, 0.0], (0.07, 0.148)),
    ("leg_lower_r", Some(6), [0.0, -0.4, 0.0], (0.055, 0.164)),
    ("foot_r", Some(7), [0.0, -0.3, 0.0], (0.04, 0.05)),
    ("arm_upper_l", Some(1), [-0.25, -0.1, 0.0], (0.04, 0.085)),
    ("arm_lower_l", Some(9), [0.0, -0.25, 0.0], (0.035, 0.065)),
    ("palm_l", Some(10), [0.0, -0.2, 0.0], (0.03, 0.02)),
    ("arm_upper_r", Some(1), [0.25, -0.1, 0.0], (0.04, 0.085)),
    ("arm_lower_r", Some(12), [0.0, -0.25, 0.0], (0.035, 0.065)),
    ("palm_r", Some(13), [0.0, -0.2, 0.0], (0.03, 0.02)),
];

fn ct7() -> (Rig, LocalPose) {
    let skeleton = Skeleton {
        joints: CT7
            .iter()
            .map(|(name, parent, _, _)| Joint {
                name: (*name).into(),
                parent: *parent,
            })
            .collect(),
    };
    let shapes = CT7
        .iter()
        .map(|(_, _, _, (radius, half_height))| BoneShape {
            radius: *radius,
            half_height: *half_height,
        })
        .collect();
    let pose = LocalPose {
        positions: CT7.iter().map(|row| row.2).collect(),
        rotations: vec![Transform::IDENTITY.rot; CT7.len()],
    };
    (Rig::new(skeleton, shapes).expect("CT7 rig"), pose)
}

/// The index of the CT7 bone called `name`.
fn ct7_bone(name: &str) -> usize {
    CT7.iter().position(|row| row.0 == name).expect("bone")
}

/// The transform of one body this tick.
fn placed(fx: &PhysFx, key: Key) -> Transform {
    fx.transforms(1.0)
        .expect("transforms")
        .into_iter()
        .find(|one| one.key == key)
        .expect("body")
        .at
}

#[test]
fn a_severed_arm_leaves_the_ragdoll_and_falls_twice_the_same() {
    let (rig, pose) = ct7();
    let arm = ct7_bone("arm_upper_l");
    let palm = ct7_bone("palm_l");
    let part = 7u32;
    let run = || {
        let mut fx = arena(Tier::Comfort);
        fx.spawn_ragdoll(0, &rig, &moved(&pose, [0.0, 3.0, 0.0]), [0.0; 3], [0.0; 3])
            .expect("ragdoll");
        assert_eq!(fx.ragdoll_bones(0), Some(CT7.len()));
        let bone = u16::try_from(arm).expect("bone fits");
        let severed = fx.sever(0, bone, part).expect("sever").expect("ragdoll");
        assert_eq!(severed.bone, bone);
        assert_eq!(
            severed.parent,
            Some(u16::try_from(ct7_bone("torso")).expect("fits")),
            "the joint at the shoulder is the one removed"
        );
        println!(
            "sever: bone={} parent={:?} did={:?} part={}",
            severed.bone, severed.parent, severed.part, part
        );
        // The parent keeps fourteen of the fifteen bones; the arm is a limb group of its own.
        assert_eq!(fx.ragdoll_bones(0), Some(CT7.len() - 1));
        assert_eq!(fx.limb_bodies(part), Some(1));
        let limb = Key::Bone { slot: 0, bone };
        let hand = Key::Bone {
            slot: 0,
            bone: u16::try_from(palm).expect("fits"),
        };
        let start = placed(&fx, limb).pos;
        let hand_start = placed(&fx, hand).pos;
        let mut fall = Vec::new();
        for _ in 0..40 {
            fx.step();
            fall.push(placed(&fx, limb).pos);
        }
        let drop = fall.last().expect("steps")[1] - start[1];
        assert!(drop < -0.5, "the arm falls under gravity: {drop:.2} m");
        let hand_drop = placed(&fx, hand).pos[1] - hand_start[1];
        assert!(
            hand_drop < -0.25,
            "the hand follows the arm through its joint: {hand_drop:.2} m"
        );
        println!("sever: arm drop {drop:.2} m, hand drop {hand_drop:.2} m in 40 ticks");
        // One bone severs once; the root never leaves; a slot with no ragdoll takes nothing.
        assert_eq!(
            fx.sever(0, bone, part + 1),
            Err(Error::OutOfRange(Field::Bone))
        );
        assert_eq!(
            fx.sever(0, 0, part + 2),
            Err(Error::OutOfRange(Field::Bone))
        );
        assert_eq!(fx.sever(9, bone, part + 3), Ok(None));
        fall
    };
    let first = run();
    let second = run();
    assert_eq!(first, second, "the same sever twice gives the same fall");
}
