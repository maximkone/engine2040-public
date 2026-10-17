//! crc32: bitwise IEEE CRC-32 (the PNG chunk and save envelope checksum).
//! Units: bytes in, 32-bit checksum out. Determinism: pure integer math, no
//! table, no allocation.

/// Bitwise IEEE CRC-32 over `bytes` (reflected, poly `0xEDB88320`). Empty
/// input gives `0`.
#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for b in bytes {
        crc ^= u32::from(*b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::crc32;

    #[test]
    fn empty_input_is_zero() {
        assert_eq!(crc32(&[]), 0);
    }

    #[test]
    fn check_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}
