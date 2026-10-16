//! adler: the one Adler-32 (the zlib trailer checksum). PNG writers in game
//! and editor share it, so every zlib stream carries the same bits.
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
    fn wikipedia_vector() {
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }
}
