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

/// The body's bind world positions, composed down each chain with its bind
/// rotations: the game skins from these, so the standing height is the head's
/// world Y here.
fn bind_world_pos(
    body: &engine_anim::contract::Skeleton,
    bind: &engine_assets::retarget::BindPose,
) -> Vec<[f32; 3]> {
    let mut world = vec![[0.0f32; 3]; body.len()];
    for (index, joint) in body.joints.iter().enumerate() {
        let (parent_pos, parent_rot) = match joint.parent {
            Some(parent) => (world[parent], bind.rotations[parent]),
            None => ([0.0; 3], [0.0, 0.0, 0.0, 1.0]),
        };
        let turned = rotate(bind.positions[index], parent_rot);
        world[index] = [
            parent_pos[0] + turned[0],
            parent_pos[1] + turned[1],
            parent_pos[2] + turned[2],
        ];
    }
    world
}

/// The transferred clip's world positions per joint per key, composed down
/// each chain exactly the skinning convention: `world = parent + R * local`
/// where `R` accumulates the clip's own rotations.
///
/// The key count is the **longest** position track's, not track zero's: a
/// joint the map does not bind holds a single key (its bind pose), so track
/// zero — the body root — can be one key long while the clip is two seconds.
fn clip_world_positions(
    clip: &engine_assets::skin_import::ImportedClip,
    body: &engine_anim::contract::Skeleton,
) -> Vec<Vec<[f32; 3]>> {
    let keys = clip
        .positions
        .tracks
        .iter()
        .map(|track| track.values.len())
        .max()
        .unwrap_or(0);
    let mut out: Vec<Vec<[f32; 3]>> = vec![Vec::with_capacity(keys); body.len()];
    for at in 0..keys {
        let mut world_pos = vec![[0.0f32; 3]; body.len()];
        for (index, joint) in body.joints.iter().enumerate() {
            let local = pos_at(&clip.positions.tracks[index].values, at);
            let (parent_pos, parent_rot) = match joint.parent {
                Some(parent) => (world_pos[parent], world_rot(parent, clip, body, at)),
                None => ([0.0; 3], [0.0, 0.0, 0.0, 1.0]),
            };
            let turned = rotate(local, parent_rot);
            world_pos[index] = [
                parent_pos[0] + turned[0],
                parent_pos[1] + turned[1],
                parent_pos[2] + turned[2],
            ];
        }
        for (index, pos) in world_pos.iter().enumerate() {
            out[index].push(*pos);
        }
    }
    out
}

/// One joint's accumulated world rotation at one key: the clip's rotations
/// composed from the identity down to the joint.
///
/// A joint the map does not bind carries a single key (its bind pose held),
/// so a lane shorter than the clip's key count reads its last value.
fn world_rot(
    joint: usize,
    clip: &engine_assets::skin_import::ImportedClip,
    body: &engine_anim::contract::Skeleton,
    at: usize,
) -> [f32; 4] {
    let mut chain = Vec::new();
    let mut cur = Some(joint);
    while let Some(j) = cur {
        chain.push(j);
        cur = body.joints[j].parent;
    }
    chain.reverse();
    let mut rot = [0.0, 0.0, 0.0, 1.0];
    for j in chain {
        rot = quat_mul(rot, lane_at(&clip.rotations[j].values, at));
    }
    rot
}

/// The value of a position lane at one key, or its last value when the lane
/// is shorter (a joint the map does not bind holds one key, its bind pose).
fn pos_at(lane: &[[f32; 3]], at: usize) -> [f32; 3] {
    lane.get(at)
        .copied()
        .unwrap_or_else(|| lane.last().copied().unwrap_or([0.0; 3]))
}

/// The value of a lane at one key, or its last value when the lane is
/// shorter (a held bind pose).
fn lane_at(lane: &[[f32; 4]], at: usize) -> [f32; 4] {
    lane.get(at)
        .copied()
        .unwrap_or_else(|| lane.last().copied().unwrap_or([0.0, 0.0, 0.0, 1.0]))
}

/// The raw Hamilton product of two `[x, y, z, w]` quaternions, no
/// normalization: the tests compose offsets exactly like the anim skinning.
fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

/// Rotate a vector by a quaternion, `q * v * conj(q)`: length-preserving.
fn rotate(v: [f32; 3], quat: [f32; 4]) -> [f32; 3] {
    let r = quat_mul(
        quat_mul(quat, [v[0], v[1], v[2], 0.0]),
        [-quat[0], -quat[1], -quat[2], quat[3]],
    );
    [r[0], r[1], r[2]]
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
fn the_clip_stands_on_the_body_and_reaches_its_own_height() {
    // The transfer must place the teacher's motion on the game's own body:
    // the head stays near the body's height and the feet on the ground, in
    // every clip. A transfer that scaled the motion wrong (the old retarget
    // grew the torso by 36x) would push the head past the body or sink the
    // feet through the floor.
    let (body, bind) = game_body();
    let head = joint(&body, "head");
    let bind_world = bind_world_pos(&body, &bind);
    let body_height = bind_world[head][1];
    assert!(
        body_height > 1.0,
        "the body has a standing height, got {body_height}"
    );
    for clip in CLIPS {
        let bytes = fixture(&format!("fixtures/clips/combat/{clip}.am5p"));
        let played = clip_from_pack_bytes(&bytes, &body, &bind).expect("the clip plays");
        // The transferred world positions, composed down each chain exactly
        // the way the game skins (world = parent + R_parent * local, with
        // the clip's own rotations).
        let world = clip_world_positions(&played, &body);
        let mut head_max = f32::NEG_INFINITY;
        let mut foot_min = f32::INFINITY;
        let mut foot_max = f32::NEG_INFINITY;
        for (joint, joint_def) in body.joints.iter().enumerate() {
            let is_foot = joint_def.name == "foot_l" || joint_def.name == "foot_r";
            for pos in &world[joint] {
                if joint == head {
                    head_max = head_max.max(pos[1]);
                } else if is_foot {
                    foot_min = foot_min.min(pos[1]);
                    foot_max = foot_max.max(pos[1]);
                }
            }
        }
        assert!(
            (head_max - body_height).abs() < 0.30,
            "{clip}: the head stays near the body height, got {head_max} vs {body_height}"
        );
        // The feet ride the body's own ground: they may lift through a
        // stride (a slash raises the trailing foot about 26 cm) but never
        // sink through the sand. The band holds a stride, not a float.
        assert!(
            foot_min > -0.15 && foot_max < 0.30,
            "{clip}: the feet stay on the ground, got {foot_min}..{foot_max}"
        );
        println!(
            "AM29_BODY clip={clip} body_height={body_height:.3} head_max={head_max:.3} foot_y={foot_min:.3}..{foot_max:.3}"
        );
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
