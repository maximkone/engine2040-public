//! terrain: one cooked height record becomes a Rapier collider (WG5's physics accept).
//!
//! Owner: `engine-physfx`; data owner: the terrain cook in `engine-assets`
//! (`HeightfieldRecord`). The record crosses this seam as plain fields, never as the
//! asset type, so this crate keeps no edge to `engine-assets`: integer millimetre
//! heights in row-major order, a row and a column count, and one spacing for both axes.
//! Everything becomes meters once, here.
//!
//! Shape: Rapier's heightfield (parry's) takes exactly this grid, one row per Z and one
//! column per X, so every well-formed record fits it and that is the default shape.
//! [`TerrainShape::Mesh`] builds the same integer grid as a triangle mesh, two triangles
//! per quad, for a caller that needs triangles instead of the heightfield's cells.
//!
//! Placement: a record carries no origin, so the caller names one. `origin_mm` is the
//! world position of sample `(0, 0)` in whole millimetres, the units the cook wrote.
//! Rapier centers a heightfield and a mesh on their body, so the body sits at the grid's
//! center and the record stays integer: the half extent is computed once from
//! `(samples - 1) * spacing_mm`, never accumulated sample by sample.
//!
//! Determinism: the same record and origin give the same shape parameters bit for bit.
//! Heights are `i32` millimetres divided by 1000 exactly once; nothing here reads a
//! clock, a thread or an RNG, so a replay rebuilds the same collider.

use crate::groups;
use engine_physics::layers as table;
use rapier3d::dynamics::RigidBodyBuilder;
use rapier3d::geometry::{Array2, ColliderBuilder, SharedShape};
use rapier3d::math::Vector;
use rapier3d::parry::shape::{HeightField, TriMesh};
use std::fmt;

/// Most samples one record may carry: the terrain cook's own cap (1024 x 1024).
pub const MAX_SAMPLES: usize = 1024 * 1024;
/// Friction of a terrain collider, the same value the static boxes use.
pub const TERRAIN_FRICTION: f32 = 0.8;

/// Which shape carries one record into Rapier.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainShape {
    /// Rapier's heightfield: one shape of uniform cells, cheapest to query, and what the
    /// terrain cook's grid is made for.
    Heightfield,
    /// The same integer grid as a triangle mesh. A caller that needs triangles (a render
    /// cook, a per-triangle query) takes this one; it resting a body at the same height as
    /// the heightfield is the seam's own test.
    Mesh,
}

/// One cooked height record as physics reads it: whole millimetres, row-major.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeightTerrain {
    rows: u32,
    cols: u32,
    spacing_mm: u32,
    heights_mm: Vec<i32>,
}

/// Why a record was refused, named by the field that is wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainError {
    /// No rows or no columns: there is no grid.
    EmptyGrid,
    /// One row or one column: no quad exists, so there is nothing to stand on.
    FlatGrid {
        /// Declared rows.
        rows: u32,
        /// Declared columns.
        cols: u32,
    },
    /// The heights do not carry exactly one height per sample.
    ShortHeights {
        /// Declared rows.
        rows: u32,
        /// Declared columns.
        cols: u32,
        /// Heights actually carried.
        got: usize,
    },
    /// A spacing of zero would put every sample on one point.
    ZeroSpacing,
    /// More samples than [`MAX_SAMPLES`].
    TooManySamples {
        /// Samples the record declares.
        samples: usize,
        /// The cap.
        cap: usize,
    },
    /// Rapier refused a grid that passed the checks above; the record cannot be built.
    MeshRefused,
}

impl fmt::Display for TerrainError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyGrid => write!(f, "terrain: a grid needs rows and columns"),
            Self::FlatGrid { rows, cols } => write!(
                f,
                "terrain: {rows}x{cols} grid has no quad; a heightfield needs two rows and two columns"
            ),
            Self::ShortHeights { rows, cols, got } => write!(
                f,
                "terrain: {got} heights for a {rows}x{cols} grid ({} needed)",
                usize::try_from(rows.saturating_mul(*cols)).unwrap_or(usize::MAX)
            ),
            Self::ZeroSpacing => write!(f, "terrain: spacing is zero; samples would coincide"),
            Self::TooManySamples { samples, cap } => {
                write!(f, "terrain: {samples} samples past the {cap} cap")
            }
            Self::MeshRefused => write!(f, "terrain: Rapier refused the triangle mesh"),
        }
    }
}

impl std::error::Error for TerrainError {}

/// The exact parameters of one built shape, so two cooks can be compared bit for bit.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeParams {
    /// Heights in meters, row-major over the grid.
    pub heights_m: Vec<f32>,
    /// Extent in meters: X span, height multiplier (always 1.0), Z span.
    pub scale: [f32; 3],
    /// Body position in meters: the grid's center.
    pub center_m: [f32; 3],
}

impl HeightTerrain {
    /// Takes a cooked record's plain fields and checks its shape.
    ///
    /// Errors: [`TerrainError`], one case per wrong field.
    pub fn new(
        rows: u32,
        cols: u32,
        spacing_mm: u32,
        heights_mm: Vec<i32>,
    ) -> Result<Self, TerrainError> {
        if rows == 0 || cols == 0 {
            return Err(TerrainError::EmptyGrid);
        }
        let samples = usize::try_from(rows).unwrap_or(0) * usize::try_from(cols).unwrap_or(0);
        if samples > MAX_SAMPLES {
            return Err(TerrainError::TooManySamples {
                samples,
                cap: MAX_SAMPLES,
            });
        }
        if heights_mm.len() != samples {
            return Err(TerrainError::ShortHeights {
                rows,
                cols,
                got: heights_mm.len(),
            });
        }
        if spacing_mm == 0 {
            return Err(TerrainError::ZeroSpacing);
        }
        if rows < 2 || cols < 2 {
            return Err(TerrainError::FlatGrid { rows, cols });
        }
        Ok(Self {
            rows,
            cols,
            spacing_mm,
            heights_mm,
        })
    }

    /// Rows along Z.
    #[must_use]
    pub fn rows(&self) -> u32 {
        self.rows
    }

    /// Columns along X.
    #[must_use]
    pub fn cols(&self) -> u32 {
        self.cols
    }

    /// Sample spacing, whole millimetres.
    #[must_use]
    pub fn spacing_mm(&self) -> u32 {
        self.spacing_mm
    }

    /// Heights, whole millimetres, row-major.
    #[must_use]
    pub fn heights_mm(&self) -> &[i32] {
        &self.heights_mm
    }

    /// The shape parameters of this record with sample `(0, 0)` at `origin_mm`.
    ///
    /// Pure: integer inputs divided by 1000 exactly once, so two calls agree bit for bit.
    #[must_use]
    pub fn params(&self, origin_mm: [i32; 3]) -> ShapeParams {
        let heights_m = self.heights_mm.iter().map(|mm| meters_i32(*mm)).collect();
        let span_x = self.span_mm(self.cols);
        let span_z = self.span_mm(self.rows);
        ShapeParams {
            heights_m,
            scale: [meters_i32(span_x), 1.0, meters_i32(span_z)],
            center_m: [
                meters_i32(origin_mm[0]) + meters_i32(span_x) / 2.0,
                meters_i32(origin_mm[1]),
                meters_i32(origin_mm[2]) + meters_i32(span_z) / 2.0,
            ],
        }
    }

    /// The fixed body and collider for this record, sample `(0, 0)` at `origin_mm`.
    ///
    /// The collider sits on the table's world layer, so fighters, props and debris meet
    /// it. Errors: [`TerrainError::MeshRefused`] when Rapier refuses the mesh; the
    /// heightfield never refuses a record this type accepted.
    pub fn body_and_collider(
        &self,
        origin_mm: [i32; 3],
        shape: TerrainShape,
    ) -> Result<(RigidBodyBuilder, ColliderBuilder), TerrainError> {
        let params = self.params(origin_mm);
        let built = match shape {
            TerrainShape::Heightfield => self.heightfield(&params),
            TerrainShape::Mesh => self.mesh(&params)?,
        };
        let body = RigidBodyBuilder::fixed().translation(Vector::new(
            params.center_m[0],
            params.center_m[1],
            params.center_m[2],
        ));
        Ok((body, self.finish(built)))
    }

    /// Rapier's heightfield over the record's grid: row = Z, column = X, heights already
    /// meters and a height multiplier of one. Parry stores a grid column by column, so the
    /// row-major heights are walked column first.
    fn heightfield(&self, params: &ShapeParams) -> SharedShape {
        let rows = usize::try_from(self.rows).unwrap_or(0);
        let cols = usize::try_from(self.cols).unwrap_or(0);
        let mut data = Vec::with_capacity(rows * cols);
        for col in 0..cols {
            for row in 0..rows {
                data.push(
                    params
                        .heights_m
                        .get(row * cols + col)
                        .copied()
                        .unwrap_or(0.0),
                );
            }
        }
        let heights = Array2::new(rows, cols, data);
        let scale = Vector::new(params.scale[0], params.scale[1], params.scale[2]);
        SharedShape::new(HeightField::new(heights, scale))
    }

    /// The same grid as triangles, two per quad, wound so the faces point up.
    fn mesh(&self, params: &ShapeParams) -> Result<SharedShape, TerrainError> {
        let spacing = self.spacing_mm;
        let half_x = params.scale[0] / 2.0;
        let half_z = params.scale[2] / 2.0;
        let mut vertices = Vec::with_capacity(self.heights_mm.len());
        for row in 0..self.rows {
            for col in 0..self.cols {
                let index = usize::try_from(row * self.cols + col).unwrap_or(0);
                let height = params.heights_m.get(index).copied().unwrap_or(0.0);
                vertices.push(Vector::new(
                    meters_u32(col.saturating_mul(spacing)) - half_x,
                    height,
                    meters_u32(row.saturating_mul(spacing)) - half_z,
                ));
            }
        }
        let quads = self.rows.saturating_sub(1) * self.cols.saturating_sub(1);
        let mut indices = Vec::with_capacity(usize::try_from(quads).unwrap_or(0) * 2);
        for row in 0..self.rows - 1 {
            for col in 0..self.cols - 1 {
                let a = row * self.cols + col;
                let b = a + 1;
                let c = a + self.cols;
                let d = c + 1;
                indices.push([a, c, b]);
                indices.push([b, c, d]);
            }
        }
        TriMesh::new(vertices, indices)
            .map(SharedShape::new)
            .map_err(|_| TerrainError::MeshRefused)
    }

    /// The one place a terrain collider is tuned, so both shapes agree.
    fn finish(&self, shape: SharedShape) -> ColliderBuilder {
        ColliderBuilder::new(shape)
            .friction(TERRAIN_FRICTION)
            .restitution(0.0)
            .collision_groups(groups::of(table::WORLD))
    }

    /// Span of one axis in whole millimetres: samples minus one, times the spacing.
    fn span_mm(&self, samples: u32) -> i32 {
        let gaps = samples.saturating_sub(1);
        i32::try_from(gaps.saturating_mul(self.spacing_mm)).unwrap_or(i32::MAX)
    }
}

/// Whole millimetres to meters, the conversion of one height or origin, signed.
///
/// No `as` cast: the magnitude converts through the two halves of a `u32`, each through
/// `From<u16>`, and the sign returns afterwards. Exact for every magnitude below 2^24 mm
/// (16.7 km), far past any terrain sample.
fn meters_i32(mm: i32) -> f32 {
    let sign = if mm < 0 { -1.0 } else { 1.0 };
    sign * meters_u32(mm.unsigned_abs())
}

/// Whole millimetres to meters, the conversion of one span or sample offset, unsigned.
///
/// Same split as [`meters_i32`]; the sum is exact below 2^24 mm.
fn meters_u32(mm: u32) -> f32 {
    let high = u16::try_from(mm >> 16).unwrap_or(u16::MAX);
    let low = u16::try_from(mm & 0xffff).unwrap_or(0);
    (f32::from(high) * 65_536.0 + f32::from(low)) / 1000.0
}
