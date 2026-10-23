//! png: a deterministic RGBA8 PNG encoder in memory, no image crate.
//!
//! Owner: `engine-material`, because a bake must be byte-equal on every host
//! and the material crate keeps no file and no codec dependency in its
//! default build. The zlib stream uses stored (uncompressed) deflate blocks,
//! the filter byte per row is 0, and the checksums come from the existing
//! `engine_core` owners ([`engine_core::crc32::crc32`],
//! [`engine_core::adler::adler32`]): no platform compressor, so the same
//! samples always give the same bytes. Cost is one file about as large as
//! the raw samples; a 512 map is 1 MiB.

use engine_core::adler::adler32;
use engine_core::crc32::crc32;

/// Appendix D of the PNG spec: the eight-byte file signature.
const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
/// Largest payload one stored deflate block may carry.
const STORED_MAX: usize = 65_535;

/// One PNG chunk: length, type, payload, CRC over type and payload.
fn chunk(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let length = u32::try_from(payload.len()).unwrap_or(u32::MAX);
    let mut out = Vec::with_capacity(payload.len().saturating_add(12));
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(payload);
    let mut crc_input = Vec::with_capacity(payload.len().saturating_add(4));
    crc_input.extend_from_slice(kind);
    crc_input.extend_from_slice(payload);
    out.extend_from_slice(&crc32(&crc_input).to_be_bytes());
    out
}

/// The zlib stream wrapping a raw byte run: header, stored blocks, Adler-32.
fn zlib(raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(raw.len().saturating_add(raw.len() / STORED_MAX * 5 + 16));
    out.extend_from_slice(&[0x78, 0x01]);
    let mut rest = raw;
    loop {
        let take = rest.len().min(STORED_MAX);
        let last = take == rest.len();
        let (block, tail) = rest.split_at(take);
        out.push(u8::from(last));
        let len = u16::try_from(block.len()).unwrap_or(0);
        out.extend_from_slice(&len.to_le_bytes());
        out.extend_from_slice(&(!len).to_le_bytes());
        out.extend_from_slice(block);
        rest = tail;
        if last {
            break;
        }
    }
    out.extend_from_slice(&adler32(raw).to_be_bytes());
    out
}

/// Encode `width * height` RGBA8 samples as one PNG file. A sample run
/// shorter than the size claims is zero-filled, so a caller cannot index
/// past its buffer through this function.
#[must_use]
pub fn encode_rgba(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    let stride = usize::try_from(width).unwrap_or(0).saturating_mul(4);
    let rows = usize::try_from(height).unwrap_or(0);
    let mut raw = Vec::with_capacity(stride.saturating_mul(rows).saturating_add(rows));
    for row in 0..rows {
        raw.push(0);
        let start = row.saturating_mul(stride);
        let end = start.saturating_add(stride);
        match rgba.get(start..end) {
            Some(line) => raw.extend_from_slice(line),
            None => raw.resize(raw.len().saturating_add(stride), 0),
        }
    }
    let mut header = Vec::with_capacity(13);
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut out = SIGNATURE.to_vec();
    out.extend_from_slice(&chunk(b"IHDR", &header));
    out.extend_from_slice(&chunk(b"IDAT", &zlib(&raw)));
    out.extend_from_slice(&chunk(b"IEND", &[]));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_two_by_two_png_has_the_signature_and_the_chunk_order() {
        let png = encode_rgba(
            2,
            2,
            &[255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 9, 9, 9, 255],
        );
        assert_eq!(png.get(..8), Some(&SIGNATURE[..]));
        assert_eq!(png.get(12..16), Some(&b"IHDR"[..]));
        assert!(png.windows(4).any(|run| run == b"IDAT"));
        let tail = png.len().saturating_sub(12);
        assert_eq!(png.get(tail..tail + 4), Some(&[0u8; 4][..]));
        assert_eq!(png.get(tail + 4..tail + 8), Some(&b"IEND"[..]));
        assert_eq!(encode_rgba(2, 2, &[7; 4]), encode_rgba(2, 2, &[7; 4]));
    }

    #[test]
    fn the_zlib_stream_carries_the_adler_checksum_of_the_raw_run() {
        let raw = [1u8, 2, 3, 4];
        let stream = zlib(&raw);
        assert_eq!(stream.get(..2), Some(&[0x78, 0x01][..]));
        assert_eq!(
            stream.get(stream.len().saturating_sub(4)..),
            Some(&adler32(&raw).to_be_bytes()[..])
        );
    }
}
