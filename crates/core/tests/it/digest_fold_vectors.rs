//! Digest fold vectors: the `engine_core::digest::TwinFold` a caller builds
//! must fold two word streams into the u64 the module documents.
//! Owner: `hash::Fnv64` owns the mixing, `digest` owns the twin fold.
//! Consumer: the RC9/RC22 incremental-digest experiment, which reads
//! `TwinFold::finish` beside the full rehash.
//! Units: one little-endian u32 per `push_*_u32` call, u64 hash out.
//! Determinism: integer compares only, no clock, no IO, no network.
//! Errors: panics via asserts.
//!
//! Why this file exists: `crates/core/src/digest.rs` changed 4 times in 30 days
//! yet holds zero `#[test]`, and no module of this folder names it. Every value
//! below is a published FNV-1a 64 vector for the same little-endian bytes, and
//! every structural claim is recomputed from `Fnv64` directly, so a changed
//! combine, a swapped stream, a reordered push or a dropped prime multiply
//! fails here instead of silently seeding a stale digest.

use engine_core::digest::TwinFold;
use engine_core::hash::{Fnv64, FNV64_PRIME};

/// FNV-1a 64 over the words' little-endian bytes, mixed by the hash owner.
fn fnv_words(words: &[u32]) -> u64 {
    let mut hasher = Fnv64::new();
    for word in words {
        hasher.write_u32(*word);
    }
    hasher.finish()
}

/// The combine the module documents: `A ^ (B * prime)` modulo 2^64.
fn expected(a: &[u32], b: &[u32]) -> u64 {
    fnv_words(a) ^ fnv_words(b).wrapping_mul(FNV64_PRIME)
}

/// Fold two word streams the way a caller does.
fn fold(a: &[u32], b: &[u32]) -> u64 {
    let mut twin = TwinFold::new();
    for word in a {
        twin.push_a_u32(*word);
    }
    for word in b {
        twin.push_b_u32(*word);
    }
    twin.finish()
}

#[test]
fn empty_fold_is_the_two_offset_basis_streams() {
    // Published FNV-1a 64 of no bytes, combined by the documented rule.
    assert_eq!(TwinFold::new().finish(), 0x6491_21a8_0223_94fa);
    assert_eq!(TwinFold::new().finish(), expected(&[], &[]));
    assert_eq!(
        TwinFold::default().finish(),
        TwinFold::new().finish(),
        "the derived default must be the fresh fold"
    );
}

#[test]
fn pushed_words_keep_their_stream_and_index_order() {
    let a = [0x0102_0304, 0xffee_ddcc];
    let b = [0x1111_2222];
    assert_eq!(fold(&a, &b), 0x6a7b_bded_2cf7_5184);
    assert_eq!(fold(&a, &b), expected(&a, &b));
    // Stream tag matters: the same words on the other stream are another value.
    assert_eq!(fold(&b, &a), 0xc72e_60b6_da6b_57f4);
    assert_ne!(fold(&a, &b), fold(&b, &a));
    // Index order matters inside one stream.
    assert_ne!(fold(&[a[1], a[0]], &b), fold(&a, &b));
}

#[test]
fn one_word_on_both_streams_still_splits() {
    // 0xdeadbeef on both streams is not the same as either single-stream fold.
    let both = fold(&[0xdead_beef], &[0xdead_beef]);
    assert_eq!(both, 0x2582_3280_fdde_113a);
    assert_eq!(fold(&[], &[0xdead_beef]), 0x4a3e_8384_08ac_c634);
    assert_ne!(both, fold(&[0xdead_beef], &[]));
    assert_ne!(both, fold(&[], &[0xdead_beef]));
}

#[test]
fn the_prime_multiply_is_load_bearing() {
    let a = [0x0102_0304, 0xffee_ddcc];
    let b = [0x1111_2222];
    let naive = fnv_words(&a) ^ fnv_words(&b);
    // The obvious wrong simplification (plain xor, no prime) must not match.
    assert_ne!(fold(&a, &b), naive, "stream B must be prime-multiplied");
    assert_eq!(
        fold(&a, &b),
        fnv_words(&a) ^ fnv_words(&b).wrapping_mul(FNV64_PRIME)
    );
}

#[test]
fn the_same_words_repeat_bit_for_bit() {
    let a = [7, 11, 13];
    let b = [17, 19];
    let first = fold(&a, &b);
    let mut second = TwinFold::new();
    for word in a {
        second.push_a_u32(word);
    }
    for word in b {
        second.push_b_u32(word);
    }
    assert_eq!(first, second.finish(), "no state crosses two folds");
    assert_eq!(fold(&a, &b), first, "a fresh fold replays the same value");
}
