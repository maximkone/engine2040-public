//! grid (HC1, docs/SPRINT-34-hero-city.md): Manhattan as data. A 1.6 km square cut by
//! north-south avenues, east-west streets, one diagonal (Broadway) and one park; the blocks
//! split into lots, each with a height and a style. Presentation only, seeded and
//! deterministic. Units: meters; x runs east, z runs south, y is up.
//!
//! Winding: a footprint is counter-clockwise seen from above (normal +y). In the (x, z)
//! plane that is a negative shoelace sum, so `ccw_area` reports it positive.

use super::super::num::{f32_of, floor_idx, unit_f32};
use super::{Lot, RoadKind, RoadRect, Style};
use engine_core::hash::GOLDEN_GAMMA;
use engine_core::{mix64, splitmix64};

/// Side of the square the city fills, meters; it runs -800..800 on x and z.
pub(super) const SIDE_M: f32 = 1600.0;
/// Half of [`SIDE_M`], meters.
const HALF_SIDE_M: f32 = SIDE_M * 0.5;
/// Avenue centre spacing along x, meters.
pub(super) const AVENUE_SPACING_M: f32 = 260.0;
/// Avenue width, meters.
pub(super) const AVENUE_W_M: f32 = 30.0;
/// Avenues across the square (north-south roads).
const AVENUE_COUNT: usize = 7;
/// x of the westmost avenue centre, meters (the avenues are centred on x = 0).
const AVENUE_FIRST_M: f32 = -780.0;
/// Street centre spacing along z, meters.
pub(super) const STREET_SPACING_M: f32 = 80.0;
/// Street width, meters.
pub(super) const STREET_W_M: f32 = 18.0;
/// Streets down the square (east-west roads).
const STREET_COUNT: usize = 20;
/// z of the northmost street centre, meters (the streets are centred on z = 0).
const STREET_FIRST_M: f32 = -760.0;
/// Sidewalk between a road's edge and a lot, meters.
pub(super) const SIDEWALK_M: f32 = 5.0;
/// Narrowest lot along a block's long side (x), meters.
const LOT_MIN_W_M: f32 = 12.0;
/// Widest lot along a block's long side (x), meters.
const LOT_MAX_W_M: f32 = 40.0;
/// Smallest piece Broadway leaves of a lot that is kept, square meters.
const WEDGE_MIN_AREA_M2: f32 = 60.0;

/// Lowest valley base height, meters.
const VALLEY_MIN_M: f32 = 15.0;
/// Highest valley base height, meters.
const VALLEY_MAX_M: f32 = 40.0;
/// Lowest seeded jitter on a lot's height (a factor).
const JITTER_MIN: f32 = 0.6;
/// Highest seeded jitter on a lot's height (a factor).
const JITTER_MAX: f32 = 1.4;
/// Shortest building, meters.
const HEIGHT_MIN_M: f32 = 8.0;
/// Tallest building, meters.
pub(super) const HEIGHT_MAX_M: f32 = 380.0;
/// Radius at which a peak's bump has fallen to 1/e, meters.
const PEAK_RADIUS_M: f32 = 300.0;
/// Midtown's peak centre (x, z), meters: the north third.
const MIDTOWN: [f32; 2] = [330.0, -450.0];
/// Midtown's extra height at its centre before jitter, meters.
const MIDTOWN_AMP_M: f32 = 350.0;
/// Downtown's peak centre (x, z), meters: the south end.
const DOWNTOWN: [f32; 2] = [-200.0, 700.0];
/// Downtown's extra height at its centre before jitter, meters.
const DOWNTOWN_AMP_M: f32 = 300.0;
/// Heights under this are Brick, meters.
const BRICK_BELOW_M: f32 = 40.0;
/// Heights from BRICK_BELOW_M up to this are Stone, meters.
const STONE_BELOW_M: f32 = 100.0;
/// Heights from STONE_BELOW_M up to this are Stone or Deco, meters; above it Glass or Deco.
const GLASS_FROM_M: f32 = 150.0;

/// Broadway's width, meters.
const BROADWAY_W_M: f32 = 25.0;
/// x part of Broadway's unit normal: cos 30 degrees (Broadway runs 30 degrees off the avenues).
const BROADWAY_NX: f32 = 0.866_025_4;
/// z part of Broadway's unit normal: minus sin 30 degrees.
const BROADWAY_NZ: f32 = -0.5;
/// Broadway's centreline x per unit of z: tan 30 degrees.
const BROADWAY_TAN: f32 = 0.577_350_3;
/// Length along Broadway of each axis-aligned rect that approximates it, meters.
const BROADWAY_SEG_M: f32 = 5.0;

/// The first avenue block (counted from the west) the park covers.
const PARK_FIRST_X_BLOCK: usize = 1;
/// Avenue blocks the park is wide.
const PARK_X_BLOCKS: usize = 3;
/// The first street block (counted from the north) the park covers.
const PARK_FIRST_Z_BLOCK: usize = 1;
/// Street blocks the park is long.
const PARK_Z_BLOCKS: usize = 10;

/// The whole grid: its lots, its road rectangles and the park.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct NycGrid {
    /// Every lot, in build order.
    pub lots: Vec<Lot>,
    /// Road rectangles: the avenues, the streets and Broadway's approximating rects.
    pub roads: Vec<RoadRect>,
    /// The park as `([min x, min z], [max x, max z])`, meters; no lot touches it.
    pub park: ([f32; 2], [f32; 2]),
}

/// A seeded stream of words: each draw steps the state by the golden gamma and mixes it.
struct Stream(u64);

impl Stream {
    /// The next 64-bit word.
    fn word(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(GOLDEN_GAMMA);
        mix64(self.0)
    }

    /// The next draw, in `[0, 1)`.
    fn unit(&mut self) -> f32 {
        unit_f32(self.word())
    }
}

/// What the lot pass threads through the blocks.
struct Builder {
    /// The grid seed; each lot's seed is mixed from it and the lot's index.
    seed: u64,
    /// The stream the lot widths are drawn from.
    rng: Stream,
    /// The park box, meters; a lot with a corner in it is dropped.
    park: ([f32; 2], [f32; 2]),
    /// The lots so far.
    lots: Vec<Lot>,
}

impl NycGrid {
    /// The grid for `seed`: the same seed always gives the same lots, roads and park.
    pub(super) fn generate(seed: u64) -> NycGrid {
        let park = park_box();
        let mut builder = Builder {
            seed,
            rng: Stream(seed),
            park,
            lots: Vec::new(),
        };
        for k in 0..AVENUE_COUNT - 1 {
            for j in 0..STREET_COUNT - 1 {
                builder.add_block(k, j);
            }
        }
        NycGrid {
            lots: builder.lots,
            roads: road_rects(),
            park,
        }
    }

    /// Whether `(x, z)` is on a road: inside a road rect or inside Broadway's band.
    pub(super) fn is_road(&self, x: f32, z: f32) -> bool {
        let on_rect = self
            .roads
            .iter()
            .any(|r| (r.min[0]..=r.max[0]).contains(&x) && (r.min[1]..=r.max[1]).contains(&z));
        on_rect || band_offset([x, z]).abs() <= BROADWAY_W_M * 0.5
    }
}

impl Builder {
    /// Adds the lots of block `(k, j)`: two rows back to back across the block's middle, each
    /// row cut into lots along the block's long side (x), facing one of the two streets.
    fn add_block(&mut self, k: usize, j: usize) {
        let (x0, x1) = block_x(k);
        let (z0, z1) = block_z(j);
        let zm = (z0 + z1) * 0.5;
        for (za, zb) in [(z0, zm), (zm, z1)] {
            for (xa, xb) in self.split_row(x0, x1) {
                // Counter-clockwise from above: south-west, south-east, north-east, north-west.
                self.add_lot(vec![[xa, zb], [xb, zb], [xb, za], [xa, za]]);
            }
        }
    }

    /// Cuts the span `x0..x1` into lot widths in `LOT_MIN_W_M..=LOT_MAX_W_M`, returned as
    /// `(start, end)` spans. Every width but the last is drawn; the last takes what is left.
    fn split_row(&mut self, x0: f32, x1: f32) -> Vec<(f32, f32)> {
        let mut spans = Vec::new();
        let mut cursor = x0;
        let mut left = x1 - x0;
        while left > LOT_MAX_W_M {
            let room = (left - LOT_MIN_W_M).min(LOT_MAX_W_M) - LOT_MIN_W_M;
            let w = LOT_MIN_W_M + room * self.rng.unit();
            spans.push((cursor, cursor + w));
            cursor += w;
            left -= w;
        }
        if left >= LOT_MIN_W_M {
            spans.push((cursor, cursor + left));
        }
        spans
    }

    /// Adds a lot on `footprint`, unless it has a corner in the park or Broadway takes it whole.
    fn add_lot(&mut self, footprint: Vec<[f32; 2]>) {
        if footprint.iter().any(|p| in_box(*p, self.park)) {
            return;
        }
        let Some(footprint) = broadway_remnant(footprint) else {
            return;
        };
        let index = self.lots.len();
        let seed = mixed(self.seed, index);
        let (height_m, style) = massing(seed, footprint_centre(&footprint));
        self.lots.push(Lot {
            footprint,
            height_m,
            style,
            seed,
        });
    }
}

/// x span of avenue block `k` (between avenues k and k + 1), inset from both avenues.
fn block_x(k: usize) -> (f32, f32) {
    let inset = AVENUE_W_M * 0.5 + SIDEWALK_M;
    (avenue_x(k) + inset, avenue_x(k + 1) - inset)
}

/// z span of street block `j` (between streets j and j + 1), inset from both streets.
fn block_z(j: usize) -> (f32, f32) {
    let inset = STREET_W_M * 0.5 + SIDEWALK_M;
    (street_z(j) + inset, street_z(j + 1) - inset)
}

/// x of avenue `k`'s centre, meters.
fn avenue_x(k: usize) -> f32 {
    AVENUE_FIRST_M + f32_of(k) * AVENUE_SPACING_M
}

/// z of street `j`'s centre, meters.
fn street_z(j: usize) -> f32 {
    STREET_FIRST_M + f32_of(j) * STREET_SPACING_M
}

/// The park box, `([min x, min z], [max x, max z])`, meters: the avenue and street blocks it
/// covers, edge to edge. Its roads stay in the road list.
fn park_box() -> ([f32; 2], [f32; 2]) {
    let min = [
        avenue_x(PARK_FIRST_X_BLOCK) - AVENUE_W_M * 0.5,
        street_z(PARK_FIRST_Z_BLOCK) - STREET_W_M * 0.5,
    ];
    let max = [
        avenue_x(PARK_FIRST_X_BLOCK + PARK_X_BLOCKS) + AVENUE_W_M * 0.5,
        street_z(PARK_FIRST_Z_BLOCK + PARK_Z_BLOCKS) + STREET_W_M * 0.5,
    ];
    (min, max)
}

/// Every avenue, street and Broadway rect of the square.
fn road_rects() -> Vec<RoadRect> {
    let mut roads: Vec<RoadRect> = (0..AVENUE_COUNT)
        .map(|k| {
            let x = avenue_x(k);
            RoadRect {
                min: [x - AVENUE_W_M * 0.5, -HALF_SIDE_M],
                max: [x + AVENUE_W_M * 0.5, HALF_SIDE_M],
                kind: RoadKind::Avenue,
            }
        })
        .collect();
    roads.extend((0..STREET_COUNT).map(|j| {
        let z = street_z(j);
        RoadRect {
            min: [-HALF_SIDE_M, z - STREET_W_M * 0.5],
            max: [HALF_SIDE_M, z + STREET_W_M * 0.5],
            kind: RoadKind::Street,
        }
    }));
    roads.extend(broadway_rects());
    roads
}

/// Broadway as axis-aligned rects, one per `BROADWAY_SEG_M` of its length. Each is inscribed in
/// the band (its left edge at the band's steepest left point of the segment, its right edge at
/// the band's right point at the segment's start), so it never reaches past the band.
fn broadway_rects() -> Vec<RoadRect> {
    // Half the band's width measured along x at a fixed z.
    let half_x = BROADWAY_W_M * 0.5 / BROADWAY_NX;
    let count = floor_idx(SIDE_M / BROADWAY_SEG_M);
    (0..count)
        .map(|i| {
            let z0 = -HALF_SIDE_M + f32_of(i) * BROADWAY_SEG_M;
            let z1 = z0 + BROADWAY_SEG_M;
            RoadRect {
                min: [BROADWAY_TAN * z1 - half_x, z0],
                max: [BROADWAY_TAN * z0 + half_x, z1],
                kind: RoadKind::Broadway,
            }
        })
        .collect()
}

/// Signed offset of `p` from Broadway's centreline, meters: zero on it, positive on its
/// north-east side.
fn band_offset(p: [f32; 2]) -> f32 {
    BROADWAY_NX * p[0] + BROADWAY_NZ * p[1]
}

/// The part of a lot Broadway leaves: `None` when the lot's centre is in the band (the lot is
/// dropped); otherwise the larger side piece left once the band and its sidewalk are cut away.
fn broadway_remnant(footprint: Vec<[f32; 2]>) -> Option<Vec<[f32; 2]>> {
    if band_offset(footprint_centre(&footprint)).abs() <= BROADWAY_W_M * 0.5 {
        return None;
    }
    let clear = BROADWAY_W_M * 0.5 + SIDEWALK_M;
    let north_east = clip_side(&footprint, 1.0, clear);
    let south_west = clip_side(&footprint, -1.0, clear);
    [north_east, south_west]
        .into_iter()
        .filter(|piece| piece.len() >= 3 && ccw_area(piece) >= WEDGE_MIN_AREA_M2)
        .max_by(|a, b| ccw_area(a).total_cmp(&ccw_area(b)))
}

/// `poly` cut to the side where `sign * band_offset(p) >= offset`, by one line
/// (Sutherland-Hodgman). Corners keep their winding.
fn clip_side(poly: &[[f32; 2]], sign: f32, offset: f32) -> Vec<[f32; 2]> {
    let g = |p: [f32; 2]| sign * band_offset(p) - offset;
    let mut out = Vec::with_capacity(poly.len() + 1);
    for (a, b) in poly.iter().zip(poly.iter().cycle().skip(1)) {
        let (ga, gb) = (g(*a), g(*b));
        if ga >= 0.0 {
            out.push(*a);
        }
        if (ga < 0.0 && gb > 0.0) || (ga > 0.0 && gb < 0.0) {
            let t = ga / (ga - gb);
            out.push([a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])]);
        }
    }
    out
}

/// Area of `poly` in square meters, positive when counter-clockwise seen from above.
fn ccw_area(poly: &[[f32; 2]]) -> f32 {
    let mut twice = 0.0_f32;
    for (a, b) in poly.iter().zip(poly.iter().cycle().skip(1)) {
        twice += a[0] * b[1] - b[0] * a[1];
    }
    -0.5 * twice
}

/// The mean of a footprint's corners `[x, z]`, meters; the origin for none.
fn footprint_centre(poly: &[[f32; 2]]) -> [f32; 2] {
    if poly.is_empty() {
        return [0.0; 2];
    }
    let n = f32_of(poly.len());
    let (sx, sz) = poly
        .iter()
        .fold((0.0_f32, 0.0_f32), |(sx, sz), p| (sx + p[0], sz + p[1]));
    [sx / n, sz / n]
}

/// Whether `p` lies in the box `min..=max`.
fn in_box(p: [f32; 2], (min, max): ([f32; 2], [f32; 2])) -> bool {
    (min[0]..=max[0]).contains(&p[0]) && (min[1]..=max[1]).contains(&p[1])
}

/// Height (meters) and style of a lot centred at `c`, from the lot's own seed: a valley base,
/// the two peaks' bumps, then a seeded jitter, clamped to the height range.
fn massing(seed: u64, c: [f32; 2]) -> (f32, Style) {
    let mut rng = Stream(seed);
    let base = VALLEY_MIN_M + (VALLEY_MAX_M - VALLEY_MIN_M) * rng.unit();
    let jitter = JITTER_MIN + (JITTER_MAX - JITTER_MIN) * rng.unit();
    let bump = peak_bump(c, MIDTOWN, MIDTOWN_AMP_M) + peak_bump(c, DOWNTOWN, DOWNTOWN_AMP_M);
    let height_m = ((base + bump) * jitter).clamp(HEIGHT_MIN_M, HEIGHT_MAX_M);
    (height_m, style_for(height_m, rng.unit()))
}

/// The extra height (meters) a peak of amplitude `amp_m` adds at `c`: a Gaussian-like falloff
/// with `PEAK_RADIUS_M` as its e-folding radius.
fn peak_bump(c: [f32; 2], peak: [f32; 2], amp_m: f32) -> f32 {
    let (dx, dz) = (c[0] - peak[0], c[1] - peak[1]);
    amp_m * (-(dx * dx + dz * dz) / (PEAK_RADIUS_M * PEAK_RADIUS_M)).exp()
}

/// The style of a lot of `height_m` with the draw `r` in `[0, 1)`: Brick in the low valley,
/// Stone mid-height, Deco now and then from 100 m, Glass mostly above 150 m.
fn style_for(height_m: f32, r: f32) -> Style {
    if height_m < BRICK_BELOW_M {
        Style::Brick
    } else if height_m < STONE_BELOW_M {
        Style::Stone
    } else if height_m < GLASS_FROM_M {
        if r < 0.3 {
            Style::Deco
        } else {
            Style::Stone
        }
    } else if r < 0.7 {
        Style::Glass
    } else {
        Style::Deco
    }
}

/// The seed of salt `salt` under `seed`: SplitMix64 of the grid seed mixed with the salt.
fn mixed(seed: u64, salt: usize) -> u64 {
    splitmix64(seed.wrapping_add(GOLDEN_GAMMA.wrapping_mul(word_of(salt))))
}

/// `n` as a word (saturating; every count here is far below the limit).
fn word_of(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Distance between two `[x, z]` points, meters.
    fn dist(a: [f32; 2], b: [f32; 2]) -> f32 {
        ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
    }

    /// The mean of `values`; zero for none.
    fn mean(values: &[f32]) -> f32 {
        if values.is_empty() {
            return 0.0;
        }
        values.iter().sum::<f32>() / f32_of(values.len())
    }

    /// Whether `v` is within a millimeter of `edge`.
    fn on_edge(v: f32, edge: f32) -> bool {
        (v - edge).abs() < 1e-3
    }

    /// Whether `poly` is a four-corner axis-aligned rectangle; its min and max x are then
    /// its width's ends.
    fn is_axis_rect(poly: &[[f32; 2]]) -> bool {
        let min_x = poly.iter().map(|p| p[0]).fold(f32::INFINITY, f32::min);
        let max_x = poly.iter().map(|p| p[0]).fold(f32::NEG_INFINITY, f32::max);
        let min_z = poly.iter().map(|p| p[1]).fold(f32::INFINITY, f32::min);
        let max_z = poly.iter().map(|p| p[1]).fold(f32::NEG_INFINITY, f32::max);
        poly.len() == 4
            && poly.iter().all(|p| {
                (on_edge(p[0], min_x) || on_edge(p[0], max_x))
                    && (on_edge(p[1], min_z) || on_edge(p[1], max_z))
            })
    }

    #[test]
    fn the_same_seed_gives_the_same_lots() {
        assert_eq!(NycGrid::generate(7), NycGrid::generate(7));
    }

    #[test]
    fn another_seed_gives_other_lots() {
        assert_ne!(NycGrid::generate(7).lots, NycGrid::generate(8).lots);
    }

    #[test]
    fn no_lot_corner_is_on_a_road_or_in_the_park() {
        let grid = NycGrid::generate(7);
        for lot in &grid.lots {
            for p in &lot.footprint {
                assert!(!grid.is_road(p[0], p[1]), "corner {p:?} is on a road");
                assert!(!in_box(*p, grid.park), "corner {p:?} is in the park");
            }
        }
    }

    #[test]
    fn every_corner_is_inside_the_square() {
        for lot in &NycGrid::generate(7).lots {
            for p in &lot.footprint {
                assert!(
                    p[0].abs() <= HALF_SIDE_M && p[1].abs() <= HALF_SIDE_M,
                    "{p:?}"
                );
            }
        }
    }

    #[test]
    fn the_lot_count_is_in_range() {
        let n = NycGrid::generate(7).lots.len();
        assert!((800..=3000).contains(&n), "{n} lots");
    }

    #[test]
    fn every_footprint_is_a_ccw_polygon_with_area() {
        for lot in &NycGrid::generate(7).lots {
            assert!(lot.footprint.len() >= 3, "{:?}", lot.footprint);
            assert!(
                ccw_area(&lot.footprint) >= WEDGE_MIN_AREA_M2,
                "{:?}",
                lot.footprint
            );
        }
    }

    #[test]
    fn unclipped_lots_are_12_to_40_m_wide() {
        let mut checked = 0;
        for lot in &NycGrid::generate(7).lots {
            if !is_axis_rect(&lot.footprint) {
                continue;
            }
            checked += 1;
            let min_x = lot
                .footprint
                .iter()
                .map(|p| p[0])
                .fold(f32::INFINITY, f32::min);
            let max_x = lot
                .footprint
                .iter()
                .map(|p| p[0])
                .fold(f32::NEG_INFINITY, f32::max);
            let width = max_x - min_x;
            assert!(
                (LOT_MIN_W_M - 1e-3..=LOT_MAX_W_M + 1e-3).contains(&width),
                "width {width} at {:?}",
                lot.footprint
            );
        }
        assert!(checked > 500, "only {checked} unclipped lots");
    }

    #[test]
    fn heights_stay_in_the_range() {
        for lot in &NycGrid::generate(7).lots {
            assert!(
                (HEIGHT_MIN_M..=HEIGHT_MAX_M).contains(&lot.height_m),
                "{}",
                lot.height_m
            );
        }
    }

    #[test]
    fn the_peaks_are_taller_than_the_valley_between_them() {
        let grid = NycGrid::generate(7);
        let valley: Vec<f32> = grid
            .lots
            .iter()
            .filter(|l| {
                let c = footprint_centre(&l.footprint);
                c[1] > MIDTOWN[1]
                    && c[1] < DOWNTOWN[1]
                    && dist(c, MIDTOWN) > 400.0
                    && dist(c, DOWNTOWN) > 400.0
            })
            .map(|l| l.height_m)
            .collect();
        assert!(!valley.is_empty());
        let valley_mean = mean(&valley);
        for peak in [MIDTOWN, DOWNTOWN] {
            let near: Vec<f32> = grid
                .lots
                .iter()
                .filter(|l| dist(footprint_centre(&l.footprint), peak) <= 250.0)
                .map(|l| l.height_m)
                .collect();
            assert!(!near.is_empty(), "no lots near {peak:?}");
            assert!(
                mean(&near) > valley_mean,
                "peak {peak:?} mean {} <= valley mean {valley_mean}",
                mean(&near)
            );
        }
    }
}
