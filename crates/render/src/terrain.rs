//! terrain: the one draw path a cooked height grid takes to the GPU.
//!
//! Owner: render lane; consumer is the frame that owns the arena ground.
//! Units: the cook is integer millimetres and q15 normals
//! (`engine_assets::collider_cook::{HeightGrid, cook_height_grid}`); world space
//! is meters, so the seam scales once per vertex. This module takes exactly the
//! fields `CookedTerrain::mesh` carries and keeps no edge to `engine-assets`,
//! because the default render graph is std-only.
//! Determinism: integer input, one division per vertex, no accumulation; the
//! bound sphere comes from [`crate::draw::bound_sphere`].
//! Budget: one terrain per frame, `MAX_TERRAIN_TRIANGLES`; buffer upload stays
//! in the existing v1 path, this module only names the mesh and counts its
//! triangles. Errors: typed `TerrainError`, never a panic.

use crate::draw::{bound_sphere, SceneNode, SceneSnapshot};
use engine_core::checked::{f32_from_f64, f64_from_u64};
use std::error::Error;
use std::fmt;

/// Name the frame lists the one terrain draw under.
pub const TERRAIN_DRAW_NAME: &str = "terrain";

/// Mesh identity a terrain carries into the draw list (`"TERRAIN1"` bytes).
pub const TERRAIN_MESH_ID: u64 = 0x5445_5252_4149_4e31;

/// Most triangles one terrain may carry: the 1024 x 1024 cook cap plus slack.
pub const MAX_TERRAIN_TRIANGLES: usize = 2_100_000;

/// The fixed-point scale a cooked normal arrives at (`engine-assets`).
const NORMAL_ONE: u64 = 32767;

/// Millimetres in one meter.
const MM_PER_M: f64 = 1000.0;

/// Typed terrain failures; every variant names what to fix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerrainError {
    /// The cook carries no vertex at all, so there is nothing to draw.
    NoMesh,
    /// The mesh's own shape is wrong; the reason is short and fixed.
    BadMesh(&'static str),
    /// More triangles than `MAX_TERRAIN_TRIANGLES`; split the grid.
    TooLarge {
        /// Triangle count the mesh asked for.
        triangles: usize,
    },
    /// One index points past the last vertex; nothing is guessed.
    IndexOutOfRange {
        /// The offending index.
        index: u32,
        /// Vertices the mesh actually has.
        verts: u32,
    },
}

impl fmt::Display for TerrainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoMesh => write!(f, "terrain: the cook carries no vertex"),
            Self::BadMesh(why) => write!(f, "terrain: bad mesh, {why}"),
            Self::TooLarge { triangles } => write!(
                f,
                "terrain: {triangles} triangles is over the {MAX_TERRAIN_TRIANGLES} cap"
            ),
            Self::IndexOutOfRange { index, verts } => {
                write!(f, "terrain: index {index} is past the {verts} vertices")
            }
        }
    }
}

impl Error for TerrainError {}

/// One uploaded terrain: the cooked mesh in meters plus the name it draws under.
#[derive(Clone, Debug, PartialEq)]
pub struct TerrainDraw {
    /// The name the frame lists this draw under; always [`TERRAIN_DRAW_NAME`].
    pub name: &'static str,
    /// Mesh identity; always [`TERRAIN_MESH_ID`].
    pub mesh: u64,
    /// Vertex positions in meters, row-major over the cooked grid.
    pub positions_m: Vec<[f32; 3]>,
    /// Unit normals, one per vertex.
    pub normals: Vec<[f32; 3]>,
    /// Three indices per triangle, wound to face up.
    pub indices: Vec<u32>,
    /// Triangle count, `indices.len() / 3`.
    pub triangles: usize,
    /// Bounding sphere in meters, `[x, y, z, radius]`.
    pub bounds: [f32; 4],
}

impl TerrainDraw {
    /// Take a cooked height-grid mesh: integer millimetres and q15 normals.
    ///
    /// Refuses a mesh whose parts disagree, an index past the last vertex, and a
    /// count over `MAX_TERRAIN_TRIANGLES`.
    pub fn from_cooked(
        positions_mm: &[[i32; 3]],
        normals_q15: &[[i16; 3]],
        indices: &[u32],
    ) -> Result<Self, TerrainError> {
        if positions_mm.is_empty() {
            return Err(TerrainError::NoMesh);
        }
        if normals_q15.len() != positions_mm.len() {
            return Err(TerrainError::BadMesh(
                "the normals do not match the vertices",
            ));
        }
        if indices.is_empty() || !indices.len().is_multiple_of(3) {
            return Err(TerrainError::BadMesh(
                "the indices are empty or not in threes",
            ));
        }
        let triangles = indices.len() / 3;
        if triangles > MAX_TERRAIN_TRIANGLES {
            return Err(TerrainError::TooLarge { triangles });
        }
        let verts = u32::try_from(positions_mm.len()).unwrap_or(u32::MAX);
        if let Some(&index) = indices.iter().find(|&&index| index >= verts) {
            return Err(TerrainError::IndexOutOfRange { index, verts });
        }
        let positions_m: Vec<[f32; 3]> = positions_mm
            .iter()
            .map(|point| [meters(point[0]), meters(point[1]), meters(point[2])])
            .collect();
        let normals: Vec<[f32; 3]> = normals_q15
            .iter()
            .map(|normal| {
                [
                    f32_from_f64(f64::from(normal[0]) / f64_from_u64(NORMAL_ONE)),
                    f32_from_f64(f64::from(normal[1]) / f64_from_u64(NORMAL_ONE)),
                    f32_from_f64(f64::from(normal[2]) / f64_from_u64(NORMAL_ONE)),
                ]
            })
            .collect();
        let bounds = bound_sphere(&positions_m);
        Ok(Self {
            name: TERRAIN_DRAW_NAME,
            mesh: TERRAIN_MESH_ID,
            positions_m,
            normals,
            indices: indices.to_vec(),
            triangles,
            bounds,
        })
    }

    /// The scene node this terrain posts: one opaque draw, one LOD level.
    #[must_use]
    pub fn scene_node(&self, id: u64, lod_table: u64) -> SceneNode {
        SceneNode {
            id,
            mesh: self.mesh,
            center_m: [self.bounds[0], self.bounds[1], self.bounds[2]],
            radius_m: self.bounds[3],
            lod_levels: 1,
            lod_table,
            prev_lod: 0,
        }
    }

    /// Post this terrain into a frame's node list under `id`.
    pub fn push_node(&self, snapshot: &mut SceneSnapshot, id: u64, lod_table: u64) {
        snapshot.nodes.push(self.scene_node(id, lod_table));
    }
}

/// One integer millimetre count as meters.
fn meters(mm: i32) -> f32 {
    f32_from_f64(f64::from(mm) / MM_PER_M)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::{Camera, DrawList, LodTable};

    /// The 3 x 3 grid the assets cook reads: same integers, same order.
    fn cooked_three_by_three() -> (Vec<[i32; 3]>, Vec<[i16; 3]>, Vec<u32>) {
        let heights = [0, 100, 200, 50, 150, 250, 100, 200, 300];
        let spacing_mm = 500;
        let mut positions = Vec::new();
        for row in 0..3i32 {
            for col in 0..3i32 {
                let at = usize::try_from(row * 3 + col).expect("in range");
                positions.push([col * spacing_mm, heights[at], row * spacing_mm]);
            }
        }
        let normals = vec![[0i16, 32767, 0]; 9];
        let mut indices = Vec::new();
        for row in 0..2u32 {
            for col in 0..2u32 {
                let a = row * 3 + col;
                let b = a + 1;
                let c = a + 3;
                let d = c + 1;
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
        }
        (positions, normals, indices)
    }

    #[test]
    fn a_cooked_grid_becomes_one_named_terrain_draw() {
        let (positions, normals, indices) = cooked_three_by_three();
        let terrain = TerrainDraw::from_cooked(&positions, &normals, &indices).expect("draws");
        assert_eq!(terrain.name, TERRAIN_DRAW_NAME);
        assert_eq!(terrain.name, "terrain");
        assert_eq!(terrain.mesh, TERRAIN_MESH_ID);
        assert_eq!(terrain.triangles, 8, "two triangles per quad");
        assert_eq!(terrain.indices.len(), 24);
        assert_eq!(terrain.positions_m.len(), 9);
        assert_eq!(terrain.normals.len(), 9);
        // Millimetres scale once: the last vertex is one meter out, 300 mm up.
        assert_eq!(terrain.positions_m[8], [1.0, 0.3, 1.0]);
        assert_eq!(terrain.normals[0], [0.0, 1.0, 0.0]);
        // The bound sphere covers the whole grid in meters.
        let [x, y, z, radius] = terrain.bounds;
        assert!((0.0..=1.0).contains(&x), "bounds center x {x}");
        assert!((0.0..=1.0).contains(&y), "bounds center y {y}");
        assert!((0.0..=1.0).contains(&z), "bounds center z {z}");
        assert!(radius > 0.5, "the grid is one meter across: {radius}");
    }

    #[test]
    fn the_terrain_is_the_one_draw_the_frame_lists() {
        let (positions, normals, indices) = cooked_three_by_three();
        let terrain = TerrainDraw::from_cooked(&positions, &normals, &indices).expect("draws");
        let table = LodTable {
            id: 7,
            bands_m: vec![10.0, 50.0],
            hysteresis_m: 0.5,
            pixel_thresholds_px: None,
        };
        let mut snapshot = SceneSnapshot::default();
        terrain.push_node(&mut snapshot, 42, table.id);
        assert_eq!(snapshot.nodes.len(), 1, "one terrain node");
        let camera = Camera {
            pos_m: [1.0, 2.0, -1.0],
            forward: [0.0, -1.0, 1.0],
            up: [0.0, 1.0, 0.0],
            fov_y_rad: 1.0,
            aspect: 1.6,
            near_m: 0.1,
            far_m: 100.0,
        };
        let mut list = DrawList::new();
        let draws = list.build(&snapshot, &camera, &table).expect("builds");
        assert_eq!(draws.len(), 1, "the ground is visible in the shot");
        assert_eq!(draws[0].id, 42);
        assert_eq!(draws[0].mesh, TERRAIN_MESH_ID);
        assert_eq!(draws[0].lod, 0);
        assert_eq!(list.draws()[0].mesh, TERRAIN_MESH_ID);
    }

    #[test]
    fn a_mesh_that_does_not_check_out_is_refused() {
        let (positions, normals, indices) = cooked_three_by_three();
        assert_eq!(
            TerrainDraw::from_cooked(&[], &[], &[0, 1, 2]).expect_err("no vertices"),
            TerrainError::NoMesh
        );
        assert_eq!(
            TerrainDraw::from_cooked(&positions, &normals[..8], &indices)
                .expect_err("short normals"),
            TerrainError::BadMesh("the normals do not match the vertices")
        );
        assert_eq!(
            TerrainDraw::from_cooked(&positions, &normals, &[0, 1]).expect_err("two indices"),
            TerrainError::BadMesh("the indices are empty or not in threes")
        );
        assert_eq!(
            TerrainDraw::from_cooked(&positions, &normals, &[0, 1, 9]).expect_err("past the end"),
            TerrainError::IndexOutOfRange { index: 9, verts: 9 }
        );
        let huge = vec![0u32; (MAX_TERRAIN_TRIANGLES + 1) * 3];
        assert_eq!(
            TerrainDraw::from_cooked(&[[0, 0, 0]], &[[0, 32767, 0]], &huge)
                .expect_err("over the cap"),
            TerrainError::TooLarge {
                triangles: MAX_TERRAIN_TRIANGLES + 1
            }
        );
    }
}
