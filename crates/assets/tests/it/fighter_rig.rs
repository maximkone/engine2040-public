//! The fighter rig the teacher lane lands on, pinned in the engine.
//!
//! The bake tool used to ship its own copy of this rig, so the tick could
//! not know the joint order a baked pack was written in. The rig now lives in
//! `engine-assets` (`fighter.rs`) and this module pins it: the fighter is 65
//! joints in the SOMA pair order, the parents chain backwards, the bind is the
//! SOMA rest of the mapped joints with the ankles stretched, and the teacher's
//! rest pose covers all 77 SOMA joints so the teacher FK has a full body.
// The rig rides the `import` feature: that is the lane that carries
// `engine-anim`, whose joint and skeleton types the rig is built from.
#![cfg(feature = "import")]

use engine_assets::fighter::{
    fighter_bind, fighter_skeleton, joint_index, soma_offsets, soma_skeleton, FOOT_STRETCH,
    SOMA_REST,
};
use engine_assets::soma::{soma_pairs, SOMA77};

/// The fighter rig the anim contract names.
#[test]
fn fighter_rig_is_the_pair_order_with_a_backwards_chain() {
    let fighter = fighter_skeleton().expect("the pair table is parent-first");
    let pairs = soma_pairs();
    assert_eq!(fighter.len(), pairs.len(), "one joint per mapped pair");
    for (index, (target, _)) in pairs.iter().enumerate() {
        assert_eq!(
            fighter.joints[index].name, *target,
            "joint {index} is the pair target"
        );
        if let Some(parent) = fighter.joints[index].parent {
            assert!(parent < index, "the parent of {target} comes first");
        }
    }
    assert_eq!(
        joint_index(&fighter, "hips"),
        Some(0),
        "the root is the hips"
    );
}

#[test]
fn fighter_bind_is_the_soma_rest_with_stretched_ankles() {
    let fighter = fighter_skeleton().expect("the pair table is parent-first");
    let bind = fighter_bind().expect("every mapped joint has a rest row");
    assert_eq!(bind.positions.len(), fighter.len(), "one offset per joint");
    assert_eq!(
        bind.rotations.len(),
        fighter.len(),
        "one rotation per joint"
    );
    for (index, (target, soma)) in soma_pairs().iter().enumerate() {
        let rest = SOMA_REST
            .iter()
            .find(|(name, _)| *name == *soma)
            .map(|(_, offset)| *offset)
            .expect("the mapped joint has a rest row");
        let mut want = rest;
        if *target == "leftfoot" || *target == "rightfoot" {
            want[1] *= FOOT_STRETCH;
        }
        assert_eq!(
            bind.positions[index], want,
            "{target} keeps the SOMA rest offset"
        );
    }
    let ankle =
        bind.positions[joint_index(&fighter, "leftfoot").expect("the fighter has a left foot")];
    let rest = SOMA_REST
        .iter()
        .find(|(name, _)| *name == "leftfoot")
        .map(|(_, offset)| *offset)
        .expect("the foot has a rest row");
    assert!(
        ankle[1] < rest[1],
        "the stretched ankle is longer than the SOMA rest"
    );
    assert_ne!(
        ankle[1], rest[1],
        "the bind scale is not 1.0, so the path is exercised"
    );
}

#[test]
fn the_teacher_skeleton_covers_every_soma_joint() {
    let teacher = soma_skeleton().expect("the teacher table is parent-first");
    assert_eq!(teacher.len(), SOMA77.len(), "one joint per SOMA row");
    assert_eq!(
        soma_offsets().positions.len(),
        SOMA77.len(),
        "one rest row per SOMA joint"
    );
    for (index, &(name, _)) in SOMA77.iter().enumerate() {
        assert_eq!(
            teacher.joints[index].name, name,
            "joint {index} is the SOMA row"
        );
    }
}
