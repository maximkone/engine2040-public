//! deterministic_order_vectors: shuffled contact pairs keep island bytes identical.
//!
//! PORT(mrdav30/Gravitas@MIT:https://github.com/mrdav30/Gravitas:Determinism
//! idea-only, fresh Rust): Gravitas keeps one deterministic order over contacts
//! and islands; this pins the same shape here over canonical islands.
//! Owner: engine-physics islands. Units: body ids, LE bytes. Determinism:
//! same pairs any order give same islands, bytes and hash. Errors: none.

use engine_core::hash::hash_bytes;
use engine_physics::islands::{build_islands, combined_hash, island_hash};

const BODY_COUNT: usize = 8;

/// Two contact groups plus one singleton: chain 0-3, pair 5-7, body 4 alone.
fn base_pairs() -> Vec<(usize, usize)> {
    vec![(0, 1), (1, 2), (2, 3), (5, 6), (6, 7)]
}

fn body_hashes() -> Vec<u64> {
    vec![11, 22, 33, 44, 55, 66, 77, 88]
}

/// Deterministic Fisher-Yates over pairs with the shared Rng owner.
fn shuffled(mut pairs: Vec<(usize, usize)>, seed: u64) -> Vec<(usize, usize)> {
    let mut rng = engine_core::Rng::new(seed);
    let len = pairs.len();
    if len < 2 {
        return pairs;
    }
    for i in (1..len).rev() {
        let bound = u64::try_from(i + 1).unwrap_or(u64::MAX);
        let pick = rng.next_range(0, bound);
        let j = usize::try_from(pick).unwrap_or(0);
        pairs.swap(i, j);
    }
    pairs
}

/// Canonical island bytes: count, per-island len, member ids, island hash.
/// All words are little-endian so shuffled input gives identical bytes.
fn island_bytes(n: usize, pairs: &[(usize, usize)], body_hash: &[u64]) -> Vec<u8> {
    let islands = build_islands(n, pairs);
    let mut out = Vec::new();
    out.extend_from_slice(
        &u64::try_from(islands.len())
            .unwrap_or(u64::MAX)
            .to_le_bytes(),
    );
    for members in &islands {
        out.extend_from_slice(
            &u64::try_from(members.len())
                .unwrap_or(u64::MAX)
                .to_le_bytes(),
        );
        for m in members {
            out.extend_from_slice(&u64::try_from(*m).unwrap_or(u64::MAX).to_le_bytes());
        }
        out.extend_from_slice(&island_hash(body_hash, members).to_le_bytes());
    }
    out
}

#[test]
fn shuffled_contact_order_keeps_island_bytes_identical_x2() {
    let base = base_pairs();
    let hashes = body_hashes();
    let want_islands = build_islands(BODY_COUNT, &base);
    assert_eq!(
        want_islands,
        vec![vec![0, 1, 2, 3], vec![4], vec![5, 6, 7]],
        "fixture topology must stay two groups plus one singleton"
    );
    let want_bytes = island_bytes(BODY_COUNT, &base, &hashes);
    let want_hash = combined_hash(BODY_COUNT, &base, &hashes);
    assert!(!want_bytes.is_empty(), "island bytes must encode topology");
    for seed in [2040u64, 7, 99, 12_345] {
        let shuffled_pairs = shuffled(base.clone(), seed);
        assert_ne!(
            shuffled_pairs, base,
            "seed must actually reorder the fixture"
        );
        let islands = build_islands(BODY_COUNT, &shuffled_pairs);
        assert_eq!(islands, want_islands, "shuffled order must keep islands");
        let bytes = island_bytes(BODY_COUNT, &shuffled_pairs, &hashes);
        assert_eq!(bytes, want_bytes, "shuffled order must keep island bytes");
        assert_eq!(
            bytes,
            island_bytes(BODY_COUNT, &shuffled_pairs, &hashes),
            "same shuffled input must encode same bytes x2"
        );
        assert_eq!(
            combined_hash(BODY_COUNT, &shuffled_pairs, &hashes),
            want_hash,
            "shuffled order must keep the combined hash"
        );
        assert_eq!(
            hash_bytes(&bytes),
            hash_bytes(&want_bytes),
            "byte digest must match x2"
        );
    }
}

#[test]
fn shuffled_noisy_pairs_match_clean_and_split_moves_bytes() {
    let base = base_pairs();
    let hashes = body_hashes();
    let want_bytes = island_bytes(BODY_COUNT, &base, &hashes);
    let want_hash = combined_hash(BODY_COUNT, &base, &hashes);
    // Reversed ends, duplicates, self pairs and out-of-range pairs are noise
    // the canonical builder ignores; shuffling them must still match clean.
    let mut noisy = base.clone();
    noisy.extend_from_slice(&[(3, 2), (1, 0), (0, 1), (4, 4), (9, 9), (7, 6)]);
    let noisy_shuffled = shuffled(noisy, 2040);
    assert_eq!(
        build_islands(BODY_COUNT, &noisy_shuffled),
        build_islands(BODY_COUNT, &base),
        "noise must not move islands"
    );
    assert_eq!(
        island_bytes(BODY_COUNT, &noisy_shuffled, &hashes),
        want_bytes,
        "noise must not move island bytes"
    );
    assert_eq!(
        island_bytes(BODY_COUNT, &noisy_shuffled, &hashes),
        island_bytes(BODY_COUNT, &noisy_shuffled, &hashes),
        "noisy bytes must be stable x2"
    );
    // Negative cases: a bridge pair merges the groups, a body edit moves the
    // per-island hash; both must move bytes so the test catches regressions.
    let mut bridged = base.clone();
    bridged.push((3, 5));
    let bridged_bytes = island_bytes(BODY_COUNT, &bridged, &hashes);
    assert_ne!(
        bridged_bytes, want_bytes,
        "merged islands must move island bytes"
    );
    assert_ne!(
        combined_hash(BODY_COUNT, &bridged, &hashes),
        want_hash,
        "merged islands must move the combined hash"
    );
    let mut moved_hashes = hashes.clone();
    moved_hashes[0] = 12;
    assert_ne!(
        island_bytes(BODY_COUNT, &base, &moved_hashes),
        want_bytes,
        "body change must move island bytes"
    );
    assert_ne!(
        combined_hash(BODY_COUNT, &base, &moved_hashes),
        want_hash,
        "body change must move the combined hash"
    );
}
