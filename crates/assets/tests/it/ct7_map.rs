//! The CT7 name map: the bridge a baked combat pack needs to play on the
//! game's body.
//!
//! The engine's fighter rig names its 65 joints SOMA-style; the CT7 body the
//! game plays names its 44 joints Mixamo-style. This test proves the map
//! covers the body, that every row names a joint the fighter rig really has,
//! and that the map drives the right bone: a swapped left/right arm would
//! look right in a still and wrong in a fight.

#![cfg(feature = "import")]

use engine_anim::contract::{Joint, Skeleton};
use engine_assets::ct7_map::{ct7_map, fighter_track_names, CT7_TO_BODY};
use engine_assets::fighter::fighter_skeleton;

/// The CT7 rig's 44 joints, in the order the body glb lists them.
fn ct7_skeleton() -> Skeleton {
    let names: Vec<&str> = CT7_TO_BODY.iter().map(|(name, _)| *name).collect();
    let mut joints = Vec::with_capacity(names.len());
    for (index, name) in names.iter().enumerate() {
        joints.push(Joint {
            name: (*name).to_string(),
            parent: if index == 0 { None } else { Some(index - 1) },
        });
    }
    Skeleton { joints }
}

#[test]
fn the_map_covers_every_ct7_joint_the_body_carries() {
    let body = ct7_skeleton();
    let map = ct7_map(&body).expect("the map builds");
    // The map has one row per body joint, driven or not: a joint the fighter
    // rig has no counterpart for holds its bind pose.
    assert_eq!(map.len(), body.len(), "one row per body joint");
    // The joints the fighter rig has no counterpart for are the root, the
    // two weapon sockets and the two heels; they hold their bind pose and
    // carry no motion.
    let without = CT7_TO_BODY
        .iter()
        .filter(|(_, fighter)| fighter.is_none())
        .count();
    assert_eq!(without, 5, "the root, two sockets and two heels");
    // Every other joint is driven by a fighter track.
    let driven = (0..body.len())
        .filter(|joint| map.source_of(*joint).is_some())
        .count();
    assert_eq!(driven, body.len() - without, "every other joint is driven");
}

#[test]
fn every_row_names_a_joint_the_fighter_rig_really_has() {
    let fighter = fighter_skeleton().expect("the fighter rig builds");
    for (ct7, source) in CT7_TO_BODY {
        let Some(source) = source else {
            continue;
        };
        assert!(
            fighter.joints.iter().any(|joint| joint.name == source),
            "{ct7} names {source}, which the fighter rig does not have"
        );
    }
}

#[test]
fn the_map_drives_the_right_bone_on_each_side() {
    let body = ct7_skeleton();
    let map = ct7_map(&body).expect("the map builds");
    let fighter = fighter_skeleton().expect("the fighter rig builds");
    // The left hand rides the left fighter hand, not the right one.
    let left_hand = body
        .joints
        .iter()
        .position(|joint| joint.name == "palm_l")
        .expect("the body has the left hand");
    let right_hand = body
        .joints
        .iter()
        .position(|joint| joint.name == "palm_r")
        .expect("the body has the right hand");
    let left_arm = fighter
        .joints
        .iter()
        .position(|joint| joint.name == "lefthand")
        .expect("the fighter rig has the left hand");
    let right_arm = fighter
        .joints
        .iter()
        .position(|joint| joint.name == "righthand")
        .expect("the fighter rig has the right hand");
    assert_eq!(map.source_of(left_hand), Some(left_arm), "the left hand");
    assert_eq!(map.source_of(right_hand), Some(right_arm), "the right hand");
    // The feet, the same way.
    let left_foot = body
        .joints
        .iter()
        .position(|joint| joint.name == "foot_l")
        .expect("the body has the left foot");
    let fighter_foot = fighter
        .joints
        .iter()
        .position(|joint| joint.name == "leftfoot")
        .expect("the fighter rig has the left foot");
    assert_eq!(
        map.source_of(left_foot),
        Some(fighter_foot),
        "the left foot"
    );
}

#[test]
fn the_fighter_track_names_are_the_pack_order() {
    let fighter = fighter_skeleton().expect("the fighter rig builds");
    let names = fighter_track_names().expect("the fighter rig names itself");
    assert_eq!(names.len(), 65, "one name per fighter joint");
    for (index, joint) in fighter.joints.iter().enumerate() {
        assert_eq!(names[index], joint.name, "the rig's own order");
    }
    // The names are the anim contract's charset, so a pack can carry them.
    for name in &names {
        assert!(
            name.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "{name} is a contract name"
        );
    }
}

#[test]
fn a_body_without_the_ct7_joints_still_builds_a_map() {
    // A body that carries none of the CT7 joints gets an empty map, not a
    // refusal: the map is about the body's own joints.
    let other = Skeleton {
        joints: vec![Joint {
            name: "pelvis".to_string(),
            parent: None,
        }],
    };
    let map = ct7_map(&other).expect("the map builds");
    assert_eq!(map.len(), 1, "one row for the one shared joint");
    let fighter = fighter_skeleton().expect("the fighter rig builds");
    let hips = fighter
        .joints
        .iter()
        .position(|joint| joint.name == "hips")
        .expect("the fighter rig has the hips");
    assert_eq!(map.source_of(0), Some(hips), "the pelvis rides the hips");
}
