// PORT idea: donmccurdy/glTF-Transform (MIT)
// https://github.com/donmccurdy/glTF-Transform/blob/main/packages/functions/src/dedup.ts
// Idea only: hash-group candidates, byte-compare to confirm, merge duplicates.
// Fresh implementation below; no donor code copied.

/// import dedup vectors: hash-group plus byte-compare merge on cooked bytes.
///
/// Owner: engine-assets import lane (`crates/assets/src/import.rs` read-only;
/// this file adds vectors only, no `src` edit). Units: blobs in, counts plus
/// byte totals out. Determinism: fixed literals only, no RNG, no clock, no IO.
/// Errors: none; mismatches fail via `assert`.
use engine_assets::registry::content_hash;
use std::collections::BTreeMap;

/// One dedup pass over property blobs (accessor/image byte runs).
struct Dedup {
    /// Unique blobs kept, first-seen order.
    unique: Vec<Vec<u8>>,
    /// Per-input index into `unique`.
    remap: Vec<usize>,
    /// Inputs removed (`inputs.len() - unique.len()`).
    merged: usize,
    /// Raw byte total before dedup.
    raw_len: usize,
    /// Cooked byte total after dedup.
    cooked_len: usize,
}

/// Merge duplicates: group by content hash, confirm with byte compare.
///
/// Hash groups avoid O(n^2) compares; equality still needs the full bytes so
/// a hash collision never merges distinct blobs. Stable: first-seen order.
fn dedup_blobs(blobs: &[Vec<u8>]) -> Dedup {
    let mut by_hash: BTreeMap<u64, Vec<usize>> = BTreeMap::new();
    let mut unique: Vec<Vec<u8>> = Vec::new();
    let mut remap: Vec<usize> = Vec::with_capacity(blobs.len());
    for blob in blobs {
        let hash = content_hash(blob);
        let mut found: Option<usize> = None;
        if let Some(candidates) = by_hash.get(&hash) {
            for &slot in candidates {
                if unique[slot] == *blob {
                    found = Some(slot);
                    break;
                }
            }
        }
        match found {
            Some(slot) => remap.push(slot),
            None => {
                let slot = unique.len();
                unique.push(blob.clone());
                by_hash.entry(hash).or_default().push(slot);
                remap.push(slot);
            }
        }
    }
    let raw_len: usize = blobs.iter().map(Vec::len).sum();
    let cooked_len: usize = unique.iter().map(Vec::len).sum();
    let merged = blobs.len() - unique.len();
    Dedup {
        unique,
        remap,
        merged,
        raw_len,
        cooked_len,
    }
}

/// Deterministic 256-byte tile: same seed means same bytes.
fn tile_bytes(seed: u8) -> Vec<u8> {
    (0..256)
        .map(|i| seed.wrapping_add((i as u8).wrapping_mul(31)))
        .collect()
}

/// Dupe-seeded arena-kit fixture: floor plus wall tiles repeat, one
/// same-length near-dupe proves the byte-compare guard.
fn arena_kit_blobs() -> Vec<Vec<u8>> {
    let floor = tile_bytes(11);
    let wall = tile_bytes(23);
    let mut near_wall = wall.clone();
    let last = near_wall.len() - 1;
    near_wall[last] ^= 0x01;
    vec![
        floor.clone(),
        wall.clone(),
        floor,
        tile_bytes(37),
        wall,
        near_wall,
    ]
}

#[test]
fn dupe_seeded_arena_kit_merges_duplicates() {
    let blobs = arena_kit_blobs();
    assert_eq!(blobs.len(), 6, "arena-kit fixture carries 6 blobs");
    assert_eq!(blobs[0], blobs[2], "floor tile repeats");
    assert_eq!(blobs[1], blobs[4], "wall tile repeats");
    assert_ne!(blobs[1], blobs[5], "near-dupe differs by one byte");
    let out = dedup_blobs(&blobs);
    assert_eq!(out.merged, 2, "two duplicate blobs merge");
    assert_eq!(out.unique.len(), 4, "four unique blobs remain");
    assert_eq!(out.remap[2], out.remap[0], "floor remaps to first copy");
    assert_eq!(out.remap[4], out.remap[1], "wall remaps to first copy");
    assert_ne!(
        out.remap[5], out.remap[1],
        "same length is not enough: bytes must match"
    );
    for (i, blob) in blobs.iter().enumerate() {
        assert_eq!(&out.unique[out.remap[i]], blob, "remap round-trips {i}");
    }
    let again = dedup_blobs(&blobs);
    assert_eq!(
        (again.merged, again.cooked_len, again.remap),
        (out.merged, out.cooked_len, out.remap),
        "same fixture dedups the same way twice"
    );
}

#[test]
fn merged_duplicate_count_goes_zero_to_many() {
    let clean = vec![tile_bytes(11), tile_bytes(23), tile_bytes(37)];
    let none = dedup_blobs(&clean);
    assert_eq!(none.merged, 0, "distinct blobs merge nothing");
    assert_eq!(none.unique.len(), 3, "distinct blobs all survive");
    let duped = arena_kit_blobs();
    let some = dedup_blobs(&duped);
    assert!(
        some.merged >= 1,
        "dupe-seeded fixture merges at least one, got {}",
        some.merged
    );
    assert_eq!(some.merged, 2, "fixture merges exactly two dupes");
    assert!(some.unique.len() < duped.len(), "unique count drops");
}

#[test]
fn cooked_bytes_strictly_down() {
    let blobs = arena_kit_blobs();
    let out = dedup_blobs(&blobs);
    assert_eq!(out.raw_len, 1536, "6 tiles x 256 bytes raw");
    assert_eq!(out.cooked_len, 1024, "4 unique tiles x 256 bytes cooked");
    assert!(
        out.cooked_len < out.raw_len,
        "cooked {} must sit below raw {}",
        out.cooked_len,
        out.raw_len
    );
    assert_eq!(
        out.raw_len - out.cooked_len,
        512,
        "two merged tiles save 512 bytes"
    );
}
