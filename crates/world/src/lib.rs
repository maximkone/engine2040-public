//! world: deterministic bake. Seed N + size M -> versioned bytes, std-only.
//! Pathfinding lives in engine-nav (single A* owner); world only emits cells.

pub mod cells;
pub mod ground;
pub mod scatter;
pub mod terrain;

/// FNV-1a 64 over bytes: the one content hash every bake and
/// terrain gate in this crate uses.
pub use engine_core::hash_bytes as world_hash;
use engine_core::{hash::GOLDEN_GAMMA, mix64};

pub const MAGIC: [u8; 4] = *b"W204";
pub const VERSION: u8 = 1;

/// Largest accepted bake side in texels. A 256 x 256 map is under 300 KB;
/// `u16` alone would let one request reserve gigabytes.
pub const MAX_BAKE_SIZE: u16 = 256;

/// Why a bake request was refused. Nothing is allocated on refusal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BakeErr {
    /// Side exceeds [`MAX_BAKE_SIZE`].
    TooLarge,
}

impl std::fmt::Display for BakeErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BakeErr::TooLarge => write!(f, "bake size exceeds MAX_BAKE_SIZE"),
        }
    }
}

impl std::error::Error for BakeErr {}

/// mix seed+x+y via splitmix-style avalanche. Order-independent, pure.
fn lattice(seed: u64, x: i32, y: i32) -> u64 {
    let z = seed
        .wrapping_add((x as u64).wrapping_mul(GOLDEN_GAMMA))
        .wrapping_add((y as u64).wrapping_mul(0xbf58476d1ce4e5b9));
    mix64(z)
}

/// Deterministic value-noise height 0..255. Integer lattice + bilinear smootherstep.
pub fn height(seed: u64, x: i32, y: i32) -> u8 {
    let xi = x.div_euclid(8);
    let yi = y.div_euclid(8);
    let xf = u64::from(x.rem_euclid(8) as u32 * 65536 / 8);
    let yf = u64::from(y.rem_euclid(8) as u32 * 65536 / 8);
    // smootherstep weights: 6t^5-15t^4+10t^3 in Q16
    let w = |t: u64| -> u64 {
        let t2 = (t * t) >> 16;
        let t3 = (t2 * t) >> 16;
        let t4 = (t2 * t2) >> 16;
        let t5 = (t4 * t) >> 16;
        (6 * t5).wrapping_sub(15 * t4).wrapping_add(10 * t3) >> 16
    };
    let (wx, wy) = (w(xf), w(yf));
    let v = |dx: i32, dy: i32| lattice(seed, xi + dx, yi + dy) >> 56; // top byte 0..255
    let (v00, v10, v01, v11) = (v(0, 0), v(1, 0), v(0, 1), v(1, 1));
    let top = v00 * (65536 - wx) + v10 * wx;
    let bot = v01 * (65536 - wx) + v11 * wx;
    (((top * (65536 - wy) + bot * wy) >> 32) & 0xFF) as u8
}

/// Bake versioned bytes: MAGIC(4) + ver(1) + seed LE(8) + size LE(2) + size*size heights.
/// Units: texels in, bytes out. Errors: [`BakeErr::TooLarge`] past
/// [`MAX_BAKE_SIZE`], checked before any allocation. Determinism: same seed
/// and size give the same bytes.
pub fn bake(seed: u64, size: u16) -> Result<Vec<u8>, BakeErr> {
    if size > MAX_BAKE_SIZE {
        return Err(BakeErr::TooLarge);
    }
    let n = usize::from(size);
    let mut out = Vec::with_capacity(15 + n * n);
    out.extend_from_slice(&MAGIC);
    out.push(VERSION);
    out.extend_from_slice(&seed.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    for y in 0..n {
        for x in 0..n {
            out.push(height(seed, x as i32, y as i32));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn baked(seed: u64, size: u16) -> Vec<u8> {
        bake(seed, size).expect("size within MAX_BAKE_SIZE")
    }

    #[test]
    fn bake_x2_identical() {
        assert_eq!(baked(7, 16), baked(7, 16));
        assert_eq!(world_hash(&baked(7, 16)), world_hash(&baked(7, 16)));
        assert_ne!(baked(7, 16), baked(8, 16));
    }

    #[test]
    fn same_seed_same_height() {
        assert_eq!(height(7, 3, 5), height(7, 3, 5));
        let b = baked(7, 8);
        assert_eq!(b[15 + 5 * 8 + 3], height(7, 3, 5));
    }

    #[test]
    fn bake_cap_is_exact_and_refuses_before_allocating() {
        let n = usize::from(MAX_BAKE_SIZE);
        assert_eq!(baked(7, MAX_BAKE_SIZE).len(), 15 + n * n);
        assert_eq!(baked(7, 0).len(), 15);
        // `u16::MAX` would reserve about 4 GiB if the check ran after the
        // allocation; the refusal returns at once.
        assert_eq!(bake(7, MAX_BAKE_SIZE + 1), Err(BakeErr::TooLarge));
        assert_eq!(bake(7, u16::MAX), Err(BakeErr::TooLarge));
        assert_eq!(
            BakeErr::TooLarge.to_string(),
            "bake size exceeds MAX_BAKE_SIZE"
        );
    }
}
