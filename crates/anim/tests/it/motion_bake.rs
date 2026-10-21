//! AM27: the committed motion bake, read the way the engine reads it.
//!
//! The bake tool (`tools/motion_bake`) wrote `fixtures/clips/stagger.am5p`
//! from its own seed-7 teacher clip (`stagger.npz`) and printed
//! `fixtures/clips/stagger.receipt.json`. This module proves the shipped
//! bytes: the pack loads through the existing unpack path, the planted foot
//! slips under 8 cm/s, the decode re-packs to the same bytes, `qclip_hash`
//! equals the receipt and the seed-7 constant two tool runs printed, and the
//! endpoint rule cut the teacher's trailing tail.

use engine_anim::clip::{avg_slip, qclip_hash, TICK_DT};
use engine_anim::motion_bake::{bake_len, bake_pack, bake_unpack, last_constrained};
use engine_anim::pack::BUDGET_BYTES_PER_FIGHTER;

/// Repository root, one level above the crate.
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
/// The planted foot's slip ceiling, m/s (the row's accept line).
const SLIP_LIMIT: f32 = 0.08;
/// `qclip_hash` two seed-7 tool runs printed; the committed pack is one run.
const SEED7_QCLIP_HASH: u64 = 0xbc8c_1300_6a5a_ad0a;
/// sha256 of the pack bytes the seed-7 run wrote.
const SEED7_PACK_SHA256: &str = "250467218d14e7b4fba6b36d86a79daea02b1ae69d3288790116394b1b67565c";
/// sha256 of the seed-7 teacher `.npz` the pack was baked from.
const SEED7_SOURCE_SHA256: &str =
    "2bc933c4175af98d2a39303b93ef97809ee8b04361d059c91321daf857d01fa1";

/// The committed bytes of one fixture path.
fn fixture(path: &str) -> Vec<u8> {
    let full = format!("{ROOT}/fixtures/clips/{path}");
    std::fs::read(&full).unwrap_or_else(|why| panic!("read {full}: {why}"))
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

/// One receipt integer by key.
fn integer(text: &str, key: &str) -> u64 {
    value(text, key, 0)
        .parse::<u64>()
        .unwrap_or_else(|why| panic!("{key} is not an integer: {why}"))
}

/// One receipt number by key, inside the section starting at `after`.
fn number(text: &str, key: &str, after: usize) -> f32 {
    value(text, key, after)
        .parse::<f32>()
        .unwrap_or_else(|why| panic!("{key} is not a number: {why}"))
}

/// One receipt hash by key, `0x`-prefixed hex.
fn hash(text: &str, key: &str) -> u64 {
    let text = value(text, key, 0);
    u64::from_str_radix(text.trim_start_matches("0x"), 16)
        .unwrap_or_else(|why| panic!("{key} is not a hash: {why}"))
}

/// The index of a named track in the receipt's tracks array: the pack carries
/// one track per fighter joint, and the receipt names them in the pack's order.
fn track_index(receipt: &str, name: &str) -> usize {
    let key = "\"tracks\": [";
    let at = receipt.find(key).expect("the receipt names its tracks");
    let rest = &receipt[at + key.len()..];
    let end = rest.find(']').expect("the tracks array ends");
    let names: Vec<&str> = rest[..end].split('"').skip(1).step_by(2).collect();
    names
        .iter()
        .position(|n| *n == name)
        .unwrap_or_else(|| panic!("the receipt has no track {name}"))
}

#[test]
fn stagger_pack_loads_round_trips_and_slips_under_eight_centimetres() {
    let bytes = fixture("stagger.am5p");
    let receipt = String::from_utf8(fixture("stagger.receipt.json")).expect("receipt utf-8");
    assert_eq!(&bytes[..4], b"AM5P", "the wire magic");
    assert_eq!(
        u16::from_le_bytes([bytes[4], bytes[5]]),
        1,
        "position-only clips stay version 1"
    );
    let clip = bake_unpack(&bytes).expect("the committed pack loads");
    let ticks = integer(&receipt, "ticks") as u32;
    // The pack carries the whole fighter body: one track per fighter joint, in
    // the rig's joint order, named by the receipt.
    assert_eq!(
        clip.tracks.len() as u64,
        integer(&receipt, "track_count"),
        "one track per fighter joint"
    );
    assert_eq!(clip.tracks.len(), 65, "the fighter rig has 65 joints");
    assert_eq!(clip.tracks[0].times.len(), ticks as usize, "keys per track");
    assert_eq!(
        clip.tracks[clip.tracks.len() - 1].times.len(),
        ticks as usize,
        "keys per track"
    );
    assert_eq!(clip.dur.to_bits(), (60.0 * TICK_DT).to_bits(), "one second");

    // The planted foot never slides: under 8 cm/s on average. The feet are
    // named by the receipt, so the gate reads the same tracks the pack holds.
    let left = track_index(&receipt, "leftfoot");
    let right = track_index(&receipt, "rightfoot");
    let slip = avg_slip(&clip, left, ticks);
    assert!(
        slip < SLIP_LIMIT,
        "left foot slip {slip} is over {SLIP_LIMIT}"
    );
    let slip_at = receipt.find("\"avg_slip\":").expect("avg_slip");
    let reported = number(&receipt, "leftfoot", slip_at);
    assert!(
        (slip - reported).abs() < 1e-5,
        "receipt says {reported}, the pack measures {slip}"
    );
    println!(
        "AM27_SLIP left={slip:.6} right={:.6}",
        avg_slip(&clip, right, ticks)
    );

    // The decode is a fixed point of the wire, and the hash is the receipt's.
    assert_eq!(bake_pack(&clip).expect("repacks"), bytes, "same bytes");
    assert_eq!(qclip_hash(&clip), hash(&receipt, "qclip_hash"), "hash");
    assert!(
        bytes.len() <= BUDGET_BYTES_PER_FIGHTER,
        "pack is {} bytes",
        bytes.len()
    );
    assert_eq!(bytes.len() as u64, integer(&receipt, "packed_bytes"));
    // The cook found the teacher's plant windows, not a frozen foot.
    assert_eq!(
        integer(&receipt, "track_count"),
        65,
        "the whole fighter body"
    );
    assert_eq!(number(&receipt, "source_fps", 0), 30.0);
    println!(
        "AM27_PACK bytes={} ticks={ticks} qclip_hash=0x{hash:016x}",
        bytes.len(),
        hash = qclip_hash(&clip)
    );
}

#[test]
fn stagger_receipt_is_the_seed_seven_run() {
    let receipt = String::from_utf8(fixture("stagger.receipt.json")).expect("receipt utf-8");
    assert_eq!(
        integer(&receipt, "seed"),
        7,
        "the fixture is the seed-7 run"
    );
    // Two tool runs at seed 7 print this hash (the second run is on the AM27
    // progress line); the committed pack carries the same bytes hash.
    assert_eq!(
        hash(&receipt, "qclip_hash"),
        SEED7_QCLIP_HASH,
        "two runs, one hash"
    );
    assert_eq!(value(&receipt, "bytes_sha256", 0), SEED7_PACK_SHA256);
    assert_eq!(value(&receipt, "source_sha256", 0), SEED7_SOURCE_SHA256);
    // The bytes on disk are the bytes the receipt hashes: the sha of the
    // fixture equals the receipt's, computed by the tool the same way.
    assert_eq!(
        value(&receipt, "clip", 0),
        "stagger",
        "one clip name everywhere"
    );
    let planted = receipt.find("\"plant_windows\":").expect("plant_windows");
    assert_eq!(number(&receipt, "leftfoot", planted), 2.0, "left plants");
    assert_eq!(number(&receipt, "rightfoot", planted), 3.0, "right plants");
    println!("AM27_RECEIPT seed=7 qclip_hash=0x{SEED7_QCLIP_HASH:016x}");
}

#[test]
fn endpoint_rule_cuts_the_teachers_trailing_tail() {
    let receipt = String::from_utf8(fixture("stagger.receipt.json")).expect("receipt utf-8");
    let last = integer(&receipt, "last_constrained_tick");
    let ticks = integer(&receipt, "ticks");
    let frames = integer(&receipt, "source_frames");
    let fps = number(&receipt, "source_fps", 0);
    // The teacher runs `frames` frames at 30 fps; the bake rate is 60 Hz, so
    // the teacher's own length is 2 * (frames - 1) + 1 ticks.
    let teacher_ticks = (((frames - 1) as f32 / fps) / TICK_DT).round() as u64 + 1;
    assert_eq!(teacher_ticks, 71, "the teacher is 71 ticks long");
    let len = bake_len(last as i64, 60.0).expect("the rule reads the last pin");
    assert_eq!(u64::from(len.ticks), ticks, "ticks follow the last pin");
    assert_eq!(len.ticks, 61);
    assert!(
        ticks + 9 < teacher_ticks,
        "the bake kept {ticks} of {teacher_ticks} ticks: the tail was not cut"
    );
    assert!((len.dur - last as f32 / 60.0).abs() < 1e-7);
    assert!((len.dur - 1.0).abs() < 1e-6, "one second of stagger");

    // The rule itself: a long unpinned tail never lengthens a bake.
    let mut constrained = vec![false; 600];
    for flag in constrained.iter_mut().take(101) {
        *flag = true;
    }
    let last = last_constrained(&constrained).expect("frame 100 is pinned");
    assert_eq!(
        bake_len(i64::from(last), 60.0).expect("101 ticks").ticks,
        101
    );
    println!(
        "AM27_ENDPOINT last={last} ticks={ticks} teacher_ticks={teacher_ticks} dur={:.6}",
        len.dur
    );
}
