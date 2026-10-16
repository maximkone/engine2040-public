//! terrain: heightmap + hole bitmask + splat weights, seeded. Layout: MAGIC(4) TVER(1) seed(8LE) size(2LE) + heights + holebits + splat.
use crate::{height, lattice, world_hash, BakeErr, MAGIC, MAX_BAKE_SIZE};
use engine_core::hash::GOLDEN_GAMMA;
pub const TVERSION: u8 = 1;
pub const SPLAT_N: usize = 3;
pub const SLOPE_DEFAULT: u8 = 64;
const TALUS: i16 = 10;
const HOLE_CUT: u8 = 250;
const HOLE_TAG: u64 = GOLDEN_GAMMA ^ 0x51ab;
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Terrain {
    pub seed: u64,
    pub size: u16,
    pub heights: Vec<u8>,
    pub holes: Vec<bool>,
    pub splat: Vec<[u8; 3]>,
}
// 2-octave value fBm, integer-only. The cell bake shares it, so cell
// interiors match a whole-map bake wherever the halo covers erosion reach.
pub(crate) fn base_height(seed: u64, x: i32, y: i32) -> u8 {
    let a = u16::from(height(seed, x, y));
    let b = u16::from(height(
        seed.wrapping_add(GOLDEN_GAMMA),
        x.wrapping_mul(2),
        y.wrapping_mul(2),
    ));
    ((a * 3 + b) / 4) as u8
}
// Thermal erosion: pairwise talus exchange, Jacobi, fixed order.
fn erode(h: &mut [u8], n: usize) {
    if n < 2 {
        return;
    }
    let mut d = vec![0i16; n * n];
    for _ in 0..4 {
        let src: Vec<i16> = h.iter().map(|&v| i16::from(v)).collect();
        d.fill(0);
        for y in 0..n {
            for x in 0..n {
                let i = y * n + x;
                for (dx, dy) in [(1i32, 0), (0, 1)] {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx >= n as i32 || ny >= n as i32 {
                        continue;
                    }
                    let j = ny as usize * n + nx as usize;
                    let diff = src[i] - src[j];
                    if diff > TALUS {
                        let m = (diff - TALUS) / 4;
                        d[i] -= m;
                        d[j] += m;
                    } else if -diff > TALUS {
                        let m = (-diff - TALUS) / 4;
                        d[j] -= m;
                        d[i] += m;
                    }
                }
            }
        }
        for (v, dd) in h.iter_mut().zip(&d) {
            *v = ((i16::from(*v) + *dd).clamp(0, 255)) as u8;
        }
    }
}
fn raw_holes(seed: u64, n: usize) -> Vec<bool> {
    let mut r = vec![false; n * n];
    for y in 0..n {
        for x in 0..n {
            if x == 0 || y == 0 || x + 1 == n || y + 1 == n {
                continue;
            }
            let v = (lattice(seed.wrapping_add(HOLE_TAG), x as i32, y as i32) >> 56) as u8;
            r[y * n + x] = v > HOLE_CUT;
        }
    }
    r
}
// Drop holes touching any 8-neighbour: single cells never disconnect the grid.
fn fix_holes(raw: &[bool], n: usize) -> Vec<bool> {
    let mut out = vec![false; n * n];
    for y in 0..n {
        for x in 0..n {
            if !raw[y * n + x] {
                continue;
            }
            let mut adj = false;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    if dx == 0 && dy == 0 {
                        continue;
                    }
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= n as i32 || ny >= n as i32 {
                        continue;
                    }
                    if raw[ny as usize * n + nx as usize] {
                        adj = true;
                        break;
                    }
                }
                if adj {
                    break;
                }
            }
            out[y * n + x] = !adj;
        }
    }
    out
}
/// Max abs neighbour step of one interior height texel, in height units.
pub(crate) fn slope_of(h: &[u8], n: usize, x: usize, y: usize) -> u8 {
    let c = i32::from(h[y * n + x]);
    let mut m = 0i32;
    if x > 0 {
        m = m.max((c - i32::from(h[y * n + x - 1])).abs());
    }
    if x + 1 < n {
        m = m.max((c - i32::from(h[y * n + x + 1])).abs());
    }
    if y > 0 {
        m = m.max((c - i32::from(h[(y - 1) * n + x])).abs());
    }
    if y + 1 < n {
        m = m.max((c - i32::from(h[(y + 1) * n + x])).abs());
    }
    m as u8
}
// sand=low, grass=mid, rock=high+steep; weights sum to exactly 255.
fn splat_for(hi: u8, slope: u8) -> [u8; 3] {
    let (h, s) = (i32::from(hi), i32::from(slope));
    let sc = [
        255 - h,
        (255 - (h - 128).abs() * 2).max(0),
        (h / 2 + s * 3).min(255),
    ];
    let tot: i32 = sc.iter().sum();
    if tot == 0 {
        return [0, 0, 255];
    }
    let mut w = [0u8; 3];
    let mut rem = 255i32;
    for i in 0..3 {
        w[i] = (sc[i] * 255 / tot) as u8;
        rem -= i32::from(w[i]);
    }
    let (mut bi, mut bv) = (0, -1i32);
    for i in 0..3 {
        let v = sc[i] * 255 - i32::from(w[i]) * tot;
        if v > bv {
            bv = v;
            bi = i;
        }
    }
    w[bi] = (i32::from(w[bi]) + rem) as u8;
    w
}
/// Bakes versioned terrain bytes. Units: texels in, bytes out. Errors:
/// [`BakeErr::TooLarge`] past `MAX_BAKE_SIZE`, checked before any allocation.
pub fn bake_terrain(seed: u64, size: u16) -> Result<Vec<u8>, BakeErr> {
    if size > MAX_BAKE_SIZE {
        return Err(BakeErr::TooLarge);
    }
    let n = usize::from(size);
    let mut h = vec![0u8; n * n];
    for y in 0..n {
        for x in 0..n {
            h[y * n + x] = base_height(seed, x as i32, y as i32);
        }
    }
    erode(&mut h, n);
    let holes = fix_holes(&raw_holes(seed, n), n);
    let mut spl = vec![[0u8; 3]; n * n];
    for y in 0..n {
        for x in 0..n {
            spl[y * n + x] = splat_for(h[y * n + x], slope_of(&h, n, x, y));
        }
    }
    let hb = (n * n).div_ceil(8);
    let mut out = Vec::with_capacity(15 + n * n + hb + n * n * SPLAT_N);
    out.extend_from_slice(&MAGIC);
    out.push(TVERSION);
    out.extend_from_slice(&seed.to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&h);
    for i in 0..hb {
        let mut b = 0u8;
        for k in 0..8 {
            let idx = i * 8 + k;
            if idx < n * n && holes[idx] {
                b |= 1 << k;
            }
        }
        out.push(b);
    }
    for w in &spl {
        out.extend_from_slice(w);
    }
    Ok(out)
}
/// Parses baked terrain bytes. `None` for a wrong header, a side past
/// `MAX_BAKE_SIZE` or a length that does not match the header, checked
/// before any allocation.
pub fn parse_terrain(b: &[u8]) -> Option<Terrain> {
    if b.len() < 15 || b[0..4] != MAGIC || b[4] != TVERSION {
        return None;
    }
    let seed = u64::from_le_bytes(b[5..13].try_into().ok()?);
    let size = u16::from_le_bytes(b[13..15].try_into().ok()?);
    if size > MAX_BAKE_SIZE {
        return None;
    }
    let n = usize::from(size);
    let hb = (n * n).div_ceil(8);
    if b.len() != 15 + n * n + hb + n * n * SPLAT_N {
        return None;
    }
    let heights = b[15..15 + n * n].to_vec();
    let mut holes = vec![false; n * n];
    for i in 0..n * n {
        holes[i] = ((b[15 + n * n + i / 8] >> (i % 8)) & 1) == 1;
    }
    let base = 15 + n * n + hb;
    let mut splat = vec![[0u8; 3]; n * n];
    for i in 0..n * n {
        splat[i] = [b[base + i * 3], b[base + i * 3 + 1], b[base + i * 3 + 2]];
    }
    Some(Terrain {
        seed,
        size,
        heights,
        holes,
        splat,
    })
}
// Blockers: holes + over-steep. Endpoints forced free.
pub fn blocked(t: &Terrain, limit: u8) -> Vec<bool> {
    let n = usize::from(t.size);
    let mut b = vec![false; n * n];
    for y in 0..n {
        for x in 0..n {
            let i = y * n + x;
            b[i] = t.holes[i] || slope_of(&t.heights, n, x, y) > limit;
        }
    }
    if n > 0 {
        b[0] = false;
        b[n * n - 1] = false;
    }
    b
}
pub fn terrain_hash(b: &[u8]) -> u64 {
    world_hash(b)
}
/// Hash of the terrain bake. Errors: [`BakeErr::TooLarge`] past `MAX_BAKE_SIZE`.
pub fn bake_hash(seed: u64, size: u16) -> Result<u64, BakeErr> {
    Ok(world_hash(&bake_terrain(seed, size)?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected_len(size: u16) -> usize {
        let n = usize::from(size);
        15 + n * n + (n * n).div_ceil(8) + n * n * SPLAT_N
    }

    #[test]
    fn byte_counts_match_the_layout() {
        // Hand-computed: 15 + 16 + 2 + 48 and 15 + 256 + 32 + 768.
        assert_eq!(bake_terrain(7, 4).expect("bakes").len(), 81);
        assert_eq!(bake_terrain(7, 16).expect("bakes").len(), 1_071);
        assert_eq!(
            bake_terrain(7, MAX_BAKE_SIZE).expect("bakes").len(),
            expected_len(MAX_BAKE_SIZE)
        );
    }

    #[test]
    fn oversize_bake_refuses_before_allocating() {
        assert_eq!(bake_terrain(7, MAX_BAKE_SIZE + 1), Err(BakeErr::TooLarge));
        assert_eq!(bake_terrain(7, u16::MAX), Err(BakeErr::TooLarge));
        assert_eq!(bake_hash(7, u16::MAX), Err(BakeErr::TooLarge));
    }

    #[test]
    fn forged_oversize_header_parses_to_none() {
        // A header that claims a huge side with a tiny body must not size
        // any buffer from the claim.
        for claimed in [MAX_BAKE_SIZE + 1, 512, u16::MAX] {
            let mut head = Vec::new();
            head.extend_from_slice(&MAGIC);
            head.push(TVERSION);
            head.extend_from_slice(&7u64.to_le_bytes());
            head.extend_from_slice(&claimed.to_le_bytes());
            assert_eq!(head.len(), 15);
            assert!(parse_terrain(&head).is_none());
        }
        let max = bake_terrain(7, MAX_BAKE_SIZE).expect("bakes");
        assert_eq!(
            parse_terrain(&max).expect("max side parses").size,
            MAX_BAKE_SIZE
        );
    }
}
