//! terrain (SD1, docs/SPRINT-33-speed-demo.md): the generated ground of the speed demo.
//!
//! `engine_world::terrain::bake_terrain` gives a seeded height grid, one byte per sample, at
//! the engine's largest bake size (256 x 256 samples). Here it is laid over a 2 km square
//! centered on the origin with hills of tens of meters. `height_at` reads it bilinearly
//! between samples and clamps at the square's edge. `meshes` turns it into render tiles;
//! normals come from central differences over the whole grid, so tile seams shade alike.
//! Sand, grass and rock by slope are not here: `MeshData` has no colour channel yet.
//! Presentation only, deterministic. Units: meters.

use engine_render::v1::{upload_mesh_tiled, MeshData};
use engine_world::terrain::{bake_terrain, parse_terrain};
use engine_world::MAX_BAKE_SIZE;

use super::num::{f32_of, floor_idx, u32_of};

/// Samples per side of the height grid: the bake maximum, 256.
const GRID_SIDE: usize = 256;
/// Cells per side. The last sample index is this value.
const GRID_CELLS: usize = GRID_SIDE - 1;
/// Side of the square world, meters.
const WORLD_SIDE_M: f32 = 2000.0;
/// Half the side: the square runs from -HALF_M to +HALF_M on x and on z, meters.
const HALF_M: f32 = WORLD_SIDE_M * 0.5;
/// Distance between neighbouring samples, meters (about 7.8 m).
const CELL_M: f32 = WORLD_SIDE_M / 255.0;
/// Height span of the ground, meters: byte 0 sits at -30 m, byte 255 at +30 m.
const RELIEF_M: f32 = 60.0;
/// Cells per tile side: a 64-cell tile is 65 x 65 = 4,225 vertices.
const TILE_CELLS: usize = 64;
/// Most vertices one mesh may carry (the upload limit).
const VERTEX_LIMIT: usize = 65_535;
/// Edge of one ground texture repeat, meters.
const TEXTURE_TILE_M: f32 = 16.0;

/// The generated ground: `GRID_SIDE` x `GRID_SIDE` heights in meters, row by row along z.
#[derive(Debug)]
pub(super) struct DemoTerrain {
    heights_m: Vec<f32>,
}

impl DemoTerrain {
    /// Bakes the seed's terrain at the engine's largest size and scales it to the square.
    /// Errors: the bake or its parse was refused (neither happens at this fixed size).
    pub(super) fn generate(seed: u64) -> Result<Self, String> {
        let bytes = bake_terrain(seed, MAX_BAKE_SIZE).map_err(|e| e.to_string())?;
        let grid = parse_terrain(&bytes).ok_or("terrain bake did not parse")?;
        if usize::from(grid.size) != GRID_SIDE {
            return Err(format!(
                "terrain bake is {} wide, not {GRID_SIDE}",
                grid.size
            ));
        }
        let heights_m = grid
            .heights
            .iter()
            .map(|&b| (f32::from(b) / 255.0 - 0.5) * RELIEF_M)
            .collect();
        Ok(Self { heights_m })
    }

    /// Ground height at world (x, z), meters: bilinear between the four samples around the
    /// point. Outside the square the point clamps to the edge; a NaN coordinate reads as 0.
    pub(super) fn height_at(&self, x: f32, z: f32) -> f32 {
        let (gx, gz) = (grid_coord(x), grid_coord(z));
        let i = floor_idx(gx).min(GRID_CELLS - 1);
        let j = floor_idx(gz).min(GRID_CELLS - 1);
        let (fx, fz) = (gx - f32_of(i), gz - f32_of(j));
        let near = self.h(i, j) + (self.h(i + 1, j) - self.h(i, j)) * fx;
        let far = self.h(i, j + 1) + (self.h(i + 1, j + 1) - self.h(i, j + 1)) * fx;
        near + (far - near) * fz
    }

    /// The ground as render meshes: one per tile of `TILE_CELLS` cells a side (the last tile
    /// in each row and column may be smaller). Every mesh is under the vertex limit.
    pub(super) fn meshes(&self) -> Result<Vec<MeshData>, String> {
        let mut out = Vec::new();
        for j0 in (0..GRID_CELLS).step_by(TILE_CELLS) {
            for i0 in (0..GRID_CELLS).step_by(TILE_CELLS) {
                out.push(self.tile(i0, j0)?);
            }
        }
        Ok(out)
    }

    /// Height of sample (i, j), meters.
    fn h(&self, i: usize, j: usize) -> f32 {
        self.heights_m[j * GRID_SIDE + i]
    }

    /// Unit normal at sample (i, j) from central differences (one-sided at the edges).
    fn normal_at(&self, i: usize, j: usize) -> [f32; 3] {
        let (il, ir) = (i.saturating_sub(1), (i + 1).min(GRID_CELLS));
        let (jl, jr) = (j.saturating_sub(1), (j + 1).min(GRID_CELLS));
        let dx = (self.h(ir, j) - self.h(il, j)) / (f32_of(ir - il) * CELL_M);
        let dz = (self.h(i, jr) - self.h(i, jl)) / (f32_of(jr - jl) * CELL_M);
        let inv = (dx * dx + dz * dz + 1.0).sqrt().recip();
        [-dx * inv, inv, -dz * inv]
    }

    /// One tile whose first sample is (i0, j0), running to the grid's edge or `TILE_CELLS` on.
    fn tile(&self, i0: usize, j0: usize) -> Result<MeshData, String> {
        let i1 = (i0 + TILE_CELLS).min(GRID_CELLS);
        let j1 = (j0 + TILE_CELLS).min(GRID_CELLS);
        let width = i1 - i0 + 1;
        let depth = j1 - j0 + 1;
        let mut positions = Vec::with_capacity(width * depth);
        let mut normals = Vec::with_capacity(width * depth);
        let mut uvs = Vec::with_capacity(width * depth);
        for j in j0..=j1 {
            for i in i0..=i1 {
                let (x, z) = (sample_m(i), sample_m(j));
                positions.push([x, self.h(i, j), z]);
                normals.push(self.normal_at(i, j));
                uvs.push([x / TEXTURE_TILE_M, z / TEXTURE_TILE_M]);
            }
        }
        let mut indices = Vec::with_capacity((width - 1) * (depth - 1) * 6);
        for r in 0..depth - 1 {
            for c in 0..width - 1 {
                // a = (c, r), b = (c, r + 1), d = (c + 1, r): both triangles face up.
                let a = u32_of(r * width + c);
                let b = a + u32_of(width);
                indices.extend_from_slice(&[a, b, b + 1, a, b + 1, a + 1]);
            }
        }
        upload_mesh_tiled(&positions, &normals, &uvs, &indices)
            .map_err(|e| format!("terrain tile ({i0}, {j0}): {e}"))
    }
}

/// World coordinate of sample `i` along x or z, meters.
fn sample_m(i: usize) -> f32 {
    -HALF_M + f32_of(i) * CELL_M
}

/// Fractional sample index of world coordinate `v`: 0 at -HALF_M, `GRID_CELLS` at +HALF_M,
/// clamped to that range. A NaN reads as the origin.
fn grid_coord(v: f32) -> f32 {
    let v = if v.is_nan() {
        0.0
    } else {
        v.clamp(-HALF_M, HALF_M)
    };
    (v + HALF_M) / CELL_M
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: u64 = 7;

    fn terrain() -> DemoTerrain {
        DemoTerrain::generate(SEED).expect("the demo terrain bakes")
    }

    /// Vertex bytes and indices of every mesh in order: the mesh's identity on the GPU.
    fn mesh_bytes(meshes: &[MeshData]) -> Vec<u8> {
        let mut out = Vec::new();
        for m in meshes {
            out.extend(m.vertex_bytes());
            for i in &m.indices {
                out.extend_from_slice(&i.to_le_bytes());
            }
        }
        out
    }

    #[test]
    fn same_seed_gives_identical_mesh_bytes() {
        let a = mesh_bytes(&terrain().meshes().expect("tiles"));
        let b = mesh_bytes(&terrain().meshes().expect("tiles"));
        assert!(a == b, "the same seed must bake the same mesh bytes");
        let other = DemoTerrain::generate(SEED + 1).expect("bakes");
        assert!(
            a != mesh_bytes(&other.meshes().expect("tiles")),
            "another seed must give other ground"
        );
    }

    #[test]
    fn height_at_matches_the_grid_at_its_samples() {
        let t = terrain();
        for j in 0..=GRID_CELLS {
            for i in 0..=GRID_CELLS {
                let d = (t.height_at(sample_m(i), sample_m(j)) - t.h(i, j)).abs();
                assert!(d < 1e-3, "sample ({i}, {j}) is off by {d} m");
            }
        }
    }

    #[test]
    fn height_at_is_continuous_between_samples() {
        let t = terrain();
        // Along a line the bilinear ground changes no faster than the steepest sample step.
        let mut steepest = 0.0f32;
        for j in 0..GRID_CELLS {
            for i in 0..GRID_CELLS {
                steepest = steepest.max((t.h(i + 1, j) - t.h(i, j)).abs() / CELL_M);
                steepest = steepest.max((t.h(i, j + 1) - t.h(i, j)).abs() / CELL_M);
            }
        }
        const STEP_M: f32 = 0.25;
        let bound = steepest * STEP_M + 1e-3;
        for k in 0..20 {
            let fixed = -HALF_M + (k as f32 + 0.37) * (WORLD_SIDE_M / 20.0);
            let (mut prev_x, mut prev_z) =
                (t.height_at(-HALF_M, fixed), t.height_at(fixed, -HALF_M));
            for s in 1..=(WORLD_SIDE_M / STEP_M) as usize {
                let p = -HALF_M + s as f32 * STEP_M;
                let (hx, hz) = (t.height_at(p, fixed), t.height_at(fixed, p));
                assert!((hx - prev_x).abs() <= bound, "x step jumps at x={p}");
                assert!((hz - prev_z).abs() <= bound, "z step jumps at z={p}");
                (prev_x, prev_z) = (hx, hz);
            }
        }
    }

    #[test]
    fn every_normal_is_unit_and_points_up() {
        for m in terrain().meshes().expect("tiles") {
            for n in &m.normals {
                let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                assert!((len - 1.0).abs() < 1e-4, "normal {n:?} is not unit");
                assert!(n[1] > 0.0, "normal {n:?} points down");
            }
        }
    }

    #[test]
    fn every_tile_fits_the_upload_limit() {
        let meshes = terrain().meshes().expect("tiles");
        assert!(
            meshes.len() > 1,
            "the full 256 x 256 grid is over the limit, so it must tile"
        );
        for m in &meshes {
            assert!(
                m.positions.len() <= VERTEX_LIMIT,
                "tile has {} vertices",
                m.positions.len()
            );
        }
    }

    #[test]
    fn outside_the_square_clamps_without_nan() {
        let t = terrain();
        let same = |a: f32, b: f32| (a - b).abs() < 1e-4;
        assert!(
            same(t.height_at(1.0e6, 0.0), t.height_at(HALF_M, 0.0)),
            "east of the square clamps to its edge"
        );
        assert!(
            same(
                t.height_at(-1.0e6, f32::NEG_INFINITY),
                t.height_at(-HALF_M, -HALF_M)
            ),
            "a far corner clamps to the corner"
        );
        assert!(t.height_at(f32::NAN, 0.0).is_finite());
        assert!(t.height_at(f32::INFINITY, f32::NAN).is_finite());
    }
}
