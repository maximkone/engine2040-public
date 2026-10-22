//! streets: the ground of the procedural city. Road rectangles are asphalt laid flat at
//! y = 0. Every other part of the square is sidewalk raised to 0.15 m, with a curb face
//! wherever it meets a road, and each park rectangle is grass at the same height. Lamp
//! posts stand 1 m in from the road edge on both sides of every road, 30 m apart.
//! The square is cut into chunks; each chunk makes one mesh per surface, and a mesh starts
//! again before it reaches the upload limit. Presentation only: the same roads give the
//! same bytes. Units: meters; x runs east, z runs south.

use super::super::num::{cross, dot, f32_of, floor_idx, len, sub, u32_of};
use super::RoadRect;

/// Height of the sidewalk and park tops above the road, meters.
const RAISED_M: f32 = 0.15;
/// Height of a lamp's light, meters; the arm hangs at this height.
const LIGHT_Y_M: f32 = 6.5;
/// Width of a lamp post and its arm, meters.
const POST_W_M: f32 = 0.15;
/// Height of a lamp post above the sidewalk, meters.
const POST_H_M: f32 = 6.5;
/// Length of a lamp arm reaching toward the road, meters.
const ARM_M: f32 = 1.2;
/// Distance from a road edge to a lamp on the sidewalk, meters.
const SETBACK_M: f32 = 1.0;
/// Spacing of the lamps along a road, meters.
const LAMP_SPACING_M: f32 = 30.0;
/// Metres of ground per texture tile: UVs are the position divided by this.
const TILE_M: f32 = 4.0;
/// Vertex count a mesh is filled to before a new one starts: under the 65,535 upload limit
/// and a multiple of four, so whole quads always fit.
const MESH_VERTS: usize = 65_532;
/// How far past a cell edge the neighbouring surface is probed, meters.
const PROBE_M: f32 = 0.005;
/// How far a chunk's reach extends for roads that touch its neighbours, meters.
const MARGIN_M: f32 = 1.0;
/// A park rectangle as `(min, max)` corners `[x, z]`; meters.
type Park = ([f32; 2], [f32; 2]);
/// Quad corners that go around a unit square.
const UNIT_UV: [[f32; 2]; 4] = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
/// The up normal.
const UP: [f32; 3] = [0.0, 1.0, 0.0];

/// Which surface a ground mesh is drawn with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Ground {
    /// Road asphalt at y = 0; lane dashes and crosswalks come from its UVs.
    Asphalt,
    /// Sidewalk tops and their curb faces, raised to 0.15 m.
    Sidewalk,
    /// Park grass, raised to 0.15 m.
    Grass,
    /// Lamp posts and arms.
    Lamp,
}

/// One mesh of ground geometry. Corners are not shared between quads, so every mesh is
/// self-contained and ready for upload.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct GroundMesh {
    /// The surface this mesh is drawn with.
    pub ground: Ground,
    /// Vertex positions; meters.
    pub positions: Vec<[f32; 3]>,
    /// Unit vertex normals.
    pub normals: Vec<[f32; 3]>,
    /// Texture coordinates: meters divided by the tile size, or unit squares for lamps.
    pub uvs: Vec<[f32; 2]>,
    /// Triangle list into this mesh's own vertices.
    pub indices: Vec<u32>,
}

/// One chunk's footprint on the ground; meters.
#[derive(Clone, Copy, Debug)]
struct Area {
    /// West edge.
    x0: f32,
    /// East edge.
    x1: f32,
    /// North edge.
    z0: f32,
    /// South edge.
    z1: f32,
}

/// A lamp: where its light is, and the unit direction from its post back toward the road.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Site {
    /// The light's position; meters.
    at: [f32; 3],
    /// Flat unit direction toward the road.
    toward: [f32; 3],
}

/// Collects the quads of one chunk, one mesh list per surface. A list starts a new mesh
/// whenever the current one would pass the vertex limit.
#[derive(Default)]
struct Sink {
    /// Asphalt meshes.
    asphalt: Vec<GroundMesh>,
    /// Sidewalk meshes.
    sidewalk: Vec<GroundMesh>,
    /// Grass meshes.
    grass: Vec<GroundMesh>,
    /// Lamp meshes.
    lamp: Vec<GroundMesh>,
}

impl Sink {
    /// The list that collects the meshes of `ground`.
    fn list(&mut self, ground: Ground) -> &mut Vec<GroundMesh> {
        match ground {
            Ground::Asphalt => &mut self.asphalt,
            Ground::Sidewalk => &mut self.sidewalk,
            Ground::Grass => &mut self.grass,
            Ground::Lamp => &mut self.lamp,
        }
    }

    /// One quad of `ground`. `corners` go around the rectangle; the winding is set so the
    /// front face looks along `normal`.
    fn quad(
        &mut self,
        ground: Ground,
        corners: [[f32; 3]; 4],
        normal: [f32; 3],
        uvs: [[f32; 2]; 4],
    ) {
        let list = self.list(ground);
        let fits = matches!(list.last(), Some(m) if m.positions.len() + 4 <= MESH_VERTS);
        if !fits {
            list.push(GroundMesh {
                ground,
                positions: Vec::new(),
                normals: Vec::new(),
                uvs: Vec::new(),
                indices: Vec::new(),
            });
        }
        let Some(mesh) = list.last_mut() else {
            return;
        };
        let base = u32_of(mesh.positions.len());
        let [a, b, c, _] = corners;
        let front = dot(cross(sub(b, a), sub(c, a)), normal) >= 0.0;
        let order: [u32; 6] = if front {
            [0, 1, 2, 0, 2, 3]
        } else {
            [0, 2, 1, 0, 3, 2]
        };
        mesh.positions.extend_from_slice(&corners);
        mesh.normals.extend_from_slice(&[normal; 4]);
        mesh.uvs.extend_from_slice(&uvs);
        mesh.indices.extend(order.iter().map(|i| base + *i));
    }

    /// An axis-aligned box from `lo` to `hi`: its four sides and its top. The bottom sits on
    /// the sidewalk and is not drawn.
    fn boxed(&mut self, ground: Ground, lo: [f32; 3], hi: [f32; 3]) {
        let [x0, y0, z0] = lo;
        let [x1, y1, z1] = hi;
        let faces = [
            (
                [[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]],
                [1.0, 0.0, 0.0],
            ),
            (
                [[x0, y0, z0], [x0, y1, z0], [x0, y1, z1], [x0, y0, z1]],
                [-1.0, 0.0, 0.0],
            ),
            (
                [[x0, y0, z1], [x1, y0, z1], [x1, y1, z1], [x0, y1, z1]],
                [0.0, 0.0, 1.0],
            ),
            (
                [[x0, y0, z0], [x1, y0, z0], [x1, y1, z0], [x0, y1, z0]],
                [0.0, 0.0, -1.0],
            ),
            ([[x0, y1, z0], [x1, y1, z0], [x1, y1, z1], [x0, y1, z1]], UP),
        ];
        for (corners, normal) in faces {
            self.quad(ground, corners, normal, UNIT_UV);
        }
    }

    /// Every mesh collected so far, asphalt first.
    fn into_meshes(self) -> Vec<GroundMesh> {
        let mut out = self.asphalt;
        out.extend(self.sidewalk);
        out.extend(self.grass);
        out.extend(self.lamp);
        out
    }
}

/// The ground square of the city, cut into chunks: one mesh per surface per chunk, with
/// lamp posts in the chunk that holds their light. `side_m` is the square's width and
/// `chunk_m` the chunk step, both meters.
pub(super) fn street_meshes(
    roads: &[RoadRect],
    park: Park,
    side_m: f32,
    chunk_m: f32,
) -> Vec<GroundMesh> {
    let mut out = Vec::new();
    if !side_m.is_finite() || !chunk_m.is_finite() || side_m <= 0.0 || chunk_m <= 0.0 {
        return out;
    }
    let half = side_m * 0.5;
    let n = floor_idx((side_m / chunk_m).ceil());
    let sites = lamp_sites(roads, LAMP_SPACING_M);
    for i in 0..n {
        for j in 0..n {
            let area = chunk_area(i, j, half, chunk_m);
            let mut sink = Sink::default();
            chunk_ground(&mut sink, area, &near_roads(roads, area), park);
            for site in sites.iter().filter(|s| {
                chunk_of(s.at[0], half, chunk_m, n) == i && chunk_of(s.at[2], half, chunk_m, n) == j
            }) {
                lamp_post(&mut sink, site);
            }
            out.extend(sink.into_meshes());
        }
    }
    out
}

/// The light positions of the lamps along both sides of every road, `spacing_m` apart, at
/// the light's height. A spot that lies on a road is skipped. Meters.
pub(super) fn lamps(roads: &[RoadRect], spacing_m: f32) -> Vec<[f32; 3]> {
    lamp_sites(roads, spacing_m)
        .into_iter()
        .map(|s| s.at)
        .collect()
}

/// The footprint of chunk `(i, j)` of a square `2 * half` wide, cut into `chunk_m` steps.
fn chunk_area(i: usize, j: usize, half: f32, chunk_m: f32) -> Area {
    let x0 = -half + chunk_m * f32_of(i);
    let z0 = -half + chunk_m * f32_of(j);
    Area {
        x0,
        x1: (x0 + chunk_m).min(half),
        z0,
        z1: (z0 + chunk_m).min(half),
    }
}

/// The index (below `n`) of the chunk that holds the ground coordinate `v`.
fn chunk_of(v: f32, half: f32, chunk_m: f32, n: usize) -> usize {
    floor_idx((v + half) / chunk_m).min(n.saturating_sub(1))
}

/// The roads that reach a chunk or its margin, so the curb probes of its neighbours see them.
fn near_roads(roads: &[RoadRect], area: Area) -> Vec<RoadRect> {
    roads
        .iter()
        .filter(|r| {
            r.min[0] <= area.x1 + MARGIN_M
                && r.max[0] >= area.x0 - MARGIN_M
                && r.min[1] <= area.z1 + MARGIN_M
                && r.max[1] >= area.z0 - MARGIN_M
        })
        .copied()
        .collect()
}

/// True when the ground point `p` lies in the closed rectangle `min..=max`.
fn inside(min: [f32; 2], max: [f32; 2], p: [f32; 2]) -> bool {
    (min[0]..=max[0]).contains(&p[0]) && (min[1]..=max[1]).contains(&p[1])
}

/// The surface under the ground point `p`: park grass wins over a road, a road is asphalt,
/// and everything else is sidewalk.
fn surface(roads: &[RoadRect], park: Park, p: [f32; 2]) -> Ground {
    if inside(park.0, park.1, p) {
        Ground::Grass
    } else if roads.iter().any(|r| inside(r.min, r.max, p)) {
        Ground::Asphalt
    } else {
        Ground::Sidewalk
    }
}

/// True when the lamp at `at` stands on a road rectangle.
fn on_road(roads: &[RoadRect], at: [f32; 3]) -> bool {
    let p = [at[0], at[2]];
    roads.iter().any(|r| inside(r.min, r.max, p))
}

/// Texture coordinates of a ground point: meters divided by the tile size.
fn ground_uv(p: [f32; 3]) -> [f32; 2] {
    [p[0] / TILE_M, p[2] / TILE_M]
}

/// Every road and park edge on `axis` (0 for x, 1 for z), meters.
fn axis_edges(roads: &[RoadRect], park: Park, axis: usize) -> Vec<f32> {
    let mut v: Vec<f32> = roads
        .iter()
        .flat_map(|r| [r.min[axis], r.max[axis]])
        .collect();
    v.extend([park.0[axis], park.1[axis]]);
    v
}

/// The sorted cut coordinates from `lo` to `hi`: both ends and every edge strictly between.
fn cuts(lo: f32, hi: f32, inner: &[f32]) -> Vec<f32> {
    let mut v = vec![lo, hi];
    v.extend(inner.iter().copied().filter(|c| *c > lo && *c < hi));
    v.sort_by(f32::total_cmp);
    v.dedup();
    v
}

/// The ground of one chunk: its cells, cut at every road and park edge inside it.
fn chunk_ground(sink: &mut Sink, area: Area, roads: &[RoadRect], park: Park) {
    let xs = cuts(area.x0, area.x1, &axis_edges(roads, park, 0));
    let zs = cuts(area.z0, area.z1, &axis_edges(roads, park, 1));
    for xw in xs.windows(2) {
        for zw in zs.windows(2) {
            if let ([x0, x1], [z0, z1]) = (xw, zw) {
                cell(sink, roads, park, [*x0, *x1], [*z0, *z1]);
            }
        }
    }
}

/// One cell of the cut grid: its top at its surface's height, and a curb face on each raised
/// side that meets a road.
fn cell(sink: &mut Sink, roads: &[RoadRect], park: Park, xr: [f32; 2], zr: [f32; 2]) {
    let [x0, x1] = xr;
    let [z0, z1] = zr;
    let mid = [(x0 + x1) * 0.5, (z0 + z1) * 0.5];
    let kind = surface(roads, park, mid);
    let top = if kind == Ground::Asphalt {
        0.0
    } else {
        RAISED_M
    };
    let corners = [[x0, top, z0], [x0, top, z1], [x1, top, z1], [x1, top, z0]];
    sink.quad(kind, corners, UP, corners.map(ground_uv));
    if kind == Ground::Asphalt {
        return;
    }
    let road_at = |p: [f32; 2]| surface(roads, park, p) == Ground::Asphalt;
    if road_at([x0 - PROBE_M, mid[1]]) {
        curb(sink, kind, [x0, z0], [x0, z1], [-1.0, 0.0, 0.0]);
    }
    if road_at([x1 + PROBE_M, mid[1]]) {
        curb(sink, kind, [x1, z0], [x1, z1], [1.0, 0.0, 0.0]);
    }
    if road_at([mid[0], z0 - PROBE_M]) {
        curb(sink, kind, [x0, z0], [x1, z0], [0.0, 0.0, -1.0]);
    }
    if road_at([mid[0], z1 + PROBE_M]) {
        curb(sink, kind, [x0, z1], [x1, z1], [0.0, 0.0, 1.0]);
    }
}

/// A curb face from the ground point `a` to `b` (`[x, z]`), rising from the road to the
/// sidewalk top. `normal` points into the road.
fn curb(sink: &mut Sink, ground: Ground, a: [f32; 2], b: [f32; 2], normal: [f32; 3]) {
    let run = len(sub([b[0], 0.0, b[1]], [a[0], 0.0, a[1]])) / TILE_M;
    let corners = [
        [a[0], 0.0, a[1]],
        [b[0], 0.0, b[1]],
        [b[0], RAISED_M, b[1]],
        [a[0], RAISED_M, a[1]],
    ];
    let uvs = [
        [0.0, 0.0],
        [run, 0.0],
        [run, RAISED_M / TILE_M],
        [0.0, RAISED_M / TILE_M],
    ];
    sink.quad(ground, corners, normal, uvs);
}

/// Where the lamps stand: along both sides of every road, `spacing_m` apart, 1 m off the road
/// edge, and not on a road.
fn lamp_sites(roads: &[RoadRect], spacing_m: f32) -> Vec<Site> {
    let mut out = Vec::new();
    if !spacing_m.is_finite() || spacing_m <= 0.0 {
        return out;
    }
    for road in roads {
        let along_z = road.max[1] - road.min[1] >= road.max[0] - road.min[0];
        let (run, across) = if along_z {
            ([road.min[1], road.max[1]], [road.min[0], road.max[0]])
        } else {
            ([road.min[0], road.max[0]], [road.min[1], road.max[1]])
        };
        // Each sidewalk: its setback coordinate across the road, and the flat direction back
        // toward the road from there.
        let sides = if along_z {
            [
                (across[0] - SETBACK_M, [1.0, 0.0]),
                (across[1] + SETBACK_M, [-1.0, 0.0]),
            ]
        } else {
            [
                (across[0] - SETBACK_M, [0.0, 1.0]),
                (across[1] + SETBACK_M, [0.0, -1.0]),
            ]
        };
        for k in 0usize.. {
            let t = run[0] + spacing_m * (f32_of(k) + 0.5);
            if t >= run[1] {
                break;
            }
            for (s, dir) in sides {
                let at = if along_z {
                    [s, LIGHT_Y_M, t]
                } else {
                    [t, LIGHT_Y_M, s]
                };
                if !on_road(roads, at) {
                    out.push(Site {
                        at,
                        toward: [dir[0], 0.0, dir[1]],
                    });
                }
            }
        }
    }
    out
}

/// One lamp: a post from the sidewalk top up, and an arm reaching toward the road with its
/// top at the post's top.
fn lamp_post(sink: &mut Sink, site: &Site) {
    let [x, _, z] = site.at;
    let h = POST_W_M * 0.5;
    sink.boxed(
        Ground::Lamp,
        [x - h, RAISED_M, z - h],
        [x + h, RAISED_M + POST_H_M, z + h],
    );
    let ex = x + site.toward[0] * ARM_M;
    let ez = z + site.toward[2] * ARM_M;
    sink.boxed(
        Ground::Lamp,
        [x.min(ex) - h, LIGHT_Y_M, z.min(ez) - h],
        [x.max(ex) + h, LIGHT_Y_M + POST_W_M, z.max(ez) + h],
    );
}

#[cfg(test)]
mod tests {
    use super::super::super::num::{idx, unit_f32};
    use super::super::RoadKind;
    use super::*;
    use engine_core::hash::GOLDEN_GAMMA;
    use engine_core::splitmix64;

    /// A seeded stream of uniform values in `[0, 1)`, one SplitMix64 step per draw.
    struct Stream(u64);

    impl Stream {
        /// The next draw in `[0, 1)`.
        fn unit(&mut self) -> f32 {
            self.0 = self.0.wrapping_add(GOLDEN_GAMMA);
            unit_f32(splitmix64(self.0))
        }

        /// A ground point inside the rectangle `min..max`.
        fn point(&mut self, min: [f32; 2], max: [f32; 2]) -> [f32; 2] {
            let u = self.unit();
            let v = self.unit();
            [
                min[0] + (max[0] - min[0]) * u,
                min[1] + (max[1] - min[1]) * v,
            ]
        }
    }

    /// True when the ground point `p` lies in the triangle `a`, `b`, `c` seen from above.
    fn in_tri(a: [f32; 3], b: [f32; 3], c: [f32; 3], p: [f32; 2]) -> bool {
        let side = |u: [f32; 3], v: [f32; 3]| {
            (v[0] - u[0]) * (p[1] - u[2]) - (v[2] - u[2]) * (p[0] - u[0])
        };
        let (d1, d2, d3) = (side(a, b), side(b, c), side(c, a));
        let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
        let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
        !(neg && pos)
    }

    /// The heights of the vertices of every `ground` triangle that covers the ground point
    /// `p`. Vertical faces project to lines and are skipped.
    fn heights_at(meshes: &[GroundMesh], ground: Ground, p: [f32; 2]) -> Vec<f32> {
        let mut ys = Vec::new();
        for m in meshes.iter().filter(|m| m.ground == ground) {
            for tri in m.indices.chunks_exact(3) {
                let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| m.positions[idx(i)]);
                let flat = (b[0] - a[0]) * (c[2] - a[2]) - (b[2] - a[2]) * (c[0] - a[0]);
                if flat.abs() > 1e-9 && in_tri(a, b, c, p) {
                    ys.extend([a[1], b[1], c[1]]);
                }
            }
        }
        ys
    }

    /// Two crossing roads: an avenue along z and a street along x.
    fn test_roads() -> Vec<RoadRect> {
        vec![
            RoadRect {
                min: [100.0, -400.0],
                max: [130.0, 400.0],
                kind: RoadKind::Avenue,
            },
            RoadRect {
                min: [-400.0, 200.0],
                max: [400.0, 218.0],
                kind: RoadKind::Street,
            },
        ]
    }

    /// A park away from the test roads: x -300..-200, z -350..-250.
    const PARK: Park = ([-300.0, -350.0], [-200.0, -250.0]);

    /// A grid of streets and avenues every 20 m over a 2 km square.
    fn dense_city() -> Vec<RoadRect> {
        let mut roads = Vec::new();
        for k in 0..100 {
            let c = -990.0 + f32_of(k) * 20.0;
            roads.push(RoadRect {
                min: [-1000.0, c],
                max: [1000.0, c + 4.0],
                kind: RoadKind::Street,
            });
            roads.push(RoadRect {
                min: [c, -1000.0],
                max: [c + 4.0, 1000.0],
                kind: RoadKind::Avenue,
            });
        }
        roads
    }

    #[test]
    fn every_road_rect_is_asphalt_at_zero() {
        let roads = test_roads();
        let meshes = street_meshes(&roads, PARK, 1000.0, 200.0);
        let mut rng = Stream(7);
        for road in &roads {
            for _ in 0..50 {
                let p = rng.point(road.min, road.max);
                let ys = heights_at(&meshes, Ground::Asphalt, p);
                assert!(!ys.is_empty(), "no asphalt under {p:?}");
                assert!(
                    ys.iter().all(|y| *y == 0.0),
                    "asphalt not at y = 0 under {p:?}"
                );
            }
        }
    }

    #[test]
    fn sidewalks_rise_to_fifteen_cm_and_curbs_run_zero_to_fifteen_cm() {
        let roads = test_roads();
        let meshes = street_meshes(&roads, PARK, 1000.0, 200.0);
        let mut rng = Stream(11);
        let mut checked = 0;
        while checked < 50 {
            let p = rng.point([-500.0, -500.0], [500.0, 500.0]);
            if surface(&roads, PARK, p) != Ground::Sidewalk {
                continue;
            }
            checked += 1;
            let ys = heights_at(&meshes, Ground::Sidewalk, p);
            assert!(
                !ys.is_empty() && ys.iter().all(|y| *y == RAISED_M),
                "sidewalk not at 0.15 m under {p:?}"
            );
        }
        let sidewalk: Vec<&GroundMesh> = meshes
            .iter()
            .filter(|m| m.ground == Ground::Sidewalk)
            .collect();
        let curb_ys: Vec<f32> = sidewalk
            .iter()
            .flat_map(|m| {
                m.positions
                    .iter()
                    .zip(&m.normals)
                    .filter(|(_, n)| n[1] == 0.0)
                    .map(|(p, _)| p[1])
            })
            .collect();
        assert!(curb_ys.contains(&0.0), "no curb foot at y = 0");
        assert!(curb_ys.contains(&RAISED_M), "no curb top at 0.15 m");
        assert!(
            curb_ys.iter().all(|y| *y == 0.0 || *y == RAISED_M),
            "a curb vertex sits between 0 and 0.15 m"
        );
        // The avenue spans x 100..130: the sidewalk west of it faces east at x = 100, the
        // sidewalk east of it faces west at x = 130.
        let curb_at = |x: f32, normal: [f32; 3]| {
            sidewalk.iter().any(|m| {
                m.positions
                    .iter()
                    .zip(&m.normals)
                    .any(|(p, n)| p[0] == x && *n == normal)
            })
        };
        assert!(
            curb_at(100.0, [1.0, 0.0, 0.0]),
            "no curb on the avenue's west edge facing the road"
        );
        assert!(
            curb_at(130.0, [-1.0, 0.0, 0.0]),
            "no curb on the avenue's east edge facing the road"
        );
    }

    #[test]
    fn no_asphalt_over_the_park() {
        // An avenue runs straight through the park.
        let roads = vec![RoadRect {
            min: [-250.0, -400.0],
            max: [-240.0, 400.0],
            kind: RoadKind::Avenue,
        }];
        let meshes = street_meshes(&roads, PARK, 1000.0, 200.0);
        for m in meshes.iter().filter(|m| m.ground == Ground::Asphalt) {
            for tri in m.indices.chunks_exact(3) {
                let c = tri
                    .iter()
                    .map(|i| m.positions[idx(*i)])
                    .fold([0.0_f32; 2], |acc, v| {
                        [acc[0] + v[0] / 3.0, acc[1] + v[2] / 3.0]
                    });
                assert!(!inside(PARK.0, PARK.1, c), "asphalt over the park at {c:?}");
            }
        }
        let grass = heights_at(&meshes, Ground::Grass, [-245.0, -300.0]);
        assert!(
            !grass.is_empty() && grass.iter().all(|y| *y == RAISED_M),
            "park grass missing where the avenue crosses it"
        );
        let road = heights_at(&meshes, Ground::Asphalt, [-245.0, 0.0]);
        assert!(!road.is_empty() && road.iter().all(|y| *y == 0.0));
    }

    #[test]
    fn every_mesh_stays_under_the_vertex_limit() {
        let roads = dense_city();
        let meshes = street_meshes(&roads, PARK, 2000.0, 2000.0);
        let sidewalks = meshes
            .iter()
            .filter(|m| m.ground == Ground::Sidewalk)
            .count();
        assert!(sidewalks > 1, "the dense city must split its sidewalk");
        for m in &meshes {
            assert!(m.positions.len() < 65_535, "mesh over the upload limit");
            assert_eq!(m.positions.len(), m.normals.len());
            assert_eq!(m.positions.len(), m.uvs.len());
            assert!(m.indices.iter().all(|i| idx(*i) < m.positions.len()));
        }
    }

    #[test]
    fn lamps_stand_thirty_metres_apart_on_both_sides() {
        let roads = vec![RoadRect {
            min: [-500.0, -9.0],
            max: [500.0, 9.0],
            kind: RoadKind::Street,
        }];
        let sites = lamps(&roads, LAMP_SPACING_M);
        for side in [-10.0_f32, 10.0] {
            let mut xs: Vec<f32> = sites
                .iter()
                .filter(|p| p[2] == side)
                .map(|p| p[0])
                .collect();
            xs.sort_by(f32::total_cmp);
            assert!(xs.len() > 30, "too few lamps on the side at z = {side}");
            for w in xs.windows(2) {
                let gap = w[1] - w[0];
                assert!((29.5..=30.5).contains(&gap), "lamp gap {gap} m");
            }
        }
        assert!(sites.iter().all(|p| p[1] == LIGHT_Y_M));
        let posts = street_meshes(&roads, PARK, 1000.0, 200.0);
        let top = posts
            .iter()
            .filter(|m| m.ground == Ground::Lamp)
            .flat_map(|m| m.positions.iter().map(|p| p[1]))
            .fold(f32::MIN, f32::max);
        assert!(
            (top - (RAISED_M + POST_H_M)).abs() < 1e-5,
            "post top at {top} m"
        );
    }

    #[test]
    fn normals_are_unit_and_triangles_face_them() {
        let meshes = street_meshes(&test_roads(), PARK, 1000.0, 200.0);
        for m in &meshes {
            for n in &m.normals {
                assert!((len(*n) - 1.0).abs() < 1e-6, "normal {n:?} is not unit");
            }
            for tri in m.indices.chunks_exact(3) {
                let [a, b, c] = [tri[0], tri[1], tri[2]].map(|i| m.positions[idx(i)]);
                let n = m.normals[idx(tri[0])];
                assert!(
                    dot(cross(sub(b, a), sub(c, a)), n) >= 0.0,
                    "a triangle faces away from its normal"
                );
            }
        }
    }

    #[test]
    fn same_input_gives_the_same_bytes() {
        let a = street_meshes(&test_roads(), PARK, 1000.0, 200.0);
        let b = street_meshes(&test_roads(), PARK, 1000.0, 200.0);
        assert_eq!(a, b);
    }
}
