//! hash: the engine's one owner of non-cryptographic state and content hashes.
//! Owner: engine-core. Determinism: pure integer math over little-endian
//! bytes, identical on every platform and build. Not collision-resistant
//! against an adversary; trust boundaries use a cryptographic digest.
//!
//! `Fnv64` deliberately does not implement `std::hash::Hasher`: derived
//! `Hash` impls write native-endian integers and `usize` lengths, so the same
//! value would hash differently on 32- and 64-bit or big-endian targets.

/// FNV-1a 64 offset basis.
pub const FNV64_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64 prime.
pub const FNV64_PRIME: u64 = 0x0000_0100_0000_01b3;
/// SplitMix64 increment (2^64 / golden ratio, odd).
pub const GOLDEN_GAMMA: u64 = 0x9e37_79b9_7f4a_7c15;
/// First SplitMix64 finalizer multiplier (the lane multiplier some mixers reuse).
pub const MIX64_MUL_A: u64 = 0xbf58_476d_1ce4_e5b9;

/// Streaming FNV-1a 64. Integers are written little-endian and floats as
/// their IEEE-754 bits, so a digest never depends on the host.
///
/// Writes are concatenated without separators: when two variable-length
/// fields follow each other, write a length first so `("ab", "c")` and
/// `("a", "bc")` cannot collide.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fnv64(u64);

impl Default for Fnv64 {
    fn default() -> Self {
        Self::new()
    }
}

impl Fnv64 {
    /// Fresh hasher at the FNV-1a offset basis.
    #[must_use]
    pub const fn new() -> Self {
        Self(FNV64_OFFSET)
    }

    /// Resume from an earlier `finish()` value, for chained digests.
    #[must_use]
    pub const fn with_state(state: u64) -> Self {
        Self(state)
    }

    /// Mix one byte.
    #[inline]
    pub fn write_u8(&mut self, byte: u8) {
        self.0 = (self.0 ^ u64::from(byte)).wrapping_mul(FNV64_PRIME);
    }

    /// Mix bytes in order.
    #[inline]
    pub fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.write_u8(byte);
        }
    }

    /// Mix a `u16` as two little-endian bytes.
    #[inline]
    pub fn write_u16(&mut self, value: u16) {
        self.write(&value.to_le_bytes());
    }

    /// Mix a `u32` as four little-endian bytes.
    #[inline]
    pub fn write_u32(&mut self, value: u32) {
        self.write(&value.to_le_bytes());
    }

    /// Mix a `u64` as eight little-endian bytes.
    #[inline]
    pub fn write_u64(&mut self, value: u64) {
        self.write(&value.to_le_bytes());
    }

    /// Mix a collection length as a `u64` (the bytes `write_u64(n as u64)` wrote).
    #[inline]
    pub fn write_len(&mut self, len: usize) {
        self.write_u64(u64::try_from(len).unwrap_or(u64::MAX));
    }

    /// Mix a collection length as a `u32`, saturating at `u32::MAX` (the bytes
    /// `write_u32(n as u32)` wrote for every length below 2^32).
    #[inline]
    pub fn write_len32(&mut self, len: usize) {
        self.write_u32(u32::try_from(len).unwrap_or(u32::MAX));
    }

    /// Mix an `i32` as four little-endian bytes.
    #[inline]
    pub fn write_i32(&mut self, value: i32) {
        self.write(&value.to_le_bytes());
    }

    /// Mix an `i64` as eight little-endian bytes.
    #[inline]
    pub fn write_i64(&mut self, value: i64) {
        self.write(&value.to_le_bytes());
    }

    /// Mix an `f32` by its bit pattern (`-0.0` and `0.0` differ; so do NaNs).
    #[inline]
    pub fn write_f32(&mut self, value: f32) {
        self.write_u32(value.to_bits());
    }

    /// Mix a `bool` as one byte (0 or 1).
    #[inline]
    pub fn write_bool(&mut self, value: bool) {
        self.write_u8(u8::from(value));
    }

    /// Mix a string's UTF-8 bytes (no length prefix; see the type docs).
    #[inline]
    pub fn write_str(&mut self, value: &str) {
        self.write(value.as_bytes());
    }

    /// The digest so far; the hasher stays usable.
    #[must_use]
    pub const fn finish(&self) -> u64 {
        self.0
    }
}

/// FNV-1a 64 over `bytes`.
#[must_use]
pub fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = Fnv64::new();
    hasher.write(bytes);
    hasher.finish()
}

/// FNV-1a 64 over each word's little-endian bytes, in order.
#[must_use]
pub fn hash_u64s(words: &[u64]) -> u64 {
    let mut hasher = Fnv64::new();
    for &word in words {
        hasher.write_u64(word);
    }
    hasher.finish()
}

/// Lowercase 16-char hex of a `u64` digest (receipts, manifests, diagnostics).
#[must_use]
pub fn hex16(value: u64) -> String {
    format!("{value:016x}")
}

/// FNV-1a 64 of `bytes` as 16-char hex (single owner for content tags).
#[must_use]
pub fn hash_hex(bytes: &[u8]) -> String {
    hex16(hash_bytes(bytes))
}

/// SplitMix64 finalizer: every input bit reaches every output bit, and the
/// map is a bijection on `u64`.
#[inline]
#[must_use]
pub const fn mix64(mut z: u64) -> u64 {
    z = (z ^ (z >> 30)).wrapping_mul(MIX64_MUL_A);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// One SplitMix64 step from state `z`: `mix64(z + GOLDEN_GAMMA)`.
#[inline]
#[must_use]
pub const fn splitmix64(z: u64) -> u64 {
    mix64(z.wrapping_add(GOLDEN_GAMMA))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fnv_matches_published_vectors() {
        // FNV-1a 64 test vectors from the reference suite (Noll).
        assert_eq!(hash_bytes(b""), FNV64_OFFSET);
        assert_eq!(hash_bytes(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(hash_bytes(b"foobar"), 0x8594_4171_f739_67e8);
    }

    #[test]
    fn typed_writes_are_little_endian_bytes() {
        let mut typed = Fnv64::new();
        typed.write_u32(0x0102_0304);
        typed.write_f32(-0.0);
        typed.write_bool(true);
        let mut raw = Fnv64::new();
        raw.write(&[4, 3, 2, 1]);
        raw.write(&(-0.0f32).to_bits().to_le_bytes());
        raw.write_u8(1);
        assert_eq!(typed.finish(), raw.finish());
        let mut positive = Fnv64::new();
        positive.write_f32(0.0);
        assert_ne!(positive.finish(), Fnv64::new().finish());
        assert_ne!(hash_u64s(&[1, 2]), hash_u64s(&[2, 1]), "order matters");
    }

    #[test]
    fn length_writes_match_the_plain_integer_writes() {
        let mut wide = Fnv64::new();
        wide.write_len(5);
        let mut plain = Fnv64::new();
        plain.write_u64(5);
        assert_eq!(wide.finish(), plain.finish());
        let mut narrow = Fnv64::new();
        narrow.write_len32(5);
        let mut plain32 = Fnv64::new();
        plain32.write_u32(5);
        assert_eq!(narrow.finish(), plain32.finish());
        assert_ne!(wide.finish(), narrow.finish(), "the two widths differ");
    }

    #[test]
    fn chained_state_resumes_exactly() {
        let mut whole = Fnv64::new();
        whole.write(b"engine2040");
        let mut first = Fnv64::new();
        first.write(b"engine");
        let mut resumed = Fnv64::with_state(first.finish());
        resumed.write(b"2040");
        assert_eq!(resumed.finish(), whole.finish());
    }

    #[test]
    fn splitmix_matches_reference_sequence() {
        // SplitMix64 seeded with 0: the first outputs of Vigna's reference.
        let mut state = 0u64;
        let mut next = || {
            let out = splitmix64(state);
            state = state.wrapping_add(GOLDEN_GAMMA);
            out
        };
        assert_eq!(next(), 0xe220_a839_7b1d_cdaf);
        assert_eq!(next(), 0x6e78_9e6a_a1b9_65f4);
        assert_eq!(mix64(0), 0, "zero is the finalizer's fixed point");
    }

    #[test]
    fn hex_helpers_match_known_vectors() {
        assert_eq!(hex16(0xd57b5c8ae3096614), "d57b5c8ae3096614");
        assert_eq!(hex16(0), "0000000000000000");
        assert_eq!(hash_hex(b""), "cbf29ce484222325");
        assert_eq!(hash_hex(b"a"), hex16(hash_bytes(b"a")));
    }

    #[test]
    fn hex_helpers_are_lowercase_padded() {
        for v in [0u64, 1, 0xaf63dc4c8601ec8c, u64::MAX] {
            let s = hex16(v);
            assert_eq!(s.len(), 16);
            assert!(s
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        }
        assert_eq!(hash_hex(b"a").len(), 16);
        assert_ne!(hash_hex(b"a"), hash_hex(b"b"));
    }
}
