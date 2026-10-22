//! city (SD3, docs/SPRINT-33-speed-demo.md). A seeded generator places 40 to 120 box
//! towers on the flat spots of the ground, each 8 to 120 m tall, with a colour per tower.
//! Pure and deterministic: the same seed gives the same towers, no window and no sim. The
//! ground is a `height_at(x, z)` closure, the same as the movers take in `speed_demo/mod.rs`.
//! Units: meters.

use engine_core::hash::GOLDEN_GAMMA;
use engine_core::splitmix64;
use engine_destruction::csg::PropMesh;

use super::num::{below, unit_f32};

/// Fewest towers one city places.
const MIN_TOWERS: usize = 40;
/// Most towers one city places.
const MAX_TOWERS: usize = 120;
/// Shortest visible tower, from its lowest ground corner up to the roof; meters.
const MIN_HEIGHT_M: f32 = 8.0;
/// Tallest visible tower, from its lowest ground corner up to the roof; meters.
const MAX_HEIGHT_M: f32 = 120.0;
/// Smallest footprint side; meters.
const MIN_FOOTPRINT_M: f32 = 6.0;
/// Largest footprint side; meters.
const MAX_FOOTPRINT_M: f32 = 30.0;
/// Most the ground may vary across one footprint, sampled at its corners and centre; meters.
const MAX_GROUND_SPREAD_M: f32 = 2.0;
/// Clear ground kept between two footprints; meters.
const GAP_M: f32 = 4.0;
/// How far a base sinks below its lowest ground corner, so no tower floats; meters.
const BASE_SINK_M: f32 = 1.0;
/// Placement attempts before the city stops short of its target; bounds the search.
const PLACE_TRIES: usize = 4_000;
/// Triangles in one tower box: six faces, two triangles each.
const TOWER_TRIS: usize = 12;
/// Triangle budget for the uncut city; the cut (SD4) splits towers, so SD7 counts pieces
/// against the frame budget on top of this.
pub(super) const TOWER_TRI_BUDGET: usize = 2_048;
/// Concrete, sandstone and glass tones, linear RGBA.
const PALETTE: [[f32; 4]; 4] = [
    [0.62, 0.60, 0.57, 1.0],
    [0.78, 0.77, 0.74, 1.0],
    [0.80, 0.66, 0.48, 1.0],
    [0.45, 0.58, 0.66, 1.0],
];

/// One box tower: an axis-aligned box in meters and a linear RGBA colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Tower {
    /// Box minimum corner; meters. `min[1]` is the base, which sits under the ground.
    pub min: [f32; 3],
    /// Box maximum corner; meters. `max[1]` is the roof.
    pub max: [f32; 3],
    /// Linear RGBA colour, one of the palette tones.
    pub colour: [f32; 4],
}

/// SplitMix64 over engine-core's step: a tiny seeded generator, enough to scatter towers.
struct SplitMix64(u64);

impl SplitMix64 {
    fn next_u64(&mut self) -> u64 {
        let out = splitmix64(self.0);
        self.0 = self.0.wrapping_add(GOLDEN_GAMMA);
        out
    }

    /// Uniform in [0, 1): 24 bits, so every value is exact in f32.
    fn unit(&mut self) -> f32 {
        unit_f32(self.next_u64())
    }
}

/// Footprint as `[x0, x1, z0, z1]`; meters.
fn footprint(t: &Tower) -> [f32; 4] {
    [t.min[0], t.max[0], t.min[2], t.max[2]]
}

/// Clear ground between two footprints: the largest gap along either axis. Negative when
/// they overlap on both axes; meters.
fn gap_m(a: [f32; 4], b: [f32; 4]) -> f32 {
    (b[0] - a[1])
        .max(a[0] - b[1])
        .max(b[2] - a[3])
        .max(a[2] - b[3])
}

/// Places the city on `ground`. Each tower has a random footprint and height, is rejected
/// when its corners and centre spread more than `MAX_GROUND_SPREAD_M` or it comes within
/// `GAP_M` of another tower, and sits on its lowest corner minus `BASE_SINK_M`. The
/// footprints stay within `+-half_extent_m` of the origin.
pub(super) fn place_towers(
    seed: u64,
    half_extent_m: f32,
    ground: &dyn Fn(f32, f32) -> f32,
) -> Vec<Tower> {
    let mut rng = SplitMix64(seed);
    let target = MIN_TOWERS + below(rng.next_u64(), MAX_TOWERS - MIN_TOWERS + 1);
    let mut towers: Vec<Tower> = Vec::with_capacity(target);
    for _ in 0..PLACE_TRIES {
        if towers.len() == target {
            break;
        }
        let side = MIN_FOOTPRINT_M + rng.unit() * (MAX_FOOTPRINT_M - MIN_FOOTPRINT_M);
        let height = MIN_HEIGHT_M + rng.unit() * (MAX_HEIGHT_M - MIN_HEIGHT_M);
        let reach = half_extent_m - side * 0.5;
        let cx = (rng.unit() * 2.0 - 1.0) * reach;
        let cz = (rng.unit() * 2.0 - 1.0) * reach;
        let colour = PALETTE[below(rng.next_u64(), PALETTE.len())];
        if reach <= 0.0 {
            continue;
        }
        let (x0, x1) = (cx - side * 0.5, cx + side * 0.5);
        let (z0, z1) = (cz - side * 0.5, cz + side * 0.5);
        let corners = [
            ground(x0, z0),
            ground(x1, z0),
            ground(x0, z1),
            ground(x1, z1),
        ];
        let centre = ground(cx, cz);
        let spread_hi = corners.iter().copied().fold(centre, f32::max);
        let spread_lo = corners.iter().copied().fold(centre, f32::min);
        if spread_hi - spread_lo >= MAX_GROUND_SPREAD_M {
            continue;
        }
        let lowest = corners.iter().copied().fold(f32::INFINITY, f32::min);
        let fp = [x0, x1, z0, z1];
        if !towers.iter().all(|t| gap_m(footprint(t), fp) >= GAP_M) {
            continue;
        }
        towers.push(Tower {
            min: [x0, lowest - BASE_SINK_M, z0],
            max: [x1, lowest + height, z1],
            colour,
        });
    }
    towers
}

/// Outward-wound box between `min` and `max` as a closed triangle shell; meters.
fn tower_box(min: [f32; 3], max: [f32; 3]) -> PropMesh {
    let [x0, y0, z0] = min;
    let [x1, y1, z1] = max;
    let positions = vec![
        [x0, y0, z0],
        [x1, y0, z0],
        [x1, y1, z0],
        [x0, y1, z0],
        [x0, y0, z1],
        [x1, y0, z1],
        [x1, y1, z1],
        [x0, y1, z1],
    ];
    let indices = vec![
        0, 2, 1, 0, 3, 2, // -z
        4, 5, 6, 4, 6, 7, // +z
        0, 4, 7, 0, 7, 3, // -x
        1, 2, 6, 1, 6, 5, // +x
        0, 1, 5, 0, 5, 4, // -y
        2, 3, 7, 2, 7, 6, // +y
    ];
    PropMesh { positions, indices }
}

/// The tower as a closed box shell for `csg::knife` (SD4); meters.
pub(super) fn tower_shell(t: &Tower) -> PropMesh {
    tower_box(t.min, t.max)
}

/// Triangles the uncut city draws.
pub(super) fn total_tris(towers: &[Tower]) -> usize {
    towers.len() * TOWER_TRIS
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Flat ground at height zero.
    fn flat(_x: f32, _z: f32) -> f32 {
        0.0
    }

    /// Gentle rolling hills: 12 m high, about 80 m between crests.
    fn hills(x: f32, z: f32) -> f32 {
        12.0 * (x / 80.0).sin() * (z / 80.0).cos()
    }

    /// Bit pattern of every number in a tower, so equality is bit-exact.
    fn bits(t: &Tower) -> Vec<u32> {
        t.min
            .iter()
            .chain(&t.max)
            .chain(&t.colour)
            .map(|v| v.to_bits())
            .collect()
    }

    /// Lowest of a tower's four base corners on `ground`.
    fn lowest_corner(t: &Tower, ground: &dyn Fn(f32, f32) -> f32) -> f32 {
        let [x0, _, z0] = t.min;
        let [x1, _, z1] = t.max;
        [
            ground(x0, z0),
            ground(x1, z0),
            ground(x0, z1),
            ground(x1, z1),
        ]
        .into_iter()
        .fold(f32::INFINITY, f32::min)
    }

    /// Signed volume of a closed shell: positive when its winding faces outward.
    fn signed_volume(m: &PropMesh) -> f32 {
        m.indices
            .chunks_exact(3)
            .map(|t| {
                let a = m.positions[t[0] as usize];
                let b = m.positions[t[1] as usize];
                let c = m.positions[t[2] as usize];
                a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                    + a[2] * (b[0] * c[1] - b[1] * c[0])
            })
            .sum::<f32>()
            / 6.0
    }

    #[test]
    fn same_seed_same_towers() {
        let a = place_towers(7, 400.0, &hills);
        let b = place_towers(7, 400.0, &hills);
        assert!(!a.is_empty());
        let bits_a: Vec<Vec<u32>> = a.iter().map(bits).collect();
        let bits_b: Vec<Vec<u32>> = b.iter().map(bits).collect();
        assert_eq!(bits_a, bits_b);
    }

    #[test]
    fn different_seed_different_towers() {
        let a = place_towers(7, 400.0, &flat);
        let b = place_towers(8, 400.0, &flat);
        assert_ne!(a, b);
    }

    #[test]
    fn no_two_footprints_overlap_and_all_keep_the_gap() {
        for seed in [1, 2, 3] {
            for towers in [
                place_towers(seed, 300.0, &hills),
                place_towers(seed, 300.0, &flat),
            ] {
                for (i, a) in towers.iter().enumerate() {
                    for b in &towers[i + 1..] {
                        let gap = gap_m(footprint(a), footprint(b));
                        assert!(gap >= GAP_M - 1e-3, "gap {gap} between towers");
                    }
                }
            }
        }
    }

    #[test]
    fn every_base_sits_on_the_ground_and_none_floats() {
        for t in place_towers(5, 400.0, &hills) {
            let [x0, base, z0] = t.min;
            let [x1, _, z1] = t.max;
            let corners = [hills(x0, z0), hills(x1, z0), hills(x0, z1), hills(x1, z1)];
            let lowest = lowest_corner(&t, &hills);
            // Base at or under the ground at every corner: nothing floats.
            for g in corners {
                assert!(base <= g, "base {base} floats above ground {g}");
                // Every corner stays within the spread limit plus the sink.
                assert!(g - base <= MAX_GROUND_SPREAD_M + BASE_SINK_M + 1e-3);
            }
            // At the lowest corner the base is BASE_SINK_M under the ground, within 1.5 m.
            assert!(
                lowest - base <= 1.5 + 1e-3,
                "base sinks {} m",
                lowest - base
            );
        }
    }

    #[test]
    fn count_is_in_range_on_flat_and_positive_on_hilly_ground() {
        for seed in [9, 10, 11] {
            let flat_n = place_towers(seed, 400.0, &flat).len();
            assert!(
                (MIN_TOWERS..=MAX_TOWERS).contains(&flat_n),
                "flat count {flat_n}"
            );
        }
        assert!(!place_towers(9, 400.0, &hills).is_empty());
    }

    #[test]
    fn sizes_colours_and_extent_hold() {
        let half = 400.0;
        let towers = place_towers(11, half, &hills);
        for t in &towers {
            let (w, d) = (t.max[0] - t.min[0], t.max[2] - t.min[2]);
            assert!((MIN_FOOTPRINT_M - 1e-3..=MAX_FOOTPRINT_M + 1e-3).contains(&w));
            assert!((MIN_FOOTPRINT_M - 1e-3..=MAX_FOOTPRINT_M + 1e-3).contains(&d));
            let visible = t.max[1] - lowest_corner(t, &hills);
            assert!((MIN_HEIGHT_M - 1e-3..=MAX_HEIGHT_M + 1e-3).contains(&visible));
            assert!(t.min[0] >= -half && t.max[0] <= half);
            assert!(t.min[2] >= -half && t.max[2] <= half);
            assert!(PALETTE.contains(&t.colour));
        }
    }

    #[test]
    fn tower_shell_is_a_closed_outward_box() {
        let t = Tower {
            min: [0.0, 0.0, 0.0],
            max: [10.0, 20.0, 30.0],
            colour: PALETTE[0],
        };
        let shell = tower_shell(&t);
        assert_eq!(shell.tri_count(), TOWER_TRIS);
        // Every directed edge has its reverse: no open seam.
        let mut edges: Vec<(u32, u32)> = Vec::new();
        for tri in shell.indices.chunks_exact(3) {
            let (a, b, c) = (tri[0], tri[1], tri[2]);
            edges.extend([(a, b), (b, c), (c, a)]);
        }
        for &(a, b) in &edges {
            assert!(edges.contains(&(b, a)), "edge {a}-{b} is open");
        }
        assert!((signed_volume(&shell) - 6_000.0).abs() < 1e-2);
    }

    #[test]
    fn city_fits_the_triangle_budget() {
        let towers = place_towers(3, 400.0, &hills);
        assert_eq!(total_tris(&towers), towers.len() * TOWER_TRIS);
        assert!(total_tris(&towers) < TOWER_TRI_BUDGET);
        for t in &towers {
            assert_eq!(tower_shell(t).tri_count(), TOWER_TRIS);
        }
    }
}
