//! `geo.uv`, `geo.lod`, `geo.hull`, `geo.bake`: the cook-time mesh tools over a glTF file.
//!
//! The recipe verbs ([`super::TOOL_GEO_PREVIEW`], [`super::TOOL_GEO_APPLY`]) model a
//! primitive from a typed op list; these verbs take a mesh that already exists on disk
//! (a `.glb` or `.gltf`) and run one of the four `engine-geo` modules over every triangle
//! primitive in document order. Each verb has a read-only preview that reports the numbers
//! an artist budgets against, and an apply that writes the payload:
//!
//! - `geo.uv` previews charts, atlas size, texel density and `overlapping_chart_pairs`; apply
//!   writes the unwrapped mesh payload and commits the `mesh` component (`asset
//!   geo/<digest>`) through the same previewed, capability-gated batch as `geo.apply`, so
//!   undo removes it.
//! - `geo.lod` previews one row per ratio (triangles against a triangle budget, seam-locked
//!   vertices) and names every level over its budget; apply writes one payload per level.
//! - `geo.hull` previews the hull count and a convex containment check; apply writes the hull
//!   payload.
//! - `geo.bake` previews chart coverage and the sample count (texels times triangles, held
//!   under [`MAX_BAKE_SAMPLES`]); apply bakes the object-space normal map and writes one PNG
//!   per primitive.
//!
//! Units: meters for geometry, texels for maps. Determinism: the donor modules are pure
//! functions of the mesh and options, primitives are walked in document order, and payloads
//! are little-endian with a fixed header, so the same file gives the same bytes and digest
//! twice. Offline cook only: nothing here runs in the fixed tick. Bounds: at most
//! [`MAX_FILE_TRIANGLES`] triangles per file, [`MAX_LOD_LEVELS`] LOD ratios, atlas edges
//! [`MIN_ATLAS`]..=[`MAX_ATLAS`], and [`MAX_BAKE_SAMPLES`] texel-triangle samples per map.
//! Errors: [`McpToolError`] codes `bad_input` (path, mode, kind, ratios, atlas),
//! `too_big` (a bound above), `cook_failed` (a donor refusal) and `io_failed`.
//!
//! Feature gates: every verb here reads a glTF, so the module is built only when
//! `engine-endpoint` carries both the `geo` and the `intake` features (off by default); a
//! build without them has none of these verbs.

use std::path::Path;

use engine_assets::{encode_png_rgb8, import_gltf_bytes, CookedMesh};
use engine_core::checked::{f32_from_f64, saturating_f64_to_u64};
use engine_geo::bake::{bake_normals, NormalMap};
use engine_geo::collision as geo_collision;
use engine_geo::lod as geo_lod;
use engine_geo::uv as geo_uv;
use engine_geo::{GeoError, GeoMesh};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use crate::mcp::{McpServer, McpToolError, OpDto, TxApplyArgs, TxPreviewArgs};

/// Verb: report unwrap facts for a glTF file, no writes.
pub const TOOL_GEO_UV_PREVIEW: &str = "geo.uv.preview";
/// Verb: unwrap a glTF file and commit its mesh.
pub const TOOL_GEO_UV_APPLY: &str = "geo.uv.apply";
/// Verb: report the LOD chain of a glTF file, no writes.
pub const TOOL_GEO_LOD_PREVIEW: &str = "geo.lod.preview";
/// Verb: write the LOD chain of a glTF file.
pub const TOOL_GEO_LOD_APPLY: &str = "geo.lod.apply";
/// Verb: report the collision hulls of a glTF file, no writes.
pub const TOOL_GEO_HULL_PREVIEW: &str = "geo.hull.preview";
/// Verb: write the collision hulls of a glTF file.
pub const TOOL_GEO_HULL_APPLY: &str = "geo.hull.apply";
/// Verb: report bake coverage and cost for a glTF file, no writes.
pub const TOOL_GEO_BAKE_PREVIEW: &str = "geo.bake.preview";
/// Verb: bake the normal map of a glTF file.
pub const TOOL_GEO_BAKE_APPLY: &str = "geo.bake.apply";

/// Most triangles one file may hold across all primitives.
pub const MAX_FILE_TRIANGLES: u32 = 20_000;
/// Most LOD ratios one call may ask for.
pub const MAX_LOD_LEVELS: usize = 6;
/// Smallest LOD ratio accepted.
pub const MIN_LOD_RATIO: f32 = 0.05;
/// Default LOD ratios: full, half, quarter.
pub const DEFAULT_LOD_RATIOS: [f32; 3] = [1.0, 0.5, 0.25];
/// Smallest atlas edge accepted, texels.
pub const MIN_ATLAS: u32 = 256;
/// Largest atlas edge accepted, texels.
pub const MAX_ATLAS: u32 = 1024;
/// Most convex parts one `geo.hull` call may ask for.
pub const MAX_HULLS: u32 = 32;
/// Most texel-triangle samples one `geo.bake` apply may spend on one map.
pub const MAX_BAKE_SAMPLES: u64 = 150_000_000;
/// Default payload folder, relative to the door's working directory.
pub const DEFAULT_OUT_DIR: &str = "target/geo";

fn bad(message: impl Into<String>) -> McpToolError {
    McpToolError::new("bad_input", message)
}

fn io_failed(what: &str, path: &Path, error: &std::io::Error) -> McpToolError {
    McpToolError::new(
        "io_failed",
        format!(
            "cannot {what} `{}`: {error}. Next: check the path and that the folder is writable.",
            path.display()
        ),
    )
}

/// A donor refusal, named with the step that raised it and one recovery.
fn cook_failed(step: &str, error: &GeoError) -> McpToolError {
    let fix = match error {
        GeoError::Empty => "; the mesh has no triangles, export it with geometry",
        GeoError::NotClosed => "; this step needs a closed mesh, weld the seams in the DCC first",
        GeoError::Malformed("uvs missing") => "; run `geo.uv.apply` first to give the mesh UVs",
        _ => "; check the mesh opens in the engine and has finite values",
    };
    McpToolError::new("cook_failed", format!("{step} refused: {error}{fix}"))
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn wide(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// The mesh behind one glTF primitive.
fn geo_of(mesh: &CookedMesh) -> GeoMesh {
    GeoMesh {
        positions: mesh.positions.clone(),
        normals: mesh.normals.clone(),
        uvs: mesh.uv0.clone(),
        indices: mesh.indices.clone(),
    }
}

/// One glTF file loaded into the geo mesh type.
struct Loaded {
    /// One mesh per triangle primitive, document order.
    meshes: Vec<GeoMesh>,
    /// Triangles across every primitive.
    triangles: u32,
}

impl Loaded {
    /// Triangles of one mesh.
    fn tris_of(&self, index: usize) -> u32 {
        count(self.meshes[index].indices.len() / 3)
    }
}

fn check_tri_budget(triangles: usize) -> Result<(), McpToolError> {
    if triangles > usize::try_from(MAX_FILE_TRIANGLES).unwrap_or(usize::MAX) {
        return Err(McpToolError::new(
            "too_big",
            format!(
                "the file holds {triangles} triangles; at most {MAX_FILE_TRIANGLES} per file; \
                 split the asset or decimate it first"
            ),
        ));
    }
    Ok(())
}

fn load(path: &str) -> Result<Loaded, McpToolError> {
    let file = Path::new(path);
    let extension = file
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if extension != "glb" && extension != "gltf" {
        return Err(bad(format!(
            "path `{path}` is not a .glb or .gltf file; pass a mesh file"
        )));
    }
    let bytes = std::fs::read(file).map_err(|e| io_failed("read", file, &e))?;
    let imported = import_gltf_bytes(&bytes).map_err(|e| {
        McpToolError::new(
            "bad_input",
            format!("`{path}` did not import: {e}. Next: check the file opens in a glTF viewer."),
        )
    })?;
    let triangles: usize = imported.meshes.iter().map(|m| m.indices.len() / 3).sum();
    if triangles == 0 {
        return Err(bad(format!(
            "`{path}` holds no triangles; export a mesh with geometry"
        )));
    }
    check_tri_budget(triangles)?;
    Ok(Loaded {
        meshes: imported.meshes.iter().map(geo_of).collect(),
        triangles: count(triangles),
    })
}

/// Little-endian payload of one mesh list: magic, count, then each mesh's lane lengths and
/// lanes (positions, normals, UVs, indices).
fn mesh_payload(meshes: &[GeoMesh]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"E2GEOM1\0");
    out.extend_from_slice(&count(meshes.len()).to_le_bytes());
    for mesh in meshes {
        for length in [
            mesh.positions.len(),
            mesh.normals.len(),
            mesh.uvs.len(),
            mesh.indices.len(),
        ] {
            out.extend_from_slice(&count(length).to_le_bytes());
        }
        for position in &mesh.positions {
            for lane in position {
                out.extend_from_slice(&lane.to_le_bytes());
            }
        }
        for normal in &mesh.normals {
            for lane in normal {
                out.extend_from_slice(&lane.to_le_bytes());
            }
        }
        for uv in &mesh.uvs {
            for lane in uv {
                out.extend_from_slice(&lane.to_le_bytes());
            }
        }
        for index in &mesh.indices {
            out.extend_from_slice(&index.to_le_bytes());
        }
    }
    out
}

/// Lowercase hex digest of a payload.
fn digest_of_bytes(bytes: &[u8]) -> String {
    format!("{:016x}", engine_core::hash_bytes(bytes))
}

fn out_dir(given: Option<&str>) -> std::path::PathBuf {
    std::path::PathBuf::from(given.unwrap_or(DEFAULT_OUT_DIR))
}

fn write_payload(
    folder: &Path,
    name: &str,
    bytes: &[u8],
) -> Result<std::path::PathBuf, McpToolError> {
    std::fs::create_dir_all(folder).map_err(|e| io_failed("create", folder, &e))?;
    let path = folder.join(name);
    std::fs::write(&path, bytes).map_err(|e| io_failed("write", &path, &e))?;
    Ok(path)
}

fn atlas_of(given: Option<u32>) -> Result<u32, McpToolError> {
    let atlas = given.unwrap_or(MAX_ATLAS);
    if !(MIN_ATLAS..=MAX_ATLAS).contains(&atlas) {
        return Err(bad(format!(
            "atlas {atlas} is outside {MIN_ATLAS}..={MAX_ATLAS}; send 256, 512 or 1024"
        )));
    }
    Ok(atlas)
}

fn ratios_of(given: &[f32]) -> Result<Vec<f32>, McpToolError> {
    let ratios = if given.is_empty() {
        DEFAULT_LOD_RATIOS.to_vec()
    } else {
        given.to_vec()
    };
    if ratios.len() > MAX_LOD_LEVELS {
        return Err(bad(format!(
            "{} ratios asked; at most {MAX_LOD_LEVELS} levels",
            ratios.len()
        )));
    }
    for ratio in &ratios {
        if !ratio.is_finite() || !(MIN_LOD_RATIO..=1.0).contains(ratio) {
            return Err(bad(format!(
                "ratio {ratio} is outside {MIN_LOD_RATIO}..=1; send the fraction of triangles to keep"
            )));
        }
    }
    Ok(ratios)
}

// ---------------------------------------------------------------- geo.uv

/// Args for `geo.uv.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct UvPreviewArgs {
    /// Path of the `.glb` or `.gltf` file.
    pub path: String,
    /// Target atlas edge in texels: 256, 512 or 1024 (default 1024).
    #[serde(default)]
    pub atlas: Option<u32>,
}

/// Unwrap facts of one primitive.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct UvMeshOut {
    /// Primitive index, document order.
    pub mesh: u32,
    /// Triangles in this primitive.
    pub triangles: u32,
    /// Charts the unwrap cut.
    pub charts: u32,
    /// Atlas size in texels `[width, height]`.
    pub atlas: [u32; 2],
    /// Chart pairs sharing atlas area (0 is the goal).
    pub overlapping_chart_pairs: u32,
    /// Texels per meter the packer chose.
    pub texels_per_unit: f32,
    /// UV triangle area over atlas area (1.0 = no wasted texel).
    pub fill: f32,
    /// Spread of per-triangle texel density (`(max - min) / mean`, 0 is even).
    pub density_spread: f32,
}

/// Result of `geo.uv.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct UvPreviewOutput {
    /// File that was read.
    pub path: String,
    /// Per-primitive facts, document order.
    pub meshes: Vec<UvMeshOut>,
    /// Charts across every primitive.
    pub charts: u32,
    /// Overlapping chart pairs across every primitive (0 is the goal).
    pub overlapping_chart_pairs: u32,
    /// Largest atlas per axis over the primitives, texels.
    pub atlas: [u32; 2],
    /// Mean texels per meter over the primitives.
    pub texels_per_unit: f32,
    /// Lowercase hex digest of the unwrapped payload.
    pub digest: String,
}

/// Args for `geo.uv.apply`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct UvApplyArgs {
    /// Path of the `.glb` or `.gltf` file.
    pub path: String,
    /// Target atlas edge in texels: 256, 512 or 1024 (default 1024).
    #[serde(default)]
    pub atlas: Option<u32>,
    /// Node (UUID text) that receives the `mesh` component.
    pub node: String,
    /// Must equal the document revision, else nothing runs.
    pub expected_revision: u64,
    /// Retry identity of the committed batch.
    pub idempotency_key: u64,
    /// Payload folder (default `target/geo`).
    #[serde(default)]
    pub out_dir: Option<String>,
}

/// Result of `geo.uv.apply`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct UvApplyOutput {
    /// The preview numbers this apply committed.
    pub preview: UvPreviewOutput,
    /// Mesh asset name written to the node (`geo/<digest>`).
    pub asset: String,
    /// Where the unwrapped payload was written.
    pub payload_path: String,
    /// Payload size in bytes.
    pub payload_bytes: u64,
    /// Document revision after the commit.
    pub revision: u64,
    /// The batch effect lines.
    pub effects: Vec<String>,
}

/// Unwrap every primitive and collect its facts.
fn uv_all(loaded: &Loaded, atlas: u32) -> Result<(Vec<GeoMesh>, Vec<UvMeshOut>), McpToolError> {
    let options = geo_uv::UnwrapOptions {
        resolution: atlas,
        ..geo_uv::UnwrapOptions::default()
    };
    let mut meshes = Vec::with_capacity(loaded.meshes.len());
    let mut rows = Vec::with_capacity(loaded.meshes.len());
    for (index, source) in loaded.meshes.iter().enumerate() {
        let unwrapped = geo_uv::unwrap(source, &options)
            .map_err(|e| cook_failed(&format!("mesh {index} unwrap"), &e))?;
        let overlaps = geo_uv::overlapping_chart_pairs(
            &unwrapped.mesh.uvs,
            &unwrapped.mesh.indices,
            &unwrapped.chart_of_triangle,
        );
        let quality = geo_uv::quality(&unwrapped, source);
        rows.push(UvMeshOut {
            mesh: count(index),
            triangles: count(unwrapped.mesh.indices.len() / 3),
            charts: unwrapped.chart_count,
            atlas: unwrapped.atlas_size,
            overlapping_chart_pairs: count(overlaps),
            texels_per_unit: unwrapped.texels_per_unit,
            fill: quality.fill,
            density_spread: quality.density_spread,
        });
        meshes.push(unwrapped.mesh);
    }
    Ok((meshes, rows))
}

fn uv_preview(_server: &McpServer, args: &UvPreviewArgs) -> Result<UvPreviewOutput, McpToolError> {
    let atlas = atlas_of(args.atlas)?;
    let loaded = load(&args.path)?;
    let (meshes, rows) = uv_all(&loaded, atlas)?;
    let digest = digest_of_bytes(&mesh_payload(&meshes));
    let charts = rows.iter().map(|r| r.charts).sum();
    let overlaps = rows.iter().map(|r| r.overlapping_chart_pairs).sum();
    let width = rows.iter().map(|r| r.atlas[0]).max().unwrap_or(0);
    let height = rows.iter().map(|r| r.atlas[1]).max().unwrap_or(0);
    let density = if rows.is_empty() {
        0.0
    } else {
        let sum: f32 = rows.iter().map(|r| r.texels_per_unit).sum();
        sum / f32_from_f64(f64::from(count(rows.len())))
    };
    Ok(UvPreviewOutput {
        path: args.path.clone(),
        meshes: rows,
        charts,
        overlapping_chart_pairs: overlaps,
        atlas: [width, height],
        texels_per_unit: density,
        digest,
    })
}

fn uv_apply(server: &McpServer, args: &UvApplyArgs) -> Result<UvApplyOutput, McpToolError> {
    let preview = uv_preview(
        server,
        &UvPreviewArgs {
            path: args.path.clone(),
            atlas: args.atlas,
        },
    )?;
    // Re-unwrap for the payload: the preview numbers and the written bytes come from one path.
    let atlas = atlas_of(args.atlas)?;
    let loaded = load(&args.path)?;
    let (meshes, _) = uv_all(&loaded, atlas)?;
    let payload = mesh_payload(&meshes);
    if digest_of_bytes(&payload) != preview.digest {
        return Err(McpToolError::new(
            "cook_failed",
            "the unwrap gave two digests in one call; report the verb".to_string(),
        ));
    }
    let asset = format!("geo/{}", preview.digest);
    let ops = vec![OpDto::SetComponent {
        id: args.node.clone(),
        component: "mesh".to_owned(),
        value: rmcp::serde_json::json!({ "asset": asset }),
        version: None,
    }];
    let applied = server.dispatch(TOOL_GEO_UV_APPLY, |state| {
        let drafted = state.tx_preview(&TxPreviewArgs {
            expected_revision: args.expected_revision,
            idempotency_key: args.idempotency_key,
            ops: ops.clone(),
        })?;
        state.tx_apply(&TxApplyArgs {
            expected_revision: args.expected_revision,
            idempotency_key: args.idempotency_key,
            ops,
            approval: drafted.approval,
            delta: Some(false),
            origin: None,
        })
    })?;
    let folder = out_dir(args.out_dir.as_deref());
    let name = format!("uv-{}.geom", preview.digest);
    let path = write_payload(&folder, &name, &payload)?;
    Ok(UvApplyOutput {
        asset,
        payload_path: path.display().to_string(),
        payload_bytes: wide(payload.len()),
        revision: applied.revision,
        effects: applied.effects,
        preview,
    })
}

crate::e2_tool!(
    /// `geo.uv.preview`: unwrap a glTF file and report its chart facts.
    UvPreviewTool,
    name = TOOL_GEO_UV_PREVIEW,
    description = "Unwrap every primitive of a .glb/.gltf (xatlas through engine-geo) and return charts, atlas size, texels per meter, UV fill and overlapping_chart_pairs per mesh. No writes.",
    readonly = true,
    args = UvPreviewArgs,
    output = UvPreviewOutput,
    invoke = uv_preview
);

crate::e2_tool!(
    /// `geo.uv.apply`: unwrap a glTF file, write the payload and commit its mesh.
    UvApplyTool,
    name = TOOL_GEO_UV_APPLY,
    description = "Unwrap every primitive of a .glb/.gltf, write the unwrapped payload under out_dir, and set the mesh component (asset geo/<digest>) on one node through the previewed, gated batch; undoable.",
    readonly = false,
    args = UvApplyArgs,
    output = UvApplyOutput,
    invoke = uv_apply
);

// ---------------------------------------------------------------- geo.lod

/// Args for `geo.lod.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LodPreviewArgs {
    /// Path of the `.glb` or `.gltf` file.
    pub path: String,
    /// Fractions of LOD0 triangles to keep, each in [0.05, 1] (default [1, 0.5, 0.25]).
    #[serde(default)]
    pub ratios: Vec<f32>,
    /// LOD0 triangle budget; each level's budget is this times its ratio (default: LOD0's own count).
    #[serde(default)]
    pub tri_budget: Option<u32>,
}

/// One level of the LOD chain.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LodLevelOut {
    /// Level index, 0 is LOD0.
    pub level: u32,
    /// Ratio this level asked for.
    pub ratio: f32,
    /// Triangles across every primitive at this level.
    pub triangles: u32,
    /// Triangles this level aimed for.
    pub target: u32,
    /// Triangles this level may hold (the budget miss threshold).
    pub budget: u32,
    /// True when `triangles > budget`: this level is over budget.
    pub over_budget: bool,
    /// Vertices held fixed on UV seams and borders.
    pub locked_vertices: u32,
}

/// Result of `geo.lod.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LodPreviewOutput {
    /// File that was read.
    pub path: String,
    /// LOD0 triangles across every primitive.
    pub base_triangles: u32,
    /// One row per ratio, in caller order.
    pub levels: Vec<LodLevelOut>,
    /// Levels whose triangles exceed their budget (empty is the goal).
    pub over_budget_levels: Vec<u32>,
    /// Lowercase hex digest of the LOD0 payload.
    pub digest: String,
}

/// Args for `geo.lod.apply`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LodApplyArgs {
    /// Path of the `.glb` or `.gltf` file.
    pub path: String,
    /// Fractions of LOD0 triangles to keep, each in [0.05, 1] (default [1, 0.5, 0.25]).
    #[serde(default)]
    pub ratios: Vec<f32>,
    /// LOD0 triangle budget; each level's budget is this times its ratio.
    #[serde(default)]
    pub tri_budget: Option<u32>,
    /// Payload folder (default `target/geo`).
    #[serde(default)]
    pub out_dir: Option<String>,
}

/// Result of `geo.lod.apply`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LodApplyOutput {
    /// The preview numbers this apply wrote.
    pub preview: LodPreviewOutput,
    /// One payload path per level, level order.
    pub files: Vec<String>,
    /// One digest per level, level order.
    pub digests: Vec<String>,
    /// Bytes written across every level.
    pub bytes: u64,
}

/// The LOD chain: one mesh list per level, plus one row per level.
fn lod_chain(
    loaded: &Loaded,
    ratios: &[f32],
) -> Result<(Vec<Vec<GeoMesh>>, Vec<LodLevelOut>), McpToolError> {
    let base = loaded.triangles;
    let budget = |ratio: f32| -> u32 {
        let scaled = f64::from(base) * f64::from(ratio);
        let floored = saturating_f64_to_u64(scaled.floor().max(1.0));
        u32::try_from(floored).unwrap_or(1)
    };
    let mut levels: Vec<Vec<GeoMesh>> = Vec::with_capacity(ratios.len());
    let mut rows = Vec::with_capacity(ratios.len());
    for (level, ratio) in ratios.iter().copied().enumerate() {
        let options = geo_lod::LodOptions {
            target_ratio: ratio,
            ..geo_lod::LodOptions::default()
        };
        let mut meshes = Vec::with_capacity(loaded.meshes.len());
        let mut triangles = 0u32;
        let mut locked = 0u32;
        for (index, source) in loaded.meshes.iter().enumerate() {
            let result = geo_lod::simplify(source, &[], 0, 0.0, &options)
                .map_err(|e| cook_failed(&format!("mesh {index} lod {level}"), &e))?;
            triangles = triangles.saturating_add(count(result.mesh.indices.len() / 3));
            locked = locked.saturating_add(count(result.locked_vertices));
            meshes.push(result.mesh);
        }
        let target = budget(ratio);
        rows.push(LodLevelOut {
            level: count(level),
            ratio,
            triangles,
            target,
            budget: target,
            over_budget: triangles > target,
            locked_vertices: locked,
        });
        levels.push(meshes);
    }
    Ok((levels, rows))
}

fn lod_preview(
    _server: &McpServer,
    args: &LodPreviewArgs,
) -> Result<LodPreviewOutput, McpToolError> {
    let ratios = ratios_of(&args.ratios)?;
    let loaded = load(&args.path)?;
    let (levels, mut rows) = lod_chain(&loaded, &ratios)?;
    if let Some(budget) = args.tri_budget {
        for row in &mut rows {
            let scaled = f64::from(budget) * f64::from(row.ratio);
            let floored = saturating_f64_to_u64(scaled.floor().max(1.0));
            row.budget = u32::try_from(floored).unwrap_or(1);
            row.over_budget = row.triangles > row.budget;
        }
    }
    let over: Vec<u32> = rows
        .iter()
        .filter(|row| row.over_budget)
        .map(|row| row.level)
        .collect();
    Ok(LodPreviewOutput {
        path: args.path.clone(),
        base_triangles: loaded.triangles,
        levels: rows,
        over_budget_levels: over,
        digest: digest_of_bytes(&mesh_payload(&levels[0])),
    })
}

fn lod_apply(_server: &McpServer, args: &LodApplyArgs) -> Result<LodApplyOutput, McpToolError> {
    let preview = lod_preview(
        _server,
        &LodPreviewArgs {
            path: args.path.clone(),
            ratios: args.ratios.clone(),
            tri_budget: args.tri_budget,
        },
    )?;
    let ratios = ratios_of(&args.ratios)?;
    let loaded = load(&args.path)?;
    let (levels, _) = lod_chain(&loaded, &ratios)?;
    let folder = out_dir(args.out_dir.as_deref());
    let mut files = Vec::with_capacity(levels.len());
    let mut digests = Vec::with_capacity(levels.len());
    let mut bytes = 0u64;
    for (level, meshes) in levels.iter().enumerate() {
        let payload = mesh_payload(meshes);
        let digest = digest_of_bytes(&payload);
        let name = format!("lod-{digest}-l{level}.geom");
        let path = write_payload(&folder, &name, &payload)?;
        files.push(path.display().to_string());
        digests.push(digest);
        bytes = bytes.saturating_add(wide(payload.len()));
    }
    Ok(LodApplyOutput {
        preview,
        files,
        digests,
        bytes,
    })
}

crate::e2_tool!(
    /// `geo.lod.preview`: report a seam-locked LOD chain against a triangle budget.
    LodPreviewTool,
    name = TOOL_GEO_LOD_PREVIEW,
    description = "Simplify every primitive of a .glb/.gltf to each ratio (seam-locked, meshoptimizer through engine-geo) and return one row per level: triangles, target, budget, over_budget and locked vertices. No writes.",
    readonly = true,
    args = LodPreviewArgs,
    output = LodPreviewOutput,
    invoke = lod_preview
);

crate::e2_tool!(
    /// `geo.lod.apply`: write the seam-locked LOD chain of a glTF file.
    LodApplyTool,
    name = TOOL_GEO_LOD_APPLY,
    description = "Simplify every primitive of a .glb/.gltf to each ratio and write one payload per level under out_dir; a level over its budget is a warning row in the preview, never a silent pass.",
    readonly = false,
    args = LodApplyArgs,
    output = LodApplyOutput,
    invoke = lod_apply
);

// ---------------------------------------------------------------- geo.hull

/// Args for `geo.hull.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct HullPreviewArgs {
    /// Path of the `.glb` or `.gltf` file.
    pub path: String,
    /// `hull` (one convex hull per primitive) or `convex` (a decomposition, default `hull`).
    #[serde(default)]
    pub mode: Option<String>,
    /// Most parts a `convex` decomposition may return, 1..=32 (default 8).
    #[serde(default)]
    pub max_hulls: Option<u32>,
}

/// Collision facts of one primitive.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct HullMeshOut {
    /// Primitive index, document order.
    pub mesh: u32,
    /// Triangles of the source mesh.
    pub triangles: u32,
    /// Hull parts built for this primitive.
    pub parts: u32,
    /// Vertices of each part.
    pub part_vertices: Vec<u32>,
    /// Source vertices inside at least one hull part.
    pub contained_vertices: u32,
    /// Source vertices checked.
    pub vertices: u32,
    /// True when every source vertex sits inside a hull part.
    pub containment_ok: bool,
    /// Lowercase hex digest of this primitive's hulls.
    pub digest: String,
}

/// Result of `geo.hull.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct HullPreviewOutput {
    /// File that was read.
    pub path: String,
    /// `hull` or `convex`.
    pub mode: String,
    /// Per-primitive facts, document order.
    pub meshes: Vec<HullMeshOut>,
    /// Hull parts across every primitive.
    pub hulls: u32,
    /// True when every primitive's every source vertex is inside a hull part.
    pub containment_ok: bool,
}

/// Args for `geo.hull.apply`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct HullApplyArgs {
    /// Path of the `.glb` or `.gltf` file.
    pub path: String,
    /// `hull` or `convex` (default `hull`).
    #[serde(default)]
    pub mode: Option<String>,
    /// Most parts a `convex` decomposition may return, 1..=32 (default 8).
    #[serde(default)]
    pub max_hulls: Option<u32>,
    /// Payload folder (default `target/geo`).
    #[serde(default)]
    pub out_dir: Option<String>,
}

/// Result of `geo.hull.apply`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct HullApplyOutput {
    /// The preview numbers this apply wrote.
    pub preview: HullPreviewOutput,
    /// Where the hull payload was written.
    pub payload_path: String,
    /// Payload size in bytes.
    pub payload_bytes: u64,
    /// Lowercase hex digest of the whole payload.
    pub digest: String,
}

/// `hull` or `convex`, with the legal values in the refusal.
fn hull_mode(given: Option<&str>) -> Result<&'static str, McpToolError> {
    match given.unwrap_or("hull") {
        "hull" => Ok("hull"),
        "convex" => Ok("convex"),
        other => Err(bad(format!(
            "mode `{other}` is not a collision mode; send `hull` or `convex`"
        ))),
    }
}

/// Point inside a convex closed mesh: inside every face plane, with the planes oriented
/// outward from the part's vertex centroid.
fn inside_convex(hull: &GeoMesh, point: [f32; 3], epsilon: f32) -> bool {
    if hull.indices.len() < 12 || hull.positions.is_empty() {
        return false;
    }
    let centroid = {
        let mut sum = [0.0f64; 3];
        for position in &hull.positions {
            for (axis, lane) in position.iter().enumerate() {
                sum[axis] += f64::from(*lane);
            }
        }
        let n = f64::from(count(hull.positions.len())).max(1.0);
        [
            f32_from_f64(sum[0] / n),
            f32_from_f64(sum[1] / n),
            f32_from_f64(sum[2] / n),
        ]
    };
    for triangle in hull.indices.chunks_exact(3) {
        let at = |index: u32| hull.positions[usize::try_from(index).unwrap_or(0)];
        let (a, b, c) = (at(triangle[0]), at(triangle[1]), at(triangle[2]));
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let normal = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        let to_centroid = [centroid[0] - a[0], centroid[1] - a[1], centroid[2] - a[2]];
        let facing =
            normal[0] * to_centroid[0] + normal[1] * to_centroid[1] + normal[2] * to_centroid[2];
        if facing == 0.0 {
            continue;
        }
        // Flip so the plane normal points away from the part centroid.
        let sign = if facing > 0.0 { -1.0 } else { 1.0 };
        let to_point = [point[0] - a[0], point[1] - a[1], point[2] - a[2]];
        let distance =
            sign * (normal[0] * to_point[0] + normal[1] * to_point[1] + normal[2] * to_point[2]);
        if distance > epsilon {
            return false;
        }
    }
    true
}

/// Longest box diagonal of a mesh.
fn diagonal(mesh: &GeoMesh) -> f32 {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for position in &mesh.positions {
        for axis in 0..3 {
            lo[axis] = lo[axis].min(position[axis]);
            hi[axis] = hi[axis].max(position[axis]);
        }
    }
    let d = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// Hulls of every primitive, plus one row per primitive.
fn hull_all(
    loaded: &Loaded,
    mode: &str,
    max_hulls: u32,
) -> Result<(Vec<Vec<GeoMesh>>, Vec<HullMeshOut>), McpToolError> {
    let mut all = Vec::with_capacity(loaded.meshes.len());
    let mut rows = Vec::with_capacity(loaded.meshes.len());
    for (index, source) in loaded.meshes.iter().enumerate() {
        let parts = if mode == "hull" {
            let hull = geo_collision::single_hull(source, 64)
                .map_err(|e| cook_failed(&format!("mesh {index} hull"), &e))?;
            vec![hull]
        } else {
            let options = geo_collision::HullOptions {
                max_hulls,
                ..geo_collision::HullOptions::default()
            };
            geo_collision::decompose(source, &options)
                .map_err(|e| cook_failed(&format!("mesh {index} convex"), &e))?
        };
        let epsilon = (diagonal(source) * 1e-3).max(1e-4);
        let mut contained = 0u32;
        for position in &source.positions {
            if parts
                .iter()
                .any(|part| inside_convex(part, *position, epsilon))
            {
                contained = contained.saturating_add(1);
            }
        }
        let vertices = count(source.positions.len());
        let payload = mesh_payload(&parts);
        rows.push(HullMeshOut {
            mesh: count(index),
            triangles: count(source.indices.len() / 3),
            parts: count(parts.len()),
            part_vertices: parts
                .iter()
                .map(|part| count(part.positions.len()))
                .collect(),
            contained_vertices: contained,
            vertices,
            containment_ok: contained == vertices,
            digest: digest_of_bytes(&payload),
        });
        all.push(parts);
    }
    Ok((all, rows))
}

fn hull_max(given: Option<u32>) -> Result<u32, McpToolError> {
    let max = given.unwrap_or(8);
    if !(1..=MAX_HULLS).contains(&max) {
        return Err(bad(format!(
            "max_hulls {max} is outside 1..={MAX_HULLS}; send a part count in range"
        )));
    }
    Ok(max)
}

fn hull_preview(
    _server: &McpServer,
    args: &HullPreviewArgs,
) -> Result<HullPreviewOutput, McpToolError> {
    let mode = hull_mode(args.mode.as_deref())?;
    let max_hulls = hull_max(args.max_hulls)?;
    let loaded = load(&args.path)?;
    let (_, rows) = hull_all(&loaded, mode, max_hulls)?;
    let hulls = rows.iter().map(|row| row.parts).sum();
    let containment = rows.iter().all(|row| row.containment_ok);
    Ok(HullPreviewOutput {
        path: args.path.clone(),
        mode: mode.to_owned(),
        meshes: rows,
        hulls,
        containment_ok: containment,
    })
}

fn hull_apply(_server: &McpServer, args: &HullApplyArgs) -> Result<HullApplyOutput, McpToolError> {
    let preview = hull_preview(
        _server,
        &HullPreviewArgs {
            path: args.path.clone(),
            mode: args.mode.clone(),
            max_hulls: args.max_hulls,
        },
    )?;
    let mode = hull_mode(args.mode.as_deref())?;
    let max_hulls = hull_max(args.max_hulls)?;
    let loaded = load(&args.path)?;
    let (parts, _) = hull_all(&loaded, mode, max_hulls)?;
    let flat: Vec<GeoMesh> = parts.into_iter().flatten().collect();
    let payload = mesh_payload(&flat);
    let digest = digest_of_bytes(&payload);
    let folder = out_dir(args.out_dir.as_deref());
    let name = format!("hull-{digest}.geom");
    let path = write_payload(&folder, &name, &payload)?;
    Ok(HullApplyOutput {
        preview,
        payload_path: path.display().to_string(),
        payload_bytes: wide(payload.len()),
        digest,
    })
}

crate::e2_tool!(
    /// `geo.hull.preview`: report the collision hulls of a glTF file.
    HullPreviewTool,
    name = TOOL_GEO_HULL_PREVIEW,
    description = "Build the collision hulls of every primitive of a .glb/.gltf (`hull` = one convex hull, `convex` = V-HACD decomposition through engine-geo) and return part counts plus a convex containment check per mesh. No writes.",
    readonly = true,
    args = HullPreviewArgs,
    output = HullPreviewOutput,
    invoke = hull_preview
);

crate::e2_tool!(
    /// `geo.hull.apply`: write the collision hulls of a glTF file.
    HullApplyTool,
    name = TOOL_GEO_HULL_APPLY,
    description = "Build the collision hulls of every primitive of a .glb/.gltf and write the hull payload under out_dir; the preview names the parts and whether every source vertex is inside a hull.",
    readonly = false,
    args = HullApplyArgs,
    output = HullApplyOutput,
    invoke = hull_apply
);

// ---------------------------------------------------------------- geo.bake

/// Args for `geo.bake.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BakePreviewArgs {
    /// Path of the `.glb` or `.gltf` file.
    pub path: String,
    /// Map edge in texels: 256, 512 or 1024 (default 256).
    #[serde(default)]
    pub resolution: Option<u32>,
    /// Map kind: `normal` only today.
    #[serde(default)]
    pub kind: Option<String>,
}

/// Bake cost and coverage of one primitive.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BakeMeshOut {
    /// Primitive index, document order.
    pub mesh: u32,
    /// Triangles in the primitive.
    pub triangles: u32,
    /// Triangles with non-zero UV area (0 means the primitive has no UVs).
    pub uv_triangles: u32,
    /// Sum of the UV triangle areas (chart coverage estimate, 1.0 = the whole atlas).
    pub chart_coverage: f32,
    /// Texel-triangle samples this bake spends.
    pub samples: u64,
    /// True when `samples` is over [`MAX_BAKE_SAMPLES`].
    pub over_cap: bool,
}

/// Result of `geo.bake.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BakePreviewOutput {
    /// File that was read.
    pub path: String,
    /// Map edge in texels.
    pub resolution: u32,
    /// Map kind.
    pub kind: String,
    /// Per-primitive cost, document order.
    pub meshes: Vec<BakeMeshOut>,
    /// Mean chart coverage over the primitives.
    pub chart_coverage: f32,
    /// Texel-triangle samples across every primitive.
    pub samples: u64,
    /// True when any primitive is over [`MAX_BAKE_SAMPLES`].
    pub over_cap: bool,
}

/// Args for `geo.bake.apply`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BakeApplyArgs {
    /// Path of the `.glb` or `.gltf` file.
    pub path: String,
    /// Map edge in texels: 256, 512 or 1024 (default 256).
    #[serde(default)]
    pub resolution: Option<u32>,
    /// Map kind: `normal` only today.
    #[serde(default)]
    pub kind: Option<String>,
    /// Payload folder (default `target/geo`).
    #[serde(default)]
    pub out_dir: Option<String>,
}

/// One written map.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BakeMapOut {
    /// Primitive this map bakes.
    pub mesh: u32,
    /// Where the PNG was written.
    pub path: String,
    /// PNG size in bytes.
    pub bytes: u64,
    /// Texels the baker covered (baked or dilated).
    pub covered_texels: u32,
    /// Covered texels over total texels.
    pub covered_fraction: f32,
    /// Lowercase hex digest of the PNG bytes.
    pub digest: String,
}

/// Result of `geo.bake.apply`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BakeApplyOutput {
    /// The preview numbers this apply wrote.
    pub preview: BakePreviewOutput,
    /// One map per primitive, document order.
    pub maps: Vec<BakeMapOut>,
    /// PNG bytes written across every map.
    pub bytes: u64,
}

fn bake_resolution(given: Option<u32>) -> Result<u32, McpToolError> {
    let resolution = given.unwrap_or(MIN_ATLAS);
    if !(MIN_ATLAS..=MAX_ATLAS).contains(&resolution) {
        return Err(bad(format!(
            "resolution {resolution} is outside {MIN_ATLAS}..={MAX_ATLAS}; send 256, 512 or 1024"
        )));
    }
    Ok(resolution)
}

fn bake_kind(given: Option<&str>) -> Result<&'static str, McpToolError> {
    match given.unwrap_or("normal") {
        "normal" => Ok("normal"),
        other => Err(bad(format!(
            "kind `{other}` is not a map kind; send `normal`"
        ))),
    }
}

/// Chart area of one mesh: triangles with UV area, and the area sum.
fn chart_area(mesh: &GeoMesh) -> (u32, f32) {
    let mut triangles = 0u32;
    let mut area = 0.0f64;
    for triangle in mesh.indices.chunks_exact(3) {
        let at = |index: u32| mesh.uvs[usize::try_from(index).unwrap_or(0)];
        let (a, b, c) = (at(triangle[0]), at(triangle[1]), at(triangle[2]));
        let u = [f64::from(b[0] - a[0]), f64::from(b[1] - a[1])];
        let v = [f64::from(c[0] - a[0]), f64::from(c[1] - a[1])];
        let twice = (u[0] * v[1] - u[1] * v[0]).abs();
        if twice > 1e-12 {
            triangles = triangles.saturating_add(1);
            area += twice * 0.5;
        }
    }
    (triangles, f32_from_f64(area))
}

fn bake_preview(
    _server: &McpServer,
    args: &BakePreviewArgs,
) -> Result<BakePreviewOutput, McpToolError> {
    let resolution = bake_resolution(args.resolution)?;
    let kind = bake_kind(args.kind.as_deref())?;
    let loaded = load(&args.path)?;
    let texels = u64::from(resolution) * u64::from(resolution);
    let mut rows = Vec::with_capacity(loaded.meshes.len());
    let mut total = 0u64;
    let mut area_sum = 0.0f32;
    for (index, mesh) in loaded.meshes.iter().enumerate() {
        let triangles = loaded.tris_of(index);
        if mesh.uvs.len() != mesh.positions.len() {
            return Err(McpToolError::new(
                "cook_failed",
                format!(
                    "mesh {index} has no UVs ({} UV lanes for {} vertices); run `geo.uv.apply` \
                     first and bake the payload",
                    mesh.uvs.len(),
                    mesh.positions.len()
                ),
            ));
        }
        let (uv_triangles, coverage) = chart_area(mesh);
        if uv_triangles == 0 {
            return Err(McpToolError::new(
                "cook_failed",
                format!(
                    "mesh {index} has no UV area (every UV triangle is degenerate); run \
                     `geo.uv.apply` first"
                ),
            ));
        }
        let samples = texels.saturating_mul(u64::from(triangles));
        total = total.saturating_add(samples);
        area_sum += coverage;
        rows.push(BakeMeshOut {
            mesh: count(index),
            triangles,
            uv_triangles,
            chart_coverage: coverage,
            samples,
            over_cap: samples > MAX_BAKE_SAMPLES,
        });
    }
    let mean = if rows.is_empty() {
        0.0
    } else {
        area_sum / f32_from_f64(f64::from(count(rows.len())))
    };
    Ok(BakePreviewOutput {
        path: args.path.clone(),
        resolution,
        kind: kind.to_owned(),
        over_cap: rows.iter().any(|row| row.over_cap),
        meshes: rows,
        chart_coverage: mean.clamp(0.0, 1.0),
        samples: total,
    })
}

/// Bake one primitive at `resolution`; the cage is 1% of its box diagonal.
fn bake_one(mesh: &GeoMesh, resolution: u32) -> Result<NormalMap, McpToolError> {
    let cage = (diagonal(mesh) * 0.01).max(1e-4);
    bake_normals(mesh, mesh, resolution, cage).map_err(|e| cook_failed("normal bake", &e))
}

fn rgb_of(map: &NormalMap) -> Vec<u8> {
    let mut rgb = Vec::with_capacity(map.rgb.len() * 3);
    for texel in &map.rgb {
        rgb.extend_from_slice(texel);
    }
    rgb
}

fn bake_apply(_server: &McpServer, args: &BakeApplyArgs) -> Result<BakeApplyOutput, McpToolError> {
    let preview = bake_preview(
        _server,
        &BakePreviewArgs {
            path: args.path.clone(),
            resolution: args.resolution,
            kind: args.kind.clone(),
        },
    )?;
    if preview.over_cap {
        let worst = preview
            .meshes
            .iter()
            .find(|row| row.over_cap)
            .map(|row| row.samples)
            .unwrap_or(preview.samples);
        return Err(McpToolError::new(
            "too_big",
            format!(
                "a {} map over this file spends {worst} texel-triangle samples; at most \
                 {MAX_BAKE_SAMPLES}; pass a smaller resolution (256 for a 1920-triangle prop) or \
                 decimate the mesh first",
                preview.resolution
            ),
        ));
    }
    let loaded = load(&args.path)?;
    let folder = out_dir(args.out_dir.as_deref());
    let mut maps = Vec::with_capacity(loaded.meshes.len());
    let mut bytes = 0u64;
    for (index, mesh) in loaded.meshes.iter().enumerate() {
        let map = bake_one(mesh, preview.resolution)?;
        let rgb = rgb_of(&map);
        let png = encode_png_rgb8(map.size, map.size, &rgb).map_err(|e| {
            McpToolError::new(
                "cook_failed",
                format!("mesh {index} PNG encode refused: {e}; report the verb"),
            )
        })?;
        let digest = digest_of_bytes(&png);
        let name = format!("bake-{digest}-m{index:02}.png");
        let path = write_payload(&folder, &name, &png)?;
        let covered = count(map.covered.iter().filter(|&&c| c).count());
        let total = map.size.saturating_mul(map.size).max(1);
        bytes = bytes.saturating_add(wide(png.len()));
        maps.push(BakeMapOut {
            mesh: count(index),
            path: path.display().to_string(),
            bytes: wide(png.len()),
            covered_texels: covered,
            covered_fraction: f32_from_f64(f64::from(covered) / f64::from(total)),
            digest,
        });
    }
    Ok(BakeApplyOutput {
        preview,
        maps,
        bytes,
    })
}

crate::e2_tool!(
    /// `geo.bake.preview`: report bake coverage and sample cost.
    BakePreviewTool,
    name = TOOL_GEO_BAKE_PREVIEW,
    description = "Report what a normal bake of a .glb/.gltf would cost: every primitive's UV triangles, chart coverage, texel-triangle samples against the bake cap, and the map size. No writes.",
    readonly = true,
    args = BakePreviewArgs,
    output = BakePreviewOutput,
    invoke = bake_preview
);

crate::e2_tool!(
    /// `geo.bake.apply`: bake the object-space normal map and write PNG files.
    BakeApplyTool,
    name = TOOL_GEO_BAKE_APPLY,
    description = "Bake the object-space normal map of every primitive of a .glb/.gltf at 256, 512 or 1024 texels (parry rays through engine-geo) and write one PNG per primitive under out_dir; a bake over the sample cap refuses with the count.",
    readonly = false,
    args = BakeApplyArgs,
    output = BakeApplyOutput,
    invoke = bake_apply
);
