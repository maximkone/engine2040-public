//! adler: Adler-32 (the zlib trailer checksum), one of several local copies.
//! PNG writers in game, editor and endpoint share this one so every zlib
//! stream they emit carries the same bits. Sibling copies live at
//! `game/staged_fight_frames.rs`, `game/camera_feed.rs` and
//! `game/bin/arena_capture.rs`; keep them bit-identical when touching this
//! routine.
//! Units: bytes in, 32-bit checksum out. Determinism: pure integer math,
//! no allocation.

/// Adler-32 modulus: the largest prime below 2^16.
const MOD: u32 = 65521;
/// Bytes summed before each modulo; keeps `b` inside `u32` between reductions.
const CHUNK: usize = 5552;

/// Adler-32 over `data`, reduced per chunk so the running sums never overflow
/// `u32`. Empty input gives `1`.
#[must_use]
pub fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for piece in data.chunks(CHUNK) {
        for byte in piece {
            a += u32::from(*byte);
            b += a;
        }
        a %= MOD;
        b %= MOD;
    }
    (b << 16) | a
}

#[cfg(test)]
mod tests {
    use super::adler32;

    #[test]
    fn empty_input_is_one() {
        assert_eq!(adler32(&[]), 1);
    }

    #[test]
    fn the_halves_are_disjoint_so_or_and_xor_agree() {
        // a < MOD < 2^16 and b < MOD, so `b << 16` owns the high half only (AF5 note:
        // the `|` -> `^` survivor is equivalent for this reason).
        let value = adler32(b"Wikipedia");
        assert_eq!(value & 0xffff, 0x0398);
        assert_eq!(value >> 16, 0x11e6);
    }

    #[test]
    fn wikipedia_vector() {
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }
}
