//! `asset.cook`, `asset.lint` and `asset.budget`: the artist's import loop over a GLB.
//!
//! `asset.cook` is card S3 of `docs/research/editor-art-gaps-2026-10-24/README.md`: one
//! verb takes a GLB and options (`uv: keep|unwrap`, `lod: [ratios]`, `collision:
//! none|hull|convex`, `bake: none|normal`, `atlas: 512|1024`), chains the `engine-geo`
//! modules (unwrap, seam-locked simplify, convex hulls, normal bake) and then the
//! `engine-assets` bundle cook, and returns a budget table: triangles per LOD against a
//! triangle budget, hull count, UV overlapping chart pairs, map bytes and a draw-call
//! estimate, with one warning row per budget miss. The same GLB and options cook the same
//! bundle bytes twice (the verb cooks twice in one call and reports the comparison).
//!
//! `asset.lint` is W1: a strict import check that returns codes, one fix per finding and a
//! pass/fail summary, in a fixed order.
//!
//! `asset.budget` is W6: the per-asset cost row (triangles per LOD, texture bytes, hull
//! count, draw estimate) from the cooked data, and a level total when a placement JSON is
//! given.
//!
//! Units: meters for geometry, texels for maps, bytes for payloads. Determinism: every step
//! is a pure function of the input bytes and options (document-order primitive walk, sorted
//! names, no clock, no RNG), so two cooks of one input are byte-equal. Offline cook only:
//! nothing here runs in the fixed tick. Errors: [`McpToolError`] codes `bad_input`,
//! `too_big`, `cook_failed` and `io_failed`, each naming the legal values and one recovery.
//!
//! Feature gate: built only with both the `intake` (glTF import, bundle cook) and `geo`
//! (mesh tools) features; both are off by default.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use engine_assets::bundle_cook::{cook_bundle_bytes, verify_bundle_bytes, StagedCooked};
use engine_assets::mesh_cook::{
    tri_count, MeshClass, MAX_HULL_VERTS, MAX_LOD_LEVELS, PROP_MAX_TRIS,
};
use engine_assets::{
    cook_gltf_bytes, cook_texture, encode_png_rgb8, import_gltf_bytes, CookedMaterial, CookedMesh,
    GltfImport, TextureRef, TextureSlot,
};
use engine_core::checked::{f32_from_f64, saturating_f64_to_u64};
use engine_geo::bake::bake_normals;
use engine_geo::collision as geo_collision;
use engine_geo::lod as geo_lod;
use engine_geo::uv as geo_uv;
use engine_geo::{GeoError, GeoMesh};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json::Value;

use super::{TOOL_ASSET_BUDGET, TOOL_ASSET_COOK, TOOL_ASSET_LINT};
use crate::mcp::{McpServer, McpToolError};

/// Default atlas edge in texels when `asset.cook` is not told one.
pub const DEFAULT_ATLAS: u32 = 512;
/// Atlas edges `asset.cook` accepts.
pub const ATLAS_EDGES: [u32; 2] = [512, 1024];
/// Default LOD ratios: full, half, quarter.
pub const DEFAULT_LOD_RATIOS: [f32; 3] = [1.0, 0.5, 0.25];
/// Smallest LOD ratio accepted.
pub const MIN_LOD_RATIO: f32 = 0.05;
/// Most hulls a `convex` cook asks for.
pub const MAX_CONVEX_HULLS: u32 = 8;
/// Most texel-triangle samples one baked map may spend (the bake budget).
pub const MAX_BAKE_SAMPLES: u64 = 150_000_000;
/// Largest accepted asset name in chars.
pub const MAX_NAME_CHARS: usize = 64;
/// Largest deviation from the declared size that still lints clean, as a fraction.
pub const MAX_SIZE_DEVIATION: f32 = 0.05;
/// Largest distance from ground to the pivot that still lints clean, meters.
pub const PIVOT_EPS_M: f32 = 1e-3;
/// Most materials one prop may carry and still lint clean.
pub const MAX_MATERIALS: usize = 4;
/// Default payload folder, relative to the door's working directory.
pub const DEFAULT_OUT_DIR: &str = "target/asset-cook";
/// Default folder of cooked map GLBs, relative to the door's working directory.
pub const DEFAULT_ASSET_DIR: &str = "content/maps/cooked";

fn bad(message: impl Into<String>) -> McpToolError {
    McpToolError::new("bad_input", message)
}

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn wide(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
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

fn cook_failed(step: &str, error: &GeoError) -> McpToolError {
    let fix = match error {
        GeoError::Empty => "; the mesh has no triangles, export it with geometry",
        GeoError::Malformed("uvs missing") => "; run the cook with `uv: unwrap` first",
        _ => "; check the mesh opens in the engine and has finite values",
    };
    McpToolError::new("cook_failed", format!("{step} refused: {error}{fix}"))
}

fn read_glb(path: &str) -> Result<Vec<u8>, McpToolError> {
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
    std::fs::read(file).map_err(|e| io_failed("read", file, &e))
}

/// The mesh behind one glTF primitive, in the geo type.
fn geo_of(mesh: &CookedMesh) -> GeoMesh {
    GeoMesh {
        positions: mesh.positions.clone(),
        normals: mesh.normals.clone(),
        uvs: mesh.uv0.clone(),
        indices: mesh.indices.clone(),
    }
}

/// Unit tangent from the UVs, with the handedness `w` exactly `-1` or `1`.
///
/// The unwrap splits vertices, so the imported tangents no longer line up: this rebuilds
/// them from positions, normals and UV0 with per-vertex accumulation in index order.
fn tangents_from_uv(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    uvs: &[[f32; 2]],
    indices: &[u32],
) -> Vec<[f32; 4]> {
    // Collision hulls and other UV-less meshes keep the cook's default tangent.
    if uvs.len() != positions.len() {
        return vec![[1.0, 0.0, 0.0, 1.0]; positions.len()];
    }
    let mut sums = vec![[0.0f64; 3]; positions.len()];
    let mut batches = vec![0.0f64; positions.len()];
    for triangle in indices.chunks_exact(3) {
        let at = |index: u32| usize::try_from(index).unwrap_or(0);
        let (i0, i1, i2) = (at(triangle[0]), at(triangle[1]), at(triangle[2]));
        let (p0, p1, p2) = (positions[i0], positions[i1], positions[i2]);
        let e1 = [p1[0] - p0[0], p1[1] - p0[1], p1[2] - p0[2]];
        let e2 = [p2[0] - p0[0], p2[1] - p0[1], p2[2] - p0[2]];
        let (u0, u1, u2) = (uvs[i0], uvs[i1], uvs[i2]);
        let (du1, dv1) = (f64::from(u1[0] - u0[0]), f64::from(u1[1] - u0[1]));
        let (du2, dv2) = (f64::from(u2[0] - u0[0]), f64::from(u2[1] - u0[1]));
        let determinant = du1 * dv2 - du2 * dv1;
        if determinant.abs() < 1e-12 {
            continue;
        }
        let f = 1.0 / determinant;
        let tangent = [
            f64::from(e1[0]) * dv2 - f64::from(e2[0]) * dv1,
            f64::from(e1[1]) * dv2 - f64::from(e2[1]) * dv1,
            f64::from(e1[2]) * dv2 - f64::from(e2[2]) * dv1,
        ];
        let bitangent = [
            f64::from(e2[0]) * du1 - f64::from(e1[0]) * du2,
            f64::from(e2[1]) * du1 - f64::from(e1[1]) * du2,
            f64::from(e2[2]) * du1 - f64::from(e1[2]) * du2,
        ];
        let normal = normals.get(i0).copied().unwrap_or([0.0, 1.0, 0.0]);
        let n = [
            f64::from(normal[0]),
            f64::from(normal[1]),
            f64::from(normal[2]),
        ];
        let handedness = if bitangent[0] * (n[1] * tangent[2] - n[2] * tangent[1])
            - bitangent[1] * (n[0] * tangent[2] - n[2] * tangent[0])
            + bitangent[2] * (n[0] * tangent[1] - n[1] * tangent[0])
            < 0.0
        {
            -1.0
        } else {
            1.0
        };
        for index in [i0, i1, i2] {
            for axis in 0..3 {
                sums[index][axis] += tangent[axis] * f;
            }
            batches[index] += handedness;
        }
    }
    let mut tangents = Vec::with_capacity(positions.len());
    for (index, normal) in normals.iter().enumerate().map(|(i, n)| (i, *n)) {
        let mut tangent = sums.get(index).copied().unwrap_or([1.0, 0.0, 0.0]);
        let dot = tangent[0] * f64::from(normal[0])
            + tangent[1] * f64::from(normal[1])
            + tangent[2] * f64::from(normal[2]);
        for axis in 0..3 {
            tangent[axis] -= dot * f64::from(normal[axis]);
        }
        let length =
            (tangent[0] * tangent[0] + tangent[1] * tangent[1] + tangent[2] * tangent[2]).sqrt();
        let unit = if length > 1e-12 {
            [
                f32_from_f64(tangent[0] / length),
                f32_from_f64(tangent[1] / length),
                f32_from_f64(tangent[2] / length),
            ]
        } else {
            [1.0, 0.0, 0.0]
        };
        let handedness = if batches.get(index).copied().unwrap_or(0.0) < 0.0 {
            -1.0
        } else {
            1.0
        };
        tangents.push([unit[0], unit[1], unit[2], handedness]);
    }
    tangents
}

/// A cooked mesh from a geo mesh, with tangents rebuilt from the UVs.
fn cooked_of(mesh: &GeoMesh) -> CookedMesh {
    CookedMesh {
        positions: mesh.positions.clone(),
        normals: if mesh.normals.len() == mesh.positions.len() {
            mesh.normals.clone()
        } else {
            vec![[0.0, 1.0, 0.0]; mesh.positions.len()]
        },
        uv0: if mesh.uvs.len() == mesh.positions.len() {
            mesh.uvs.clone()
        } else {
            vec![[0.0, 0.0]; mesh.positions.len()]
        },
        tangents: tangents_from_uv(&mesh.positions, &mesh.normals, &mesh.uvs, &mesh.indices),
        indices: mesh.indices.clone(),
    }
}

fn digest_hex(bytes: &[u8]) -> String {
    format!("{:016x}", engine_core::hash_bytes(bytes))
}

// ---------------------------------------------------------------- asset.cook

/// `uv` option of `asset.cook`: keep the imported UVs or unwrap a new atlas.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde", rename_all = "snake_case")]
#[schemars(crate = "rmcp::schemars")]
pub enum UvMode {
    /// Keep the UVs the file arrived with.
    #[default]
    Keep,
    /// Cut and pack a new atlas with xatlas.
    Unwrap,
}

/// `collision` option of `asset.cook`: no collider, one hull, or a convex decomposition.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde", rename_all = "snake_case")]
#[schemars(crate = "rmcp::schemars")]
pub enum CollisionMode {
    /// No collider.
    #[default]
    None,
    /// One convex hull per primitive.
    Hull,
    /// A V-HACD convex decomposition per primitive.
    Convex,
}

/// `bake` option of `asset.cook`: no map, or an object-space normal map.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde", rename_all = "snake_case")]
#[schemars(crate = "rmcp::schemars")]
pub enum BakeMode {
    /// No map.
    #[default]
    None,
    /// Object-space normal map of every primitive.
    Normal,
}

/// Options of `asset.cook`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CookOptions {
    /// `keep` or `unwrap` (default `keep`).
    #[serde(default)]
    pub uv: UvMode,
    /// Fractions of LOD0 triangles to keep, each in [0.05, 1] (default [1, 0.5, 0.25]).
    #[serde(default)]
    pub lod: Vec<f32>,
    /// `none`, `hull` or `convex` (default `none`).
    #[serde(default)]
    pub collision: CollisionMode,
    /// `none` or `normal` (default `none`).
    #[serde(default)]
    pub bake: BakeMode,
    /// Atlas edge in texels: 512 or 1024 (default 512).
    #[serde(default)]
    pub atlas: Option<u32>,
}

impl Default for CookOptions {
    fn default() -> Self {
        Self {
            uv: UvMode::Keep,
            lod: DEFAULT_LOD_RATIOS.to_vec(),
            collision: CollisionMode::None,
            bake: BakeMode::None,
            atlas: Some(DEFAULT_ATLAS),
        }
    }
}

/// Args for `asset.cook`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CookArgs {
    /// Asset name to cook: a-z, 0-9, `_`, at most 64 chars.
    pub name: String,
    /// Path of the `.glb` or `.gltf` file.
    pub glb_path: String,
    /// Cook options; omitted fields take the defaults above.
    #[serde(default)]
    pub options: CookOptions,
    /// Output folder (default `target/asset-cook`).
    #[serde(default)]
    pub out_dir: Option<String>,
}

/// One LOD row of the budget table.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LodBudgetRow {
    /// Level index, 0 is LOD0.
    pub level: u32,
    /// Ratio this level asked for.
    pub ratio: f32,
    /// Triangles across every primitive.
    pub triangles: u32,
    /// Triangle budget of this level (LOD0's count times the ratio).
    pub budget: u32,
    /// True when `triangles > budget`: a warning row was added.
    pub over_budget: bool,
}

/// Result of `asset.cook`: the bundle receipt plus the budget table.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct CookOutput {
    /// Asset name.
    pub asset: String,
    /// Where the bundle was written.
    pub bundle_path: String,
    /// Bundle size in bytes.
    pub bundle_bytes: u64,
    /// Lowercase hex digest of the bundle bytes.
    pub bundle_digest: String,
    /// Triangles per LOD against its budget.
    pub lod: Vec<LodBudgetRow>,
    /// Hull parts staged (`COL_` meshes).
    pub hulls: u32,
    /// UV triangle pairs sharing atlas area (0 is the goal).
    pub overlapping_chart_pairs: u32,
    /// Normal map bytes in the bundle.
    pub map_bytes: u64,
    /// Draw calls at LOD0: one per triangle primitive.
    pub draw_calls: u32,
    /// One warning per budget miss, in a fixed order.
    pub warnings: Vec<String>,
    /// True when two cooks in this call gave byte-equal bundles.
    pub identical_twice: bool,
    /// The budget constants this report judged against.
    pub budget_constants: Vec<String>,
}

fn atlas_of(given: Option<u32>) -> Result<u32, McpToolError> {
    let atlas = given.unwrap_or(DEFAULT_ATLAS);
    if !ATLAS_EDGES.contains(&atlas) {
        return Err(bad(format!(
            "atlas {atlas} is not an atlas edge; send {} or {}",
            ATLAS_EDGES[0], ATLAS_EDGES[1]
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
            "{} LOD ratios asked; at most {MAX_LOD_LEVELS} levels",
            ratios.len()
        )));
    }
    for ratio in &ratios {
        if !ratio.is_finite() || !(MIN_LOD_RATIO..=1.0).contains(ratio) {
            return Err(bad(format!(
                "LOD ratio {ratio} is outside {MIN_LOD_RATIO}..=1; send the fraction of triangles to keep"
            )));
        }
    }
    Ok(ratios)
}

fn name_ok(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME_CHARS
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// What one cook staged, before the budget rows are judged.
#[derive(Default)]
struct Built {
    triangles: Vec<u32>,
    hulls: u32,
    overlaps: u32,
    map_bytes: u64,
    draws: u32,
    warnings: Vec<String>,
}

/// Chart ids for the "keep" path: every triangle is its own chart, so an overlapping pair is
/// exactly two UV triangles that share area.
fn per_triangle_charts(mesh: &GeoMesh) -> Vec<u32> {
    (0..mesh.indices.len() / 3).map(count).collect()
}

fn cage_of(mesh: &GeoMesh) -> f32 {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    for position in &mesh.positions {
        for axis in 0..3 {
            lo[axis] = lo[axis].min(position[axis]);
            hi[axis] = hi[axis].max(position[axis]);
        }
    }
    let d = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
    let diagonal = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    (diagonal * 0.01).max(1e-4)
}

/// Import the file or refuse with the door's typed codes.
fn import_or_refuse(bytes: &[u8]) -> Result<GltfImport, McpToolError> {
    let imported = import_gltf_bytes(bytes).map_err(|e| {
        McpToolError::new(
            "bad_input",
            format!("the GLB did not import: {e}. Next: check the file opens in a glTF viewer."),
        )
    })?;
    if imported.meshes.is_empty() {
        return Err(bad(
            "the GLB holds no triangle meshes; export geometry and try again",
        ));
    }
    Ok(imported)
}

/// LOD0: the imported UVs kept, or a fresh unwrap; its overlapping chart pairs.
fn uv_low(
    base: &GeoMesh,
    uv: UvMode,
    atlas: u32,
    index: usize,
) -> Result<(GeoMesh, u32), McpToolError> {
    match uv {
        UvMode::Keep => {
            let charts = per_triangle_charts(base);
            let overlaps = count(geo_uv::overlapping_chart_pairs(
                &base.uvs,
                &base.indices,
                &charts,
            ));
            Ok((base.clone(), overlaps))
        }
        UvMode::Unwrap => {
            let unwrap_options = geo_uv::UnwrapOptions {
                resolution: atlas,
                ..geo_uv::UnwrapOptions::default()
            };
            let unwrapped = geo_uv::unwrap(base, &unwrap_options)
                .map_err(|e| cook_failed(&format!("mesh {index} unwrap"), &e))?;
            let overlaps = count(geo_uv::overlapping_chart_pairs(
                &unwrapped.mesh.uvs,
                &unwrapped.mesh.indices,
                &unwrapped.chart_of_triangle,
            ));
            Ok((unwrapped.mesh, overlaps))
        }
    }
}

/// Stage one LOD mesh per ratio and add its triangles to that level's total.
fn stage_lods(
    low: &GeoMesh,
    ratios: &[f32],
    name: &str,
    index: usize,
    staged: &mut StagedCooked,
    built: &mut Built,
) -> Result<(), McpToolError> {
    for (level, ratio) in ratios.iter().copied().enumerate() {
        let mesh = if ratio >= 1.0 {
            low.clone()
        } else {
            let lod_options = geo_lod::LodOptions {
                target_ratio: ratio,
                ..geo_lod::LodOptions::default()
            };
            geo_lod::simplify(low, &[], 0, 0.0, &lod_options)
                .map_err(|e| cook_failed(&format!("mesh {index} lod {level}"), &e))?
                .mesh
        };
        if let Some(row) = built.triangles.get_mut(level) {
            *row = row.saturating_add(count(mesh.indices.len() / 3));
        }
        staged
            .meshes
            .push((format!("{name}/m{index:02}_lod{level}"), cooked_of(&mesh)));
    }
    Ok(())
}

/// Stage the collision meshes the option asks for (none, one hull, or a convex set).
fn stage_hulls(
    low: &GeoMesh,
    mode: CollisionMode,
    name: &str,
    index: usize,
    staged: &mut StagedCooked,
    built: &mut Built,
) -> Result<(), McpToolError> {
    let parts = match mode {
        CollisionMode::None => Vec::new(),
        CollisionMode::Hull => vec![geo_collision::single_hull(low, MAX_HULL_VERTS)
            .map_err(|e| cook_failed(&format!("mesh {index} hull"), &e))?],
        CollisionMode::Convex => {
            let hull_options = geo_collision::HullOptions {
                max_hulls: MAX_CONVEX_HULLS,
                max_hull_vertices: MAX_HULL_VERTS,
                ..geo_collision::HullOptions::default()
            };
            geo_collision::decompose(low, &hull_options)
                .map_err(|e| cook_failed(&format!("mesh {index} convex"), &e))?
        }
    };
    for (part_index, part) in parts.iter().enumerate() {
        staged.meshes.push((
            format!("{name}/COL_m{index:02}_{part_index:02}"),
            cooked_of(part),
        ));
        built.hulls = built.hulls.saturating_add(1);
    }
    Ok(())
}

/// Bake the object-space normal map into one material and stage the texture.
fn stage_material(
    material: &mut CookedMaterial,
    base: &GeoMesh,
    low: &GeoMesh,
    index: usize,
    name: &str,
    bake: BakeMode,
    atlas: u32,
    staged: &mut StagedCooked,
    built: &mut Built,
) -> Result<(), McpToolError> {
    if bake != BakeMode::Normal {
        return Ok(());
    }
    let samples = u64::from(atlas)
        .saturating_mul(u64::from(atlas))
        .saturating_mul(u64::from(count(low.indices.len() / 3)));
    if samples > MAX_BAKE_SAMPLES {
        built.warnings.push(format!(
            "mesh {index}: a {atlas} normal bake spends {samples} texel-triangle samples, over MAX_BAKE_SAMPLES ({MAX_BAKE_SAMPLES}); pass atlas 512 or decimate"
        ));
        return Ok(());
    }
    let map = bake_normals(low, base, atlas, cage_of(low))
        .map_err(|e| cook_failed(&format!("mesh {index} normal bake"), &e))?;
    let mut rgb = Vec::with_capacity(map.rgb.len() * 3);
    for texel in &map.rgb {
        rgb.extend_from_slice(texel);
    }
    let png = encode_png_rgb8(map.size, map.size, &rgb).map_err(|e| {
        McpToolError::new(
            "cook_failed",
            format!("mesh {index} PNG encode refused: {e}; report the verb"),
        )
    })?;
    let texture = cook_texture(&png, TextureSlot::Normal).map_err(|e| {
        McpToolError::new(
            "cook_failed",
            format!("mesh {index} normal map did not cook: {e:?}; report the verb"),
        )
    })?;
    let texture_id = engine_assets::texture_content_id(&png);
    // The bundle rejects a duplicate slot, so the baked map replaces any normal already on the material.
    material.textures.retain(|t| t.slot != TextureSlot::Normal);
    material.textures.push(TextureRef {
        asset: texture_id,
        slot: TextureSlot::Normal,
    });
    built.map_bytes = built.map_bytes.saturating_add(wide(png.len()));
    staged
        .textures
        .push((format!("{name}/normal_m{index:02}"), texture));
    Ok(())
}

/// Stage one cook: the LOD chain, hulls, maps and the bundle bytes.
fn build_bundle(
    bytes: &[u8],
    name: &str,
    options: &CookOptions,
    atlas: u32,
    ratios: &[f32],
) -> Result<(Vec<u8>, Built), McpToolError> {
    let imported = import_or_refuse(bytes)?;
    let mut staged = StagedCooked::new();
    let mut built = Built {
        triangles: vec![0; ratios.len()],
        ..Built::default()
    };
    for (index, primitive) in imported.meshes.iter().enumerate() {
        let base = geo_of(primitive);
        let (low, overlaps) = uv_low(&base, options.uv, atlas, index)?;
        built.overlaps = built.overlaps.saturating_add(overlaps);
        stage_lods(&low, ratios, name, index, &mut staged, &mut built)?;
        built.draws = built.draws.saturating_add(1);
        stage_hulls(
            &low,
            options.collision,
            name,
            index,
            &mut staged,
            &mut built,
        )?;
        let mut material = imported.materials.get(index).cloned().unwrap_or_default();
        stage_material(
            &mut material,
            &base,
            &low,
            index,
            name,
            options.bake,
            atlas,
            &mut staged,
            &mut built,
        )?;
        staged
            .materials
            .push((format!("{name}/mat_m{index:02}"), material));
    }
    let bytes = cook_bundle_bytes(&staged).map_err(|e| {
        McpToolError::new(
            "cook_failed",
            format!("the bundle cook refused: {e}. Next: check the staged mesh sizes."),
        )
    })?;
    Ok((bytes, built))
}

fn cook(args: &CookArgs) -> Result<CookOutput, McpToolError> {
    if !name_ok(&args.name) {
        return Err(bad(format!(
            "name `{}` is not a-z 0-9 _ in 1..={MAX_NAME_CHARS} chars; rename it (lower_snake)",
            args.name
        )));
    }
    let atlas = atlas_of(args.options.atlas)?;
    let ratios = ratios_of(&args.options.lod)?;
    let bytes = read_glb(&args.glb_path)?;
    let (first, built) = build_bundle(&bytes, &args.name, &args.options, atlas, &ratios)?;
    let (second, _) = build_bundle(&bytes, &args.name, &args.options, atlas, &ratios)?;
    let identical_twice = first == second;
    if !identical_twice {
        return Err(McpToolError::new(
            "cook_failed",
            "two cooks of the same GLB gave different bundle bytes; report the verb with the path",
        ));
    }
    verify_bundle_bytes(&first, false).map_err(|e| {
        McpToolError::new(
            "cook_failed",
            format!("the cooked bundle did not verify: {e}; report the verb"),
        )
    })?;
    let folder = PathBuf::from(args.out_dir.as_deref().unwrap_or(DEFAULT_OUT_DIR));
    std::fs::create_dir_all(&folder).map_err(|e| io_failed("create", &folder, &e))?;
    let bundle_path = folder.join(format!("{}.e204bndl", args.name));
    std::fs::write(&bundle_path, &first).map_err(|e| io_failed("write", &bundle_path, &e))?;
    let mut warnings = built.warnings.clone();
    let rows = budget_rows(&built.triangles, &ratios, &mut warnings);
    if built.overlaps > 0 {
        warnings.push(format!(
            "UV: {} triangle pairs share atlas area; cook with uv: unwrap",
            built.overlaps
        ));
    }
    Ok(CookOutput {
        asset: args.name.clone(),
        bundle_path: bundle_path.display().to_string(),
        bundle_bytes: wide(first.len()),
        bundle_digest: digest_hex(&first),
        lod: rows,
        hulls: built.hulls,
        overlapping_chart_pairs: built.overlaps,
        map_bytes: built.map_bytes,
        draw_calls: built.draws,
        warnings,
        identical_twice,
        budget_constants: vec![
            format!("engine_assets::mesh_cook::PROP_MAX_TRIS ({PROP_MAX_TRIS})"),
            format!("engine_assets::mesh_cook::MAX_LOD_LEVELS ({MAX_LOD_LEVELS})"),
            format!("engine_assets::mesh_cook::MAX_HULL_VERTS ({MAX_HULL_VERTS})"),
            format!("crate::packs::asset::cook_verbs::MAX_BAKE_SAMPLES ({MAX_BAKE_SAMPLES})"),
        ],
    })
}

/// Judge each LOD against its budget and add one warning per miss. LOD0's own count times
/// the level ratio is the budget; a level that keeps more triangles than that (seam locks or
/// an error cap stop the collapse) is a warning row, never a silent pass.
fn budget_rows(lod_tris: &[u32], ratios: &[f32], warnings: &mut Vec<String>) -> Vec<LodBudgetRow> {
    let lod0 = lod_tris.first().copied().unwrap_or(0);
    let mut rows = Vec::with_capacity(ratios.len());
    for (level, ratio) in ratios.iter().copied().enumerate() {
        let triangles = lod_tris.get(level).copied().unwrap_or(0);
        let scaled = f64::from(lod0) * f64::from(ratio);
        let budget = u32::try_from(saturating_f64_to_u64(scaled.floor().max(1.0))).unwrap_or(1);
        let over_budget = triangles > budget;
        if over_budget {
            warnings.push(format!(
                "LOD{level} holds {triangles} triangles over its {budget} target (LOD0 {lod0} x ratio {ratio}); decimate the source or raise the ratio"
            ));
        }
        rows.push(LodBudgetRow {
            level: count(level),
            ratio,
            triangles,
            budget,
            over_budget,
        });
    }
    if lod0 > PROP_MAX_TRIS {
        warnings.push(format!(
            "LOD0 holds {lod0} triangles over PROP_MAX_TRIS ({PROP_MAX_TRIS}); split the prop or decimate it"
        ));
    }
    rows
}

fn cook_verb(_server: &McpServer, args: &CookArgs) -> Result<CookOutput, McpToolError> {
    cook(args)
}

crate::e2_tool!(
    /// `asset.cook`: the whole mesh chain over one GLB, with a budget table.
    CookTool,
    name = TOOL_ASSET_COOK,
    description = "Cook one .glb/.gltf to an E204BNDL: optional xatlas unwrap, a seam-locked LOD chain, collision hulls, an object-space normal map, then the assets bundle cook. Returns triangles per LOD against a budget, hull count, UV overlapping pairs, map bytes, draw calls and one warning per budget miss. Two cooks of one input are byte-equal.",
    readonly = false,
    args = CookArgs,
    output = CookOutput,
    invoke = cook_verb
);

// ---------------------------------------------------------------- asset.lint

/// Args for `asset.lint`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LintArgs {
    /// Path of the `.glb` or `.gltf` file.
    pub glb_path: String,
    /// Asset name to check (default: the file stem).
    #[serde(default)]
    pub name: Option<String>,
    /// Declared unit of the mesh: `m`, `cm` or `mm` (default `m`).
    #[serde(default)]
    pub unit: Option<String>,
    /// Expected longest side in the declared unit (default 1.0).
    #[serde(default)]
    pub expected_size: Option<f32>,
    /// LOD0 triangle budget (default PROP_MAX_TRIS, 5000).
    #[serde(default)]
    pub max_tris: Option<u32>,
}

/// One lint finding: a code, what was measured, and the one fix.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LintFinding {
    /// Stable code, `L001`..`L007`, ascending in the result.
    pub code: String,
    /// Which check this is.
    pub check: String,
    /// What was measured.
    pub measured: String,
    /// The one fix.
    pub fix: String,
}

/// Result of `asset.lint`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LintOutput {
    /// File that was checked.
    pub path: String,
    /// Asset name that was checked.
    pub asset: String,
    /// One finding per failed check, code order.
    pub findings: Vec<LintFinding>,
    /// The codes of the findings, code order.
    pub codes: Vec<String>,
    /// True when no check failed.
    pub passed: bool,
    /// One line: `pass` or `fail` with the counts.
    pub summary: String,
}

fn unit_scale(given: Option<&str>) -> Result<f32, McpToolError> {
    match given.unwrap_or("m") {
        "m" => Ok(1.0),
        "cm" => Ok(0.01),
        "mm" => Ok(0.001),
        other => Err(bad(format!(
            "unit `{other}` is not a unit; send `m`, `cm` or `mm`"
        ))),
    }
}

fn finding(code: &str, check: &str, measured: String, fix: &str) -> LintFinding {
    LintFinding {
        code: code.to_owned(),
        check: check.to_owned(),
        measured,
        fix: fix.to_owned(),
    }
}

/// Bounds, triangle count and UV-area triangle count of one import.
struct MeshFacts {
    lo: [f32; 3],
    hi: [f32; 3],
    triangles: u64,
    uv_triangles: u64,
}

fn mesh_facts(imported: &GltfImport) -> Result<MeshFacts, McpToolError> {
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    let mut triangles = 0u64;
    let mut uv_triangles = 0u64;
    for mesh in &imported.meshes {
        triangles = triangles.saturating_add(wide(mesh.indices.len() / 3));
        let mesh_geo = geo_of(mesh);
        let mut with_uv = 0u32;
        if mesh_geo.uvs.len() == mesh_geo.positions.len() {
            for triangle in mesh_geo.indices.chunks_exact(3) {
                let at = |index: u32| mesh_geo.uvs[usize::try_from(index).unwrap_or(0)];
                let (a, b, c) = (at(triangle[0]), at(triangle[1]), at(triangle[2]));
                let u = [f64::from(b[0] - a[0]), f64::from(b[1] - a[1])];
                let v = [f64::from(c[0] - a[0]), f64::from(c[1] - a[1])];
                if (u[0] * v[1] - u[1] * v[0]).abs() > 1e-12 {
                    with_uv = with_uv.saturating_add(1);
                }
            }
        }
        uv_triangles = uv_triangles.saturating_add(u64::from(with_uv));
        for position in &mesh.positions {
            for axis in 0..3 {
                lo[axis] = lo[axis].min(position[axis]);
                hi[axis] = hi[axis].max(position[axis]);
            }
        }
    }
    if !lo[0].is_finite() || hi[0] < lo[0] {
        return Err(bad(
            "the GLB holds no vertices; export geometry and try again",
        ));
    }
    Ok(MeshFacts {
        lo,
        hi,
        triangles,
        uv_triangles,
    })
}

/// L001 and L002: longest side against the declared unit, and pivot at ground.
fn size_findings(
    facts: &MeshFacts,
    unit: Option<&str>,
    expected: f32,
    scale: f32,
) -> Vec<LintFinding> {
    let mut findings = Vec::new();
    let size_m = f64::from(facts.hi[0] - facts.lo[0])
        .max(f64::from(facts.hi[1] - facts.lo[1]))
        .max(f64::from(facts.hi[2] - facts.lo[2]));
    let want_m = f64::from(expected) * f64::from(scale);
    let deviation = if want_m > 0.0 {
        (size_m - want_m).abs() / want_m
    } else {
        0.0
    };
    if deviation > f64::from(MAX_SIZE_DEVIATION) {
        findings.push(finding(
            "L001",
            "scale",
            format!(
                "longest side {:.4} m, declared {expected} {} ({want_m:.4} m), deviation {:.1}%",
                size_m,
                unit.unwrap_or("m"),
                deviation * 100.0
            ),
            "export in meters at the declared size, or pass the unit the file was authored in",
        ));
    }
    let pivot = f64::from(facts.lo[1]);
    if pivot.abs() > f64::from(PIVOT_EPS_M) {
        findings.push(finding(
            "L002",
            "pivot_at_ground",
            format!("lowest point sits at y {pivot:.4} m"),
            "set the object origin at the lowest point (apply transforms with the 3D cursor at the base)",
        ));
    }
    findings
}

/// The file's own material count: the import expands one material per primitive.
fn document_materials(bytes: &[u8]) -> usize {
    let json = if bytes.starts_with(b"{") {
        bytes
    } else {
        match engine_assets::gltf::glb_json(bytes) {
            Ok(json) => json,
            Err(_) => return 0,
        }
    };
    rmcp::serde_json::from_slice::<Value>(json)
        .ok()
        .and_then(|doc| doc.get("materials").and_then(Value::as_array).map(Vec::len))
        .unwrap_or(0)
}

/// L003 to L007: UVs, materials, collider, triangle budget and the name rule.
fn rule_findings(
    bytes: &[u8],
    facts: &MeshFacts,
    materials: usize,
    budget: u32,
    asset: &str,
) -> Vec<LintFinding> {
    let mut findings = Vec::new();
    // L003: UV presence.
    if facts.triangles == 0 || facts.uv_triangles < facts.triangles {
        findings.push(finding(
            "L003",
            "uv_presence",
            format!(
                "{} of {} triangles carry UV area",
                facts.uv_triangles, facts.triangles
            ),
            "cook with `asset.cook` and `uv: unwrap`",
        ));
    }
    // L004: material count.
    if materials == 0 || materials > MAX_MATERIALS {
        findings.push(finding(
            "L004",
            "materials",
            format!("{materials} materials, limit {MAX_MATERIALS}"),
            "assign one material per prop part, at most four, then cook",
        ));
    }
    // L005: collider presence (a node named COL_*).
    let has_collider = engine_assets::gltf::glb_json(bytes)
        .map(|json| String::from_utf8_lossy(json).contains("COL_"))
        .unwrap_or(false);
    if !has_collider {
        findings.push(finding(
            "L005",
            "collider",
            "no node named COL_*".to_owned(),
            "add a COL_ node in the DCC, or cook with `collision: hull`",
        ));
    }
    // L006: triangle budget.
    if facts.triangles > u64::from(budget) {
        findings.push(finding(
            "L006",
            "triangles",
            format!("{} triangles, budget {budget}", facts.triangles),
            "decimate the mesh or cook with a smaller LOD0",
        ));
    }
    // L007: name rule.
    if !name_ok(asset) {
        findings.push(finding(
            "L007",
            "name",
            format!("`{asset}` is not lower_snake"),
            "rename to a-z 0-9 _ with no spaces or dashes",
        ));
    }
    findings
}

fn lint(args: &LintArgs) -> Result<LintOutput, McpToolError> {
    let scale = unit_scale(args.unit.as_deref())?;
    let expected = args.expected_size.unwrap_or(1.0);
    if !expected.is_finite() || expected <= 0.0 {
        return Err(bad(format!(
            "expected_size {expected} is not a positive size; send the longest side in the declared unit"
        )));
    }
    let budget = args.max_tris.unwrap_or(PROP_MAX_TRIS);
    let bytes = read_glb(&args.glb_path)?;
    let stem = Path::new(&args.glb_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_owned();
    let asset = args.name.clone().unwrap_or(stem);
    let imported = import_or_refuse(&bytes)?;
    let facts = mesh_facts(&imported)?;
    let materials = document_materials(&bytes);
    let mut findings = size_findings(&facts, args.unit.as_deref(), expected, scale);
    findings.extend(rule_findings(&bytes, &facts, materials, budget, &asset));
    findings.sort_by(|a, b| a.code.cmp(&b.code));
    let codes: Vec<String> = findings.iter().map(|f| f.code.clone()).collect();
    let passed = findings.is_empty();
    let triangles = facts.triangles;
    let summary = if passed {
        format!("pass: 7 of 7 checks, {triangles} triangles, {materials} materials")
    } else {
        format!(
            "fail: {} of 7 checks, codes {}",
            findings.len(),
            codes.join(", ")
        )
    };
    Ok(LintOutput {
        path: args.glb_path.clone(),
        asset,
        findings,
        codes,
        passed,
        summary,
    })
}

fn lint_verb(_server: &McpServer, args: &LintArgs) -> Result<LintOutput, McpToolError> {
    lint(args)
}

crate::e2_tool!(
    /// `asset.lint`: the strict import check with codes and one fix per finding.
    LintTool,
    name = TOOL_ASSET_LINT,
    description = "Check one .glb/.gltf on import: bbox scale against the declared unit and expected size (L001), pivot at ground (L002), UV presence (L003), material count (L004), COL_ collider presence (L005), triangle budget (L006) and lower_snake name (L007). Returns codes, one fix per finding and a pass/fail summary in code order.",
    readonly = true,
    args = LintArgs,
    output = LintOutput,
    invoke = lint_verb
);

// ---------------------------------------------------------------- asset.budget

/// Args for `asset.budget`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BudgetArgs {
    /// One `.glb`/`.gltf` to cost; omit when a placement JSON is given.
    #[serde(default)]
    pub glb_path: Option<String>,
    /// Placement JSON (`{"actors": [{"asset": "name", ...}]}`) for a level total.
    #[serde(default)]
    pub placement_json: Option<String>,
    /// Folder of cooked `<asset>.glb` files the placement names (default `content/maps/cooked`).
    #[serde(default)]
    pub asset_dir: Option<String>,
    /// Level name in the total (default: the placement file stem).
    #[serde(default)]
    pub map_name: Option<String>,
}

/// The cooked cost of one asset.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct AssetCost {
    /// Asset name.
    pub asset: String,
    /// Triangles per LOD, LOD0 first.
    pub lod_tris: Vec<u32>,
    /// LOD0 triangles.
    pub triangles: u32,
    /// Encoded texture bytes the file carries.
    pub texture_bytes: u64,
    /// Collision hulls in the cooked chain.
    pub hulls: u32,
    /// Draw calls at LOD0 (one per triangle primitive).
    pub draw_calls: u32,
}

/// Level total over a placement.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LevelTotal {
    /// Level name.
    pub map: String,
    /// Actors placed.
    pub placements: u32,
    /// LOD0 triangles over every placement.
    pub lod0_tris: u64,
    /// Texture bytes over every placement.
    pub texture_bytes: u64,
    /// Hulls over every placement.
    pub hulls: u64,
    /// Draw calls over every placement.
    pub draw_calls: u64,
}

/// Result of `asset.budget`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct BudgetOutput {
    /// One cost row per asset, name order.
    pub assets: Vec<AssetCost>,
    /// Level total, set when a placement JSON was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<LevelTotal>,
    /// Lowercase hex digest of the rows.
    pub digest: String,
}

fn cost_of(path: &Path, asset: &str) -> Result<AssetCost, McpToolError> {
    let bytes = std::fs::read(path).map_err(|e| {
        McpToolError::new(
            "bad_input",
            format!(
                "cannot read `{}`: {e}. Next: cook that asset first (asset.cook writes its bundle; ship its cooked GLB here).",
                path.display()
            ),
        )
    })?;
    let cooked = cook_gltf_bytes(&bytes, MeshClass::Prop).map_err(|e| {
        McpToolError::new(
            "cook_failed",
            format!(
                "`{}` did not cook: {e}. Next: check the GLB opens.",
                path.display()
            ),
        )
    })?;
    let imported = import_gltf_bytes(&bytes).map_err(|e| {
        McpToolError::new(
            "bad_input",
            format!("`{}` did not import: {e}.", path.display()),
        )
    })?;
    let mut lod_tris = vec![0u32; MAX_LOD_LEVELS];
    let mut hulls = 0u32;
    for mesh in &cooked.meshes {
        for (level, lod) in mesh.lods.iter().enumerate() {
            if let Some(row) = lod_tris.get_mut(level) {
                *row = row.saturating_add(tri_count(lod));
            }
        }
        hulls = hulls.saturating_add(count(mesh.hulls.len()));
    }
    let texture_bytes = imported
        .images
        .values()
        .fold(0u64, |sum, bytes| sum.saturating_add(wide(bytes.len())));
    let draws = count(cooked.meshes.len());
    Ok(AssetCost {
        asset: asset.to_owned(),
        triangles: lod_tris.first().copied().unwrap_or(0),
        lod_tris,
        texture_bytes,
        hulls,
        draw_calls: draws,
    })
}

/// Asset names the placement JSON references, name order.
fn placed_assets(path: &str) -> Result<BTreeMap<String, u32>, McpToolError> {
    let text = std::fs::read_to_string(path).map_err(|e| {
        McpToolError::new(
            "bad_input",
            format!("cannot read placement_json `{path}`: {e}. Next: check the path."),
        )
    })?;
    let doc: Value = rmcp::serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| {
        McpToolError::new(
            "bad_input",
            format!("placement_json is not JSON: {e}. Next: point at content/maps/placement_*.json."),
        )
    })?;
    let actors = doc.get("actors").and_then(Value::as_array).ok_or_else(|| {
        bad(format!(
            "placement_json `{path}` has no `actors` array; send content/maps/placement_*.json"
        ))
    })?;
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for actor in actors {
        let Some(asset) = actor.get("asset").and_then(Value::as_str) else {
            return Err(bad(format!(
                "placement_json `{path}` has an actor with no `asset` name; name every actor"
            )));
        };
        *counts.entry(asset.to_owned()).or_insert(0) += 1;
    }
    Ok(counts)
}

fn budget(args: &BudgetArgs) -> Result<BudgetOutput, McpToolError> {
    let mut assets = Vec::new();
    let mut level = None;
    match (&args.glb_path, &args.placement_json) {
        (Some(glb), None) => {
            let name = Path::new(glb)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_owned();
            assets.push(cost_of(Path::new(glb), &name)?);
        }
        (None, Some(placement)) => {
            let folder = PathBuf::from(args.asset_dir.as_deref().unwrap_or(DEFAULT_ASSET_DIR));
            let counts = placed_assets(placement)?;
            let mut total = LevelTotal {
                map: args.map_name.clone().unwrap_or_else(|| {
                    Path::new(placement)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("level")
                        .to_owned()
                }),
                ..LevelTotal::default()
            };
            for (asset, placed) in &counts {
                let path = folder.join(format!("{asset}.glb"));
                let cost = cost_of(&path, asset)?;
                let multiplier = u64::from(*placed);
                total.placements = total.placements.saturating_add(*placed);
                total.lod0_tris = total
                    .lod0_tris
                    .saturating_add(u64::from(cost.triangles).saturating_mul(multiplier));
                total.texture_bytes = total
                    .texture_bytes
                    .saturating_add(cost.texture_bytes.saturating_mul(multiplier));
                total.hulls = total
                    .hulls
                    .saturating_add(u64::from(cost.hulls).saturating_mul(multiplier));
                total.draw_calls = total
                    .draw_calls
                    .saturating_add(u64::from(cost.draw_calls).saturating_mul(multiplier));
                assets.push(cost);
            }
            level = Some(total);
        }
        (Some(_), Some(_)) => {
            return Err(bad(
                "pass glb_path for one asset or placement_json for a level, not both",
            ))
        }
        (None, None) => {
            return Err(bad(
                "pass glb_path for one asset or placement_json for a level total",
            ))
        }
    }
    let mut digest_input = String::new();
    for cost in &assets {
        digest_input.push_str(&format!(
            "{}:{:?}:{}:{}:{};",
            cost.asset, cost.lod_tris, cost.texture_bytes, cost.hulls, cost.draw_calls
        ));
    }
    if let Some(total) = &level {
        digest_input.push_str(&format!(
            "{}:{}:{}:{};",
            total.map, total.lod0_tris, total.texture_bytes, total.draw_calls
        ));
    }
    Ok(BudgetOutput {
        assets,
        level,
        digest: digest_hex(digest_input.as_bytes()),
    })
}

fn budget_verb(_server: &McpServer, args: &BudgetArgs) -> Result<BudgetOutput, McpToolError> {
    budget(args)
}

crate::e2_tool!(
    /// `asset.budget`: the per-asset cost row and an optional level total.
    BudgetTool,
    name = TOOL_ASSET_BUDGET,
    description = "Price the cooked mesh data: triangles per LOD, texture bytes, hull count and a LOD0 draw estimate for one GLB, or a whole level from a placement JSON (each distinct asset cooked once, multiplied by its placements). Same input, same rows.",
    readonly = true,
    args = BudgetArgs,
    output = BudgetOutput,
    invoke = budget_verb
);

#[cfg(test)]
mod tests {
    use std::path::Path as StdPath;

    use super::*;
    use crate::mcp::McpServer;
    use engine_editor::capability::Capability;
    use rmcp::handler::server::router::tool::SyncTool;

    /// A workspace-relative path (tests run with the crate folder as the base).
    fn root(relative: &str) -> String {
        StdPath::new(env!("CARGO_MANIFEST_DIR"))
            .join(relative)
            .display()
            .to_string()
    }

    fn crate_glb() -> String {
        root("../../content/generated/props/world_crate.glb")
    }

    fn server() -> McpServer {
        McpServer::new(11, Capability::unrestricted(11))
    }

    #[test]
    fn a_lod_over_its_target_is_a_warning_row_not_a_silent_pass() {
        let mut warnings = Vec::new();
        let rows = budget_rows(&[100, 90, 10], &[1.0, 0.5, 0.25], &mut warnings);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].budget, 100);
        assert!(!rows[0].over_budget, "LOD0 equals its own budget");
        assert_eq!(rows[1].budget, 50);
        assert!(
            rows[1].over_budget,
            "90 triangles over a 50 triangle budget"
        );
        assert_eq!(warnings.len(), 1, "one warning per miss: {warnings:?}");
        assert!(warnings[0].contains("LOD1") && warnings[0].contains("over its 50 target"));
        // A level inside its budget stays quiet.
        let mut quiet = Vec::new();
        let clean = budget_rows(&[100, 50, 25], &[1.0, 0.5, 0.25], &mut quiet);
        assert!(clean.iter().all(|row| !row.over_budget));
        assert!(quiet.is_empty());
        // A LOD0 over the prop constant names the constant.
        let mut heavy = Vec::new();
        budget_rows(&[6000, 3000], &[1.0, 0.5], &mut heavy);
        assert!(
            heavy.iter().any(|w| w.contains("PROP_MAX_TRIS (5000)")),
            "{heavy:?}"
        );
    }

    #[test]
    fn cook_chains_the_whole_mesh_chain_and_two_cooks_match() {
        let server = server();
        let out = StdPath::new(env!("CARGO_MANIFEST_DIR")).join("../../target/asset-cook-test");
        let args = CookArgs {
            name: "world_crate".to_owned(),
            glb_path: crate_glb(),
            options: CookOptions {
                uv: UvMode::Unwrap,
                lod: vec![1.0, 0.5],
                collision: CollisionMode::Hull,
                bake: BakeMode::Normal,
                atlas: Some(512),
            },
            out_dir: Some(out.display().to_string()),
        };
        let first = CookTool::invoke(&server, args.clone()).expect("cooks");
        assert_eq!(first.asset, "world_crate");
        assert!(
            first.identical_twice,
            "two cooks in one call are byte-equal"
        );
        assert_eq!(first.lod.len(), 2);
        assert!(first.lod[0].triangles > 0);
        assert!(first.hulls >= 1, "one hull per primitive");
        assert_eq!(
            first.overlapping_chart_pairs, 0,
            "an unwrap leaves no overlap"
        );
        assert!(first.map_bytes > 0, "the normal map rides the bundle");
        assert!(first.draw_calls >= 1);
        assert_eq!(first.bundle_digest.len(), 16);
        assert!(first.bundle_bytes > 0);
        let bytes = std::fs::read(&first.bundle_path).expect("bundle written");
        assert_eq!(wide(bytes.len()), first.bundle_bytes);
        verify_bundle_bytes(&bytes, false).expect("the bundle verifies");
        assert!(
            first
                .budget_constants
                .iter()
                .any(|name| name.contains("PROP_MAX_TRIS")),
            "{:?}",
            first.budget_constants
        );
        // The same call again is the same bundle digest.
        let second = CookTool::invoke(&server, args).expect("cooks again");
        assert_eq!(second.bundle_digest, first.bundle_digest);
        // A bad option names the legal values.
        let refused = CookTool::invoke(
            &server,
            CookArgs {
                name: "Bad Name".to_owned(),
                glb_path: crate_glb(),
                options: CookOptions::default(),
                out_dir: None,
            },
        )
        .expect_err("refused");
        assert_eq!(refused.code, "bad_input");
        assert!(refused.message.contains("lower_snake"));
    }

    #[test]
    fn cook_keeps_uvs_and_reports_the_overlap_it_finds() {
        let server = server();
        let out = StdPath::new(env!("CARGO_MANIFEST_DIR")).join("../../target/asset-cook-test");
        let cooked = CookTool::invoke(
            &server,
            CookArgs {
                name: "crate_keep".to_owned(),
                glb_path: crate_glb(),
                options: CookOptions {
                    uv: UvMode::Keep,
                    lod: vec![1.0],
                    collision: CollisionMode::None,
                    bake: BakeMode::None,
                    atlas: Some(512),
                },
                out_dir: Some(out.display().to_string()),
            },
        )
        .expect("cooks");
        assert_eq!(cooked.hulls, 0, "collision none stages no hull");
        assert_eq!(cooked.map_bytes, 0, "bake none writes no map");
        assert_eq!(cooked.draw_calls, 3, "the fixture carries three primitives");
    }

    #[test]
    fn lint_fails_a_bad_import_with_codes_and_fixes_then_passes_a_clean_wall() {
        let server = server();
        let bad = LintTool::invoke(
            &server,
            LintArgs {
                glb_path: crate_glb(),
                name: Some("World Crate".to_owned()),
                unit: Some("m".to_owned()),
                expected_size: Some(1.0),
                max_tris: None,
            },
        )
        .expect("lints");
        // A name with a space is not lower_snake and the fixture carries no COL_ node.
        assert!(bad.codes.contains(&"L007".to_owned()), "{:?}", bad.codes);
        assert!(bad.codes.contains(&"L005".to_owned()), "{:?}", bad.codes);
        assert!(!bad.passed);
        assert!(bad.summary.starts_with("fail:"), "{}", bad.summary);
        for finding in &bad.findings {
            assert!(!finding.fix.is_empty(), "{} names a fix", finding.code);
        }
        let ordered: Vec<String> = {
            let mut codes = bad.codes.clone();
            codes.sort();
            codes
        };
        assert_eq!(bad.codes, ordered, "findings come in code order");
        // The cooked siege wall passes every check at its real size.
        let clean = LintTool::invoke(
            &server,
            LintArgs {
                glb_path: root("../../content/maps/cooked/siege_wall.glb"),
                name: Some("siege_wall".to_owned()),
                unit: Some("m".to_owned()),
                expected_size: Some(8.0),
                max_tris: Some(5_000),
            },
        )
        .expect("lints");
        assert!(clean.passed, "findings {:?}", clean.findings);
        assert!(clean.summary.starts_with("pass:"));
    }

    /// The card's number: siege_wall is 1920 triangles at LOD0 in `content/maps/report.json`.
    #[test]
    fn budget_matches_the_committed_map_report() {
        let server = server();
        let glb = root("../../content/maps/cooked/siege_wall.glb");
        let row = BudgetTool::invoke(
            &server,
            BudgetArgs {
                glb_path: Some(glb.clone()),
                placement_json: None,
                asset_dir: None,
                map_name: None,
            },
        )
        .expect("prices");
        assert_eq!(row.assets.len(), 1);
        let cost = &row.assets[0];
        assert_eq!(cost.asset, "siege_wall");
        assert_eq!(cost.lod_tris.len(), MAX_LOD_LEVELS);
        assert!(cost.draw_calls >= 1);
        assert!(cost.hulls >= 1);
        // The committed report is the fixture of record; compare against it when present.
        let report_path = root("../../content/maps/report.json");
        let text = std::fs::read_to_string(&report_path).expect("report.json is committed");
        let report: Value = rmcp::serde_json::from_str(&text).expect("report.json parses");
        let recorded = report
            .get("assets")
            .and_then(Value::as_array)
            .and_then(|rows| {
                rows.iter()
                    .find(|row| row.get("asset").and_then(Value::as_str) == Some("siege_wall"))
            })
            .and_then(|row| row.get("lod0_tris"))
            .and_then(Value::as_u64)
            .expect("report.json names siege_wall lod0_tris");
        assert_eq!(
            u64::from(cost.triangles),
            recorded,
            "LOD0 matches the report"
        );
        assert_eq!(recorded, 1920, "the card's number");
        // Same input, same rows and the same digest twice.
        let again = BudgetTool::invoke(
            &server,
            BudgetArgs {
                glb_path: Some(glb),
                placement_json: None,
                asset_dir: None,
                map_name: None,
            },
        )
        .expect("prices again");
        assert_eq!(again, row);
    }

    #[test]
    fn budget_level_total_multiplies_placements() {
        let server = server();
        let dir = StdPath::new(env!("CARGO_MANIFEST_DIR")).join("../../target/asset-budget-test");
        std::fs::create_dir_all(&dir).expect("folder");
        let placement = dir.join("placement_test.json");
        let text =
            "{\"actors\":[{\"asset\":\"siege_wall\",\"x\":0},{\"asset\":\"siege_wall\",\"x\":8}]}";
        std::fs::write(&placement, text).expect("placement written");
        let out = BudgetTool::invoke(
            &server,
            BudgetArgs {
                glb_path: None,
                placement_json: Some(placement.display().to_string()),
                asset_dir: Some(root("../../content/maps/cooked")),
                map_name: Some("test".to_owned()),
            },
        )
        .expect("prices");
        let level = out.level.expect("a level total");
        assert_eq!(level.map, "test");
        assert_eq!(level.placements, 2);
        assert_eq!(out.assets.len(), 1, "one row per distinct asset");
        let cost = &out.assets[0];
        assert_eq!(level.lod0_tris, u64::from(cost.triangles) * 2);
        assert_eq!(level.draw_calls, u64::from(cost.draw_calls) * 2);
        assert_eq!(level.hulls, u64::from(cost.hulls) * 2);
        let refused = BudgetTool::invoke(
            &server,
            BudgetArgs {
                glb_path: Some(crate_glb()),
                placement_json: Some(placement.display().to_string()),
                asset_dir: None,
                map_name: None,
            },
        )
        .expect_err("refused");
        assert!(refused.message.contains("not both"), "{}", refused.message);
    }
}
