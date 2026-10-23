//! index_codec: a FIFO plus delta codec for the cooked mesh index block.
//!
//! Consumer: [`super::bundle::encode_mesh`]. The cook already orders indices
//! for the vertex cache (`mesh_cook::optimize_index_order`), so a triangle
//! usually reuses a vertex seen a few indices ago. The codec keeps the last
//! [`FIFO_LEN`] vertices and writes one token byte per index: a FIFO position,
//! a "previous plus one" step, zero, or a zigzag delta for a new vertex.
//!
//! This is our own format, not byte-compatible with meshoptimizer's edge-FIFO
//! codec; it borrows only the idea. Deterministic integer work: same indices
//! give the same bytes, and decode is exact (lossless remap, no float).
//!
//! Wire: one token byte per index, plus a LEB128 zigzag delta after
//! [`TOKEN_DELTA`]. The index count lives in the bundle payload, not here, so
//! the decoder is told how many indices to expect and refuses a stream that
//! runs short, runs long, or names a vertex that does not exist.

use std::fmt;

/// Recent vertices remembered, most recent first.
pub const FIFO_LEN: usize = 16;
/// Token range `0x00..=0x0F`: a FIFO hit, the byte is the position.
const TOKEN_FIFO_MAX: u8 = 0x0F;
/// Token: the previous index plus one (a fresh vertex in a strip).
const TOKEN_NEXT: u8 = 0x10;
/// Token: index zero.
const TOKEN_ZERO: u8 = 0x11;
/// Token: zigzag LEB128 delta against the previous index follows.
const TOKEN_DELTA: u8 = 0x12;
/// Widest LEB128 delta accepted: five 7-bit groups cover `u32` twice over.
const MAX_VARINT_BYTES: usize = 5;

/// Why an index block failed to encode or decode; every bad input maps here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IndexCodecError {
    /// A token byte outside the defined set.
    BadToken(u8),
    /// The stream ended before the promised index count.
    Truncated,
    /// The stream holds more bytes than the promised count needs.
    TrailingBytes,
    /// An index names a vertex outside `0..vertices`.
    OutOfRange {
        /// The offending index.
        index: u32,
        /// Vertex count the block must stay under.
        vertices: usize,
    },
    /// A delta LEV128 would not fit the accepted width.
    DeltaOverflow,
    /// More indices than the caller's budget allows.
    TooManyIndices(usize),
}

impl fmt::Display for IndexCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadToken(byte) => write!(f, "bad index token {byte:#04x}"),
            Self::Truncated => write!(f, "index stream ends mid-index"),
            Self::TrailingBytes => write!(f, "index stream has trailing bytes"),
            Self::OutOfRange { index, vertices } => {
                write!(f, "index {index} outside 0..{vertices}")
            }
            Self::DeltaOverflow => write!(f, "index delta too wide"),
            Self::TooManyIndices(count) => write!(f, "{count} indices over budget"),
        }
    }
}

/// The last [`FIFO_LEN`] indices, most recent first.
struct Fifo {
    items: [u32; FIFO_LEN],
    len: usize,
}

impl Fifo {
    fn new() -> Self {
        Self {
            items: [0; FIFO_LEN],
            len: 0,
        }
    }

    fn position(&self, index: u32) -> Option<u8> {
        let at = self.items[..self.len]
            .iter()
            .position(|held| *held == index)?;
        u8::try_from(at).ok()
    }

    fn at(&self, position: u8) -> Option<u32> {
        let at = usize::from(position);
        if at < self.len {
            Some(self.items[at])
        } else {
            None
        }
    }

    fn push(&mut self, index: u32) {
        let held = self.len.min(FIFO_LEN - 1);
        self.items.copy_within(0..held, 1);
        self.items[0] = index;
        if self.len < FIFO_LEN {
            self.len += 1;
        }
    }
}

/// Encode one triangle-list index block. Fails only when an index does not
/// name a vertex of `vertices`; a validated mesh never trips it.
pub fn encode(indices: &[u32], vertices: usize) -> Result<Vec<u8>, IndexCodecError> {
    let mut fifo = Fifo::new();
    let mut last = 0u32;
    let mut out = Vec::with_capacity(indices.len().saturating_mul(2).min(1 << 20));
    for &index in indices {
        if usize::try_from(index).map_or(true, |i| i >= vertices) {
            return Err(IndexCodecError::OutOfRange { index, vertices });
        }
        match fifo.position(index) {
            Some(position) => out.push(position),
            None if index == last.saturating_add(1) => out.push(TOKEN_NEXT),
            None if index == 0 => out.push(TOKEN_ZERO),
            None => {
                out.push(TOKEN_DELTA);
                push_delta(&mut out, last, index);
            }
        }
        fifo.push(index);
        last = index;
    }
    Ok(out)
}

/// Decode exactly `count` indices for a mesh of `vertices` vertices. Refuses
/// every malformed stream with a typed error; never allocates for the caller's
/// count before the count itself is within budget.
pub fn decode(bytes: &[u8], count: usize, vertices: usize) -> Result<Vec<u32>, IndexCodecError> {
    if count > super::bundle::MAX_BUNDLE_INDICES {
        return Err(IndexCodecError::TooManyIndices(count));
    }
    let mut fifo = Fifo::new();
    let mut last = 0u32;
    let mut at = 0usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let token = *bytes.get(at).ok_or(IndexCodecError::Truncated)?;
        at += 1;
        let index = if token <= TOKEN_FIFO_MAX {
            fifo.at(token).ok_or(IndexCodecError::BadToken(token))?
        } else if token == TOKEN_NEXT {
            last.checked_add(1).ok_or(IndexCodecError::OutOfRange {
                index: u32::MAX,
                vertices,
            })?
        } else if token == TOKEN_ZERO {
            0
        } else if token == TOKEN_DELTA {
            let delta = get_delta(bytes, &mut at)?;
            i64::from(last)
                .checked_add(delta)
                .and_then(|value| u32::try_from(value).ok())
                .ok_or(IndexCodecError::OutOfRange {
                    index: u32::MAX,
                    vertices,
                })?
        } else {
            return Err(IndexCodecError::BadToken(token));
        };
        if usize::try_from(index).map_or(true, |i| i >= vertices) {
            return Err(IndexCodecError::OutOfRange { index, vertices });
        }
        fifo.push(index);
        last = index;
        out.push(index);
    }
    if at != bytes.len() {
        return Err(IndexCodecError::TrailingBytes);
    }
    Ok(out)
}

/// Zigzag a signed delta and write it as LEB128.
fn push_delta(out: &mut Vec<u8>, from: u32, to: u32) {
    let delta = i64::from(to) - i64::from(from);
    let magnitude = delta.unsigned_abs();
    let mut rest = if delta < 0 {
        (magnitude << 1) - 1
    } else {
        magnitude << 1
    };
    loop {
        // Low byte of the little-endian form: no `as` cast, and the mask keeps
        // the continuation bit clear.
        let byte = rest.to_le_bytes()[0] & 0x7f;
        rest >>= 7;
        if rest == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// Read one LEB128 zigzag delta, refusing a value wider than five groups.
fn get_delta(bytes: &[u8], at: &mut usize) -> Result<i64, IndexCodecError> {
    let mut value = 0u64;
    let mut shift = 0u32;
    for _ in 0..MAX_VARINT_BYTES {
        let byte = *bytes.get(*at).ok_or(IndexCodecError::Truncated)?;
        *at += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            let magnitude =
                i64::try_from(value >> 1).map_err(|_| IndexCodecError::DeltaOverflow)?;
            return Ok(if value & 1 == 0 {
                magnitude
            } else {
                -magnitude - 1
            });
        }
        shift += 7;
    }
    Err(IndexCodecError::DeltaOverflow)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed pseudo-random sequence: no dependency, same set every run.
    fn lcg_indices(count: usize, vertices: u32, seed: u64) -> Vec<u32> {
        let mut state = seed;
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let value = u32::try_from(state >> 33).unwrap_or(0);
            out.push(value % vertices);
        }
        out
    }

    /// A striped grid in triangle-list order: a cache-friendly real shape.
    fn grid_indices(side: u32) -> Vec<u32> {
        let mut indices = Vec::new();
        for y in 0..side {
            for x in 0..side {
                let base = y * (side + 1) + x;
                indices.extend_from_slice(&[base, base + 1, base + side + 1]);
                indices.extend_from_slice(&[base + 1, base + side + 2, base + side + 1]);
            }
        }
        indices
    }

    fn round_trip(indices: &[u32], vertices: usize) -> Vec<u8> {
        let bytes = encode(indices, vertices).unwrap();
        let back = decode(&bytes, indices.len(), vertices).unwrap();
        assert_eq!(back, indices, "round trip differs");
        bytes
    }

    #[test]
    fn round_trips_synthetic_index_sets() {
        assert!(round_trip(&[], 0).is_empty());
        assert_eq!(round_trip(&[0], 1), vec![TOKEN_ZERO]);
        round_trip(&[0, 1, 2], 3);
        round_trip(&grid_indices(8), 9 * 9);
        round_trip(&lcg_indices(3_000, 700, 2040), 700);
        // Zigzag round trip on both sides of zero.
        round_trip(&[500, 0, 7, 7, 3, 900, 1, 2], 1_000);
    }

    #[test]
    fn every_prefix_of_a_stream_is_refused() {
        let indices = grid_indices(4);
        let bytes = encode(&indices, 25).unwrap();
        for cut in 0..bytes.len() {
            assert_eq!(
                decode(&bytes[..cut], indices.len(), 25).unwrap_err(),
                IndexCodecError::Truncated,
                "a stream cut at {cut} must not decode"
            );
        }
        let mut long = bytes.clone();
        long.push(TOKEN_ZERO);
        assert_eq!(
            decode(&long, indices.len(), 25).unwrap_err(),
            IndexCodecError::TrailingBytes
        );
        // The count drives the read: a shorter count leaves trailing bytes, a
        // longer one runs short.
        assert_eq!(
            decode(&bytes, indices.len() - 3, 25).unwrap_err(),
            IndexCodecError::TrailingBytes
        );
        assert_eq!(
            decode(&bytes, indices.len() + 3, 25).unwrap_err(),
            IndexCodecError::Truncated
        );
    }

    #[test]
    fn corrupt_tokens_and_indices_are_refused() {
        let bytes = encode(&[0, 1, 2], 3).unwrap();
        let mut bad = bytes.clone();
        bad[0] = 0x7f;
        assert_eq!(
            decode(&bad, 3, 3).unwrap_err(),
            IndexCodecError::BadToken(0x7f)
        );
        // A valid token naming a vertex the mesh does not have.
        let mut wide = bytes.clone();
        wide[2] = TOKEN_DELTA;
        wide.push(0x10); // zigzag 16 -> delta +8 -> index 9
        assert_eq!(
            decode(&wide, 3, 3).unwrap_err(),
            IndexCodecError::OutOfRange {
                index: 9,
                vertices: 3
            }
        );
        // Six continuation bytes cannot be a five-group delta.
        let mut long = vec![TOKEN_ZERO, TOKEN_ZERO, TOKEN_DELTA];
        long.extend_from_slice(&[0x80; 6]);
        assert_eq!(
            decode(&long, 3, 3).unwrap_err(),
            IndexCodecError::DeltaOverflow
        );
        assert_eq!(
            decode(&bytes, super::super::bundle::MAX_BUNDLE_INDICES + 1, 3).unwrap_err(),
            IndexCodecError::TooManyIndices(super::super::bundle::MAX_BUNDLE_INDICES + 1)
        );
    }

    #[test]
    fn a_cache_ordered_grid_costs_under_half_the_raw_bytes() {
        let indices = grid_indices(32);
        let vertices = 33 * 33;
        let bytes = round_trip(&indices, vertices);
        let raw = indices.len() * 4;
        assert!(
            bytes.len() * 2 < raw,
            "coded {} B is not under half of raw {raw} B",
            bytes.len()
        );
        println!(
            "index_codec grid vertices:{vertices} indices:{} raw:{raw} coded:{} ratio:{:.3}",
            indices.len(),
            bytes.len(),
            bytes.len() as f64 / raw as f64
        );
    }
}
