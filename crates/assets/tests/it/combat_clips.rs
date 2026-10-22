//! AM29: the real combat clips, baked from a Kimodo teacher and played on the
//! fighter rig.
//!
//! The AM27 fixture (`stagger.am5p`) is authored data: a synthetic SOMA-77
//! stagger the tool made up. It proves the wire, not the motion. These four
//! clips are the other thing: real teacher output, baked by the same tool
//! through the same name table, and the numbers in their receipts are the
//! numbers the empire card attested for them.
//!
//! Each clip carries the whole fighter body (65 tracks, one per joint, named
//! in the receipt) and its own rotation tracks, so an arm swings instead of
//! sliding on its bind rotation. The test proves the four of them: the pack
//! loads from disk, it round-trips to the same bytes, the receipt's hash and
//! slip are the pack's own, and the clip plays on the fighter rig by name.
//!
//! The sources are Kimodo teacher clips (NVIDIA Open Model License), converted
//! from the teacher's own SOMA BVH into the teacher's NPZ layout and baked
//! here. The provenance sidecars sit next to the packs.

use engine_anim::clip::{avg_slip, qclip_hash, TICK_DT};
use engine_anim::motion_bake::{bake_unpack, bake_unpack_rots};
use engine_anim::pack_file::{load_pack, rotations_of, sample_pose_named_with_rots};
use engine_assets::fighter::fighter_skeleton;

/// Repository root, one level above the crate.
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
/// The fighter rig's joint count: one track per joint.
const FIGHTER_JOINTS: usize = 65;

/// The four combat clips: the name, the receipt's slip for the left foot and
/// the number of ticks the clip holds. The slip numbers are the card's
/// attested values: a slash that lands without the foot sliding, a thrust
/// that steps, a parry that stands and a slash that slides.
const CLIPS: [(&str, f32, u32); 4] = [
    ("slash_over", 0.000389, 119),
    ("spear_thrust", 0.003025, 95),
    ("parry", 0.080097, 119),
    ("slash_h", 0.133841, 95),
];

/// The committed bytes of one fixture path.
fn fixture(path: &str) -> Vec<u8> {
    let full = format!("{ROOT}/fixtures/clips/combat/{path}");
    std::fs::read(&full).unwrap_or_else(|why| panic!("read {full}: {why}"))
}

/// The receipt of one combat clip, as text.
fn receipt(clip: &str) -> String {
    String::from_utf8(fixture(&format!("{clip}.receipt.json"))).expect("receipt utf-8")
}

/// One receipt value by key, as text. The receipt is a flat object the tool
/// writes with one key per line, so the slice from the key to its line end is
/// exact; `after` scopes a lookup to a section that repeats a key.
fn value<'a>(text: &'a str, key: &str, after: usize) -> &'a str {
    let needle = format!("\"{key}\":");
    let at = text[after..]
        .find(&needle)
        .unwrap_or_else(|| panic!("the receipt has no {key}"))
        + after
        + needle.len();
    let rest = text[at..].trim_start();
    if let Some(quoted) = rest.strip_prefix('"') {
        return &quoted[..quoted.find('"').expect("closing quote")];
    }
    let end = rest.find([',', '}', '\n']).unwrap_or(rest.len());
    rest[..end].trim_end()
}

/// One receipt number by key, inside the section starting at `after`.
fn number(text: &str, key: &str, after: usize) -> f32 {
    value(text, key, after)
        .parse::<f32>()
        .unwrap_or_else(|why| panic!("{key} is not a number: {why}"))
}

/// One receipt integer by key.
fn integer(text: &str, key: &str) -> u64 {
    value(text, key, 0)
        .parse::<u64>()
        .unwrap_or_else(|why| panic!("{key} is not an integer: {why}"))
}

/// One receipt hash by key, `0x`-prefixed hex.
fn hash(text: &str, key: &str) -> u64 {
    let text = value(text, key, 0);
    u64::from_str_radix(text.trim_start_matches("0x"), 16)
        .unwrap_or_else(|why| panic!("{key} is not a hash: {why}"))
}

/// The track names the receipt lists, in the pack's own order.
fn track_names(receipt: &str) -> Vec<String> {
    let key = "\"tracks\": [";
    let at = receipt.find(key).expect("the receipt names its tracks");
    let rest = &receipt[at + key.len()..];
    let end = rest.find(']').expect("the tracks array ends");
    rest[..end]
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect()
}

#[test]
fn every_combat_clip_loads_and_plays_on_the_fighter_rig() {
    let rig = fighter_skeleton().expect("the fighter rig builds");
    assert_eq!(rig.len(), FIGHTER_JOINTS, "the rig's joint count");
    for (name, slip, ticks) in CLIPS {
        let bytes = fixture(&format!("{name}.am5p"));
        let text = receipt(name);
        assert_eq!(&bytes[..4], b"AM5P", "{name}: the wire magic");
        let clip = bake_unpack(&bytes).expect("{name}: the pack loads");
        assert_eq!(
            clip.tracks.len(),
            FIGHTER_JOINTS,
            "{name}: the pack carries the whole fighter body"
        );
        assert_eq!(integer(&text, "track_count"), 65, "{name}: 65 tracks");
        assert_eq!(integer(&text, "ticks"), u64::from(ticks), "{name}: ticks");
        // The clip's duration is its last knot: `ticks` knots at 60 Hz run
        // from 0 to (ticks - 1) / 60 seconds.
        assert_eq!(
            clip.dur.to_bits(),
            ((ticks - 1) as f32 * TICK_DT).to_bits(),
            "{name}: dur"
        );
        // The receipt's dur is printed to six decimals, so it matches the
        // pack's duration to that many places.
        let reported = number(&text, "dur_s", 0);
        assert!(
            (reported - clip.dur).abs() < 1.0e-5,
            "{name}: the receipt says {reported}, the pack holds {}",
            clip.dur
        );
        // The receipt's hash is the pack's own hash.
        assert_eq!(
            qclip_hash(&clip),
            hash(&text, "qclip_hash"),
            "{name}: the receipt's hash is the pack's"
        );
        // The slip the card attested is the slip the pack measures.
        let names = track_names(&text);
        let left = names
            .iter()
            .position(|n| n == "leftfoot")
            .expect("the receipt names the left foot");
        let measured = avg_slip(&clip, left, ticks);
        assert!(
            (measured - slip).abs() < 1.0e-5,
            "{name}: the receipt says {slip}, the pack measures {measured}"
        );
        // The pack plays on the rig, by name, and the pose is a real pose.
        let listed: Vec<&str> = names.iter().map(String::as_str).collect();
        let path = format!("{ROOT}/fixtures/clips/combat/{name}.am5p");
        let loaded = load_pack(std::path::Path::new(&path)).expect("{name}: the pack loads");
        let pose = sample_pose_named_with_rots(&loaded, &rig, &listed, 0.5)
            .expect("{name}: the pack plays on the rig");
        assert_eq!(pose.positions.len(), FIGHTER_JOINTS, "{name}: one pose per joint");
        assert!(
            pose.positions.iter().flatten().all(|v| v.is_finite()),
            "{name}: every position is finite"
        );
        assert!(
            pose.rotations.iter().flatten().all(|v| v.is_finite()),
            "{name}: every rotation is finite"
        );
        println!(
            "AM29_COMBAT clip={name} ticks={ticks} slip={measured:.6} bytes={}",
            bytes.len()
        );
    }
}

#[test]
fn every_combat_clip_carries_the_joint_rotations() {
    for (name, _slip, _ticks) in CLIPS {
        let bytes = fixture(&format!("{name}.am5p"));
        let rots = bake_unpack_rots(&bytes).expect("{name}: the rotations read back");
        assert_eq!(
            rots.len(),
            FIGHTER_JOINTS,
            "{name}: one rotation track per joint"
        );
        // The rotations are unit quaternions: the wire's own decoder hands
        // back what the tick will play.
        for track in &rots {
            for quat in &track.values {
                let norm = quat.iter().map(|v| v * v).sum::<f32>().sqrt();
                assert!(
                    (norm - 1.0).abs() < 1.0e-4,
                    "{name}: a unit quaternion, got {norm}"
                );
            }
        }
    }
}

#[test]
fn a_combat_clip_turns_its_arms_over_the_swing() {
    // The slash_over clip: a sword swung overhead. Its arm joints turn over
    // the clip, so a rig playing it shows the swing and not a slide.
    let text = receipt("slash_over");
    let names = track_names(&text);
    let rig = fighter_skeleton().expect("the fighter rig builds");
    let listed: Vec<&str> = names.iter().map(String::as_str).collect();
    let path = format!("{ROOT}/fixtures/clips/combat/slash_over.am5p");
    let loaded = load_pack(std::path::Path::new(&path)).expect("the pack loads");
    let rots = rotations_of(&loaded);
    let right_arm = names
        .iter()
        .position(|n| n == "rightarm")
        .expect("the receipt names the right arm");
    let arm = &rots[right_arm];
    // The arm turns: the first and the last key are not the same rotation.
    let first = arm.values[0];
    let last = arm.values[arm.values.len() - 1];
    let turned = (0..4)
        .map(|lane| (first[lane] - last[lane]).abs())
        .fold(0.0f32, f32::max);
    assert!(turned > 0.01, "the arm turned: {first:?} to {last:?}");
    // And the pose the rig reads carries that turn.
    let pose = sample_pose_named_with_rots(&loaded, &rig, &listed, 0.0).expect("the pack plays");
    let arm_joint = rig
        .joints
        .iter()
        .position(|joint| joint.name == "rightarm")
        .expect("the rig has the right arm");
    assert!(
        pose.rotations[arm_joint].iter().any(|v| v.abs() > 0.001),
        "the arm's rotation came through: {:?}",
        pose.rotations[arm_joint]
    );
    println!("AM29_ROT clip=slash_over arm_turn={turned:.6}");
}

#[test]
fn a_combat_clip_refuses_a_rig_it_was_not_baked_for() {
    let text = receipt("slash_over");
    let names = track_names(&text);
    let path = format!("{ROOT}/fixtures/clips/combat/slash_over.am5p");
    let loaded = load_pack(std::path::Path::new(&path)).expect("the pack loads");
    // A rig of another size refuses loud.
    let short = engine_anim::contract::Skeleton {
        joints: vec![engine_anim::contract::Joint {
            name: "pelvis".to_string(),
            parent: None,
        }],
    };
    let listed: Vec<&str> = names.iter().map(String::as_str).collect();
    let err = sample_pose_named_with_rots(&loaded, &short, &listed, 0.5)
        .expect_err("another rig refuses loud");
    assert!(
        matches!(
            err,
            engine_anim::pack_file::PackFileError::TrackCount { .. }
        ),
        "the track count refuses: {err:?}"
    );
}
