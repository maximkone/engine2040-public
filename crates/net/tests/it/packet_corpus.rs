// Donor: rustls/rustls fuzz layout (fuzz corpus + decode-no-panic pattern), Apache-2.0.
// Link: https://github.com/rustls/rustls (fuzz/ layout). Pattern only, no donor bytes.
// Donor: camshaft/bolero@7642943b6c1f8a9b5e824dd1cd29b7c8e56068b8 (lib/bolero-engine/src/test.rs, lib/bolero-engine/src/shrink.rs, book corpus-replay + input-shrinking), MIT.
// Link: https://github.com/camshaft/bolero (LICENSE read live 2026-10-09, MIT) + https://raw.githubusercontent.com/camshaft/bolero/7642943b6c1f8a9b5e824dd1cd29b7c8e56068b8/LICENSE
// Taken: byte-input Test trait for FnMut(&[u8]), shrink transforms (truncation-end, remove-chunk, byte-shrink), corpus export as hash-named entries. Left out: sort transform (packet order matters), generator framework, wall-clock budget (bounded passes keep tests deterministic). No donor bytes pasted; reimplemented in this repo style.
//! Checked-in fuzz-corpus replay for net wire packets: valid seeds decode,
//! truncated and garbage seeds reject with named errors, decoders never panic,
//! and every seed replays deterministically.

use engine_core::{hash_bytes, Rng};
use engine_net::session::{decode_input, decode_state, NetError};

/// Inline fuzz corpus: valid, truncated and garbage packet byte-strings.
mod corpus {
    /// Valid client input packet: magic, v1, kind 0, tick 7, one input.
    pub const VALID_INPUT: &[u8] = &[
        0x4E, 0x31, 0x01, 0x00, 0x07, 0x00, 0x00, 0x00, 0x01, 0x07, 0x00, 0x00, 0x00, 0x00, 0x64,
        0x00, 0x28, 0x23, 0x00,
    ];
    /// Valid server state packet: magic, v1, kind 1, tick 10, hash, one input.
    pub const VALID_STATE: &[u8] = &[
        0x4E, 0x31, 0x01, 0x01, 0x0A, 0x00, 0x00, 0x00, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22,
        0x11, 0x01, 0x0A, 0x00, 0x00, 0x00, 0x01, 0x9C, 0x32, 0x00, 0x00, 0x01,
    ];
    /// Valid input cut mid-field.
    pub const TRUNC_INPUT: &[u8] = &[
        0x4E, 0x31, 0x01, 0x00, 0x07, 0x00, 0x00, 0x00, 0x01, 0x07, 0x00, 0x00,
    ];
    /// Valid state cut mid-header.
    pub const TRUNC_STATE: &[u8] = &[0x4E, 0x31, 0x01, 0x01, 0x0A, 0x00, 0x00, 0x00, 0x88, 0x77];
    /// Empty datagram.
    pub const EMPTY: &[u8] = &[];
    /// Input-shaped datagram with bad magic.
    pub const BAD_MAGIC: &[u8] = &[0x00, 0x00, 0x01, 0x00, 0x07, 0x00, 0x00, 0x00, 0x01];
    /// State-shaped datagram with bad magic (full valid-state length).
    pub const BAD_MAGIC_STATE: &[u8] = &[
        0x00, 0x00, 0x01, 0x01, 0x0A, 0x00, 0x00, 0x00, 0x88, 0x77, 0x66, 0x55, 0x44, 0x33, 0x22,
        0x11, 0x01, 0x0A, 0x00, 0x00, 0x00, 0x01, 0x9C, 0x32, 0x00, 0x00, 0x01,
    ];
    /// Valid input with version byte 99.
    pub const BAD_VERSION: &[u8] = &[
        0x4E, 0x31, 0x63, 0x00, 0x07, 0x00, 0x00, 0x00, 0x01, 0x07, 0x00, 0x00, 0x00, 0x00, 0x64,
        0x00, 0x28, 0x23, 0x00,
    ];
    /// Valid input with unknown kind 0x7F.
    pub const BAD_KIND: &[u8] = &[
        0x4E, 0x31, 0x01, 0x7F, 0x07, 0x00, 0x00, 0x00, 0x01, 0x07, 0x00, 0x00, 0x00, 0x00, 0x64,
        0x00, 0x28, 0x23, 0x00,
    ];
    /// Input header claiming 33 inputs (over the 32-input bound).
    pub const TOO_MANY: &[u8] = &[0x4E, 0x31, 0x01, 0x00, 0x07, 0x00, 0x00, 0x00, 0x21];

    /// Every seed in this corpus.
    pub const SEEDS: &[&[u8]] = &[
        VALID_INPUT,
        VALID_STATE,
        TRUNC_INPUT,
        TRUNC_STATE,
        EMPTY,
        BAD_MAGIC,
        BAD_MAGIC_STATE,
        BAD_VERSION,
        BAD_KIND,
        TOO_MANY,
    ];
}

#[test]
fn valid_input_seed_decodes() {
    let packet = decode_input(corpus::VALID_INPUT).expect("valid input must decode");
    assert_eq!(packet.tick, 7);
    assert_eq!(packet.inputs.len(), 1);
    let input = packet.inputs[0];
    assert_eq!(
        (input.tick, input.fighter.0, input.mx, input.mz),
        (7, 0, 100, 0)
    );
    assert_eq!(input.facing, 9000);
}

#[test]
fn valid_state_seed_decodes() {
    let packet = decode_state(corpus::VALID_STATE).expect("valid state must decode");
    assert_eq!(packet.checkpoint.tick, 10);
    assert_eq!(packet.checkpoint.hash, 0x1122_3344_5566_7788);
    assert_eq!(packet.inputs.len(), 1);
    let input = packet.inputs[0];
    assert_eq!(
        (input.tick, input.fighter.0, input.mx, input.mz),
        (10, 1, -100, 50)
    );
    assert!(packet.delta.is_none());
}

#[test]
fn truncated_seeds_reject_with_truncated() {
    assert_eq!(decode_input(corpus::TRUNC_INPUT), Err(NetError::Truncated));
    assert_eq!(decode_state(corpus::TRUNC_STATE), Err(NetError::Truncated));
    assert_eq!(decode_input(corpus::EMPTY), Err(NetError::Truncated));
    assert_eq!(decode_state(corpus::EMPTY), Err(NetError::Truncated));
    let mut trailing = corpus::VALID_INPUT.to_vec();
    trailing.push(0x00);
    assert_eq!(decode_input(&trailing), Err(NetError::Truncated));
    let mut trailing_state = corpus::VALID_STATE.to_vec();
    trailing_state.push(0x00);
    assert_eq!(decode_state(&trailing_state), Err(NetError::Truncated));
}

#[test]
fn garbage_seeds_reject_with_named_errors() {
    assert_eq!(decode_input(corpus::BAD_MAGIC), Err(NetError::BadMagic));
    assert_eq!(
        decode_state(corpus::BAD_MAGIC_STATE),
        Err(NetError::BadMagic)
    );
    assert_eq!(
        decode_input(corpus::BAD_VERSION),
        Err(NetError::VersionMismatch { got: 99 })
    );
    assert_eq!(decode_input(corpus::BAD_KIND), Err(NetError::BadKind(0x7F)));
    assert_eq!(
        decode_input(corpus::TOO_MANY),
        Err(NetError::TooManyInputs(33))
    );
    assert_eq!(decode_state(corpus::VALID_INPUT), Err(NetError::BadKind(0)));
    assert_eq!(decode_input(corpus::VALID_STATE), Err(NetError::BadKind(1)));
}

#[test]
fn corpus_never_panics_and_rejects_loudly() {
    let mut ok = 0usize;
    let mut rejected = 0usize;
    for seed in corpus::SEEDS {
        let input_result = decode_input(seed);
        let state_result = decode_state(seed);
        assert!(input_result.is_ok() || input_result.is_err());
        assert!(state_result.is_ok() || state_result.is_err());
        if input_result.is_ok() || state_result.is_ok() {
            ok += 1;
        } else {
            rejected += 1;
        }
        for error in input_result.err().into_iter().chain(state_result.err()) {
            assert!(!error.to_string().is_empty());
        }
    }
    assert_eq!(ok, 2);
    assert_eq!(rejected, corpus::SEEDS.len() - 2);
    println!(
        "packet_corpus passed: {} seeds replayed ({ok} valid, {rejected} rejected)",
        corpus::SEEDS.len()
    );
}

#[test]
fn decode_is_deterministic_x2() {
    for seed in corpus::SEEDS {
        assert_eq!(
            decode_input(seed),
            decode_input(seed),
            "input decode must replay identically"
        );
        assert_eq!(
            decode_state(seed),
            decode_state(seed),
            "state decode must replay identically"
        );
    }
    let first_input = decode_input(corpus::VALID_INPUT).expect("valid input");
    let second_input = decode_input(corpus::VALID_INPUT).expect("valid input again");
    assert_eq!(first_input, second_input);
    let first_state = decode_state(corpus::VALID_STATE).expect("valid state");
    let second_state = decode_state(corpus::VALID_STATE).expect("valid state again");
    assert_eq!(first_state, second_state);
}

/// Byte-input test case: smallest bolero `Test` shape for `FnMut(&[u8])`.
/// `run` answers true while the candidate still shows the original outcome,
/// so the shrink driver keeps only outcome-preserving simplifications.
trait ByteTest {
    type Value: core::fmt::Debug;
    fn run(&mut self, input: &[u8]) -> bool;
    fn debug_value(&self, input: &[u8]) -> Self::Value;
}

impl<F> ByteTest for F
where
    F: FnMut(&[u8]) -> bool,
{
    type Value = Vec<u8>;
    fn run(&mut self, input: &[u8]) -> bool {
        (self)(input)
    }
    fn debug_value(&self, input: &[u8]) -> Self::Value {
        input.to_vec()
    }
}

/// One random byte from the engine-owned stream.
fn rng_byte(rng: &mut Rng) -> u8 {
    u8::try_from(rng.next_u64() % 256).expect("byte range fits in u8")
}

/// Deterministic 256-input driver corpus: each entry mutates one of the ten
/// checked-in seeds with a per-index fork, so order never matters and the
/// same seed always yields the same bytes.
fn generate_inputs() -> Vec<Vec<u8>> {
    const COUNT: u64 = 256;
    const SEED_COUNT: u64 = 10;
    let mut out = Vec::with_capacity(256);
    for index in 0..COUNT {
        let mut rng = Rng::new(2040).fork(index);
        let seed_at = usize::try_from(index % SEED_COUNT).expect("seed index fits");
        let mut bytes = corpus::SEEDS[seed_at].to_vec();
        let flips = usize::try_from(rng.next_u64() % 4).expect("flip count fits");
        for _ in 0..flips {
            if bytes.is_empty() {
                break;
            }
            let len_u64 = u64::try_from(bytes.len()).expect("packet len fits");
            let at = usize::try_from(rng.next_u64() % len_u64).expect("flip index fits");
            let delta = rng_byte(&mut rng);
            bytes[at] ^= delta;
        }
        let extra = usize::try_from(rng.next_u64() % 24).expect("extra len fits");
        for _ in 0..extra {
            bytes.push(rng_byte(&mut rng));
        }
        if bytes.len() > 900 {
            bytes.truncate(900);
        }
        out.push(bytes);
    }
    out
}

/// Donor truncation-end transform: keep the shortest prefix that still fails.
fn apply_truncation<T: ByteTest>(test: &mut T, bytes: &mut Vec<u8>) -> bool {
    let prev = bytes.len();
    for end in 0..prev {
        if test.run(&bytes[..end]) {
            bytes.truncate(end);
            return true;
        }
    }
    false
}

/// Donor remove-chunk transform: drop the largest span at `index` that still
/// fails. Index zero stays put since most packet headers need the first byte.
fn apply_remove_chunk<T: ByteTest>(test: &mut T, bytes: &mut Vec<u8>, index: usize) -> bool {
    if index == 0 {
        return false;
    }
    if bytes.len() < index + 2 {
        return false;
    }
    let tail_len = bytes.len() - index;
    for remove in (1..tail_len).rev() {
        let mut candidate = Vec::with_capacity(bytes.len() - remove);
        candidate.extend_from_slice(&bytes[..index]);
        candidate.extend_from_slice(&bytes[index + remove..]);
        if test.run(&candidate) {
            bytes.truncate(index);
            bytes.extend_from_slice(&candidate[index..]);
            return true;
        }
    }
    false
}

/// Donor byte-shrink transform: lower one byte to the smallest value that
/// still fails.
fn apply_byte_shrink<T: ByteTest>(test: &mut T, bytes: &mut Vec<u8>, index: usize) -> bool {
    if index >= bytes.len() {
        return false;
    }
    let prev = bytes[index];
    for candidate in 0..prev {
        bytes[index] = candidate;
        if test.run(bytes.as_slice()) {
            return true;
        }
    }
    bytes[index] = prev;
    false
}

/// Donor shrink driver: truncation plus per-index chunk and byte passes.
/// Bounded passes keep the test deterministic with no wall clock.
fn shrink_bytes<T: ByteTest>(test: &mut T, mut bytes: Vec<u8>) -> Vec<u8> {
    for _ in 0..4 {
        let mut changed = false;
        if apply_truncation(test, &mut bytes) {
            changed = true;
        }
        if bytes.is_empty() {
            break;
        }
        let mut index = 0;
        let mut restarted = false;
        while index < bytes.len() {
            if apply_remove_chunk(test, &mut bytes, index) {
                changed = true;
                restarted = true;
                break;
            }
            if apply_byte_shrink(test, &mut bytes, index) {
                changed = true;
            }
            index += 1;
        }
        if restarted {
            continue;
        }
        if !changed {
            break;
        }
    }
    bytes
}

/// Corpus export: dedup, sort for determinism, then name each entry by its
/// engine hash, mirroring the donor corpus file layout without touching disk.
fn export_corpus(minimal: &[Vec<u8>]) -> Vec<(String, Vec<u8>)> {
    let mut sorted = minimal.to_vec();
    sorted.sort();
    sorted.dedup();
    let mut out = Vec::with_capacity(sorted.len());
    for bytes in sorted {
        let name = format!("{:016x}", hash_bytes(&bytes));
        out.push((name, bytes));
    }
    out
}

#[test]
fn shrink_driver_minimizes_256_inputs() {
    let inputs = generate_inputs();
    assert_eq!(inputs.len(), 256);
    let mut orig_total = 0usize;
    let mut shrunk_total = 0usize;
    let mut strictly = 0usize;
    let mut minimals: Vec<Vec<u8>> = Vec::with_capacity(inputs.len());
    for bytes in &inputs {
        let original = decode_input(bytes);
        orig_total += bytes.len();
        let mut check = |candidate: &[u8]| decode_input(candidate) == original;
        let shrunk = shrink_bytes(&mut check, bytes.clone());
        assert_eq!(
            decode_input(&shrunk),
            original,
            "shrink must preserve the decode outcome"
        );
        assert!(
            shrunk.len() <= bytes.len(),
            "shrink must never grow the input"
        );
        if shrunk.len() < bytes.len() {
            strictly += 1;
        }
        // Truncation-minimal: no shorter prefix keeps the outcome, otherwise
        // a no-op shrink would still pass the length check above.
        for end in 0..shrunk.len() {
            assert!(
                decode_input(&shrunk[..end]) != original,
                "shrunk input must be prefix-minimal"
            );
        }
        shrunk_total += shrunk.len();
        let _ = check.debug_value(&shrunk);
        minimals.push(shrunk);
    }
    assert!(
        strictly >= 32,
        "expected many strictly-shrunk inputs, got {strictly}"
    );
    assert!(
        shrunk_total < orig_total,
        "shrunk bytes {shrunk_total} must be under original {orig_total}"
    );
    let exported = export_corpus(&minimals);
    assert!(!exported.is_empty());
    println!(
        "packet_corpus shrink: 256 inputs shrunk to minimal (orig {orig_total}B -> shrunk {shrunk_total}B, {strictly} strictly smaller, {} unique corpus entries)",
        exported.len()
    );
}

#[test]
fn corpus_export_roundtrips_deterministically() {
    let inputs = generate_inputs();
    let mut minimals = Vec::with_capacity(inputs.len());
    for bytes in &inputs {
        let original = decode_input(bytes);
        let mut check = |candidate: &[u8]| decode_input(candidate) == original;
        minimals.push(shrink_bytes(&mut check, bytes.clone()));
    }
    let first = export_corpus(&minimals);
    let second = export_corpus(&minimals);
    assert_eq!(first, second, "corpus export must replay identically");
    let dir = std::env::temp_dir().join("engine2040-packet-corpus");
    std::fs::create_dir_all(&dir).expect("corpus dir creates");
    for (name, bytes) in &first {
        let path = dir.join(name);
        std::fs::write(&path, bytes).expect("corpus entry writes");
        let back = std::fs::read(&path).expect("corpus entry reads back");
        assert_eq!(&back, bytes, "corpus entry must roundtrip bytes");
        // Re-decoding the exported bytes must give the same outcome.
        let original = decode_input(bytes);
        assert_eq!(decode_input(&back), original);
    }
    println!(
        "packet_corpus export: {} corpus entries roundtripped via {}",
        first.len(),
        dir.display()
    );
}
