//! `geo` pack: geometry recipes (modeling verbs) an AI can preview and apply.
//!
//! A recipe is data: a seed, a source primitive and a list of typed ops, with
//! faces picked by predicates (`facing`, `area_over`, `all`), never by index.
//! `geo.preview` evaluates it with no side effects and reports counts, bounds,
//! the named selections and a digest; `geo.apply` evaluates the same recipe
//! and commits one `mesh` component (asset `geo/<digest>`) on a node through
//! the same previewed, capability-gated batch path as `tx.apply`, so the
//! scene records which recipe produced the mesh and undo removes it.
//!
//! Scope: the source is a `cube` or `plane`; `scale` and `select` are plain
//! Rust here. The modeling ops (`boolean`, `unwrap`, `lod`, `collision`,
//! `mirror`, `array`, `weld`, `flip`) call
//! the cook-time `engine-geo` crate through the `kernel` module, which exists
//! only when engine-endpoint is built with its off-by-default `geo` feature;
//! the default build has no geo crate and no C++ compiler step, and without
//! the feature those ops refuse with the typed code `feature_off`. Modeling
//! ops change the topology, so they must come before every `select`
//! (selections name faces of the mesh as it stands). Units: meters; ids and digests are lowercase hex. Determinism:
//! one input recipe gives one output digest (the plain ops use IEEE f32
//! multiply, add and `sqrt` only; the modeling ops repeat on one build,
//! cross-machine equality of the donor libraries is UNVERIFIED; canonical
//! bit order, no clock, no hash maps). Errors: [`McpToolError`] codes `bad_recipe` and `too_big` naming
//! the legal values. Bounds: at most [`MAX_RECIPE_OPS`] ops, names up to
//! [`MAX_SELECTION_NAME`] bytes, [`MAX_SELECTIONS`] selections, and indices
//! echoed up to [`MAX_ECHOED_FACES`] per selection.
//!
//! The file verbs (`geo.uv`, `geo.lod`, `geo.hull`, `geo.bake`, each a preview/apply pair) run
//! the same modules over a `.glb`/`.gltf` instead of a recipe, so an artist can unwrap,
//! reduce, collide and bake a mesh that already exists. They need both the `geo` feature
//! (the tools) and the `intake` feature (the glTF reader); see `meshfile`.

use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json::json;

use super::{PackDef, PackError, VerbSpec};
use crate::mcp::{McpToolError, OpDto, TxApplyArgs, TxPreviewArgs};
use crate::tool_capabilities::CapabilityCard;
use engine_core::Fnv64;

/// Pack name.
pub const PACK: &str = "geo";
/// Verb: evaluate a recipe, no side effects.
pub const TOOL_GEO_PREVIEW: &str = "geo.preview";
/// Verb: evaluate a recipe and commit its mesh on one node.
pub const TOOL_GEO_APPLY: &str = "geo.apply";

/// Most ops in one recipe.
pub const MAX_RECIPE_OPS: usize = 32;
/// Longest selection name in bytes.
pub const MAX_SELECTION_NAME: usize = 32;
/// Most named selections in one recipe.
pub const MAX_SELECTIONS: usize = 16;
/// Most face indices echoed per selection (the count is always exact).
pub const MAX_ECHOED_FACES: usize = 64;
/// Largest source extent per axis, meters.
pub const MAX_SOURCE_SIZE_M: f32 = 1000.0;
/// Most triangles a modeling op accepts or returns.
pub const MAX_MODEL_TRIS: usize = 50_000;
/// Most V-HACD parts one `collision` op may ask for.
pub const MAX_COLLISION_PARTS: u32 = 32;
/// Most copies one `array` op may ask for.
pub const MAX_ARRAY_COPIES: u32 = 64;
/// Largest `weld` distance in meters.
pub const MAX_WELD_M: f32 = 0.1;

/// Source shape of a recipe.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde", tag = "primitive", rename_all = "snake_case")]
#[schemars(crate = "rmcp::schemars")]
pub enum GeoSource {
    /// Axis-aligned box centered on the origin: 8 vertices, 12 triangles.
    Cube {
        /// Full size `[x, y, z]` in meters, each in (0, 1000].
        size: [f32; 3],
    },
    /// Flat quad on the XZ plane, normal +y: 4 vertices, 2 triangles.
    Plane {
        /// Full size `[x, z]` in meters, each in (0, 1000].
        size: [f32; 2],
    },
}

impl Default for GeoSource {
    /// The unit cube (the tool trait needs a default for empty args).
    fn default() -> Self {
        Self::Cube { size: [1.0; 3] }
    }
}

/// Face predicate: faces are picked by what they are, never by index.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde", tag = "by", rename_all = "snake_case")]
#[schemars(crate = "rmcp::schemars")]
pub enum FacePredicate {
    /// Every face.
    All,
    /// Faces whose normal has `dot(normal, axis) >= min_dot`.
    Facing {
        /// One of `+x -x +y -y +z -z`.
        axis: String,
        /// Cosine threshold in [-1, 1] (0.9 is within about 25 degrees).
        min_dot: f32,
    },
    /// Faces with an area over `m2` square meters.
    AreaOver {
        /// Area threshold in square meters, at least 0.
        m2: f32,
    },
}

/// Cutter solid for a `boolean` op, centered on the origin before its offset.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde", tag = "shape", rename_all = "snake_case")]
#[schemars(crate = "rmcp::schemars")]
pub enum GeoSolid {
    /// Box with full size `[x, y, z]` in meters, each in (0, 1000].
    Cube {
        /// Full size in meters.
        size: [f32; 3],
    },
    /// UV sphere.
    Sphere {
        /// Radius in meters, in (0, 1000].
        radius: f32,
        /// Bands pole to pole, 2..=64.
        rings: u32,
        /// Columns around, 3..=64.
        segments: u32,
    },
    /// Capped cylinder along y.
    Cylinder {
        /// Radius in meters, in (0, 1000].
        radius: f32,
        /// Height in meters, in (0, 1000].
        height: f32,
        /// Columns around, 3..=64.
        segments: u32,
    },
}

/// The tool a `boolean` op cuts with: a solid moved by an offset.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GeoCutter {
    /// The solid.
    #[serde(flatten)]
    pub solid: GeoSolid,
    /// Where its center sits, `[x, y, z]` meters, each within 1000.
    #[serde(default)]
    pub offset: [f32; 3],
}

/// Boolean flavour.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde", rename_all = "snake_case")]
#[schemars(crate = "rmcp::schemars")]
pub enum BoolKind {
    /// Source or cutter.
    Union,
    /// Source minus cutter.
    Difference,
    /// Where both overlap.
    Intersection,
}

/// One recipe op.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde", tag = "op", rename_all = "snake_case")]
#[schemars(crate = "rmcp::schemars")]
pub enum GeoOp {
    /// Scale every vertex per axis about the origin.
    Scale {
        /// Factors `[x, y, z]`, each finite and non-zero.
        by: [f32; 3],
    },
    /// Name the faces a predicate picks in the mesh as it stands.
    Select {
        /// Selection name (1..=32 of `a-z 0-9 _`), unique in the recipe.
        name: String,
        /// Which faces.
        faces: FacePredicate,
    },
    /// Cut the (closed) mesh with a solid. Needs the `geo` build feature; before any select.
    Boolean {
        /// The cutter.
        cutter: GeoCutter,
        /// Union, difference or intersection.
        kind: BoolKind,
    },
    /// Replace the UVs with an unwrapped atlas (vertices split on cuts). Needs the `geo`
    /// feature; before any select.
    Unwrap,
    /// Keep a fraction of the triangles, UV seams locked. Needs the `geo` feature; before any
    /// select.
    Lod {
        /// Fraction of triangles to keep, in [0.05, 1].
        keep: f32,
    },
    /// Split into closed convex collision parts (reported, the render mesh is unchanged).
    /// Needs the `geo` feature.
    Collision {
        /// Most parts, 1..=32.
        max_hulls: u32,
    },
    /// Add the mirror image through the origin plane of one axis. Needs the `geo` feature;
    /// before any select.
    Mirror {
        /// `x`, `y` or `z`.
        axis: String,
    },
    /// Repeat the mesh, copy `k` moved by `k` times `offset`. Needs the `geo` feature; before
    /// any select.
    Array {
        /// Copies, 1..=64 (the result stays within the triangle cap).
        count: u32,
        /// Step between copies `[x, y, z]` in meters, each within 1000.
        offset: [f32; 3],
    },
    /// Merge vertices closer than `epsilon`; triangles that collapse are dropped. Needs the
    /// `geo` feature; before any select.
    Weld {
        /// Distance in meters, in (0, 0.1].
        epsilon: f32,
    },
    /// Reverse every winding, so inside becomes outside. Needs the `geo` feature; before any
    /// select.
    Flip,
}

/// A typed recipe: seed, source, ops.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GeoRecipe {
    /// Seed for ops that use randomness (none yet); part of the digest.
    #[serde(default)]
    pub seed: u64,
    /// Source primitive.
    pub source: GeoSource,
    /// Ops in order, at most 32.
    #[serde(default)]
    pub ops: Vec<GeoOp>,
}

/// Args for `geo.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GeoPreviewArgs {
    /// The recipe to evaluate.
    pub recipe: GeoRecipe,
}

/// One named selection in a result.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SelectionOut {
    /// Selection name.
    pub name: String,
    /// Exact number of faces picked.
    pub count: u32,
    /// The first picked face indices (at most 64), ascending.
    pub faces: Vec<u32>,
}

/// UV facts after an `unwrap` op.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct UvOut {
    /// Charts in the atlas.
    pub charts: u32,
    /// Atlas size in texels `[width, height]`.
    pub atlas: [u32; 2],
    /// Chart pairs sharing atlas area (0 is the goal).
    pub overlapping_chart_pairs: u32,
}

/// Facts after a `lod` op.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LodOut {
    /// Triangles before.
    pub triangles_before: u32,
    /// Triangles after.
    pub triangles_after: u32,
    /// Vertices held fixed on UV seams and borders.
    pub locked_vertices: u32,
}

/// Facts after a `collision` op.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CollisionOut {
    /// Convex parts.
    pub parts: u32,
    /// Vertices of each part.
    pub part_vertices: Vec<u32>,
    /// Lowercase hex digest of all parts.
    pub digest: String,
}

/// Result of `geo.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GeoPreviewOutput {
    /// Lowercase hex digest of the final mesh, selections and seed.
    pub digest: String,
    /// Vertex count.
    pub vertices: u32,
    /// Triangle count.
    pub faces: u32,
    /// Axis-aligned bounds minimum `[x, y, z]`, meters.
    pub bounds_min: [f32; 3],
    /// Axis-aligned bounds maximum `[x, y, z]`, meters.
    pub bounds_max: [f32; 3],
    /// Named selections in recipe order.
    pub selections: Vec<SelectionOut>,
    /// Ops evaluated.
    pub ops_applied: u32,
    /// Set when the recipe ran an `unwrap` op.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uv: Option<UvOut>,
    /// Set when the recipe ran a `lod` op.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lod: Option<LodOut>,
    /// Set when the recipe ran a `collision` op.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub collision: Option<CollisionOut>,
}

/// Args for `geo.apply`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GeoApplyArgs {
    /// The recipe to evaluate and commit.
    pub recipe: GeoRecipe,
    /// Node (UUID text) that receives the `mesh` component.
    pub node: String,
    /// Must equal the document revision, else nothing runs.
    pub expected_revision: u64,
    /// Retry identity of the committed batch.
    pub idempotency_key: u64,
}

/// Result of `geo.apply`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct GeoApplyOutput {
    /// Digest of the committed mesh (same as `geo.preview` reports).
    pub digest: String,
    /// Mesh asset name written to the node (`geo/<digest>`).
    pub asset: String,
    /// Document revision after the commit.
    pub revision: u64,
    /// The batch effect lines.
    pub effects: Vec<String>,
}

/// Triangle mesh in meters.
struct Mesh {
    verts: Vec<[f32; 3]>,
    tris: Vec<[u32; 3]>,
    /// One UV per vertex after an `unwrap` op, else empty.
    uvs: Vec<[f32; 2]>,
}

#[cfg(feature = "geo")]
mod kernel;

/// The file verbs (`geo.uv|geo.lod|geo.hull|geo.bake`): the same four `engine-geo` modules run
/// over a `.glb`/`.gltf` instead of a recipe. Reading a glTF needs the `intake` feature, so the
/// module exists only with both features.
#[cfg(all(feature = "geo", feature = "intake"))]
mod meshfile;
#[cfg(all(feature = "geo", feature = "intake"))]
pub use meshfile::{
    BakeApplyArgs, BakeApplyOutput, BakeApplyTool, BakeMapOut, BakeMeshOut, BakePreviewArgs,
    BakePreviewOutput, BakePreviewTool, HullApplyArgs, HullApplyOutput, HullApplyTool, HullMeshOut,
    HullPreviewArgs, HullPreviewOutput, HullPreviewTool, LodApplyArgs, LodApplyOutput,
    LodApplyTool, LodLevelOut, LodPreviewArgs, LodPreviewOutput, LodPreviewTool, UvApplyArgs,
    UvApplyOutput, UvApplyTool, UvMeshOut, UvPreviewArgs, UvPreviewOutput, UvPreviewTool,
    DEFAULT_OUT_DIR as FILE_PAYLOAD_DIR, MAX_ATLAS, MAX_BAKE_SAMPLES as FILE_MAX_BAKE_SAMPLES,
    MAX_FILE_TRIANGLES, MAX_HULLS, MIN_ATLAS,
};

/// Stand-in for `kernel` in a build without the `geo` feature: every modeling op refuses with
/// the typed code `feature_off`.
#[cfg(not(feature = "geo"))]
mod kernel {
    use super::{BoolKind, CollisionFacts, GeoCutter, LodOut, Mesh, UvOut};
    use crate::mcp::McpToolError;

    fn feature_off(index: usize) -> McpToolError {
        McpToolError::new(
            "feature_off",
            format!("ops[{index}] needs the geo kernel; build engine-endpoint with feature `geo`"),
        )
    }

    pub(super) fn boolean(
        _: &mut Mesh,
        _: &GeoCutter,
        _: BoolKind,
        index: usize,
    ) -> Result<(), McpToolError> {
        Err(feature_off(index))
    }

    pub(super) fn unwrap(_: &mut Mesh, index: usize) -> Result<UvOut, McpToolError> {
        Err(feature_off(index))
    }

    pub(super) fn lod(_: &mut Mesh, _: f32, index: usize) -> Result<LodOut, McpToolError> {
        Err(feature_off(index))
    }

    pub(super) fn collision(
        _: &Mesh,
        _: u32,
        index: usize,
    ) -> Result<CollisionFacts, McpToolError> {
        Err(feature_off(index))
    }

    pub(super) fn mirror(_: &mut Mesh, _: &str, index: usize) -> Result<(), McpToolError> {
        Err(feature_off(index))
    }

    pub(super) fn array(
        _: &mut Mesh,
        _: u32,
        _: [f32; 3],
        index: usize,
    ) -> Result<(), McpToolError> {
        Err(feature_off(index))
    }

    pub(super) fn weld(_: &mut Mesh, _: f32, index: usize) -> Result<(), McpToolError> {
        Err(feature_off(index))
    }

    pub(super) fn flip(_: &mut Mesh, index: usize) -> Result<(), McpToolError> {
        Err(feature_off(index))
    }
}

fn bad(reason: String) -> McpToolError {
    McpToolError::new("bad_recipe", reason)
}

fn check_extent(label: &str, value: f32) -> Result<(), McpToolError> {
    if value.is_finite() && value > 0.0 && value <= MAX_SOURCE_SIZE_M {
        Ok(())
    } else {
        Err(bad(format!(
            "source {label} {value} is outside (0, {MAX_SOURCE_SIZE_M}] meters; send a finite positive size"
        )))
    }
}

/// Build the source primitive.
fn build_source(source: &GeoSource) -> Result<Mesh, McpToolError> {
    match source {
        GeoSource::Cube { size } => {
            for (label, value) in ["x", "y", "z"].into_iter().zip(size) {
                check_extent(label, *value)?;
            }
            let [hx, hy, hz] = [size[0] * 0.5, size[1] * 0.5, size[2] * 0.5];
            // Vertex bit 0 is x, bit 1 is y, bit 2 is z (0 means negative).
            let verts = (0..8u32)
                .map(|i| {
                    [
                        if i & 1 == 0 { -hx } else { hx },
                        if i & 2 == 0 { -hy } else { hy },
                        if i & 4 == 0 { -hz } else { hz },
                    ]
                })
                .collect();
            // Two outward counter-clockwise triangles per side: +x -x +y -y +z -z.
            let tris = vec![
                [1, 3, 7],
                [1, 7, 5],
                [0, 4, 6],
                [0, 6, 2],
                [2, 7, 3],
                [2, 6, 7],
                [0, 1, 5],
                [0, 5, 4],
                [4, 5, 7],
                [4, 7, 6],
                [0, 2, 3],
                [0, 3, 1],
            ];
            Ok(Mesh {
                verts,
                tris,
                uvs: Vec::new(),
            })
        }
        GeoSource::Plane { size } => {
            check_extent("x", size[0])?;
            check_extent("z", size[1])?;
            let (hx, hz) = (size[0] * 0.5, size[1] * 0.5);
            Ok(Mesh {
                verts: vec![
                    [-hx, 0.0, -hz],
                    [hx, 0.0, -hz],
                    [hx, 0.0, hz],
                    [-hx, 0.0, hz],
                ],
                tris: vec![[0, 2, 1], [0, 3, 2]],
                uvs: Vec::new(),
            })
        }
    }
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

/// Unit normal and area of one triangle (a degenerate triangle has area 0 and
/// a zero normal).
fn face_frame(mesh: &Mesh, tri: [u32; 3]) -> ([f32; 3], f32) {
    let at = |index: u32| mesh.verts[usize::try_from(index).unwrap_or(0)];
    let (a, b, c) = (at(tri[0]), at(tri[1]), at(tri[2]));
    let (u, v) = (sub(b, a), sub(c, a));
    // Twice the area is the length of the edge cross product.
    let n = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let twice = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if twice > 0.0 {
        ([n[0] / twice, n[1] / twice, n[2] / twice], twice * 0.5)
    } else {
        ([0.0; 3], 0.0)
    }
}

fn axis_of(text: &str) -> Result<[f32; 3], McpToolError> {
    match text {
        "+x" => Ok([1.0, 0.0, 0.0]),
        "-x" => Ok([-1.0, 0.0, 0.0]),
        "+y" => Ok([0.0, 1.0, 0.0]),
        "-y" => Ok([0.0, -1.0, 0.0]),
        "+z" => Ok([0.0, 0.0, 1.0]),
        "-z" => Ok([0.0, 0.0, -1.0]),
        other => Err(bad(format!(
            "axis `{other}` is not an axis; use one of +x, -x, +y, -y, +z, -z"
        ))),
    }
}

/// Face indices a predicate picks, ascending.
fn pick(mesh: &Mesh, predicate: &FacePredicate) -> Result<Vec<u32>, McpToolError> {
    let mut picked = Vec::new();
    let axis = match predicate {
        FacePredicate::Facing { axis, min_dot } => {
            if !min_dot.is_finite() || !(-1.0..=1.0).contains(min_dot) {
                return Err(bad(format!(
                    "min_dot {min_dot} is outside [-1, 1]; send a cosine threshold"
                )));
            }
            Some(axis_of(axis)?)
        }
        FacePredicate::AreaOver { m2 } => {
            if !m2.is_finite() || *m2 < 0.0 {
                return Err(bad(format!(
                    "m2 {m2} is not a finite area of at least 0; send square meters"
                )));
            }
            None
        }
        FacePredicate::All => None,
    };
    for (index, tri) in mesh.tris.iter().enumerate() {
        let (normal, area) = face_frame(mesh, *tri);
        let hit = match predicate {
            FacePredicate::All => true,
            FacePredicate::Facing { min_dot, .. } => {
                let a = axis.unwrap_or([0.0; 3]);
                normal[0] * a[0] + normal[1] * a[1] + normal[2] * a[2] >= *min_dot
            }
            FacePredicate::AreaOver { m2 } => area > *m2,
        };
        if hit {
            // Triangle counts are tiny next to u32::MAX; the fallback is dead.
            picked.push(u32::try_from(index).unwrap_or(u32::MAX));
        }
    }
    Ok(picked)
}

fn name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_SELECTION_NAME
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Scale every vertex per axis; an odd number of mirrored axes flips every
/// winding so normals keep pointing outward.
fn scale_mesh(mesh: &mut Mesh, by: [f32; 3], index: usize) -> Result<(), McpToolError> {
    for factor in by {
        if !factor.is_finite() || factor == 0.0 {
            return Err(bad(format!(
                "ops[{index}].by holds {factor}; send finite non-zero scale factors"
            )));
        }
    }
    for vert in &mut mesh.verts {
        vert[0] *= by[0];
        vert[1] *= by[1];
        vert[2] *= by[2];
    }
    if by.iter().filter(|factor| **factor < 0.0).count() % 2 == 1 {
        for tri in &mut mesh.tris {
            tri.swap(1, 2);
        }
    }
    Ok(())
}

/// Axis-aligned bounds of the vertices.
fn bounds_of(mesh: &Mesh) -> ([f32; 3], [f32; 3]) {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for vert in &mesh.verts {
        for axis in 0..3 {
            lo[axis] = lo[axis].min(vert[axis]);
            hi[axis] = hi[axis].max(vert[axis]);
        }
    }
    (lo, hi)
}

/// Canonical digest of seed, final mesh bits and the named selections.
fn digest_of(seed: u64, mesh: &Mesh, selections: &[(String, Vec<u32>)], parts: Option<u64>) -> u64 {
    let mut hash = Fnv64::new();
    hash.write(&seed.to_le_bytes());
    for vert in &mesh.verts {
        for lane in vert {
            hash.write(&lane.to_bits().to_le_bytes());
        }
    }
    for tri in &mesh.tris {
        for index in tri {
            hash.write(&index.to_le_bytes());
        }
    }
    // UVs and collision parts join the digest only when a modeling op made them, so plain
    // recipes keep the digests they had before the modeling ops existed.
    if !mesh.uvs.is_empty() {
        hash.write(
            &u64::try_from(mesh.uvs.len())
                .unwrap_or(u64::MAX)
                .to_le_bytes(),
        );
        for uv in &mesh.uvs {
            for lane in uv {
                hash.write(&lane.to_bits().to_le_bytes());
            }
        }
    }
    if let Some(parts) = parts {
        hash.write(&parts.to_le_bytes());
    }
    for (name, faces) in selections {
        // Lengths prefix the variable fields so neighbors cannot collide.
        let (name_len, face_len) = (
            u64::try_from(name.len()).unwrap_or(u64::MAX),
            u64::try_from(faces.len()).unwrap_or(u64::MAX),
        );
        hash.write(&name_len.to_le_bytes());
        hash.write(name.as_bytes());
        hash.write(&face_len.to_le_bytes());
        for face in faces {
            hash.write(&face.to_le_bytes());
        }
    }
    hash.finish()
}

/// What the modeling ops reported.
#[derive(Default)]
struct Facts {
    uv: Option<UvOut>,
    lod: Option<LodOut>,
    collision: Option<CollisionFacts>,
}

/// A `collision` op's report plus the digest bits that join the recipe digest.
struct CollisionFacts {
    out: CollisionOut,
    digest_bits: u64,
}

impl GeoOp {
    /// Ops that rebuild the faces, so earlier selections would name the wrong ones.
    fn changes_topology(&self) -> bool {
        matches!(
            self,
            Self::Boolean { .. }
                | Self::Unwrap
                | Self::Lod { .. }
                | Self::Mirror { .. }
                | Self::Array { .. }
                | Self::Weld { .. }
                | Self::Flip
        )
    }
}

/// Evaluate a recipe: build the source, run the ops in order, digest the
/// result. No side effects.
pub fn evaluate(recipe: &GeoRecipe) -> Result<GeoPreviewOutput, McpToolError> {
    if recipe.ops.len() > MAX_RECIPE_OPS {
        return Err(McpToolError::new(
            "too_big",
            format!(
                "recipe holds {} ops; at most {MAX_RECIPE_OPS} per recipe; split it into two recipes",
                recipe.ops.len()
            ),
        ));
    }
    let mut mesh = build_source(&recipe.source)?;
    let mut selections: Vec<(String, Vec<u32>)> = Vec::new();
    let mut facts = Facts::default();
    for (index, op) in recipe.ops.iter().enumerate() {
        if op.changes_topology() && !selections.is_empty() {
            return Err(bad(format!(
                "ops[{index}] changes the faces but follows a select; move every select after it"
            )));
        }
        match op {
            GeoOp::Scale { by } => scale_mesh(&mut mesh, *by, index)?,
            GeoOp::Select { name, faces } => {
                if !name_ok(name) {
                    return Err(bad(format!(
                        "ops[{index}].name `{name}` is not 1..={MAX_SELECTION_NAME} bytes of a-z 0-9 _; rename it"
                    )));
                }
                if selections.iter().any(|(kept, _)| kept == name) {
                    return Err(bad(format!(
                        "ops[{index}].name `{name}` repeats an earlier selection; names are unique"
                    )));
                }
                if selections.len() >= MAX_SELECTIONS {
                    return Err(McpToolError::new(
                        "too_big",
                        format!(
                            "recipe holds more than {MAX_SELECTIONS} selections; merge predicates"
                        ),
                    ));
                }
                selections.push((name.clone(), pick(&mesh, faces)?));
            }
            GeoOp::Boolean { cutter, kind } => kernel::boolean(&mut mesh, cutter, *kind, index)?,
            GeoOp::Unwrap => facts.uv = Some(kernel::unwrap(&mut mesh, index)?),
            GeoOp::Lod { keep } => facts.lod = Some(kernel::lod(&mut mesh, *keep, index)?),
            GeoOp::Collision { max_hulls } => {
                facts.collision = Some(kernel::collision(&mesh, *max_hulls, index)?);
            }
            GeoOp::Mirror { axis } => kernel::mirror(&mut mesh, axis, index)?,
            GeoOp::Array { count, offset } => kernel::array(&mut mesh, *count, *offset, index)?,
            GeoOp::Weld { epsilon } => kernel::weld(&mut mesh, *epsilon, index)?,
            GeoOp::Flip => kernel::flip(&mut mesh, index)?,
        }
    }
    let (lo, hi) = bounds_of(&mesh);
    Ok(GeoPreviewOutput {
        digest: format!(
            "{:016x}",
            digest_of(
                recipe.seed,
                &mesh,
                &selections,
                facts.collision.as_ref().map(|c| c.digest_bits)
            )
        ),
        vertices: u32::try_from(mesh.verts.len()).unwrap_or(u32::MAX),
        faces: u32::try_from(mesh.tris.len()).unwrap_or(u32::MAX),
        bounds_min: lo,
        bounds_max: hi,
        selections: selections
            .into_iter()
            .map(|(name, faces)| SelectionOut {
                name,
                count: u32::try_from(faces.len()).unwrap_or(u32::MAX),
                faces: faces.into_iter().take(MAX_ECHOED_FACES).collect(),
            })
            .collect(),
        ops_applied: u32::try_from(recipe.ops.len()).unwrap_or(u32::MAX),
        uv: facts.uv,
        lod: facts.lod,
        collision: facts.collision.map(|c| c.out),
    })
}

/// `geo.preview`: evaluate, no side effects.
fn geo_preview(
    _server: &crate::mcp::McpServer,
    args: &GeoPreviewArgs,
) -> Result<GeoPreviewOutput, McpToolError> {
    evaluate(&args.recipe)
}

/// `geo.apply`: evaluate, then commit one `mesh` component through the
/// previewed, gated batch path. Any refusal (stale revision, unknown node,
/// capability) leaves the document untouched.
fn geo_apply(
    server: &crate::mcp::McpServer,
    args: &GeoApplyArgs,
) -> Result<GeoApplyOutput, McpToolError> {
    let result = evaluate(&args.recipe)?;
    let asset = format!("geo/{}", result.digest);
    let ops = vec![OpDto::SetComponent {
        id: args.node.clone(),
        component: "mesh".to_owned(),
        value: json!({ "asset": asset }),
        version: None,
    }];
    let applied = server.dispatch(TOOL_GEO_APPLY, |state| {
        let preview = state.tx_preview(&TxPreviewArgs {
            expected_revision: args.expected_revision,
            idempotency_key: args.idempotency_key,
            ops: ops.clone(),
        })?;
        state.tx_apply(&TxApplyArgs {
            expected_revision: args.expected_revision,
            idempotency_key: args.idempotency_key,
            ops,
            approval: preview.approval,
            delta: Some(false),
            origin: None,
        })
    })?;
    Ok(GeoApplyOutput {
        digest: result.digest,
        asset,
        revision: applied.revision,
        effects: applied.effects,
    })
}

crate::e2_tool!(
    /// `geo.preview`: evaluate a typed geometry recipe, no side effects.
    GeoPreviewTool,
    name = TOOL_GEO_PREVIEW,
    description = "Evaluate a geometry recipe (seed, source primitive, typed ops with predicate face picks); returns counts, bounds, named selections and a digest. No side effects.",
    readonly = true,
    args = GeoPreviewArgs,
    output = GeoPreviewOutput,
    invoke = geo_preview
);

crate::e2_tool!(
    /// `geo.apply`: commit a recipe's mesh on one node through the gated batch path.
    GeoApplyTool,
    name = TOOL_GEO_APPLY,
    description = "Evaluate a geometry recipe and set its mesh (asset geo/<digest>) on one node through the previewed, capability-gated batch; undoable.",
    readonly = false,
    args = GeoApplyArgs,
    output = GeoApplyOutput,
    invoke = geo_apply
);

/// The geo pack: preview and apply, loaded on demand.
pub fn pack() -> Result<PackDef, PackError> {
    let mut pack = PackDef::new(
        PACK,
        "geometry recipes: preview and apply a typed op list plus seed with predicate face \
         picks; ops scale and select, plus boolean, unwrap, lod, collision, mirror, array, \
         weld and flip when the build has the geo feature",
        false,
    );
    pack.add(VerbSpec::of::<GeoPreviewTool>(
        TOOL_GEO_PREVIEW,
        PACK,
        CapabilityCard::Observe,
    )?);
    pack.add(VerbSpec::of::<GeoApplyTool>(
        TOOL_GEO_APPLY,
        PACK,
        CapabilityCard::Commit,
    )?);
    #[cfg(all(feature = "geo", feature = "intake"))]
    {
        pack.add(VerbSpec::of::<meshfile::UvPreviewTool>(
            meshfile::TOOL_GEO_UV_PREVIEW,
            PACK,
            CapabilityCard::Observe,
        )?);
        pack.add(VerbSpec::of::<meshfile::UvApplyTool>(
            meshfile::TOOL_GEO_UV_APPLY,
            PACK,
            CapabilityCard::Commit,
        )?);
        pack.add(VerbSpec::of::<meshfile::LodPreviewTool>(
            meshfile::TOOL_GEO_LOD_PREVIEW,
            PACK,
            CapabilityCard::Observe,
        )?);
        pack.add(VerbSpec::of::<meshfile::LodApplyTool>(
            meshfile::TOOL_GEO_LOD_APPLY,
            PACK,
            CapabilityCard::Commit,
        )?);
        pack.add(VerbSpec::of::<meshfile::HullPreviewTool>(
            meshfile::TOOL_GEO_HULL_PREVIEW,
            PACK,
            CapabilityCard::Observe,
        )?);
        pack.add(VerbSpec::of::<meshfile::HullApplyTool>(
            meshfile::TOOL_GEO_HULL_APPLY,
            PACK,
            CapabilityCard::Commit,
        )?);
        pack.add(VerbSpec::of::<meshfile::BakePreviewTool>(
            meshfile::TOOL_GEO_BAKE_PREVIEW,
            PACK,
            CapabilityCard::Observe,
        )?);
        pack.add(VerbSpec::of::<meshfile::BakeApplyTool>(
            meshfile::TOOL_GEO_BAKE_APPLY,
            PACK,
            CapabilityCard::Commit,
        )?);
    }
    Ok(pack)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::{McpServer, TxUndoArgs, TxUndoTool};
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;

    fn cube(size: [f32; 3], ops: Vec<GeoOp>) -> GeoRecipe {
        GeoRecipe {
            seed: 7,
            source: GeoSource::Cube { size },
            ops,
        }
    }

    fn top() -> GeoOp {
        GeoOp::Select {
            name: "top".to_owned(),
            faces: FacePredicate::Facing {
                axis: "+y".to_owned(),
                min_dot: 0.9,
            },
        }
    }

    #[test]
    fn a_cube_has_eight_vertices_twelve_faces_and_outward_normals() {
        let out = evaluate(&cube([2.0, 2.0, 2.0], vec![])).expect("evaluates");
        assert_eq!((out.vertices, out.faces), (8, 12));
        assert_eq!(out.bounds_min, [-1.0; 3]);
        assert_eq!(out.bounds_max, [1.0; 3]);
        // Each axis direction picks exactly the two triangles of that side,
        // and every side normal points outward.
        for axis in ["+x", "-x", "+y", "-y", "+z", "-z"] {
            let out = evaluate(&cube(
                [1.0, 1.0, 1.0],
                vec![GeoOp::Select {
                    name: "side".to_owned(),
                    faces: FacePredicate::Facing {
                        axis: axis.to_owned(),
                        min_dot: 0.99,
                    },
                }],
            ))
            .expect("evaluates");
            assert_eq!(out.selections[0].count, 2, "axis {axis}");
        }
    }

    #[test]
    fn scale_and_select_by_predicate_not_index() {
        let out = evaluate(&cube(
            [1.0, 1.0, 1.0],
            vec![
                GeoOp::Scale {
                    by: [2.0, 1.0, 1.0],
                },
                top(),
            ],
        ))
        .expect("evaluates");
        assert_eq!(out.bounds_max, [1.0, 0.5, 0.5]);
        assert_eq!(out.selections.len(), 1);
        assert_eq!(out.selections[0].name, "top");
        assert_eq!(out.selections[0].count, 2, "the top side is two triangles");
        // Area predicate: the big x-z faces (area 2 x 1) beat the small ones.
        let big = evaluate(&cube(
            [2.0, 1.0, 1.0],
            vec![GeoOp::Select {
                name: "big".to_owned(),
                faces: FacePredicate::AreaOver { m2: 0.9 },
            }],
        ))
        .expect("evaluates");
        // Sides: x faces are 1x1 (two triangles of 0.5), y and z faces are 2x1
        // (two triangles of 1.0): the four y/z sides = 8 triangles.
        assert_eq!(big.selections[0].count, 8);
        let all = evaluate(&cube(
            [1.0; 3],
            vec![GeoOp::Select {
                name: "all".to_owned(),
                faces: FacePredicate::All,
            }],
        ))
        .expect("evaluates");
        assert_eq!(all.selections[0].count, 12);
    }

    #[test]
    fn a_mirrored_scale_keeps_normals_outward() {
        let out = evaluate(&cube(
            [1.0; 3],
            vec![
                GeoOp::Scale {
                    by: [-1.0, 1.0, 1.0],
                },
                top(),
            ],
        ))
        .expect("evaluates");
        assert_eq!(
            out.selections[0].count, 2,
            "mirroring does not flip the top"
        );
    }

    #[test]
    fn same_recipe_same_digest_and_any_change_moves_it() {
        let a = evaluate(&cube([1.0; 3], vec![top()])).expect("a");
        let b = evaluate(&cube([1.0; 3], vec![top()])).expect("b");
        assert_eq!(a, b, "evaluation is deterministic");
        assert_eq!(a.digest.len(), 16);
        let mut other_seed = cube([1.0; 3], vec![top()]);
        other_seed.seed = 8;
        assert_ne!(evaluate(&other_seed).expect("seed").digest, a.digest);
        let scaled = evaluate(&cube(
            [1.0; 3],
            vec![
                GeoOp::Scale {
                    by: [1.0, 2.0, 1.0],
                },
                top(),
            ],
        ))
        .expect("scaled");
        assert_ne!(scaled.digest, a.digest);
        let plane = evaluate(&GeoRecipe {
            seed: 7,
            source: GeoSource::Plane { size: [4.0, 2.0] },
            ops: vec![top()],
        })
        .expect("plane");
        assert_eq!((plane.vertices, plane.faces), (4, 2));
        assert_eq!(plane.selections[0].count, 2, "a plane faces +y");
    }

    #[test]
    fn bad_recipes_refuse_typed_with_the_legal_values() {
        let refuse = |recipe: GeoRecipe| evaluate(&recipe).expect_err("refused");
        let flat = refuse(cube([0.0, 1.0, 1.0], vec![]));
        assert_eq!(flat.code, "bad_recipe");
        assert!(flat.message.contains("(0, 1000]"), "{}", flat.message);
        assert_eq!(
            refuse(cube([f32::NAN, 1.0, 1.0], vec![])).code,
            "bad_recipe"
        );
        let zero = refuse(cube(
            [1.0; 3],
            vec![GeoOp::Scale {
                by: [0.0, 1.0, 1.0],
            }],
        ));
        assert!(zero.message.contains("non-zero"), "{}", zero.message);
        let axis = refuse(cube(
            [1.0; 3],
            vec![GeoOp::Select {
                name: "x".to_owned(),
                faces: FacePredicate::Facing {
                    axis: "up".to_owned(),
                    min_dot: 0.5,
                },
            }],
        ));
        assert!(axis.message.contains("+x, -x"), "{}", axis.message);
        let dot = refuse(cube(
            [1.0; 3],
            vec![GeoOp::Select {
                name: "x".to_owned(),
                faces: FacePredicate::Facing {
                    axis: "+y".to_owned(),
                    min_dot: 2.0,
                },
            }],
        ));
        assert!(dot.message.contains("[-1, 1]"), "{}", dot.message);
        let name = refuse(cube(
            [1.0; 3],
            vec![GeoOp::Select {
                name: "Top Face".to_owned(),
                faces: FacePredicate::All,
            }],
        ));
        assert!(name.message.contains("a-z 0-9 _"), "{}", name.message);
        let dup = refuse(cube([1.0; 3], vec![top(), top()]));
        assert!(dup.message.contains("repeats"), "{}", dup.message);
        let many = refuse(cube(
            [1.0; 3],
            (0..=MAX_RECIPE_OPS)
                .map(|_| GeoOp::Scale { by: [1.0; 3] })
                .collect(),
        ));
        assert_eq!(many.code, "too_big");
    }

    #[test]
    fn recipes_parse_from_plain_json() {
        let recipe: GeoRecipe = rmcp::serde_json::from_value(json!({
            "seed": 3,
            "source": {"primitive": "cube", "size": [1, 2, 3]},
            "ops": [
                {"op": "scale", "by": [2, 1, 1]},
                {"op": "select", "name": "top", "faces": {"by": "facing", "axis": "+y", "min_dot": 0.9}}
            ]
        }))
        .expect("plain JSON parses");
        assert_eq!(recipe.ops.len(), 2);
        assert!(evaluate(&recipe).is_ok());
        // An unknown op tag fails to parse (no silent skip).
        let unknown = rmcp::serde_json::from_value::<GeoRecipe>(json!({
            "source": {"primitive": "cube", "size": [1, 1, 1]},
            "ops": [{"op": "boolean_union"}]
        }));
        assert!(unknown.is_err());
    }

    fn server_with_node() -> (McpServer, String) {
        use crate::mcp::{TxApplyTool, TxPreviewTool};
        let server = McpServer::new(7, Capability::unrestricted(7));
        let id = engine_scene::ids::ObjectId::new().to_string();
        let ops = vec![OpDto::Insert {
            id: id.clone(),
            parent: engine_scene::ids::ObjectId::NIL.to_string(),
            local: crate::mcp::PoseDto {
                pos: [0.0; 3],
                rot: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0; 3],
            },
        }];
        let drafted = TxPreviewTool::invoke(
            &server,
            TxPreviewArgs {
                expected_revision: 0,
                idempotency_key: 1,
                ops: ops.clone(),
            },
        )
        .expect("insert previews");
        TxApplyTool::invoke(
            &server,
            TxApplyArgs {
                expected_revision: 0,
                idempotency_key: 1,
                ops,
                approval: drafted.approval,
                delta: None,
                origin: None,
            },
        )
        .expect("insert lands");
        (server, id)
    }

    #[test]
    fn preview_has_no_side_effects_and_apply_commits_a_mesh_that_undoes() {
        let (server, id) = server_with_node();
        let recipe = cube([1.0; 3], vec![top()]);
        let previewed = GeoPreviewTool::invoke(
            &server,
            GeoPreviewArgs {
                recipe: recipe.clone(),
            },
        )
        .expect("previews");
        assert_eq!(
            server.lock().expect("lock").doc.revision,
            1,
            "preview writes nothing"
        );
        let applied = GeoApplyTool::invoke(
            &server,
            GeoApplyArgs {
                recipe: recipe.clone(),
                node: id.clone(),
                expected_revision: 1,
                idempotency_key: 2,
            },
        )
        .expect("applies");
        assert_eq!(applied.digest, previewed.digest);
        assert_eq!(applied.asset, format!("geo/{}", previewed.digest));
        assert_eq!(applied.revision, 2);
        {
            let state = server.lock().expect("lock");
            let slot = state
                .doc
                .components
                .get(&id)
                .expect("node carries components");
            assert!(slot.contains_key(&engine_scene::component::ComponentKind::Mesh));
        }
        // A stale revision refuses and writes nothing.
        let stale = GeoApplyTool::invoke(
            &server,
            GeoApplyArgs {
                recipe,
                node: id,
                expected_revision: 1,
                idempotency_key: 3,
            },
        )
        .expect_err("stale revision refuses");
        assert_eq!(stale.code, "stale_revision");
        assert_eq!(server.lock().expect("lock").doc.revision, 2);
        TxUndoTool::invoke(&server, TxUndoArgs { undo_token: None }).expect("undo");
        assert!(server.lock().expect("lock").doc.components.is_empty());
    }

    #[test]
    fn the_geo_pack_registers_both_verbs_with_their_cards() {
        let registry = super::super::PackRegistry::shipped();
        assert!(registry.errors().is_empty(), "{:?}", registry.errors());
        let preview = registry.verb(TOOL_GEO_PREVIEW).expect("geo.preview");
        let apply = registry.verb(TOOL_GEO_APPLY).expect("geo.apply");
        assert_eq!(preview.card(), CapabilityCard::Observe);
        assert!(preview.readonly());
        assert_eq!(apply.card(), CapabilityCard::Commit);
        assert!(!apply.readonly());
    }

    fn tower(ops: Vec<GeoOp>) -> GeoRecipe {
        cube([2.0, 2.0, 2.0], ops)
    }

    #[test]
    fn json_parses_the_modeling_ops_and_still_rejects_unknown_ones() {
        let recipe: GeoRecipe = rmcp::serde_json::from_value(json!({
            "seed": 1,
            "source": {"primitive": "cube", "size": [2, 2, 2]},
            "ops": [
                {"op": "boolean", "kind": "difference",
                 "cutter": {"shape": "cylinder", "radius": 0.5, "height": 3, "segments": 16, "offset": [0, 0, 0]}},
                {"op": "unwrap"},
                {"op": "lod", "keep": 0.5},
                {"op": "collision", "max_hulls": 4}
            ]
        }))
        .expect("plain JSON parses");
        assert_eq!(recipe.ops.len(), 4);
        assert!(matches!(recipe.ops[0], GeoOp::Boolean { .. }));
        let unknown = rmcp::serde_json::from_value::<GeoRecipe>(json!({
            "source": {"primitive": "cube", "size": [1, 1, 1]},
            "ops": [{"op": "bevel"}]
        }));
        assert!(unknown.is_err());
    }

    #[test]
    fn json_parses_mirror_array_weld_and_flip() {
        let recipe: GeoRecipe = rmcp::serde_json::from_value(json!({
            "source": {"primitive": "cube", "size": [2, 2, 2]},
            "ops": [
                {"op": "mirror", "axis": "x"},
                {"op": "array", "count": 3, "offset": [3, 0, 0]},
                {"op": "weld", "epsilon": 0.25},
                {"op": "flip"}
            ]
        }))
        .expect("plain JSON parses");
        assert_eq!(
            recipe.ops,
            vec![
                GeoOp::Mirror {
                    axis: "x".to_owned()
                },
                GeoOp::Array {
                    count: 3,
                    offset: [3.0, 0.0, 0.0]
                },
                GeoOp::Weld { epsilon: 0.25 },
                GeoOp::Flip,
            ]
        );
        // Each one changes the faces, so a select in front of it is refused in any build.
        for op in recipe.ops {
            let error = evaluate(&cube([2.0; 3], vec![top(), op])).expect_err("refused");
            assert_eq!(error.code, "bad_recipe");
            assert!(
                error.message.contains("move every select"),
                "{}",
                error.message
            );
        }
    }

    fn hole() -> GeoOp {
        GeoOp::Boolean {
            cutter: GeoCutter {
                solid: GeoSolid::Cylinder {
                    radius: 0.5,
                    height: 3.0,
                    segments: 24,
                },
                offset: [0.0; 3],
            },
            kind: BoolKind::Difference,
        }
    }

    #[cfg(not(feature = "geo"))]
    #[test]
    fn modeling_ops_refuse_typed_without_the_geo_feature() {
        for op in [
            hole(),
            GeoOp::Unwrap,
            GeoOp::Lod { keep: 0.5 },
            GeoOp::Collision { max_hulls: 4 },
            mirror("x"),
            array(2),
            GeoOp::Weld { epsilon: 0.001 },
            GeoOp::Flip,
        ] {
            let error = evaluate(&tower(vec![op])).expect_err("refused");
            assert_eq!(error.code, "feature_off");
            assert!(error.message.contains("feature `geo`"), "{}", error.message);
        }
    }

    #[cfg(feature = "geo")]
    #[test]
    fn a_cube_with_a_hole_is_cut_unwrapped_reduced_and_given_collision() {
        let recipe = tower(vec![
            hole(),
            GeoOp::Unwrap,
            GeoOp::Lod { keep: 0.5 },
            GeoOp::Collision { max_hulls: 8 },
            top(),
        ]);
        let out = evaluate(&recipe).expect("evaluates");
        assert!(out.faces > 12, "the cut added faces: {}", out.faces);
        assert_eq!(
            out.uv.as_ref().expect("uv facts").overlapping_chart_pairs,
            0
        );
        assert!(out.uv.as_ref().is_some_and(|uv| uv.charts >= 1));
        let lod = out.lod.as_ref().expect("lod facts");
        assert!(lod.triangles_after <= lod.triangles_before);
        let hulls = out.collision.as_ref().expect("collision facts");
        assert!(hulls.parts >= 1 && hulls.parts <= 8);
        assert_eq!(
            hulls.part_vertices.len(),
            usize::try_from(hulls.parts).unwrap_or(0)
        );
        assert_eq!(out.selections.len(), 1);
        // Same recipe, same bytes; a different cutter moves the digest.
        assert_eq!(out, evaluate(&recipe).expect("again"));
        let mut other = recipe.clone();
        other.ops[0] = GeoOp::Boolean {
            cutter: GeoCutter {
                solid: GeoSolid::Cylinder {
                    radius: 0.6,
                    height: 3.0,
                    segments: 24,
                },
                offset: [0.0; 3],
            },
            kind: BoolKind::Difference,
        };
        assert_ne!(evaluate(&other).expect("other").digest, out.digest);
        // The cut shrinks the box: the difference keeps the outer bounds.
        assert!(
            out.bounds_max.iter().all(|v| (v - 1.0).abs() < 1e-4),
            "{:?}",
            out.bounds_max
        );
    }

    #[cfg(feature = "geo")]
    #[test]
    fn modeling_ops_refuse_typed_with_the_fix() {
        let refuse = |ops| evaluate(&tower(ops)).expect_err("refused");
        let after_select = refuse(vec![top(), hole()]);
        assert_eq!(after_select.code, "bad_recipe");
        assert!(
            after_select.message.contains("move every select"),
            "{}",
            after_select.message
        );
        let plane = evaluate(&GeoRecipe {
            seed: 0,
            source: GeoSource::Plane { size: [2.0, 2.0] },
            ops: vec![hole()],
        })
        .expect_err("a plane is not closed");
        assert!(plane.message.contains("closed"), "{}", plane.message);
        let keep = refuse(vec![GeoOp::Lod { keep: 0.0 }]);
        assert!(keep.message.contains("[0.05, 1]"), "{}", keep.message);
        let hulls = refuse(vec![GeoOp::Collision { max_hulls: 99 }]);
        assert!(hulls.message.contains("1..=32"), "{}", hulls.message);
        let segments = refuse(vec![GeoOp::Boolean {
            cutter: GeoCutter {
                solid: GeoSolid::Sphere {
                    radius: 1.0,
                    rings: 400,
                    segments: 8,
                },
                offset: [0.0; 3],
            },
            kind: BoolKind::Union,
        }]);
        assert!(segments.message.contains("2..=64"), "{}", segments.message);
        let offset = refuse(vec![GeoOp::Boolean {
            cutter: GeoCutter {
                solid: GeoSolid::Cube { size: [1.0; 3] },
                offset: [f32::NAN, 0.0, 0.0],
            },
            kind: BoolKind::Union,
        }]);
        assert!(offset.message.contains("offset"), "{}", offset.message);
        // A cutter that swallows the whole cube leaves nothing.
        let nothing = refuse(vec![GeoOp::Boolean {
            cutter: GeoCutter {
                solid: GeoSolid::Cube { size: [10.0; 3] },
                offset: [0.0; 3],
            },
            kind: BoolKind::Difference,
        }]);
        assert!(nothing.message.contains("no faces"), "{}", nothing.message);
    }

    fn array(count: u32) -> GeoOp {
        GeoOp::Array {
            count,
            offset: [3.0, 0.0, 0.0],
        }
    }

    fn mirror(axis: &str) -> GeoOp {
        GeoOp::Mirror {
            axis: axis.to_owned(),
        }
    }

    /// Two cubes 3 m apart, mirrored through x = 0: four cubes, no shared corners.
    #[cfg(feature = "geo")]
    #[test]
    fn array_then_mirror_numbers() {
        let out = evaluate(&tower(vec![array(2), mirror("x")])).expect("evaluates");
        assert_eq!((out.faces, out.vertices), (48, 32));
        assert_eq!(out.bounds_min, [-4.0, -1.0, -1.0]);
        assert_eq!(out.bounds_max, [4.0, 1.0, 1.0]);
        assert_eq!(
            out,
            evaluate(&tower(vec![array(2), mirror("x")])).expect("again")
        );
    }

    /// Three cubes with a 1 m gap: a 1 mm weld has nothing to merge.
    #[cfg(feature = "geo")]
    #[test]
    fn weld_after_array_keeps_distinct_corners() {
        let welded =
            evaluate(&tower(vec![array(3), GeoOp::Weld { epsilon: 0.001 }])).expect("evaluates");
        assert_eq!((welded.vertices, welded.faces), (24, 36));
        // The widest legal weld still keeps them: the corners sit 2 m or more apart.
        let wide = evaluate(&tower(vec![array(3), GeoOp::Weld { epsilon: 0.1 }])).expect("wide");
        assert_eq!(wide.vertices, 24);
    }

    #[cfg(feature = "geo")]
    #[test]
    fn flip_moves_the_digest_and_twice_restores_it() {
        let plain = evaluate(&tower(vec![])).expect("plain");
        let once = evaluate(&tower(vec![GeoOp::Flip])).expect("once");
        let twice = evaluate(&tower(vec![GeoOp::Flip, GeoOp::Flip])).expect("twice");
        assert_ne!(once.digest, plain.digest);
        assert_eq!(twice.digest, plain.digest);
        assert_eq!((once.vertices, once.faces), (plain.vertices, plain.faces));
    }

    #[cfg(feature = "geo")]
    #[test]
    fn bad_numbers_name_the_legal_values() {
        let refuse_all = |ops| evaluate(&tower(ops)).expect_err("refused");
        let refuse = |op| refuse_all(vec![op]);
        for count in [0, 65] {
            let error = refuse(array(count));
            assert_eq!(error.code, "bad_recipe");
            assert!(error.message.contains("1..=64"), "{}", error.message);
        }
        for epsilon in [0.0, 0.5, f32::NAN] {
            let error = refuse(GeoOp::Weld { epsilon });
            assert_eq!(error.code, "bad_recipe");
            assert!(error.message.contains("(0, 0.1]"), "{}", error.message);
        }
        let axis = refuse(mirror("w"));
        assert_eq!(axis.code, "bad_recipe");
        assert!(axis.message.contains("x, y, z"), "{}", axis.message);
        let offset = refuse(GeoOp::Array {
            count: 2,
            offset: [f32::INFINITY, 0.0, 0.0],
        });
        assert!(offset.message.contains("offset"), "{}", offset.message);
        // 64 x 64 cubes is 49152 triangles and fits; one more doubling passes the cap.
        let fits = evaluate(&tower(vec![array(64), array(64)])).expect("49152 triangles fit");
        assert_eq!(fits.faces, 64 * 64 * 12);
        let big = refuse_all(vec![array(64), array(64), array(2)]);
        assert_eq!(big.code, "too_big");
        assert!(big.message.contains("50000"), "{}", big.message);
    }

    #[cfg(feature = "geo")]
    #[test]
    fn apply_commits_a_modeled_mesh_and_undo_removes_it() {
        let (server, id) = server_with_node();
        let recipe = tower(vec![hole(), GeoOp::Unwrap]);
        let previewed = GeoPreviewTool::invoke(
            &server,
            GeoPreviewArgs {
                recipe: recipe.clone(),
            },
        )
        .expect("previews");
        let applied = GeoApplyTool::invoke(
            &server,
            GeoApplyArgs {
                recipe,
                node: id,
                expected_revision: 1,
                idempotency_key: 2,
            },
        )
        .expect("applies");
        assert_eq!(applied.digest, previewed.digest);
        TxUndoTool::invoke(&server, TxUndoArgs { undo_token: None }).expect("undo");
        assert!(server.lock().expect("lock").doc.components.is_empty());
    }

    /// The file verbs: a real GLB through `geo.uv|geo.lod|geo.hull|geo.bake`.
    #[cfg(all(feature = "geo", feature = "intake"))]
    mod file_verbs {
        use super::super::meshfile;
        use super::*;

        /// A committed GLB fixture with UVs and three primitives.
        fn crate_glb() -> String {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../content/generated/props/world_crate.glb")
                .display()
                .to_string()
        }

        /// A payload folder under the workspace `target/`.
        fn out(name: &str) -> String {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/geo-test")
                .join(name)
                .display()
                .to_string()
        }

        fn server() -> McpServer {
            McpServer::new(7, Capability::unrestricted(7))
        }

        #[test]
        fn uv_preview_is_deterministic_and_apply_commits_a_mesh_that_undoes() {
            let (server, id) = server_with_node();
            let args = meshfile::UvPreviewArgs {
                path: crate_glb(),
                atlas: Some(512),
            };
            let preview = meshfile::UvPreviewTool::invoke(&server, args.clone()).expect("previews");
            assert!(preview.charts >= 1, "charts {}", preview.charts);
            assert_eq!(
                preview.overlapping_chart_pairs, 0,
                "the packer leaves no chart sharing atlas area"
            );
            assert_eq!(preview.digest.len(), 16);
            assert!(preview.texels_per_unit > 0.0);
            // Same file, same numbers twice; a preview writes nothing.
            assert_eq!(
                preview,
                meshfile::UvPreviewTool::invoke(&server, args).expect("again")
            );
            assert_eq!(server.lock().expect("lock").doc.revision, 1);
            let applied = meshfile::UvApplyTool::invoke(
                &server,
                meshfile::UvApplyArgs {
                    path: crate_glb(),
                    atlas: Some(512),
                    node: id,
                    expected_revision: 1,
                    idempotency_key: 9,
                    out_dir: Some(out("uv")),
                },
            )
            .expect("applies");
            assert_eq!(applied.preview.digest, preview.digest);
            assert!(applied.payload_bytes > 0);
            assert!(std::path::Path::new(&applied.payload_path).exists());
            assert_eq!(applied.revision, 2);
            TxUndoTool::invoke(&server, TxUndoArgs { undo_token: None }).expect("undo");
            assert!(server.lock().expect("lock").doc.components.is_empty());
        }

        #[test]
        fn lod_rows_name_every_level_over_its_budget() {
            let server = server();
            let preview = meshfile::LodPreviewTool::invoke(
                &server,
                meshfile::LodPreviewArgs {
                    path: crate_glb(),
                    ratios: vec![1.0, 0.5],
                    // A one-triangle budget every level misses: the warning row is the point.
                    tri_budget: Some(1),
                },
            )
            .expect("previews");
            assert_eq!(preview.levels.len(), 2);
            assert!(preview.levels[0].triangles > 0);
            assert!(
                preview.levels[0].over_budget,
                "LOD0 misses a 1-triangle budget"
            );
            assert!(preview.levels[1].over_budget);
            assert_eq!(preview.over_budget_levels, vec![0, 1]);
            // Without a budget the levels still report, judged against LOD0 times the ratio.
            let loose = meshfile::LodPreviewTool::invoke(
                &server,
                meshfile::LodPreviewArgs {
                    path: crate_glb(),
                    ratios: vec![1.0, 0.5, 0.25],
                    tri_budget: None,
                },
            )
            .expect("previews");
            assert_eq!(loose.levels.len(), 3);
            assert_eq!(loose.levels[0].budget, loose.base_triangles);
            let applied = meshfile::LodApplyTool::invoke(
                &server,
                meshfile::LodApplyArgs {
                    path: crate_glb(),
                    ratios: vec![1.0, 0.5],
                    tri_budget: None,
                    out_dir: Some(out("lod")),
                },
            )
            .expect("applies");
            assert_eq!(applied.files.len(), 2);
            assert_eq!(applied.digests.len(), 2);
            for file in &applied.files {
                assert!(std::path::Path::new(file).exists(), "{file} written");
            }
            assert!(applied.bytes > 0);
        }

        #[test]
        fn hull_preview_counts_parts_and_refuses_an_unknown_mode() {
            let server = server();
            let hull = meshfile::HullPreviewTool::invoke(
                &server,
                meshfile::HullPreviewArgs {
                    path: crate_glb(),
                    mode: Some("hull".to_owned()),
                    max_hulls: None,
                },
            )
            .expect("previews");
            assert_eq!(hull.mode, "hull");
            assert!(hull.hulls >= 1);
            for row in &hull.meshes {
                assert_eq!(row.parts, 1, "one hull per primitive");
                assert!(row.contained_vertices <= row.vertices);
                assert_eq!(row.containment_ok, row.contained_vertices == row.vertices);
            }
            let convex = meshfile::HullPreviewTool::invoke(
                &server,
                meshfile::HullPreviewArgs {
                    path: crate_glb(),
                    mode: Some("convex".to_owned()),
                    max_hulls: Some(4),
                },
            )
            .expect("previews");
            assert!(convex.hulls >= 1);
            assert!(convex
                .meshes
                .iter()
                .all(|row| row.parts >= 1 && row.parts <= 4));
            let refused = meshfile::HullPreviewTool::invoke(
                &server,
                meshfile::HullPreviewArgs {
                    path: crate_glb(),
                    mode: Some("box".to_owned()),
                    max_hulls: None,
                },
            )
            .expect_err("refused");
            assert_eq!(refused.code, "bad_input");
            assert!(
                refused.message.contains("`hull` or `convex`"),
                "{}",
                refused.message
            );
            let applied = meshfile::HullApplyTool::invoke(
                &server,
                meshfile::HullApplyArgs {
                    path: crate_glb(),
                    mode: Some("hull".to_owned()),
                    max_hulls: None,
                    out_dir: Some(out("hull")),
                },
            )
            .expect("applies");
            assert!(std::path::Path::new(&applied.payload_path).exists());
            assert_eq!(applied.preview.hulls, hull.hulls);
        }

        #[test]
        fn bake_preview_reports_samples_and_apply_writes_a_png_that_repeats() {
            let server = server();
            let preview = meshfile::BakePreviewTool::invoke(
                &server,
                meshfile::BakePreviewArgs {
                    path: crate_glb(),
                    resolution: Some(256),
                    kind: None,
                },
            )
            .expect("previews");
            assert_eq!(preview.resolution, 256);
            assert_eq!(preview.kind, "normal");
            assert!(preview.samples > 0);
            assert!(!preview.over_cap);
            assert!(preview.chart_coverage > 0.0);
            let apply = |folder: &str| {
                meshfile::BakeApplyTool::invoke(
                    &server,
                    meshfile::BakeApplyArgs {
                        path: crate_glb(),
                        resolution: Some(256),
                        kind: None,
                        out_dir: Some(out(folder)),
                    },
                )
                .expect("applies")
            };
            let first = apply("bake-a");
            let second = apply("bake-b");
            assert!(!first.maps.is_empty());
            for map in &first.maps {
                let png = std::fs::read(&map.path).expect("png written");
                assert_eq!(&png[1..4], b"PNG", "a PNG signature");
                assert!(map.covered_texels > 0, "the bake covered texels");
                assert!(map.covered_fraction > 0.0 && map.covered_fraction <= 1.0);
            }
            assert_eq!(
                first
                    .maps
                    .iter()
                    .map(|m| m.digest.clone())
                    .collect::<Vec<_>>(),
                second
                    .maps
                    .iter()
                    .map(|m| m.digest.clone())
                    .collect::<Vec<_>>(),
                "same mesh, same PNG bytes"
            );
            let refused = meshfile::BakeApplyTool::invoke(
                &server,
                meshfile::BakeApplyArgs {
                    path: crate_glb(),
                    resolution: Some(100),
                    kind: None,
                    out_dir: None,
                },
            )
            .expect_err("refused");
            assert_eq!(refused.code, "bad_input");
            assert!(
                refused.message.contains("256, 512 or 1024"),
                "{}",
                refused.message
            );
        }
    }
}
