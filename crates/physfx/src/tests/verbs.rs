//! verbs: the four fx calls turn an event into a receipt, and the body's muscle decides the brace.

use super::*;
use crate::body::parse;
use crate::verbs::{
    fx_blast, fx_break, fx_limb, fx_ragdoll, BlastEvent, BreakEvent, LimbEvent, RagdollEvent,
};
use crate::Error;

/// A body file with three bones, one cuttable joint and the given muscle.
fn file_with_muscle(muscle: u16) -> String {
    format!(
        r#"name = "tester"
gore_permille = 800
muscle_permille = {muscle}

[bone.pelvis]
parent = ""
radius = 0.14
half_height = 0.05

[bone.spine]
parent = "pelvis"
radius = 0.13
half_height = 0.05

[bone.head]
parent = "spine"
radius = 0.11
half_height = 0.04

[cut.spine]
energy = 40
"#
    )
}

/// The pose the three bones of the test body stand in.
fn standing() -> LocalPose {
    LocalPose {
        positions: vec![[0.0, 1.0, 0.0], [0.0, 1.25, 0.0], [0.0, 1.6, 0.0]],
        rotations: vec![Transform::IDENTITY.rot; 3],
    }
}

/// The fighter that fell: slot 0, standing, at rest.
fn fell() -> RagdollEvent {
    RagdollEvent {
        slot: 0,
        pose: standing(),
        velocity: [0.0; 3],
        impulse: [0.0; 3],
    }
}

#[test]
fn a_fallen_fighter_becomes_a_ragdoll_and_the_receipt_says_what_it_measured() {
    let body = parse(&file_with_muscle(900)).expect("body file");
    let mut world = arena(Tier::Comfort);
    let receipt = fx_ragdoll(&mut world, &body, &fell()).expect("ragdoll");
    assert_eq!(receipt.slot, 0);
    assert_eq!(receipt.bones, 3);
    assert_eq!(receipt.muscle, 900);
    assert_eq!(receipt.gore, 800);
    assert_eq!(bones_of(&world, 0).len(), 3);
}

#[test]
fn a_body_with_full_muscle_braces_as_hard_as_the_motors_can() {
    let body = parse(&file_with_muscle(1000)).expect("body file");
    let mut world = arena(Tier::Comfort);
    fx_ragdoll(&mut world, &body, &fell()).expect("ragdoll");
    world
        .hit_ragdoll(0, [0.0, 1.6, 0.0], 0.5, [10.0, 0.0, 0.0])
        .expect("hit");
    assert_eq!(world.ragdoll_strength(0), Some(1.0));
}

#[test]
fn a_body_with_no_muscle_gives_way_at_once() {
    let body = parse(&file_with_muscle(0)).expect("body file");
    let mut world = arena(Tier::Comfort);
    fx_ragdoll(&mut world, &body, &fell()).expect("ragdoll");
    world
        .hit_ragdoll(0, [0.0, 1.6, 0.0], 0.5, [10.0, 0.0, 0.0])
        .expect("hit");
    assert_eq!(world.ragdoll_strength(0), Some(0.0));
}

#[test]
fn half_a_muscle_braces_half_as_hard() {
    let body = parse(&file_with_muscle(500)).expect("body file");
    let mut world = arena(Tier::Comfort);
    fx_ragdoll(&mut world, &body, &fell()).expect("ragdoll");
    world
        .hit_ragdoll(0, [0.0, 1.6, 0.0], 0.5, [10.0, 0.0, 0.0])
        .expect("hit");
    let strength = world.ragdoll_strength(0).expect("strength");
    assert!((strength - 0.5).abs() < 1e-6, "strength {strength}");
}

#[test]
fn a_limb_that_came_off_becomes_a_piece_and_the_receipt_carries_the_cuts_price() {
    let body = parse(&file_with_muscle(900)).expect("body file");
    let mut world = arena(Tier::Comfort);
    let event = LimbEvent {
        slot: 0,
        bone: "spine".to_string(),
        part: 7,
        at: at([0.0, 1.2, 0.0]),
        velocity: [1.0, 2.0, 0.0],
    };
    let receipt = fx_limb(&mut world, &body, &event).expect("limb");
    assert_eq!(receipt.part, 7);
    assert_eq!(receipt.bone, "spine");
    assert_eq!(receipt.energy, 40);
    assert_eq!(receipt.gore, 800);
    assert_eq!(limb_parts(&world), vec![7]);
}

#[test]
fn a_limb_at_a_bone_the_body_does_not_have_is_refused_and_nothing_is_spawned() {
    let body = parse(&file_with_muscle(900)).expect("body file");
    let mut world = arena(Tier::Comfort);
    let event = LimbEvent {
        slot: 0,
        bone: "tail".to_string(),
        part: 7,
        at: at([0.0, 1.2, 0.0]),
        velocity: [0.0; 3],
    };
    let err = fx_limb(&mut world, &body, &event).expect_err("unknown bone");
    assert_eq!(
        err,
        Error::UnknownBone {
            name: "tail".to_string(),
        }
    );
    assert!(limb_parts(&world).is_empty());
}

#[test]
fn a_limb_at_a_joint_a_blade_may_not_take_is_refused_and_nothing_is_spawned() {
    let body = parse(&file_with_muscle(900)).expect("body file");
    let mut world = arena(Tier::Comfort);
    let event = LimbEvent {
        slot: 0,
        bone: "head".to_string(),
        part: 7,
        at: at([0.0, 1.6, 0.0]),
        velocity: [0.0; 3],
    };
    let err = fx_limb(&mut world, &body, &event).expect_err("uncuttable");
    assert_eq!(
        err,
        Error::UncuttableJoint {
            joint: "head".to_string(),
        }
    );
    assert!(limb_parts(&world).is_empty());
}

#[test]
fn a_wall_that_broke_becomes_its_pieces() {
    let body = parse(&file_with_muscle(900)).expect("body file");
    let mut world = arena(Tier::Comfort);
    let event = BreakEvent {
        shards: debris_field(3, 1.0),
        impulse: [0.0, 200.0, 0.0],
    };
    let receipt = fx_break(&mut world, &body, &event).expect("break");
    assert_eq!(receipt.admitted, 3);
    assert_eq!(receipt.gore, 800);
}

#[test]
fn a_blast_pushes_the_bodies_it_reaches() {
    let body = parse(&file_with_muscle(900)).expect("body file");
    let mut world = arena(Tier::Comfort);
    fx_ragdoll(&mut world, &body, &fell()).expect("ragdoll");
    let event = BlastEvent {
        blast: Blast {
            center_mm: [0, 1000, 0],
            radius_mm: 5000,
            impulse_ns: 1000,
        },
    };
    let receipt = fx_blast(&mut world, &body, &event).expect("blast");
    assert_eq!(receipt.pushed, 3);
    assert_eq!(receipt.gore, 800);
}
