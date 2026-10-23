//! terrain: heightmap + hole bitmask + splat weights, seeded. Layout: MAGIC(4) TVER(1) seed(8LE) size(2LE) + heights + holebits + splat.
//! The bake, its writer ([`write_terrain`]), its parser and its bounded
//! painter ([`paint_terrain`]) share one layout owner, so a writer after a
//! parser is byte-identical.
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
    Ok(pack_terrain(&bake_data(seed, size)))
}

// Bake the height grid, its holes and its splat; the packer owns the bytes.
fn bake_data(seed: u64, size: u16) -> Terrain {
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
    Terrain {
        seed,
        size,
        heights: h,
        holes,
        splat: spl,
    }
}

// Canonical TVER bytes: zero hole padding, every vector read by index so a
// short hand-built terrain still packs to bytes the parser accepts.
fn pack_terrain(t: &Terrain) -> Vec<u8> {
    let n = usize::from(t.size);
    let cells = n * n;
    let hb = cells.div_ceil(8);
    let mut out = Vec::with_capacity(15 + cells + hb + cells * SPLAT_N);
    out.extend_from_slice(&MAGIC);
    out.push(TVERSION);
    out.extend_from_slice(&t.seed.to_le_bytes());
    out.extend_from_slice(&t.size.to_le_bytes());
    out.extend((0..cells).map(|i| t.heights.get(i).copied().unwrap_or(0)));
    for i in 0..hb {
        let mut b = 0u8;
        for k in 0..8 {
            let idx = i * 8 + k;
            if idx < cells && t.holes.get(idx).copied().unwrap_or(false) {
                b |= 1 << k;
            }
        }
        out.push(b);
    }
    for i in 0..cells {
        out.extend_from_slice(&t.splat.get(i).copied().unwrap_or([0; 3]));
    }
    out
}

/// Writes the canonical TVER bytes for one terrain: exactly the layout
/// [`parse_terrain`] accepts, so a write after a parse is byte-identical to
/// the bytes parsed. Units: texels in, bytes out. A heights, holes or splat
/// vector shorter than `size * size` reads as `0`, `false` or `[0, 0, 0]`; a
/// longer one is cut, so even a hand-built terrain writes parseable bytes.
/// Errors: [`BakeErr::TooLarge`] past [`MAX_BAKE_SIZE`], checked before any
/// allocation.
pub fn write_terrain(t: &Terrain) -> Result<Vec<u8>, BakeErr> {
    if t.size > MAX_BAKE_SIZE {
        return Err(BakeErr::TooLarge);
    }
    Ok(pack_terrain(t))
}

/// One bounded terrain edit for [`paint_terrain`]: a texel rect, a height
/// delta, and optional hole and splat writes. Edits apply in the order given,
/// so two edits over one cell see each other's result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edit {
    /// Rect as `[x, y, w, h]` in texels, with `w` and `h` at least 1 and the
    /// whole rect inside the map. A rect outside the map is refused, never
    /// clamped.
    pub rect: [u16; 4],
    /// Added to every height in the rect, then clamped to `0..=255`. `0`
    /// leaves heights alone.
    pub height_delta: i16,
    /// `Some` sets or clears every hole bit in the rect; `None` leaves them.
    pub hole: Option<bool>,
    /// `Some` sets the three splat weights in the rect. Weights must sum to
    /// exactly 255 (the bake's own sum rule); `None` leaves splat alone.
    pub splat: Option<[u8; 3]>,
}

/// Why [`paint_terrain`] refused. Every variant carries the edit index and
/// nothing changes on refusal, so the caller keeps a pristine terrain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaintErr {
    /// The edit's rect falls outside the map or has a zero side.
    Rect {
        /// Index of the refused edit in the request slice.
        edit: usize,
        /// The refused rect as given.
        rect: [u16; 4],
        /// The map side the rect must fit in.
        size: u16,
    },
    /// The edit's splat weights do not sum to 255.
    Splat {
        /// Index of the refused edit in the request slice.
        edit: usize,
        /// The refused weights as given.
        weights: [u8; 3],
    },
    /// The terrain's vectors are shorter than `size * size`.
    Shape,
}

impl std::fmt::Display for PaintErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Rect { edit, rect, size } => write!(
                f,
                "edit {edit} rect [{}, {}, {}, {}] is outside the {size} x {size} map (each \
                 coordinate 0..={}); keep x + w <= {size} and y + h <= {size}",
                rect[0],
                rect[1],
                rect[2],
                rect[3],
                size.saturating_sub(1)
            ),
            Self::Splat { edit, weights } => {
                let total: u32 = weights.iter().map(|w| u32::from(*w)).sum();
                write!(
                    f,
                    "edit {edit} splat weights [{}, {}, {}] sum to {total}; the three weights must \
                     sum to exactly 255",
                    weights[0], weights[1], weights[2]
                )
            }
            Self::Shape => write!(
                f,
                "terrain vectors are shorter than size * size; parse or bake the terrain first"
            ),
        }
    }
}

impl std::error::Error for PaintErr {}

// Every edit must be legal before the first cell moves.
fn check_edits(size: u16, edits: &[Edit]) -> Result<(), PaintErr> {
    for (edit, item) in edits.iter().enumerate() {
        let [x, y, w, h] = item.rect;
        let fits = |at: u16, len: u16| u32::from(at) + u32::from(len) <= u32::from(size);
        if w == 0 || h == 0 || !fits(x, w) || !fits(y, h) {
            return Err(PaintErr::Rect {
                edit,
                rect: item.rect,
                size,
            });
        }
        if let Some(weights) = item.splat {
            let total: u32 = weights.iter().map(|w| u32::from(*w)).sum();
            if total != 255 {
                return Err(PaintErr::Splat { edit, weights });
            }
        }
    }
    Ok(())
}

/// Applies bounded edits to a parsed or baked terrain and answers how many
/// distinct cells changed. Edits apply in the order given; a height delta is
/// clamped to `0..=255`; a cell counts once however many edits touch it.
/// Units: texels in, changed cell count out. Errors: [`PaintErr::Rect`],
/// [`PaintErr::Splat`] or [`PaintErr::Shape`], each checked before the first
/// cell moves, so a refused request leaves the terrain untouched.
pub fn paint_terrain(t: &mut Terrain, edits: &[Edit]) -> Result<usize, PaintErr> {
    let n = usize::from(t.size);
    let cells = n * n;
    if t.heights.len() < cells || t.holes.len() < cells || t.splat.len() < cells {
        return Err(PaintErr::Shape);
    }
    check_edits(t.size, edits)?;
    let mut changed = vec![false; cells];
    for item in edits {
        let [x, y, w, h] = item.rect;
        for cy in usize::from(y)..usize::from(y) + usize::from(h) {
            for cx in usize::from(x)..usize::from(x) + usize::from(w) {
                let i = cy * n + cx;
                let mut moved = false;
                if item.height_delta != 0 {
                    // The clamp proves the range, so the narrowing never fails.
                    let clamped = (i16::from(t.heights[i]) + item.height_delta).clamp(0, 255);
                    let next = u8::try_from(clamped).unwrap_or(0);
                    if next != t.heights[i] {
                        t.heights[i] = next;
                        moved = true;
                    }
                }
                if let Some(hole) = item.hole {
                    if t.holes[i] != hole {
                        t.holes[i] = hole;
                        moved = true;
                    }
                }
                if let Some(weights) = item.splat {
                    if t.splat[i] != weights {
                        t.splat[i] = weights;
                        moved = true;
                    }
                }
                if moved {
                    changed[i] = true;
                }
            }
        }
    }
    Ok(changed.iter().filter(|seen| **seen).count())
}

/// Integer summary of one terrain, as the door reports it. Units: heights are
/// texel steps `0..=255`, `height_mean_x100` is hundredths of a height step
/// (truncated), `splat_coverage` is permille of texels per layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TerrainStats {
    /// Lowest height in the map.
    pub height_min: u8,
    /// Highest height in the map.
    pub height_max: u8,
    /// Mean height times 100, truncated.
    pub height_mean_x100: u32,
    /// Texels with their hole bit set.
    pub holes: u32,
    /// Per-layer permille of texels where that layer carries the largest
    /// weight; a tie goes to the lower layer, and truncation keeps the three
    /// at or under 1000.
    pub splat_coverage: [u32; 3],
    /// Cells [`blocked`] refuses at [`SLOPE_DEFAULT`].
    pub blocked_cells: u32,
}

/// Summarises one terrain over its `size * size` cells. An empty map reports
/// zeros. Units and rules ride on [`TerrainStats`]; no clock, no RNG, so the
/// same terrain always answers the same numbers.
pub fn terrain_stats(t: &Terrain) -> TerrainStats {
    let n = usize::from(t.size);
    let cells = n * n;
    let (mut min, mut max, mut sum) = (u8::MAX, 0u8, 0u64);
    let (mut holes, mut coverage) = (0u32, [0u32; 3]);
    for i in 0..cells {
        let h = t.heights.get(i).copied().unwrap_or(0);
        min = min.min(h);
        max = max.max(h);
        sum += u64::from(h);
        if t.holes.get(i).copied().unwrap_or(false) {
            holes += 1;
        }
        let weights = t.splat.get(i).copied().unwrap_or([0; 3]);
        let mut best = 0usize;
        for (layer, weight) in weights.iter().enumerate().skip(1) {
            if *weight > weights[best] {
                best = layer;
            }
        }
        coverage[best] += 1;
    }
    let mean = if cells == 0 {
        0
    } else {
        let count = u64::try_from(cells).unwrap_or(u64::MAX);
        u32::try_from(sum * 100 / count).unwrap_or(u32::MAX)
    };
    let covered = u64::try_from(cells).unwrap_or(u64::MAX);
    let mut permille = [0u32; 3];
    if cells > 0 {
        for (out, count) in permille.iter_mut().zip(coverage) {
            *out = u32::try_from(u64::from(count) * 1000 / covered).unwrap_or(u32::MAX);
        }
    }
    let blocked_cells = blocked(t, SLOPE_DEFAULT)
        .iter()
        .filter(|cell| **cell)
        .count();
    TerrainStats {
        height_min: if cells == 0 { 0 } else { min },
        height_max: max,
        height_mean_x100: mean,
        holes,
        splat_coverage: permille,
        blocked_cells: u32::try_from(blocked_cells).unwrap_or(u32::MAX),
    }
}
/// Parses baked terrain bytes. `None` for a wrong header, a side past
/// `MAX_BAKE_SIZE`, a length that does not match the header, or non-zero
/// hole padding bits, checked before any allocation.
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
    let hole_bytes = &b[15 + n * n..15 + n * n + hb];
    for i in n * n..hb * 8 {
        if ((hole_bytes[i / 8] >> (i % 8)) & 1) == 1 {
            return None;
        }
    }
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
    fn splat_weights_sum_to_255_on_every_corner() {
        // Pins the `splat_for` contract through its narrowing conversions:
        // each weight fits `u8` and the three sum to exactly 255, so the
        // remainder fix-up never pushes a channel past the byte.
        for hi in [0u8, 1, 64, 127, 128, 200, 254, 255] {
            for slope in [0u8, 1, 63, 64, 65, 200, 255] {
                let weights = splat_for(hi, slope);
                let total: u32 = weights.iter().map(|w| u32::from(*w)).sum();
                assert_eq!(
                    total, 255,
                    "splat weights must sum to 255 for hi={hi} slope={slope}"
                );
            }
        }
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

    #[test]
    fn hole_padding_bits_must_be_zero() {
        // A 5x5 bake holds 25 hole bits in 4 bytes, so 7 padding bits stay
        // zero in canonical bytes; a blob with padding set must not parse.
        let mut bytes = bake_terrain(7, 5).expect("bakes");
        assert_eq!(bytes.len(), 15 + 25 + 4 + 25 * SPLAT_N);
        let hole_end = 15 + 25 + 4;
        let last = hole_end - 1;
        bytes[last] |= 0b1110_0000;
        assert!(
            parse_terrain(&bytes).is_none(),
            "non-zero hole padding must not parse"
        );
    }

    #[test]
    fn hole_padding_keeps_neighbours_live() {
        // 5x5 packs 25 live hole bits into 4 bytes: index 24 is the last
        // live bit and index 25 is the first padding bit. The padding
        // check must keep the neighbour live while rejecting padding.
        let bytes = bake_terrain(7, 5).expect("bakes");
        let hole_at = 15 + 25;
        let mut live = bytes.clone();
        live[hole_at + 3] |= 1 << 0;
        let parsed = parse_terrain(&live).expect("neighbour stays live");
        assert!(parsed.holes[24]);
        live[hole_at + 3] |= 1 << 1;
        assert!(parse_terrain(&live).is_none());
    }

    #[test]
    fn padded_hole_bytes_stay_byte_identical() {
        // Canonical hole bytes carry zero padding; parsing then re-packing
        // with zero padding must give byte-identical hole bytes.
        for size in [4u16, 5, 9] {
            let bytes = bake_terrain(7, size).expect("bakes");
            let parsed = parse_terrain(&bytes).expect("canonical parses");
            let n = usize::from(size) * usize::from(size);
            let hb = n.div_ceil(8);
            let hole_bytes = &bytes[15 + n..15 + n + hb];
            let mut repacked = vec![0u8; hb];
            for (i, live) in parsed.holes.iter().enumerate() {
                if *live {
                    repacked[i / 8] |= 1 << (i % 8);
                }
            }
            assert_eq!(
                repacked.as_slice(),
                hole_bytes,
                "hole bytes differ for size {size}"
            );
        }
    }

    #[test]
    fn writer_rebakes_byte_equal_for_two_seeds() {
        // The writer and the parser share one packer: bytes parsed and
        // written again equal the bake for more than one seed.
        for seed in [7u64, 2040] {
            let baked = bake_terrain(seed, 17).expect("bakes");
            let parsed = parse_terrain(&baked).expect("baked parses");
            let written = write_terrain(&parsed).expect("writes");
            assert_eq!(written, baked, "seed {seed} writes the baked bytes");
            let reparsed = parse_terrain(&written).expect("written parses");
            assert_eq!(reparsed, parsed, "seed {seed} round-trips");
        }
        // A short hand-built terrain still writes bytes the parser accepts.
        let short = Terrain {
            seed: 7,
            size: 4,
            heights: vec![9, 9],
            holes: Vec::new(),
            splat: Vec::new(),
        };
        let bytes = write_terrain(&short).expect("writes");
        let parsed = parse_terrain(&bytes).expect("short terrain parses");
        assert_eq!(parsed.heights[..2], [9, 9]);
        assert_eq!(parsed.heights[2], 0);
        assert!(!parsed.holes[0]);
        assert_eq!(parsed.splat[0], [0, 0, 0]);
        // The writer refuses a side it could never make parseable.
        let huge = Terrain {
            seed: 7,
            size: MAX_BAKE_SIZE + 1,
            heights: Vec::new(),
            holes: Vec::new(),
            splat: Vec::new(),
        };
        assert_eq!(write_terrain(&huge), Err(BakeErr::TooLarge));
    }

    fn baked_terrain(seed: u64, size: u16) -> Terrain {
        parse_terrain(&bake_terrain(seed, size).expect("bakes")).expect("baked parses")
    }

    #[test]
    fn paint_moves_heights_holes_and_splat_and_counts_cells() {
        let mut t = baked_terrain(7, 16);
        let before = t.clone();
        let edits = [
            Edit {
                rect: [0, 0, 4, 4],
                height_delta: 1000,
                hole: Some(true),
                splat: Some([255, 0, 0]),
            },
            // The second edit sees the first one's result and wins on the
            // overlapping cell.
            Edit {
                rect: [3, 3, 1, 1],
                height_delta: -1000,
                hole: None,
                splat: None,
            },
        ];
        let changed = paint_terrain(&mut t, &edits).expect("edits apply");
        assert_eq!(changed, 16, "16 distinct cells moved");
        for y in 0..4 {
            for x in 0..4 {
                let i = y * 16 + x;
                assert!(t.holes[i], "cell {x},{y} is a hole");
                assert_eq!(t.splat[i], [255, 0, 0], "cell {x},{y} is sand");
                let expected = if (x, y) == (3, 3) { 0 } else { 255 };
                assert_eq!(t.heights[i], expected, "cell {x},{y} clamps");
            }
        }
        assert_eq!(
            t.heights[16 * 4],
            before.heights[16 * 4],
            "outside the rect"
        );
        // Nothing else moved.
        for i in 16..16 * 16 {
            if i / 16 >= 4 || i % 16 >= 4 {
                assert_eq!(t.heights[i], before.heights[i]);
                assert_eq!(t.holes[i], before.holes[i]);
                assert_eq!(t.splat[i], before.splat[i]);
            }
        }
        // The same edits on a fresh bake give byte-equal terrain, twice.
        let mut again = baked_terrain(7, 16);
        paint_terrain(&mut again, &edits).expect("edits apply");
        assert_eq!(
            write_terrain(&again).expect("writes"),
            write_terrain(&t).expect("writes")
        );
        // An edit that changes nothing counts no cell.
        assert_eq!(
            paint_terrain(
                &mut t,
                &[Edit {
                    rect: [0, 0, 1, 1],
                    height_delta: 0,
                    hole: None,
                    splat: None,
                }]
            )
            .expect("no-op applies"),
            0
        );
    }

    #[test]
    fn paint_refuses_bad_edits_before_any_cell_moves() {
        let mut t = baked_terrain(7, 8);
        let before = t.clone();
        let cases = [
            (
                Edit {
                    rect: [7, 7, 2, 2],
                    height_delta: 1,
                    hole: None,
                    splat: None,
                },
                "off the map",
            ),
            (
                Edit {
                    rect: [0, 0, 0, 4],
                    height_delta: 1,
                    hole: None,
                    splat: None,
                },
                "zero width",
            ),
            (
                Edit {
                    rect: [0, 0, 2, 2],
                    height_delta: 1,
                    hole: None,
                    splat: Some([1, 1, 1]),
                },
                "weights that miss 255",
            ),
        ];
        for (edit, why) in cases {
            let refused = paint_terrain(&mut t, &[edit]).expect_err(why);
            assert!(refused.to_string().contains("edit 0"), "{refused}");
            assert_eq!(t, before, "a refused {why} moves nothing");
        }
        // The messages name the legal range and the one recovery.
        let off = paint_terrain(
            &mut t,
            &[Edit {
                rect: [7, 7, 2, 2],
                height_delta: 1,
                hole: None,
                splat: None,
            }],
        )
        .expect_err("refuses");
        assert!(off.to_string().contains("0..=7"), "{off}");
        assert!(off.to_string().contains("x + w <= 8"), "{off}");
        let sum = paint_terrain(
            &mut t,
            &[Edit {
                rect: [0, 0, 1, 1],
                height_delta: 0,
                hole: None,
                splat: Some([1, 1, 1]),
            }],
        )
        .expect_err("refuses");
        assert!(sum.to_string().contains("sum to 3"), "{sum}");
        assert!(sum.to_string().contains("exactly 255"), "{sum}");
        // A terrain whose vectors do not match its side refuses too.
        let mut short = t.clone();
        short.heights.truncate(3);
        assert_eq!(
            paint_terrain(
                &mut short,
                &[Edit {
                    rect: [0, 0, 1, 1],
                    height_delta: 1,
                    hole: None,
                    splat: None,
                }]
            ),
            Err(PaintErr::Shape)
        );
    }

    #[test]
    fn terrain_stats_are_integer_and_bounded() {
        let t = baked_terrain(2040, 32);
        let stats = terrain_stats(&t);
        assert!(stats.height_min <= stats.height_max);
        assert!(u32::from(stats.height_max) <= 255);
        assert!(stats.height_mean_x100 <= 255 * 100);
        let holes = t.holes.iter().filter(|hole| **hole).count();
        assert_eq!(stats.holes, u32::try_from(holes).expect("count fits"));
        let coverage: u32 = stats.splat_coverage.iter().sum();
        assert!(coverage <= 1000, "coverage permille: {coverage}");
        let blocked_cells = blocked(&t, SLOPE_DEFAULT).iter().filter(|b| **b).count();
        assert_eq!(
            stats.blocked_cells,
            u32::try_from(blocked_cells).expect("count fits")
        );
        // Deterministic: the same terrain answers the same numbers.
        assert_eq!(terrain_stats(&t), stats);
        // An empty map reports zeros instead of a wrapped minimum.
        let empty = Terrain {
            seed: 1,
            size: 0,
            heights: Vec::new(),
            holes: Vec::new(),
            splat: Vec::new(),
        };
        let none = terrain_stats(&empty);
        assert_eq!(none.height_min, 0);
        assert_eq!(none.height_max, 0);
        assert_eq!(none.splat_coverage, [0, 0, 0]);
    }
}
