//! world (HC9, docs/SPRINT-34-hero-city.md): the rows put together as one city. The grid (HC1)
//! gives lots and roads, each lot becomes a building (HC2) kept as one cuttable shell plus its
//! tier boxes, the streets (HC4) lay the ground, and the facades (HC3) give each style its
//! tiles. Buildings are grouped into [`CHUNK_M`] square chunks so the window draws one merged
//! mesh per chunk and style, not one per building. The sidewalk loops the crowd walks and the
//! lanes the traffic drives are read off the road rectangles. Presentation only, seeded.
//! Units: meters; y up; x east, z south.

use engine_destruction::csg::PropMesh;
use engine_render::present::TextureUpload;
use engine_render::v1::{upload_mesh_tiled, MeshData, PbrMaterial};

use super::building::massing;
use super::facade::{tiles, TILE_PX};
use super::grid::{NycGrid, SIDE_M};
use super::streets::{lamps, street_meshes, Ground, GroundMesh};
use super::{RoadKind, Style};
use crate::arena_frame::speed_demo::crowd::Loop;
use crate::arena_frame::speed_demo::num::{floor_idx, u32_of};

/// Side of one draw chunk, meters: 4 x 4 chunks over the 1.6 km square.
pub(in crate::arena_frame::speed_demo) const CHUNK_M: f32 = 400.0;
/// Chunks per side.
const CHUNKS_PER_SIDE: usize = 4;
/// Sidewalk top above the road, meters (the streets row raises it this far).
pub(in crate::arena_frame::speed_demo) const SIDEWALK_Y_M: f32 = 0.15;
/// Lamp spacing along a road, meters.
const LAMP_SPACING_M: f32 = 30.0;
/// A walker loop runs this far in from the block edge (the middle of the 5 m sidewalk).
const LOOP_INSET_M: f32 = 2.5;
/// Blocks shorter than this on a side get no walkers, meters.
const MIN_BLOCK_M: f32 = 20.0;

/// One building of the city: its shell (what the blade cuts), its look and its tier boxes.
pub(in crate::arena_frame::speed_demo) struct NycBuilding {
    /// Every part of the massing merged into one triangle shell, world meters.
    pub shell: PropMesh,
    /// Its facade family.
    pub style: Style,
    /// One axis box per tier, `(min, max)` in world meters.
    pub boxes: Vec<([f32; 3], [f32; 3])>,
    /// Which draw chunk it belongs to.
    pub chunk: usize,
}

/// The whole city.
pub(in crate::arena_frame::speed_demo) struct NycWorld {
    /// The grid it was built from.
    grid: NycGrid,
    /// Every building.
    pub buildings: Vec<NycBuilding>,
    /// The ground meshes: asphalt, sidewalks, grass and lamp posts, per chunk.
    ground: Vec<GroundMesh>,
    /// Street lamp light positions, meters.
    pub lamps: Vec<[f32; 3]>,
}

impl NycWorld {
    /// Build the city for `seed`.
    pub(in crate::arena_frame::speed_demo) fn build(seed: u64) -> Self {
        let grid = NycGrid::generate(seed);
        let buildings = grid
            .lots
            .iter()
            .map(|lot| {
                let built = massing(lot, SIDEWALK_Y_M);
                let mut shell = PropMesh {
                    positions: Vec::new(),
                    indices: Vec::new(),
                };
                for part in &built.parts {
                    let base = u32_of(shell.positions.len());
                    shell.positions.extend_from_slice(&part.positions);
                    shell.indices.extend(part.indices.iter().map(|i| i + base));
                }
                let centre = footprint_centre(&lot.footprint);
                NycBuilding {
                    shell,
                    style: lot.style,
                    boxes: built.boxes,
                    chunk: chunk_of(centre[0], centre[1]),
                }
            })
            .collect();
        let ground = street_meshes(&grid.roads, grid.park, SIDE_M, CHUNK_M);
        let lamps = lamps(&grid.roads, LAMP_SPACING_M);
        Self {
            grid,
            buildings,
            ground,
            lamps,
        }
    }

    /// The ground as render meshes with their materials (cold path: once per world).
    pub(in crate::arena_frame::speed_demo) fn ground_draws(
        &self,
    ) -> Result<Vec<(MeshData, PbrMaterial)>, String> {
        self.ground
            .iter()
            .map(|g| {
                upload_mesh_tiled(&g.positions, &g.normals, &g.uvs, &g.indices)
                    .map(|mesh| (mesh, ground_material(&g.ground)))
                    .map_err(|e| format!("nyc ground: {e:?}"))
            })
            .collect()
    }

    /// The curbs `(west, east)` of the avenue nearest `x` that never runs through the park,
    /// meters (a ride down it passes buildings the whole way).
    pub(in crate::arena_frame::speed_demo) fn avenue_near(&self, x: f32) -> Option<(f32, f32)> {
        let (pmin, pmax) = self.grid.park;
        self.grid
            .roads
            .iter()
            .filter(|r| r.kind == RoadKind::Avenue)
            .filter(|r| r.max[0] < pmin[0] - 10.0 || r.min[0] > pmax[0] + 10.0)
            .map(|r| (r.min[0], r.max[0]))
            .min_by(|a, b| {
                let da = ((a.0 + a.1) * 0.5 - x).abs();
                let db = ((b.0 + b.1) * 0.5 - x).abs();
                da.total_cmp(&db)
            })
    }

    /// Ground height at (x, z): the road at 0, everything else at the sidewalk top.
    pub(in crate::arena_frame::speed_demo) fn height_at(&self, x: f32, z: f32) -> f32 {
        if self.grid.is_road(x, z) {
            0.0
        } else {
            SIDEWALK_Y_M
        }
    }

    /// The loops walkers circle: one around every block, in the middle of its sidewalk, with
    /// the ground height under it. Blocks the park or Broadway cut are skipped.
    pub(in crate::arena_frame::speed_demo) fn walker_loops(&self) -> Vec<(Loop, f32)> {
        let xs = gaps(&self.grid.roads, RoadKind::Avenue, 0);
        let zs = gaps(&self.grid.roads, RoadKind::Street, 1);
        let (pmin, pmax) = self.grid.park;
        let mut out = Vec::new();
        for &(x0, x1) in &xs {
            for &(z0, z1) in &zs {
                let lp = Loop {
                    min: [x0 + LOOP_INSET_M, z0 + LOOP_INSET_M],
                    max: [x1 - LOOP_INSET_M, z1 - LOOP_INSET_M],
                };
                let in_park = lp.max[0] > pmin[0]
                    && lp.min[0] < pmax[0]
                    && lp.max[1] > pmin[1]
                    && lp.min[1] < pmax[1];
                let corners = [
                    [lp.min[0], lp.min[1]],
                    [lp.max[0], lp.min[1]],
                    [lp.max[0], lp.max[1]],
                    [lp.min[0], lp.max[1]],
                ];
                let crossed = corners.iter().any(|c| self.grid.is_road(c[0], c[1]));
                if !in_park && !crossed {
                    out.push((lp, SIDEWALK_Y_M));
                }
            }
        }
        out
    }

    /// Lane centre lines for traffic: two per avenue (along z) and two per street (along x),
    /// each as `(start, end)` on the ground, meters, driving start to end.
    pub(in crate::arena_frame::speed_demo) fn lanes(&self) -> Vec<([f32; 2], [f32; 2])> {
        let half = SIDE_M * 0.5;
        let mut out = Vec::new();
        for road in &self.grid.roads {
            let mid = [
                (road.min[0] + road.max[0]) * 0.5,
                (road.min[1] + road.max[1]) * 0.5,
            ];
            match road.kind {
                RoadKind::Avenue => {
                    let off = (road.max[0] - road.min[0]) * 0.2;
                    out.push(([mid[0] - off, -half], [mid[0] - off, half]));
                    out.push(([mid[0] + off, half], [mid[0] + off, -half]));
                }
                RoadKind::Street => {
                    let off = (road.max[1] - road.min[1]) * 0.2;
                    out.push(([-half, mid[1] + off], [half, mid[1] + off]));
                    out.push(([half, mid[1] - off], [-half, mid[1] - off]));
                }
                RoadKind::Broadway => {}
            }
        }
        out.dedup();
        out
    }

    /// Where an avenue meets a street: the traffic lights, meters.
    pub(in crate::arena_frame::speed_demo) fn crossings(&self) -> Vec<[f32; 2]> {
        let mut out = Vec::new();
        for a in self
            .grid
            .roads
            .iter()
            .filter(|r| r.kind == RoadKind::Avenue)
        {
            for s in self
                .grid
                .roads
                .iter()
                .filter(|r| r.kind == RoadKind::Street)
            {
                let x = (a.min[0] + a.max[0]) * 0.5;
                let z = (s.min[1] + s.max[1]) * 0.5;
                if x >= s.min[0] && x <= s.max[0] && z >= a.min[1] && z <= a.max[1] {
                    out.push([x, z]);
                }
            }
        }
        out
    }

    /// The tier boxes of standing buildings within `radius_m` of `at` (horizontal distance to
    /// the box), for web anchors, blade tests and walls a ragdoll hits.
    pub(in crate::arena_frame::speed_demo) fn boxes_near(
        &self,
        at: [f32; 3],
        radius_m: f32,
    ) -> Vec<([f32; 3], [f32; 3])> {
        let mut out = Vec::new();
        for b in &self.buildings {
            for (min, max) in &b.boxes {
                let dx = (min[0] - at[0]).max(at[0] - max[0]).max(0.0);
                let dz = (min[2] - at[2]).max(at[2] - max[2]).max(0.0);
                if dx.hypot(dz) <= radius_m {
                    out.push((*min, *max));
                }
            }
        }
        out
    }
}

/// One style's facade textures, ready to upload: base colour (sRGB), tangent-space normal
/// and ORM (linear).
pub(in crate::arena_frame::speed_demo) struct FacadeUploads {
    pub style: Style,
    pub base: TextureUpload,
    pub normal: TextureUpload,
    pub orm: TextureUpload,
}

/// The four facade texture sets, in [`Style::ALL`] order.
pub(in crate::arena_frame::speed_demo) fn facade_uploads(seed: u64) -> Vec<FacadeUploads> {
    let up = |rgba: Vec<u8>| TextureUpload {
        width: TILE_PX,
        height: TILE_PX,
        rgba,
    };
    Style::ALL
        .iter()
        .map(|s| {
            let t = tiles(*s, seed);
            FacadeUploads {
                style: *s,
                base: up(t.base),
                normal: up(t.normal),
                orm: up(t.orm),
            }
        })
        .collect()
}

/// The material a ground kind draws with.
fn ground_material(g: &Ground) -> PbrMaterial {
    let (base_color, metallic, roughness) = match g {
        Ground::Asphalt => ([0.07, 0.07, 0.075], 0.0, 0.85),
        Ground::Sidewalk => ([0.52, 0.5, 0.47], 0.0, 0.9),
        Ground::Grass => ([0.16, 0.33, 0.1], 0.0, 0.95),
        Ground::Lamp => ([0.08, 0.1, 0.09], 0.7, 0.45),
    };
    PbrMaterial {
        base_color,
        metallic,
        roughness,
        exposure: 1.0,
    }
}

/// The draw chunk a ground point (x, z) falls in.
pub(in crate::arena_frame::speed_demo) fn chunk_of(x: f32, z: f32) -> usize {
    let half = SIDE_M * 0.5;
    let cx = floor_idx((x + half) / CHUNK_M).min(CHUNKS_PER_SIDE - 1);
    let cz = floor_idx((z + half) / CHUNK_M).min(CHUNKS_PER_SIDE - 1);
    cz * CHUNKS_PER_SIDE + cx
}

/// Chunks in the city.
pub(in crate::arena_frame::speed_demo) const CHUNKS: usize = CHUNKS_PER_SIDE * CHUNKS_PER_SIDE;

fn footprint_centre(footprint: &[[f32; 2]]) -> [f32; 2] {
    let n = crate::arena_frame::speed_demo::num::f32_of(footprint.len().max(1));
    let s = footprint
        .iter()
        .fold([0.0, 0.0], |a, p| [a[0] + p[0], a[1] + p[1]]);
    [s[0] / n, s[1] / n]
}

/// The open intervals between roads of `kind` along `axis` (0 = x, 1 = z), clipped to the
/// square: the blocks' extents on that axis.
fn gaps(roads: &[super::RoadRect], kind: RoadKind, axis: usize) -> Vec<(f32, f32)> {
    let half = SIDE_M * 0.5;
    let mut bands: Vec<(f32, f32)> = roads
        .iter()
        .filter(|r| r.kind == kind)
        .map(|r| (r.min[axis], r.max[axis]))
        .collect();
    bands.sort_by(|a, b| a.0.total_cmp(&b.0));
    bands.dedup_by(|a, b| (a.0 - b.0).abs() < 0.01 && (a.1 - b.1).abs() < 0.01);
    let mut out = Vec::new();
    let mut from = -half;
    for (lo, hi) in bands {
        if lo - from >= MIN_BLOCK_M {
            out.push((from, lo));
        }
        from = from.max(hi);
    }
    if half - from >= MIN_BLOCK_M {
        out.push((from, half));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The city builds: many buildings, every one in a chunk, every shell non-empty, ground
    /// meshes and lamps present.
    #[test]
    fn the_city_builds_with_buildings_ground_and_lamps() {
        let w = NycWorld::build(0x5EED_2040);
        assert!(w.buildings.len() > 500, "{} buildings", w.buildings.len());
        assert!(w.buildings.iter().all(|b| b.chunk < CHUNKS));
        assert!(w.buildings.iter().all(|b| !b.shell.indices.is_empty()));
        assert!(w.buildings.iter().all(|b| !b.boxes.is_empty()));
        assert!(!w.ground_draws().expect("ground uploads").is_empty());
        assert_eq!(facade_uploads(1).len(), Style::ALL.len());
        assert!(w.lamps.len() > 100);
    }

    /// Walker loops sit on sidewalks, never on a road, and there are plenty of them.
    #[test]
    fn walker_loops_stay_on_the_sidewalks() {
        let w = NycWorld::build(0x5EED_2040);
        let loops = w.walker_loops();
        assert!(loops.len() > 40, "{} loops", loops.len());
        for (lp, y) in &loops {
            assert!((y - SIDEWALK_Y_M).abs() < 1e-6);
            for c in [
                lp.min,
                lp.max,
                [lp.min[0], lp.max[1]],
                [lp.max[0], lp.min[1]],
            ] {
                assert!(!w.grid.is_road(c[0], c[1]), "loop corner on a road: {c:?}");
            }
        }
    }

    /// Lanes run on roads, both directions.
    #[test]
    fn lanes_run_on_the_roads() {
        let w = NycWorld::build(0x5EED_2040);
        let lanes = w.lanes();
        assert!(lanes.len() >= 20, "{} lanes", lanes.len());
        for (a, b) in &lanes {
            let mid = [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5];
            assert!(
                w.grid.is_road(mid[0], mid[1]),
                "lane middle off the road: {mid:?}"
            );
        }
    }

    /// Boxes near a point are the ones within reach; the ground is 0 on a road.
    #[test]
    fn boxes_near_and_ground_height_read_the_city() {
        let w = NycWorld::build(0x5EED_2040);
        let b = &w.buildings[0].boxes[0];
        let at = [(b.0[0] + b.1[0]) * 0.5, 50.0, (b.0[2] + b.1[2]) * 0.5];
        assert!(!w.boxes_near(at, 1.0).is_empty());
        assert!(w.boxes_near([1.0e5, 0.0, 1.0e5], 50.0).is_empty());
        let road = w.grid.roads[0];
        let on = [
            (road.min[0] + road.max[0]) * 0.5,
            (road.min[1] + road.max[1]) * 0.5,
        ];
        assert_eq!(w.height_at(on[0], on[1]), 0.0);
        assert_eq!(chunk_of(-799.0, -799.0), 0);
        assert_eq!(chunk_of(799.0, 799.0), CHUNKS - 1);
    }
}
