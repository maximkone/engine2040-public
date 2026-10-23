//! Kernel: replay tape decode in `engine_proof::tape`.
//!
//! The envelope is 28 header bytes plus 16 bytes per input and per
//! checkpoint; `MAX_TAPE_BYTES` bounds the file before allocation. These
//! harnesses fix the magic and version so CBMC reasons about counts,
//! lengths and record bits, not symbolic floats.

use crate::tape::FORMAT_VERSION;
use crate::{Checkpoint, Config, Tape, FIXTURE_VERSION};

const HEADER: usize = 28;
const MAGIC: &[u8; 8] = b"E2040RP\0";

fn header(ticks: u32, count: u32) -> [u8; HEADER] {
    let mut bytes = [0u8; HEADER];
    bytes[..8].copy_from_slice(MAGIC);
    bytes[8..10].copy_from_slice(&FORMAT_VERSION.to_le_bytes());
    bytes[10..12].copy_from_slice(&FIXTURE_VERSION.to_le_bytes());
    bytes[20..24].copy_from_slice(&ticks.to_le_bytes());
    bytes[24..28].copy_from_slice(&count.to_le_bytes());
    bytes
}

/// Kernel: `Tape::decode` length gate.
/// Bound: every buffer shorter than the 28-byte header (0 and 27 bytes).
#[kani::proof]
pub fn tape_decode_rejects_short_buffer() {
    assert!(Tape::decode(&[]).is_err());
    assert!(Tape::decode(&[0u8; 27]).is_err());
}

/// Kernel: `Tape::decode` magic and version gates.
/// Bound: a full 60-byte buffer with bad magic, then bad format version.
#[kani::proof]
pub fn tape_decode_rejects_bad_magic_and_version() {
    assert!(Tape::decode(&[0u8; 60]).is_err());
    let mut bad = [0u8; 60];
    bad[..8].copy_from_slice(MAGIC);
    bad[8..10].copy_from_slice(&(FORMAT_VERSION + 1).to_le_bytes());
    bad[10..12].copy_from_slice(&FIXTURE_VERSION.to_le_bytes());
    bad[20..24].copy_from_slice(&2u32.to_le_bytes());
    assert!(Tape::decode(&bad).is_err());
}

/// Kernel: `Tape::decode` count/length agreement.
/// Bound: a 60-byte buffer with two records of room but counts 0 and 2
/// around a fixed tick count of 1.
#[kani::proof]
pub fn tape_decode_rejects_count_length_mismatch() {
    let mut short = header(1, 0);
    let seed: u64 = kani::any();
    short[12..20].copy_from_slice(&seed.to_le_bytes());
    assert!(Tape::decode(&short).is_err());

    let mut long = [0u8; 60];
    long[..HEADER].copy_from_slice(&header(1, 2));
    let seed: u64 = kani::any();
    long[12..20].copy_from_slice(&seed.to_le_bytes());
    assert!(Tape::decode(&long).is_err());
}

/// Kernel: checkpoint decode and byte-exact re-encode.
/// Bound: 60 bytes, seed and both checkpoint hashes symbolic, ticks 2,
/// inputs 0.
#[kani::proof]
pub fn tape_decode_two_checkpoints_roundtrips() {
    let mut bytes = [0u8; 60];
    bytes[..HEADER].copy_from_slice(&header(2, 0));
    let seed: u64 = kani::any();
    let legacy: u64 = kani::any();
    let state: u64 = kani::any();
    bytes[12..20].copy_from_slice(&seed.to_le_bytes());
    bytes[28..36].copy_from_slice(&legacy.to_le_bytes());
    bytes[36..44].copy_from_slice(&state.to_le_bytes());
    let second_legacy: u64 = kani::any();
    let second_state: u64 = kani::any();
    bytes[44..52].copy_from_slice(&second_legacy.to_le_bytes());
    bytes[52..60].copy_from_slice(&second_state.to_le_bytes());
    let Some(tape) = Tape::decode(&bytes).ok() else {
        assert!(false, "fixed header and counts are valid");
        return;
    };
    assert_eq!(tape.config, Config { seed, ticks: 2 });
    assert_eq!(tape.checkpoints.len(), 2);
    let Some(reencoded) = tape.encode().ok() else {
        assert!(false, "decoded tape re-encodes");
        return;
    };
    assert_eq!(reencoded.as_slice(), bytes.as_slice());
}

/// Kernel: `Tape::encode` / `Tape::decode` one-tick roundtrip.
/// Bound: one tick, no inputs, every seed and checkpoint hash pair.
#[kani::proof]
pub fn tape_encode_one_tick_roundtrips() {
    let tape = Tape {
        config: Config {
            seed: kani::any(),
            ticks: 1,
        },
        inputs: vec![],
        checkpoints: vec![Checkpoint {
            legacy: kani::any(),
            state: kani::any(),
        }],
    };
    let Some(bytes) = tape.encode().ok() else {
        assert!(false, "one-tick tape is valid");
        return;
    };
    let Some(back) = Tape::decode(&bytes).ok() else {
        assert!(false, "own tape decodes");
        return;
    };
    assert_eq!(back, tape);
    let Some(again) = back.encode().ok() else {
        assert!(false, "decoded tape re-encodes");
        return;
    };
    assert_eq!(again, bytes);
}
