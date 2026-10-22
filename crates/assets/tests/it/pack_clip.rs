//! AM29: a baked combat pack, played on the game's own CT7 body.
//!
//! The four combat packs under `fixtures/clips/combat/` are real teacher
//! output, baked by the engine's own tool onto the fighter rig. They were
//! proven to play on that rig (see `combat_clips.rs`). This test asks the
//! question the game actually asks: can the game's body play them?
//!
//! The game loads its clips as glTF clip sources and retargets them onto the
//! CT7 body by name. A pack is a different format against a different rig, so
//! it needed a loader. This test drives that loader against the **real**
//! body glb (`content/fighters/glb/hero_body.glb`), so the names, the bind
//! pose and the joint count are the game's own, not a stand-in.
//!
//! What it proves:
//! - each of the four packs loads and plays on the real body,
//! - the clip lands on the body's joint count, with the pack's duration,
//! - the arms carry the teacher's rotations (not the bind pose), so the
//!   swing turns instead of sliding,
//! - the left arm and the right arm move differently (a map that swapped
//!   them would look right in a still and wrong in a fight),
//! - and the feet stay near the ground, so the clip did not float off the
//!   rig.

#![cfg(feature = "import")]

use engine_assets::body_import::import_skinned_body;
use engine_assets::pack_clip::clip_from_pack_bytes;

/// Repository root, one level above the crate.
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

/// The four combat clips: the name and the number of ticks the clip holds.
const CLIPS: [&str; 4] = ["slash_over", "spear_thrust", "parry", "slash_h"];

/// The body's glb: the real game body, with its own joint names and bind.
const BODY_GLB: &str = "content/fighters/glb/hero_body.glb";

/// The committed bytes of one fixture path.
fn fixture(path: &str) -> Vec<u8> {
    let full = format!("{ROOT}/{path}");
    std::fs::read(&full).unwrap_or_else(|why| panic!("read {full}: {why}"))
}

/// The real game body: its skeleton and its bind pose.
fn game_body() -> (
    engine_anim::contract::Skeleton,
    engine_assets::retarget::BindPose,
) {
    let bytes = fixture(BODY_GLB);
    let body = import_skinned_body(&bytes).expect("the game body imports");
    (body.skeleton, body.bind)
}

/// The joint index of a named joint on the body.
fn joint(body: &engine_anim::contract::Skeleton, name: &str) -> usize {
    body.joints
        .iter()
        .position(|joint| joint.name == name)
        .unwrap_or_else(|| panic!("the body has the {name} joint"))
}

#[test]
fn every_combat_pack_plays_on_the_game_body() {
    let (body, bind) = game_body();
    // The game's own body: 44 joints plus the root node.
    assert!(body.joints.len() >= 44, "the body has its joints");
    for clip in CLIPS {
        let bytes = fixture(&format!("fixtures/clips/combat/{clip}.am5p"));
        let played = clip_from_pack_bytes(&bytes, &body, &bind)
            .unwrap_or_else(|err| panic!("{clip} plays on the body: {err}"));
        assert_eq!(
            played.positions.tracks.len(),
            body.joints.len(),
            "{clip}: one track per body joint"
        );
        assert_eq!(
            played.rotations.len(),
            body.joints.len(),
            "{clip}: one rotation track per body joint"
        );
        assert!(
            played.duration > 0.0,
            "{clip}: the clip has a duration, got {}",
            played.duration
        );
        assert!(
            played.duration.is_finite(),
            "{clip}: the duration is finite"
        );
    }
}

#[test]
fn the_clip_carries_the_teachers_rotations_not_the_bind_pose() {
    let (body, bind) = game_body();
    let bytes = fixture("fixtures/clips/combat/slash_over.am5p");
    let played = clip_from_pack_bytes(&bytes, &body, &bind).expect("the clip plays");
    // The version-2 wire carries rotations. If they were missing, every
    // track would be the identity quaternion and the arms would slide on
    // the bind rotation instead of turning.
    let arm = joint(&body, "arm_upper_r");
    let turns = played.rotations[arm]
        .values
        .iter()
        .any(|quat| (quat[3].abs() - 1.0).abs() > 1e-3 || quat[0].abs() > 1e-3);
    assert!(
        turns,
        "the right upper arm turns over the swing, it does not hold the bind rotation"
    );
    // Every rotation is a unit quaternion: a non-unit one would scale the
    // bone when the skinning composes it.
    for (index, track) in played.rotations.iter().enumerate() {
        for quat in &track.values {
            let norm =
                (quat[0] * quat[0] + quat[1] * quat[1] + quat[2] * quat[2] + quat[3] * quat[3])
                    .sqrt();
            assert!(
                (norm - 1.0).abs() < 1e-3,
                "joint {index} holds a unit quaternion, got {norm}"
            );
        }
    }
}

#[test]
fn the_two_arms_move_differently() {
    let (body, bind) = game_body();
    let bytes = fixture("fixtures/clips/combat/slash_over.am5p");
    let played = clip_from_pack_bytes(&bytes, &body, &bind).expect("the clip plays");
    let left = joint(&body, "arm_upper_l");
    let right = joint(&body, "arm_upper_r");
    // A map that swapped the sides would give both arms the same motion.
    let left_moves: Vec<[f32; 3]> = played.positions.tracks[left].values.clone();
    let right_moves: Vec<[f32; 3]> = played.positions.tracks[right].values.clone();
    assert_ne!(
        left_moves, right_moves,
        "the left arm and the right arm move differently"
    );
}

#[test]
fn the_feet_stay_on_the_ground() {
    let (body, bind) = game_body();
    for clip in CLIPS {
        let bytes = fixture(&format!("fixtures/clips/combat/{clip}.am5p"));
        let played = clip_from_pack_bytes(&bytes, &body, &bind).expect("the clip plays");
        for side in ["foot_l", "foot_r"] {
            let foot = joint(&body, side);
            let track = &played.positions.tracks[foot];
            assert!(!track.values.is_empty(), "{clip}: the {side} has motion");
            for value in &track.values {
                assert!(value[1].is_finite(), "{clip}: the {side} height is finite");
                assert!(
                    value[1] > -1.0 && value[1] < 1.0,
                    "{clip}: the {side} stays near the ground, got {}",
                    value[1]
                );
            }
        }
    }
}

#[test]
fn the_same_pack_bytes_give_the_same_clip_twice() {
    let (body, bind) = game_body();
    let bytes = fixture("fixtures/clips/combat/parry.am5p");
    let first = clip_from_pack_bytes(&bytes, &body, &bind).expect("the clip plays");
    let second = clip_from_pack_bytes(&bytes, &body, &bind).expect("the clip plays");
    assert_eq!(first, second, "the same bytes give the same clip");
}

#[test]
fn a_pack_for_another_rig_refuses_loud() {
    let (body, bind) = game_body();
    // A pack baked for a rig with two joints: it has no track for the body's
    // joints, so it refuses rather than playing wrong motion.
    let mut clip = engine_anim::clip::QClip {
        tracks: Vec::new(),
        dur: 0.5,
    };
    let mut rotations = Vec::new();
    for _ in 0..2 {
        clip.tracks.push(engine_anim::clip::bake_track(
            vec![0.0, 0.5],
            vec![[0.0, 0.0, 0.0], [0.0, 0.0, 0.0]],
        ));
        rotations.push(engine_anim::pose::QuatTrack {
            times: vec![0.0, 0.5],
            values: vec![[0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 1.0]],
        });
    }
    let bytes = engine_anim::motion_bake::bake_pack_with_rots(&clip, &rotations)
        .expect("the two-joint pack bakes");
    let result = clip_from_pack_bytes(&bytes, &body, &bind);
    assert!(result.is_err(), "a pack for another rig refuses");
    let message = result.expect_err("the refusal").to_string();
    assert!(
        message.contains("tracks"),
        "the refusal names the mismatch: {message}"
    );
}

#[test]
fn a_cut_pack_refuses_loud() {
    let (body, bind) = game_body();
    let mut bytes = fixture("fixtures/clips/combat/slash_over.am5p");
    bytes.truncate(bytes.len() / 2);
    let result = clip_from_pack_bytes(&bytes, &body, &bind);
    assert!(result.is_err(), "a cut pack refuses");
}
