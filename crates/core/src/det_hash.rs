//! det_hash: the deterministic Fx-shape hasher for entity-id keys.
//! PORT(rust-lang-nursery/rustc-hash@1.1.0:src/lib.rs) Apache-2.0/MIT: the
//! rotate-xor-multiply word mix, the 8/4/2/1 byte tiers and the fixed-seed
//! builder, reimplemented here with an explicit little-endian byte order, a
//! pinned seed and `usize` widened to `u64`.
//! Owner: engine-core. Determinism: pure `u64` integer math over explicit
//! little-endian bytes, so one key hashes to one word on Windows, Linux and
//! wasm; no process seed, no pointer, no word-size or endianness read.
//! Units: keys are entity ids (`u32`) and 64-bit words; output is one `u64`.
//! Errors: none; every write is infallible and in place.
//! Budget: hashing allocates nothing, proven by an `alloc_probe` test.
//! This is a table-key hasher, not a digest: callers compare the full key
//! after the hash, so a collision costs a probe, never a wrong result. The
//! authoritative state hash stays [`crate::hash::Fnv64`] (HASH-LAW.md).
//! Caller: none yet outside this crate; id-keyed tables in the ECS and the
//! spine adopt it next (core cannot depend on `engine-ecs`).

use std::hash::{BuildHasher, Hasher};

/// Fx multiply constant from rustc-hash's `FxHasher` (64-bit word size).
pub const FX_K: u64 = 0x517c_c1b7_2722_0a95;

/// Pinned starting state. Never zero: a zero seed plus a zero key would hash
/// to zero, and a table bucket of zero is the value that means "empty".
pub const DET_HASH_SEED: u64 = 0x243f_6a88_85a3_08d3;

/// Streaming Fx-shape hasher for entity-id keys.
///
/// Each word folds in as `state = (state.rotate_left(5) ^ word) * K`, the
/// rustc-hash shape, with every byte order fixed to little-endian. The state
/// starts at [`DET_HASH_SEED`], so the same key gives the same word on every
/// run and every platform: this is what makes an id-keyed table reproduce a
/// replay instead of depending on the process seed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DetHasher {
    state: u64,
}

impl Default for DetHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl DetHasher {
    /// Fresh hasher at the pinned seed.
    #[must_use]
    pub const fn new() -> Self {
        Self::with_seed(DET_HASH_SEED)
    }

    /// Fresh hasher from an explicit seed (chained or partitioned tables).
    #[must_use]
    pub const fn with_seed(seed: u64) -> Self {
        Self { state: seed }
    }

    /// The word so far; the hasher stays usable.
    #[must_use]
    pub const fn digest(&self) -> u64 {
        self.state
    }

    /// Mix one 64-bit word as that number, not as memory bytes.
    #[inline]
    pub fn write_word(&mut self, word: u64) {
        self.state = (self.state.rotate_left(5) ^ word).wrapping_mul(FX_K);
    }

    /// Mix a generational entity handle in `Entity { id, gen }` field order
    /// (the order `engine_ecs::archetype::Entity`'s derived `Hash` writes).
    #[inline]
    pub fn write_entity(&mut self, id: u32, gen: u32) {
        self.write_word(u64::from(id));
        self.write_word(u64::from(gen));
    }

    /// Mix bytes: 8-byte little-endian words, then the 4-, 2- and 1-byte
    /// tiers, so a short tail never pads into a neighbouring key.
    pub fn write_bytes(&mut self, bytes: &[u8]) {
        let mut chunks = bytes.chunks_exact(8);
        for chunk in &mut chunks {
            self.write_word(u64::from_le_bytes(chunk.try_into().unwrap_or([0_u8; 8])));
        }
        let mut tail = chunks.remainder();
        if tail.len() >= 4 {
            let word = u32::from_le_bytes(tail[..4].try_into().unwrap_or([0_u8; 4]));
            self.write_word(u64::from(word));
            tail = &tail[4..];
        }
        if tail.len() >= 2 {
            let word = u16::from_le_bytes(tail[..2].try_into().unwrap_or([0_u8; 2]));
            self.write_word(u64::from(word));
            tail = &tail[2..];
        }
        if let [byte] = tail {
            self.write_word(u64::from(*byte));
        }
    }
}

impl Hasher for DetHasher {
    fn finish(&self) -> u64 {
        self.state
    }

    fn write(&mut self, bytes: &[u8]) {
        self.write_bytes(bytes);
    }

    fn write_u8(&mut self, value: u8) {
        self.write_word(u64::from(value));
    }

    fn write_u16(&mut self, value: u16) {
        self.write_word(u64::from(value));
    }

    fn write_u32(&mut self, value: u32) {
        self.write_word(u64::from(value));
    }

    fn write_u64(&mut self, value: u64) {
        self.write_word(value);
    }

    fn write_u128(&mut self, value: u128) {
        self.write_word(u64::try_from(value & u128::from(u64::MAX)).unwrap_or(u64::MAX));
        self.write_word(u64::try_from(value >> 64).unwrap_or(u64::MAX));
    }

    /// Widened to `u64`, so a length or index hashes the same on a 32- and a
    /// 64-bit target (std's default would write the native width).
    fn write_usize(&mut self, value: usize) {
        self.write_word(u64::try_from(value).unwrap_or(u64::MAX));
    }

    /// Sign bits are kept (two's complement), never the native width.
    fn write_isize(&mut self, value: isize) {
        self.write_word(i64::try_from(value).unwrap_or(0).cast_unsigned());
    }
}

/// Fixed-seed builder: the hasher std tables and derived `Hash` impls need,
/// with no per-process randomness.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DetHashBuilder;

impl DetHashBuilder {
    /// The one builder: always the pinned seed.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl BuildHasher for DetHashBuilder {
    type Hasher = DetHasher;

    fn build_hasher(&self) -> DetHasher {
        DetHasher::new()
    }
}

/// One 64-bit key to its hashed word.
#[must_use]
pub fn hash_id(key: u64) -> u64 {
    let mut hasher = DetHasher::new();
    hasher.write_word(key);
    hasher.digest()
}

/// One generational entity handle to its hashed word.
#[must_use]
pub fn hash_entity_id(id: u32, gen: u32) -> u64 {
    let mut hasher = DetHasher::new();
    hasher.write_entity(id, gen);
    hasher.digest()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alloc_probe::measure;
    use std::hash::{BuildHasher, Hash};

    /// Stands in for `engine_ecs::archetype::Entity`: same fields, same derive.
    #[derive(Hash)]
    struct Handle {
        id: u32,
        gen: u32,
    }

    fn hashed<T: Hash>(value: &T) -> u64 {
        DetHashBuilder.hash_one(value)
    }

    /// Literal words from the pinned shape. They are the cross-platform bar:
    /// any target (or edit) that folds a word differently fails here.
    #[test]
    fn fx_vectors_match_the_pinned_cross_platform_literals() {
        assert_eq!(DET_HASH_SEED, 0x243f_6a88_85a3_08d3);
        assert_eq!(FX_K, 0x517c_c1b7_2722_0a95);
        assert_eq!(hash_id(0), 0x44bc_d72b_84d4_4434);
        assert_eq!(hash_id(1), 0x9639_98e2_abf6_4ec9);
        assert_eq!(hash_id(2), 0xe7b6_5a99_d318_595e);
        assert_eq!(hash_id(0xffff_ffff), 0xbdbe_0e52_5409_b137);
        assert_eq!(hash_id(0x1234_5678_9abc_def0), 0x4a00_7daa_9869_3224);
        assert_eq!(hash_entity_id(0, 0), 0x8181_f66b_dcc7_9d28);
        assert_eq!(hash_entity_id(7, 1), 0xf551_8519_a057_396b);
        assert_eq!(hash_entity_id(42, 3), 0x5937_9a7c_c73c_4027);
        assert_eq!(hash_entity_id(u32::MAX, u32::MAX), 0x33b0_69e6_b905_a1a8);
        assert_ne!(
            hash_id(0),
            0,
            "the pinned seed keeps key zero off the empty bucket"
        );
    }

    /// The `Hasher` trait's own writes fold exactly like the typed ones (AF5).
    #[test]
    fn the_hasher_trait_writes_fold_as_the_typed_ones() {
        let mut wide = DetHasher::new();
        std::hash::Hasher::write(&mut wide, b"foobar");
        assert_eq!(
            wide.finish(),
            0xffb9_4d78_33da_1dab,
            "write folds the bytes, not nothing"
        );
        let mut byte = DetHasher::new();
        byte.write_u8(1);
        assert_eq!(byte.digest(), hash_id(1), "write_u8 is one word");
        let mut half = DetHasher::new();
        half.write_u16(1);
        assert_eq!(half.digest(), hash_id(1), "write_u16 is one word");
        let mut low = DetHasher::new();
        low.write_u128(1);
        let mut low_words = DetHasher::new();
        low_words.write_word(1);
        low_words.write_word(0);
        assert_eq!(low.digest(), low_words.digest(), "the low half comes first");
        let mut high = DetHasher::new();
        high.write_u128(u128::from(u64::MAX) + 1);
        let mut high_words = DetHasher::new();
        high_words.write_word(0);
        high_words.write_word(1);
        assert_eq!(high.digest(), high_words.digest(), "the high half comes second");
    }

    /// The byte tiers: 8-byte words, then 4, 2 and 1, all little-endian.
    #[test]
    fn byte_tiers_match_the_word_path() {
        let mut wide = DetHasher::new();
        wide.write_bytes(&[1, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(wide.digest(), hash_id(1), "an 8-byte tier is one word");

        let mut four = DetHasher::new();
        four.write_bytes(&[1, 2, 3, 4]);
        let mut four_word = DetHasher::new();
        four_word.write_word(u64::from(u32::from_le_bytes([1, 2, 3, 4])));
        assert_eq!(four.digest(), four_word.digest());

        let mut two = DetHasher::new();
        two.write_bytes(&[1, 2]);
        let mut two_word = DetHasher::new();
        two_word.write_word(u64::from(u16::from_le_bytes([1, 2])));
        assert_eq!(two.digest(), two_word.digest());

        let mut one = DetHasher::new();
        one.write_bytes(b"a");
        assert_eq!(one.digest(), 0x0770_f433_ff32_56e9);

        let mut empty = DetHasher::new();
        empty.write_bytes(b"");
        assert_eq!(empty.digest(), DET_HASH_SEED, "no bytes, no mixing");
        assert_eq!(hash_bytes_literal(b"foobar"), 0xffb9_4d78_33da_1dab);
        assert_eq!(hash_bytes_literal(b"entity-ids"), 0xd429_bc73_6c27_ad4d);
    }

    fn hash_bytes_literal(bytes: &[u8]) -> u64 {
        let mut hasher = DetHasher::new();
        hasher.write_bytes(bytes);
        hasher.digest()
    }

    /// Zero drift: the same key x2, a 4096-id sequence pinned, ids spread
    /// apart in the low bucket bits a table actually reads.
    #[test]
    fn entity_ids_hash_apart_and_repeat_with_zero_drift() {
        let first = hashed(&Handle { id: 7, gen: 1 });
        let again = hashed(&Handle { id: 7, gen: 1 });
        assert_eq!(first, again, "same handle, same word");
        assert_eq!(
            first,
            hash_entity_id(7, 1),
            "derived Hash matches write_entity"
        );
        assert_ne!(hashed(&Handle { id: 7, gen: 2 }), first, "gen is hashed");
        assert_ne!(hashed(&Handle { id: 8, gen: 1 }), first, "id is hashed");
        assert_ne!(
            hashed(&Handle { id: 7, gen: 1 }),
            hashed(&Handle { id: 1, gen: 7 }),
            "field order is not commutative"
        );

        let mut chained = DetHasher::new();
        let mut buckets = [false; 4096];
        for i in 0..4096_u32 {
            chained.write_entity(i, i.wrapping_mul(3).wrapping_add(1));
            let bucket = usize::try_from(hash_id(u64::from(i)) & 0xfff).unwrap_or(0);
            assert!(
                !buckets[bucket],
                "sequential id {i} collides with another id in the low 12 bits"
            );
            buckets[bucket] = true;
        }
        assert_eq!(chained.digest(), 0x2d4b_115b_69ea_6bd4, "the sequence word");
    }

    /// No per-tick allocation: a million entity-id hashes touch no heap.
    #[test]
    fn hashing_entity_ids_allocates_nothing() {
        let (digest, heap) = measure(|| {
            let mut hasher = DetHasher::new();
            for i in 0..1_000_000_u32 {
                hasher.write_entity(i, i ^ 0x5a5a);
            }
            hasher.digest()
        });
        assert_eq!(heap.calls, 0, "hashing must not allocate: {heap:?}");
        assert_eq!(heap.bytes, 0, "hashing must not request bytes: {heap:?}");
        assert_ne!(digest, 0);
    }

    /// One fixed seed: no `RandomState`, and `usize`/`isize` widen so a
    /// 32-bit target writes the same word as a 64-bit one.
    #[test]
    fn the_builder_seed_is_fixed_and_never_random() {
        let mut a = DetHashBuilder::new().build_hasher();
        let mut b = DetHashBuilder.build_hasher();
        a.write_u64(1234);
        b.write_u64(1234);
        assert_eq!(a.finish(), b.finish(), "two builders agree");
        assert_eq!(a.finish(), hash_id(1234));

        let mut wide = DetHasher::new();
        wide.write_usize(5);
        let mut fixed = DetHasher::new();
        fixed.write_u64(5);
        assert_eq!(wide.digest(), fixed.digest(), "usize widens to u64");

        let mut signed = DetHasher::new();
        signed.write_isize(-1);
        let mut bits = DetHasher::new();
        bits.write_u64(u64::MAX);
        assert_eq!(signed.digest(), bits.digest(), "isize keeps its sign bits");
    }
}
