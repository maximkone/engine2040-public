//! arena_kit: arena glTF kit from placement into the presenter.
//!
//! Consumer: the `arena` twin (`smoke_presenter`, `frame_list`, the live
//! window) draws the kit instead of the six dressing cubes. Data owner: the
//! `kit_load` child module owns the placement parse; this module owns the
//! AABB fit and the per-actor upload;
//! the glTF files own the source meshes. Units: meters, radians (yaw about
//! +Y). Determinism: the same files give the same keys and meshes; pixels
//! never enter the sim hash. Errors: typed [`KitError`], never panics.
//! Budget: 11 draws staged once at init; zero per-tick work, so the frame
//! cost stays flat over a match.

use engine_arena::world::{MatchWorld, Obstacle};
use engine_core::hash::hash_bytes;
use engine_render::present::MeshKey;
#[cfg(feature = "app")]
use engine_render::present::{TextureKey, TextureUpload};
#[cfg(feature = "app")]
use sha2::{Digest, Sha256};
#[cfg(feature = "app")]
use std::path::{Path, PathBuf};

/// Placement table loading (parse `placement.json` into actors): the
/// `kit_load` child module owns it, re-exported here so the twin and the
/// window keep calling `arena_kit::` paths.
#[path = "kit_load.rs"]
mod kit_load;
pub use kit_load::{
    kit_actors, kit_actors_from_json, KitActor, KitAsset, KIT_ACTOR_COUNT, KIT_JSON,
};
/// Distinct kit files staged (one import per file).
pub const KIT_ASSET_COUNT: usize = 8;
/// Minimum source triangles over the staged kit files (the shipping kit
/// stages well above this; an import below it is incomplete, never staged).
///
/// Caller: crates/game/src/arena_kit.rs `import_kit` refuses imports below
/// this floor.
pub const KIT_TRIS_FLOOR: u64 = 5_000;
/// Returns true when `triangles` meet [`KIT_TRIS_FLOOR`].
///
/// Pure integer check, no allocation. Same input gives the same bool.
///
/// Caller: crates/game/src/arena_kit.rs `import_kit` gates the summed source
/// triangles through this check.
#[must_use]
pub fn kit_meets_tris_floor(triangles: u64) -> bool {
    triangles >= KIT_TRIS_FLOOR
}
/// Mesh id base for kit draws (the central cube keeps id 1, dressing keeps
/// its own base).
pub const KIT_MESH_ID_BASE: u64 = 0x4717_0000;
/// Texture id base for kit base-color images, plus the file index (one
/// texture per distinct kit file, clear of mesh ids and ring looks).
#[cfg(feature = "app")]
pub const KIT_TEXTURE_ID_BASE: u64 = 0x4717_1000;
/// Bound for diagnostic strings carried in errors.
pub const MAX_KIT_DIAG: usize = 256;
/// Largest placement half-size accepted in meters (the sand floor is 30 m).
pub const MAX_KIT_SIZE_M: f32 = 100.0;
/// Largest placement coordinate accepted in meters.
pub const MAX_KIT_POS_M: f32 = 500.0;

/// Why kit staging failed; every malformed input maps to one variant.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum KitError {
    /// Placement JSON has the wrong shape; detail bounded.
    Parse(String),
    /// A kit file cannot be read; file name plus bounded reason.
    Io(String),
    /// A kit file fails glTF import; file name plus bounded reason.
    Import(String),
    /// A mesh cannot be fitted to its placement box; bounded reason.
    Fit(String),
}

impl core::fmt::Display for KitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Parse(detail) => write!(f, "kit placement: {detail}"),
            Self::Io(detail) => write!(f, "kit file: {detail}"),
            Self::Import(detail) => write!(f, "kit import: {detail}"),
            Self::Fit(detail) => write!(f, "kit fit: {detail}"),
        }
    }
}

impl std::error::Error for KitError {}

/// Bound a diagnostic string so errors stay small and printable.
fn bound_diag(text: &str) -> String {
    text.chars().take(MAX_KIT_DIAG).collect()
}

fn parse_err(text: &str) -> KitError {
    KitError::Parse(bound_diag(text))
}

/// Buffer length as `u64` without truncation; a merged kit mesh that would
/// not fit in `u32` indices fails loudly instead of wrapping.
#[cfg(feature = "app")]
fn len_u64(len: usize, file: &'static str) -> Result<u64, KitError> {
    let words = u32::try_from(len).map_err(|_| KitError::Import(bound_diag(file)))?;
    Ok(u64::from(words))
}

/// Merged mesh buffers plus source triangles: positions, normals, uvs,
/// rebased indices, triangle count.
#[cfg(feature = "app")]
type MergedBuffers = (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>, u64);

/// One staged source file: name, merged buffers, first material, decoded
/// base-color texture (`None` when the file has no base-color image bytes
/// or they do not decode; the actors then stage untextured).
#[cfg(feature = "app")]
type KitSource = (
    &'static str,
    Vec<[f32; 3]>,
    Vec<[f32; 3]>,
    Vec<[f32; 2]>,
    Vec<u32>,
    engine_assets::cooked::CookedMaterial,
    Option<(TextureKey, TextureUpload)>,
);

/// Placement loading lives in the `kit_load` child module (re-exported above).

/// Floor-plane obstacles for the physical kit actors in slice order.
///
/// One [`Obstacle`] per actor with `physical` set: center `[x, z]`, half
/// `[sx / 2, sz / 2]`, axis `[1, 0]`. The embedded placement is
/// axis-aligned (every `yaw` is `0`), so yaw stays presentation-only and the
/// sim never calls `sin` or `cos` (mirrors `world_load::match_world` sitting
/// on [`MatchWorld::try_new`]).
fn kit_obstacles(actors: &[KitActor]) -> Vec<Obstacle> {
    let mut out = Vec::with_capacity(actors.len());
    for actor in actors {
        if !actor.physical {
            continue;
        }
        out.push(Obstacle {
            center: [actor.pos_m[0], actor.pos_m[2]],
            half: [actor.size_m[0] * 0.5, actor.size_m[2] * 0.5],
            axis: [1.0, 0.0],
        });
    }
    out
}

/// Collision world for arbitrary kit actors, following the
/// `crates/endpoint/src/play/world_load.rs:108` pattern (`blocks` to
/// `Obstacle` to [`MatchWorld::try_new`]).
///
/// Refuses nonzero-yaw physical actors and any bad box loudly so a picture
/// never hides a missing wall.
fn kit_match_world_for_actors(actors: &[KitActor]) -> Result<MatchWorld, KitError> {
    for actor in actors {
        if actor.physical && actor.yaw != 0.0 {
            return Err(parse_err("kit actor yaw must be 0 for collision"));
        }
    }
    let obstacles = kit_obstacles(actors);
    MatchWorld::try_new(obstacles).map_err(|error| KitError::Parse(bound_diag(&error.to_string())))
}

/// Match collision world for the embedded placement: the 11 physical actors
/// as floor boxes so walls stop being pictures.
///
/// Consumer: the arena session that plays the staged kit (a later caller
/// reads this world into the spine step); data owner stays this module
/// (placement parse). Units: meters. Determinism: same bytes give the same
/// sorted world; insertion order never moves the hash. Errors:
/// [`KitError::Parse`] when the placement or any box is bad. Budget: 11
/// boxes built on demand at init; zero per tick.
///
/// Caller: [`kit_prop_rows`] (same file) keeps the brief in lockstep with
/// collision (see its body).
pub fn kit_match_world() -> Result<MatchWorld, KitError> {
    let actors = kit_actors()?;
    kit_match_world_for_actors(&actors)
}

/// One view row per physical staged kit draw: kind plus bounds in
/// `kit_actors` order, skipping non-physical dressing.
///
/// Kind is the actor asset file name; bounds are the placement box
/// (`pos_m +/- size_m / 2`) passed through [`aabb_of`] so the pair keeps
/// `[min, max]` order. Pure reader: embedded placement only, no presenter,
/// no file IO. View only; feeds the brief text, never the sim hash.
///
/// Consumer: the `--view-brief` prop list. Units: meters. Errors: none; an
/// unparsable placement or a bad box reads as zero rows, never a panic.
/// Budget: one entry per physical actor, built on demand for the brief;
/// heap on the presentation path only, zero per tick.
#[must_use]
pub fn kit_prop_rows() -> Vec<(&'static str, [[f32; 3]; 2])> {
    let actors = match kit_actors() {
        Ok(actors) => actors,
        Err(_) => return Vec::new(),
    };
    // Keep view and sim in lockstep: when the placement cannot collide, show
    // no props so a picture never hides a missing wall. Production caller
    // wiring for `kit_match_world` in this file (cold brief path only).
    if kit_match_world().is_err() {
        return Vec::new();
    }
    let mut rows = Vec::with_capacity(actors.len());
    for actor in &actors {
        if !actor.physical {
            continue;
        }
        let min = [
            actor.pos_m[0] - actor.size_m[0] / 2.0,
            actor.pos_m[1] - actor.size_m[1] / 2.0,
            actor.pos_m[2] - actor.size_m[2] / 2.0,
        ];
        let max = [
            actor.pos_m[0] + actor.size_m[0] / 2.0,
            actor.pos_m[1] + actor.size_m[1] / 2.0,
            actor.pos_m[2] + actor.size_m[2] / 2.0,
        ];
        let bounds = match aabb_of(&[min, max]) {
            Ok(bounds) => bounds,
            Err(_) => continue,
        };
        rows.push((actor.asset.file_name(), bounds));
    }
    rows
}

/// Distinct kit files over the actors in first-seen order.
#[must_use]
pub fn kit_asset_files(actors: &[KitActor]) -> Vec<&'static str> {
    let mut files = Vec::new();
    for actor in actors {
        let file = actor.asset.file_name();
        if !files.contains(&file) {
            files.push(file);
        }
    }
    files
}

/// Stable mesh key per kit actor: never collides with the central cube or
/// the dressing boxes (distinct id base plus the asset digest).
#[must_use]
pub fn kit_mesh_key(actor: &KitActor) -> MeshKey {
    let digest64 = hash_bytes(actor.asset.file_name().as_bytes());
    let mut digest = [0u8; 32];
    digest[..8].copy_from_slice(&digest64.to_le_bytes());
    digest[8..12].copy_from_slice(&actor.id.to_le_bytes());
    digest[12..16].copy_from_slice(&actor.id.to_be_bytes());
    MeshKey::new(KIT_MESH_ID_BASE + u64::from(actor.id), digest)
}

/// Fit of one source mesh into its placement box: uniform scale on the
/// largest axis, yaw about +Y, then translation grounding the mesh on the
/// box bottom and centering it on the box XZ center.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FitParams {
    /// Uniform scale factor (largest source axis fills largest box axis).
    pub scale: f32,
    /// Yaw about +Y in radians (copied from the actor).
    pub yaw: f32,
    /// Translation applied after scale plus yaw.
    pub offset: [f32; 3],
}

/// AABB of a position slice; empty or non-finite input fails loudly.
pub fn aabb_of(positions: &[[f32; 3]]) -> Result<[[f32; 3]; 2], KitError> {
    let first = positions
        .first()
        .ok_or_else(|| KitError::Fit("empty mesh".to_string()))?;
    if !first.iter().all(|c| c.is_finite()) {
        return Err(KitError::Fit("non-finite vertex".to_string()));
    }
    let mut min = *first;
    let mut max = *first;
    for point in positions.iter().skip(1) {
        if !point.iter().all(|c| c.is_finite()) {
            return Err(KitError::Fit("non-finite vertex".to_string()));
        }
        for axis in 0..3 {
            if point[axis] < min[axis] {
                min[axis] = point[axis];
            }
            if point[axis] > max[axis] {
                max[axis] = point[axis];
            }
        }
    }
    Ok([min, max])
}

/// Rotate one vector about +Y by `yaw` radians.
fn yaw_rotate(vector: [f32; 3], yaw: f32) -> [f32; 3] {
    let (sin, cos) = yaw.sin_cos();
    [
        vector[0] * cos + vector[2] * sin,
        vector[1],
        -vector[0] * sin + vector[2] * cos,
    ]
}

/// Fit parameters for one source AABB into one actor box. Degenerate source
/// axes (under a micron) fail loudly instead of dividing by zero.
pub fn fit_params(
    src_min: [f32; 3],
    src_max: [f32; 3],
    actor: &KitActor,
) -> Result<FitParams, KitError> {
    let mut src_size = [0.0f32; 3];
    for axis in 0..3 {
        if !src_min[axis].is_finite() || !src_max[axis].is_finite() {
            return Err(KitError::Fit("non-finite source bounds".to_string()));
        }
        src_size[axis] = src_max[axis] - src_min[axis];
        if src_size[axis] < 1e-6 {
            return Err(KitError::Fit("degenerate source axis".to_string()));
        }
    }
    let src_largest = src_size[0].max(src_size[1]).max(src_size[2]);
    let box_largest = actor.size_m[0].max(actor.size_m[1]).max(actor.size_m[2]);
    let scale = box_largest / src_largest;
    // Rotated corners of the scaled box: the offset grounds the rotated
    // mesh on the box bottom and centers it on the box XZ center.
    let mut rmin = [f32::INFINITY; 3];
    let mut rmax = [f32::NEG_INFINITY; 3];
    for corner in 0..8 {
        let point = [
            if corner & 1 == 0 {
                src_min[0]
            } else {
                src_max[0]
            },
            if corner & 2 == 0 {
                src_min[1]
            } else {
                src_max[1]
            },
            if corner & 4 == 0 {
                src_min[2]
            } else {
                src_max[2]
            },
        ];
        let scaled = [point[0] * scale, point[1] * scale, point[2] * scale];
        let rotated = yaw_rotate(scaled, actor.yaw);
        for axis in 0..3 {
            if rotated[axis] < rmin[axis] {
                rmin[axis] = rotated[axis];
            }
            if rotated[axis] > rmax[axis] {
                rmax[axis] = rotated[axis];
            }
        }
    }
    let box_bottom = actor.pos_m[1] - actor.size_m[1] / 2.0;
    let offset = [
        actor.pos_m[0] - (rmin[0] + rmax[0]) / 2.0,
        box_bottom - rmin[1],
        actor.pos_m[2] - (rmin[2] + rmax[2]) / 2.0,
    ];
    Ok(FitParams {
        scale,
        yaw: actor.yaw,
        offset,
    })
}

/// Apply a fit to positions and normals: scale, yaw, translate; normals
/// rotate by yaw and renormalize (a zero normal becomes +Y, never NaN).
#[must_use]
pub fn apply_fit(
    positions: &[[f32; 3]],
    normals: &[[f32; 3]],
    params: &FitParams,
) -> (Vec<[f32; 3]>, Vec<[f32; 3]>) {
    let mut out_pos = Vec::with_capacity(positions.len());
    for point in positions {
        let scaled = [
            point[0] * params.scale,
            point[1] * params.scale,
            point[2] * params.scale,
        ];
        let rotated = yaw_rotate(scaled, params.yaw);
        out_pos.push([
            rotated[0] + params.offset[0],
            rotated[1] + params.offset[1],
            rotated[2] + params.offset[2],
        ]);
    }
    let mut out_nor = Vec::with_capacity(normals.len());
    for normal in normals {
        let rotated = yaw_rotate(*normal, params.yaw);
        let len2 = rotated[0] * rotated[0] + rotated[1] * rotated[1] + rotated[2] * rotated[2];
        if len2 < 1e-12 {
            out_nor.push([0.0, 1.0, 0.0]);
        } else {
            let len = len2.sqrt();
            out_nor.push([rotated[0] / len, rotated[1] / len, rotated[2] / len]);
        }
    }
    (out_pos, out_nor)
}

// ---------------------------------------------------------------------------
// App leg: file import plus presenter upload. Gated on `app` so the headless
// default graph stays std-only (no `gltf` in `cargo tree -p engine-game`).
// ---------------------------------------------------------------------------

/// Kit content dir (`content/arena`), resolved at runtime from the
/// executable's directory so no absolute dev path is baked into the binary.
/// Walks up from the exe (deployed `bin/arena.exe` or dev `target/...`)
/// to the tree holding `content/arena/placement.json`; falls back to the
/// launch directory. Same layout dev and clean room.
#[cfg(feature = "app")]
#[must_use]
pub fn kit_content_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let mut anchor = exe.parent().map(Path::to_path_buf);
        while let Some(dir) = anchor {
            let candidate = dir.join("content/arena");
            if candidate.join("placement.json").is_file() {
                return candidate;
            }
            anchor = dir.parent().map(Path::to_path_buf);
        }
    }
    PathBuf::from("content/arena")
}

/// One baked actor mesh: fitted vertices plus its draw material.
#[cfg(feature = "app")]
pub struct BakedKitMesh {
    /// Placement actor this mesh was fitted to.
    pub actor: KitActor,
    /// Stable presenter key for this actor.
    pub key: MeshKey,
    /// Fitted, validated mesh ready to upload.
    pub mesh: engine_render::v1::MeshData,
    /// PBR factors from the source file (or the fallback when invalid).
    pub material: engine_render::v1::PbrMaterial,
    /// Resident base-color texture key for this actor (`None` draws
    /// untextured against the 1x1 white fallback).
    pub texture: Option<TextureKey>,
}

/// CPU-side kit import: every actor mesh fitted and validated, no GPU yet.
#[cfg(feature = "app")]
pub struct ImportedKit {
    /// One baked mesh per placed actor (11 on the shipping placement).
    pub baked: Vec<BakedKitMesh>,
    /// Source triangles over the distinct kit files (not per-actor copies).
    pub triangles: u64,
    /// Distinct kit files imported.
    pub files: usize,
    /// Actors skipped with the reason (`"import"`, `"fit"` or `"mesh"`).
    pub skipped: Vec<(u32, &'static str)>,
    /// Actors whose material fell back to neutral gray.
    pub material_fallbacks: usize,
    /// One decoded base-color texture per distinct kit file that carries
    /// one, in first-seen file order; [`upload_kit`] uploads each once and
    /// the baked actors reference them by key.
    pub textures: Vec<(TextureKey, TextureUpload)>,
}

/// Neutral-gray fallback factors when a source material is invalid.
#[cfg(feature = "app")]
pub const KIT_FALLBACK_MATERIAL: engine_render::v1::PbrMaterial = engine_render::v1::PbrMaterial {
    base_color: [0.5, 0.5, 0.5],
    metallic: 0.0,
    roughness: 0.9,
    exposure: 1.0,
};

/// Convert cooked PBR factors to draw factors; invalid factors fall back to
/// neutral gray and report `true` so the caller counts it loudly.
#[cfg(feature = "app")]
pub fn kit_material(
    cooked: &engine_assets::cooked::CookedMaterial,
) -> (engine_render::v1::PbrMaterial, bool) {
    let candidate = engine_render::v1::PbrMaterial {
        base_color: [
            cooked.base_color_factor[0],
            cooked.base_color_factor[1],
            cooked.base_color_factor[2],
        ],
        metallic: cooked.metallic_factor,
        roughness: cooked.roughness_factor,
        exposure: 1.0,
    };
    if candidate.validate().is_ok() {
        (candidate, false)
    } else {
        (KIT_FALLBACK_MATERIAL.clone(), true)
    }
}

/// Full SHA-256 digest of image bytes for a texture key (same canon as the
/// fighter look: identical bytes keep the same key across reimports).
#[cfg(feature = "app")]
fn digest32(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Base-color texture for one kit import: the first decodable base-color
/// image over all materials (a file may dress parts in an untextured
/// material first, like the banner cloth), keyed by file index. `None`
/// when the file has no base-color image or the bytes do not decode; the
/// caller stages those actors untextured instead of refusing the kit.
/// Cold path only.
#[cfg(feature = "app")]
fn kit_texture(
    import: &engine_assets::mesh_import::GltfImport,
    file_index: usize,
) -> Option<(TextureKey, TextureUpload)> {
    for material in &import.materials {
        // One file may dress parts in an untextured material: skip those
        // slots instead of bailing on the whole file.
        let Some(asset) = material
            .textures
            .iter()
            .find(|slot| slot.slot == engine_assets::cooked::TextureSlot::BaseColor)
            .map(|slot| slot.asset)
        else {
            continue;
        };
        let Some(bytes) = import.images.get(&asset) else {
            continue;
        };
        if let Ok(upload) = TextureUpload::from_encoded(bytes) {
            let id = KIT_TEXTURE_ID_BASE + u64::try_from(file_index).unwrap_or(u64::MAX);
            return Some((TextureKey::new(id, digest32(bytes)), upload));
        }
    }
    None
}

/// Merge every triangle primitive of one import into parallel arrays with
/// rebased indices; index overflow fails loudly instead of wrapping.
#[cfg(feature = "app")]
fn merge_import(
    file: &'static str,
    import: &engine_assets::mesh_import::GltfImport,
) -> Result<MergedBuffers, KitError> {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();
    let mut triangles: u64 = 0;
    for mesh in &import.meshes {
        let base = len_u64(positions.len(), file)?;
        positions.extend_from_slice(&mesh.positions);
        normals.extend_from_slice(&mesh.normals);
        uvs.extend_from_slice(&mesh.uv0);
        for index in &mesh.indices {
            let rebased = base + u64::from(*index);
            let rebased32 =
                u32::try_from(rebased).map_err(|_| KitError::Import(bound_diag(file)))?;
            indices.push(rebased32);
        }
        triangles += len_u64(mesh.indices.len(), file)? / 3;
    }
    if positions.is_empty() {
        return Err(KitError::Import(bound_diag(file)));
    }
    Ok((positions, normals, uvs, indices, triangles))
}

/// Import plus fit every kit actor mesh from `root/src/<asset>.gltf` (CPU
/// only, no GPU): one import per distinct file, one baked mesh per actor.
/// A bad actor is skipped with its reason and counted; an empty result fails
/// loudly so a missing kit never renders as nothing.
#[cfg(feature = "app")]
pub fn import_kit(root: &Path) -> Result<ImportedKit, KitError> {
    let actors = kit_actors()?;
    let files = kit_asset_files(&actors);
    // One merged source mesh plus its first material per distinct file.
    let mut sources: Vec<KitSource> = Vec::with_capacity(files.len());
    let mut triangles: u64 = 0;
    for (file_index, file) in files.iter().enumerate() {
        let path = root.join("src").join(file);
        let bytes = std::fs::read(&path)
            .map_err(|err| KitError::Io(bound_diag(&format!("{file}: {err}"))))?;
        let import = engine_assets::mesh_import::import_gltf_path(file, &bytes)
            .map_err(|err| KitError::Import(bound_diag(&format!("{file}: {err}"))))?;
        let (pos, nor, uv, idx, tris) = merge_import(file, &import)?;
        triangles += tris;
        let material = import.materials.first().cloned().unwrap_or_default();
        let texture = kit_texture(&import, file_index);
        sources.push((file, pos, nor, uv, idx, material, texture));
    }
    let mut baked = Vec::with_capacity(actors.len());
    let mut skipped = Vec::new();
    let mut material_fallbacks = 0usize;
    for actor in &actors {
        let Some(source) = sources
            .iter()
            .find(|entry| entry.0 == actor.asset.file_name())
        else {
            skipped.push((actor.id, "import"));
            continue;
        };
        let bounds = match aabb_of(&source.1) {
            Ok(bounds) => bounds,
            Err(_) => {
                skipped.push((actor.id, "fit"));
                continue;
            }
        };
        let params = match fit_params(bounds[0], bounds[1], actor) {
            Ok(params) => params,
            Err(_) => {
                skipped.push((actor.id, "fit"));
                continue;
            }
        };
        let (fit_pos, fit_nor) = apply_fit(&source.1, &source.2, &params);
        let mesh = match engine_render::v1::upload_mesh(&fit_pos, &fit_nor, &source.3, &source.4) {
            Ok(mesh) => mesh,
            Err(_) => {
                skipped.push((actor.id, "mesh"));
                continue;
            }
        };
        // First material of the file dresses all its actors.
        let source_material = sources
            .iter()
            .find(|entry| entry.0 == actor.asset.file_name())
            .map(|entry| entry.5.clone());
        let Some(source_material) = source_material else {
            skipped.push((actor.id, "import"));
            continue;
        };
        let (material, fallback) = kit_material(&source_material);
        if fallback {
            material_fallbacks += 1;
        }
        // The file's base-color texture dresses all its actors; `None`
        // stages this actor untextured.
        let texture = source.6.as_ref().map(|(key, _)| *key);
        baked.push(BakedKitMesh {
            actor: *actor,
            key: kit_mesh_key(actor),
            mesh,
            material,
            texture,
        });
    }
    if !kit_meets_tris_floor(triangles) {
        return Err(KitError::Import(bound_diag("kit triangles below floor")));
    }
    if baked.is_empty() {
        return Err(KitError::Import("no kit actor baked".to_string()));
    }
    // One decoded upload per distinct file with base-color bytes.
    let textures: Vec<(TextureKey, TextureUpload)> =
        sources.iter().filter_map(|entry| entry.6.clone()).collect();
    Ok(ImportedKit {
        baked,
        triangles,
        files: files.len(),
        skipped,
        material_fallbacks,
        textures,
    })
}

/// One uploaded kit draw: resident key plus its material.
#[cfg(feature = "app")]
#[derive(Clone, PartialEq, Debug)]
pub struct KitDraw {
    /// Resident mesh key for one actor.
    pub key: MeshKey,
    /// PBR factors for this draw.
    pub material: engine_render::v1::PbrMaterial,
    /// Resident base-color texture key (`None` draws untextured against
    /// the 1x1 white fallback, so factors render unmodified).
    pub base_color: Option<TextureKey>,
}

/// Upload report for one kit staging.
#[cfg(feature = "app")]
#[derive(Clone, PartialEq, Debug, Default)]
pub struct KitReport {
    /// Uploaded draws in placement order.
    pub draws: Vec<KitDraw>,
    /// Distinct kit files staged (8 on the shipping placement).
    pub assets_staged: usize,
    /// Source triangles over the staged files.
    pub triangles: u64,
    /// Actors skipped at import plus uploads refused by the presenter.
    pub skipped: usize,
    /// Draws on fallback (neutral gray) materials.
    pub material_fallbacks: usize,
}

#[cfg(feature = "app")]
impl KitReport {
    /// Draw count pushed when every draw is resident.
    #[must_use]
    pub fn draw_count(&self) -> usize {
        self.draws.len()
    }
}

/// Upload an imported kit into the presenter. Cold init path only; a refused
/// upload skips that draw and counts it, so one bad actor never blanks the
/// frame.
#[cfg(feature = "app")]
pub fn upload_kit(
    presenter: &mut engine_render::present::Presenter,
    kit: &ImportedKit,
) -> KitReport {
    let mut draws = Vec::with_capacity(kit.baked.len());
    let mut refused = 0usize;
    // One texture upload per distinct file; a refused texture drops its
    // actors back to untextured draws instead of failing the frame.
    let mut tex_ok: Vec<TextureKey> = Vec::with_capacity(kit.textures.len());
    for (key, upload) in &kit.textures {
        if presenter.upload_texture(*key, upload).is_ok() {
            tex_ok.push(*key);
        }
    }
    for baked in &kit.baked {
        if presenter.upload_mesh(baked.key, &baked.mesh).is_ok() {
            let base_color = baked.texture.filter(|key| tex_ok.contains(key));
            draws.push(KitDraw {
                key: baked.key,
                material: baked.material.clone(),
                base_color,
            });
        } else {
            refused += 1;
        }
    }
    KitReport {
        draws,
        assets_staged: kit.files,
        triangles: kit.triangles,
        skipped: kit.skipped.len() + refused,
        material_fallbacks: kit.material_fallbacks,
    }
}

/// Import from `root` and upload into the presenter in one call (the twin
/// staging path).
#[cfg(feature = "app")]
pub fn stage_kit(
    presenter: &mut engine_render::present::Presenter,
    root: &Path,
) -> Result<KitReport, KitError> {
    let kit = import_kit(root)?;
    Ok(upload_kit(presenter, &kit))
}

/// Stage the content-dir kit, drawing the cube only when staging is refused
/// (twin fallback path); never fails, never panics.
#[cfg(feature = "app")]
pub fn stage_kit_or_default(presenter: &mut engine_render::present::Presenter) -> KitReport {
    stage_kit(presenter, &kit_content_dir()).unwrap_or_default()
}

/// One-line window log notice for a staged kit (`"arena: staged N kit
/// draws (M skipped)"`).
#[cfg(feature = "app")]
#[must_use]
pub fn live_kit_notice(report: &KitReport) -> String {
    format!(
        "arena: staged {} kit draws ({} skipped)",
        report.draw_count(),
        report.skipped
    )
}

/// Push one draw per resident kit mesh. Missing keys draw nothing; invalid
/// materials (never emitted by [`import_kit`]) are skipped and counted out.
#[cfg(feature = "app")]
pub fn push_kit_draws(
    list: &mut engine_render::present::DrawList,
    resident: &[MeshKey],
    report: &KitReport,
) -> usize {
    let mut pushed = 0usize;
    for draw in &report.draws {
        if !resident.contains(&draw.key) {
            continue;
        }
        if draw.material.validate().is_err() {
            continue;
        }
        list.items.push(engine_render::present::DrawItem {
            mesh: draw.key,
            material: draw.material.clone(),
            // The staged glTF base-color image when its texture uploaded;
            // `None` binds the 1x1 white fallback so factors render
            // unmodified.
            base_color: draw.base_color,
            normal_map: None,
        });
        pushed += 1;
    }
    pushed
}

#[cfg(test)]
mod tests {
    use super::kit_load::kit_actor_from_block;
    use super::*;

    #[test]
    fn placement_parses_11_actors_over_8_files() {
        let actors = kit_actors().expect("embedded placement parses");
        assert_eq!(actors.len(), KIT_ACTOR_COUNT);
        let files = kit_asset_files(&actors);
        assert_eq!(files.len(), KIT_ASSET_COUNT);
        let mut ids: Vec<u32> = actors.iter().map(|actor| actor.id).collect();
        ids.sort_unstable();
        assert_eq!(ids, (1..=11).collect::<Vec<u32>>());
    }

    #[test]
    fn kit_floor_tracks_5000_bar() {
        assert!(!kit_meets_tris_floor(0));
        assert!(!kit_meets_tris_floor(KIT_TRIS_FLOOR - 1));
        assert!(kit_meets_tris_floor(KIT_TRIS_FLOOR));
        assert!(kit_meets_tris_floor(KIT_TRIS_FLOOR + 1));
        assert_eq!(
            kit_meets_tris_floor(KIT_TRIS_FLOOR),
            kit_meets_tris_floor(KIT_TRIS_FLOOR)
        );
    }

    #[test]
    fn physical_flag_reads_true_for_embedded_placement() {
        let actors = kit_actors().expect("embedded placement parses");
        assert!(
            actors.iter().all(|actor| actor.physical),
            "all {} placed actors collide",
            actors.len()
        );
        let rows = kit_prop_rows();
        assert_eq!(rows.len(), KIT_ACTOR_COUNT);
    }

    #[test]
    fn kit_world_holds_11_colliders() {
        let world = kit_match_world().expect("embedded placement collides");
        assert_eq!(world.obstacles().len(), KIT_ACTOR_COUNT);
        assert!(!world.is_empty());
        assert!(
            world
                .obstacles()
                .iter()
                .any(|o| o.center == [-2.0, -10.0] && o.half == [4.0, 0.5]),
            "north wall blocks: {:?}",
            world.obstacles()
        );
        let again = kit_match_world().expect("world builds twice");
        assert_eq!(world.digest(), again.digest());
    }

    #[test]
    fn kit_world_refuses_bad_box_or_yaw() {
        let mut actors = kit_actors().expect("embedded placement parses");
        actors[0].size_m = [0.0, 1.0, 1.0];
        assert!(kit_match_world_for_actors(&actors).is_err());
        let mut actors = kit_actors().expect("embedded placement parses");
        actors[2].yaw = core::f32::consts::FRAC_PI_2;
        assert!(kit_match_world_for_actors(&actors).is_err());
        let mut actors = kit_actors().expect("embedded placement parses");
        actors[10].physical = false;
        let world = kit_match_world_for_actors(&actors).expect("10 walls still collide");
        assert_eq!(world.obstacles().len(), KIT_ACTOR_COUNT - 1);
    }

    #[test]
    fn kit_rejects_missing_or_bad_physical() {
        let missing = "{\"id\": 1, \"asset\": \"crate\", \"x\": 0, \"y\": 1, \"z\": 0, \"sx\": 1, \"sy\": 1, \"sz\": 1, \"yaw\": 0}";
        assert!(kit_actor_from_block(missing).is_err());
        let bad = "{\"id\": 1, \"asset\": \"crate\", \"x\": 0, \"y\": 1, \"z\": 0, \"sx\": 1, \"sy\": 1, \"sz\": 1, \"yaw\": 0, \"physical\": maybe}";
        assert!(kit_actor_from_block(bad).is_err());
        let off = "{\"id\": 1, \"asset\": \"crate\", \"x\": 0, \"y\": 1, \"z\": 0, \"sx\": 1, \"sy\": 1, \"sz\": 1, \"yaw\": 0, \"physical\": false}";
        let actor = kit_actor_from_block(off).expect("false parses");
        assert!(!actor.physical);
    }

    #[test]
    fn kit_keys_are_stable_and_distinct() {
        let actors = kit_actors().expect("embedded placement parses");
        let first = kit_mesh_key(&actors[0]);
        assert_eq!(first, kit_mesh_key(&actors[0]));
        let keys: Vec<MeshKey> = actors.iter().map(kit_mesh_key).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), KIT_ACTOR_COUNT);
        assert_ne!(keys[0], MeshKey::new(1, [0xA1; 32]));
    }

    #[test]
    fn kit_rejects_shape_drift() {
        assert!(kit_actors_from_json("{\"actors\": []}").is_err());
        assert!(kit_actors_from_json("{\"nope\": 1}").is_err());
        assert!(kit_actors_from_json("").is_err());
    }

    #[test]
    fn kit_rejects_unknown_asset() {
        let text = "{\"actors\": [{\"id\": 1, \"asset\": \"mothership\", \"x\": 0, \"y\": 1, \"z\": 0, \"sx\": 1, \"sy\": 1, \"sz\": 1, \"yaw\": 0, \"physical\": true}]}";
        assert!(kit_actors_from_json(text).is_err());
    }

    #[test]
    fn fit_grounds_and_centers_floor() {
        let actors = kit_actors().expect("embedded placement parses");
        let floor = actors
            .iter()
            .find(|a| a.id == 1)
            .expect("sand floor placed");
        // Unit cube source: exact fit math without any file.
        let params = fit_params([-0.5, -0.5, -0.5], [0.5, 0.5, 0.5], floor)
            .expect("unit cube fits the floor box");
        let box_largest = floor.size_m[0].max(floor.size_m[1]).max(floor.size_m[2]);
        assert!((params.scale - box_largest).abs() < 1e-4);
        let cube_pos: Vec<[f32; 3]> = (-1..=1)
            .flat_map(|x| {
                (-1..=1).flat_map(move |y| {
                    (-1..=1).map(move |z| [x as f32 * 0.5, y as f32 * 0.5, z as f32 * 0.5])
                })
            })
            .collect();
        let cube_nor = vec![[0.0, 1.0, 0.0]; cube_pos.len()];
        let (fit_pos, _) = apply_fit(&cube_pos, &cube_nor, &params);
        let [min, max] = aabb_of(&fit_pos).expect("fitted points bound");
        let box_bottom = floor.pos_m[1] - floor.size_m[1] / 2.0;
        assert!((min[1] - box_bottom).abs() < 1e-3);
        assert!(((min[0] + max[0]) / 2.0 - floor.pos_m[0]).abs() < 1e-3);
        assert!(((min[2] + max[2]) / 2.0 - floor.pos_m[2]).abs() < 1e-3);
        let largest = (max[0] - min[0]).max(max[1] - min[1]).max(max[2] - min[2]);
        assert!((largest - box_largest).abs() / box_largest < 0.01);
    }

    #[test]
    fn fit_yaw_turns_the_box() {
        let mut actor = kit_actors().expect("embedded placement parses")[0];
        actor.yaw = core::f32::consts::FRAC_PI_2;
        actor.size_m = [4.0, 2.0, 4.0];
        // Long source box along X: after a quarter turn it lies along Z.
        let params = fit_params([-2.0, -0.5, -0.5], [2.0, 0.5, 0.5], &actor)
            .expect("long box fits the square floor");
        let corners = [[-2.0, -0.5, -0.5], [2.0, 0.5, 0.5]];
        let normals = [[0.0, 1.0, 0.0]; 2];
        let (fit_pos, _) = apply_fit(&corners, &normals, &params);
        let [min, max] = aabb_of(&fit_pos).expect("fitted points bound");
        assert!((max[2] - min[2]) > (max[0] - min[0]));
    }

    #[test]
    fn fit_rejects_degenerate_source() {
        let actor = kit_actors().expect("embedded placement parses")[0];
        assert!(fit_params([0.0, 0.0, 0.0], [1.0, 1.0, 0.0], &actor).is_err());
        assert!(aabb_of(&[]).is_err());
    }

    #[cfg(feature = "app")]
    #[test]
    fn buffer_lengths_reject_truncation() {
        assert_eq!(len_u64(3, "test.gltf").expect("small len fits"), 3);
        assert_eq!(len_u64(0, "test.gltf").expect("empty len fits"), 0);
        // One past `u32`: loud rejection on 64-bit, unrepresentable on 32-bit.
        let huge = usize::try_from(u64::from(u32::MAX) + 1);
        #[cfg(target_pointer_width = "64")]
        assert!(len_u64(huge.expect("u32::MAX + 1 fits on 64-bit"), "test.gltf").is_err());
        #[cfg(not(target_pointer_width = "64"))]
        assert!(huge.is_err());
    }

    #[cfg(feature = "app")]
    #[test]
    fn live_notice_names_draws_and_skips() {
        let report = KitReport {
            draws: Vec::new(),
            assets_staged: 8,
            triangles: 5_001,
            skipped: 2,
            material_fallbacks: 0,
        };
        assert_eq!(
            live_kit_notice(&report),
            "arena: staged 0 kit draws (2 skipped)"
        );
    }

    #[cfg(feature = "app")]
    #[test]
    fn kit_imports_8_files_over_5000_tris() {
        let kit = import_kit(&kit_content_dir()).expect("kit imports from content");
        assert_eq!(kit.files, KIT_ASSET_COUNT);
        assert!(
            kit.triangles >= 5_000,
            "kit stages {} tris, want at least 5000",
            kit.triangles
        );
        assert_eq!(kit.baked.len(), KIT_ACTOR_COUNT);
        assert!(
            kit.skipped.is_empty(),
            "kit skipped actors: {:?}",
            kit.skipped
        );
        assert_eq!(kit.material_fallbacks, 0);
    }

    // Same bytes give the same meshes: one import per run, keys stable x2.
    #[cfg(feature = "app")]
    #[test]
    fn kit_imports_8_files_over_5000_tris_x2() {
        let root = kit_content_dir();
        let first = import_kit(&root).expect("kit imports from content");
        let second = import_kit(&root).expect("kit imports twice identically");
        let check = |run: &str, kit: &ImportedKit| {
            assert_eq!(kit.files, KIT_ASSET_COUNT, "{} run file count", run);
            assert!(
                kit.triangles >= 5_000,
                "{} run stages {} tris, want at least 5000",
                run,
                kit.triangles
            );
            assert_eq!(kit.baked.len(), KIT_ACTOR_COUNT, "{} run actor count", run);
            assert!(
                kit.skipped.is_empty(),
                "{} run skipped actors: {:?}",
                run,
                kit.skipped
            );
            assert_eq!(kit.material_fallbacks, 0, "{} run fallbacks", run);
        };
        check("first", &first);
        check("second", &second);
        assert_eq!(
            first.triangles, second.triangles,
            "triangle count identical x2"
        );
        let first_keys: Vec<MeshKey> = first.baked.iter().map(|mesh| mesh.key).collect();
        let second_keys: Vec<MeshKey> = second.baked.iter().map(|mesh| mesh.key).collect();
        assert_eq!(first_keys, second_keys, "mesh keys stable x2");
        let mut sorted = first_keys.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), KIT_ACTOR_COUNT, "keys distinct across actors");
    }

    // Every kit file carries a decodable base-color image: one texture per
    // file, every actor keyed, keys stable across reimports.
    #[cfg(feature = "app")]
    #[test]
    fn kit_import_carries_base_color_textures() {
        let root = kit_content_dir();
        let kit = import_kit(&root).expect("kit imports from content");
        assert_eq!(
            kit.textures.len(),
            KIT_ASSET_COUNT,
            "one base-color texture per kit file"
        );
        for baked in &kit.baked {
            assert!(
                baked.texture.is_some(),
                "actor {} draws its file's base color",
                baked.actor.id
            );
        }
        let mut keys: Vec<TextureKey> = kit.textures.iter().map(|(key, _)| *key).collect();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), KIT_ASSET_COUNT, "texture keys distinct");
        for (key, upload) in &kit.textures {
            assert!(
                upload.width >= 4 && upload.height >= 4,
                "texture {key:?} is a real image, not a stub"
            );
        }
    }

    /// Luminance variance over the sand region: the central patch of the
    /// close-up frame, where the floor fills the view and the lighting
    /// gradient is flattest, so albedo grain dominates the textured run.
    #[cfg(feature = "app")]
    fn sand_variance(png: &[u8]) -> f64 {
        let upload = TextureUpload::from_encoded(png).expect("capture is a valid PNG");
        let (w, h) = (upload.width as usize, upload.height as usize);
        let (hw, hh) = (w / 4, h / 4);
        let (x0, y0) = (w / 2 - hw / 2, h / 2 - hh / 2);
        let mut sum = 0.0f64;
        let mut sum2 = 0.0f64;
        let mut count = 0.0f64;
        for y in y0..(y0 + hh) {
            for x in x0..(x0 + hw) {
                let px = &upload.rgba[(y * w + x) * 4..][..3];
                let lum = 0.2126 * f64::from(px[0])
                    + 0.7152 * f64::from(px[1])
                    + 0.0722 * f64::from(px[2]);
                sum += lum;
                sum2 += lum * lum;
                count += 1.0;
            }
        }
        sum2 / count - (sum / count) * (sum / count)
    }

    /// One sand-only frame in close-up over the sand floor: the sand draw
    /// alone, textured or stripped. Fresh presenter per render so the
    /// per-frame particle step cannot drift the comparison. `None` with
    /// no adapter (skip, never fail).
    #[cfg(feature = "app")]
    fn sand_frame(textured: bool) -> Option<Vec<u8>> {
        use engine_render::present::{DrawItem, DrawList, Presenter};
        const WIDTH: u16 = 256;
        const HEIGHT: u16 = 192;
        let wide = u32::from(WIDTH);
        let high = u32::from(HEIGHT);
        let mut presenter = match Presenter::init_offscreen() {
            Ok(presenter) => presenter,
            Err(engine_render::present::PresentError::NoAdapter) => return None,
            Err(err) => panic!("presenter init failed (must fail loudly): {err}"),
        };
        presenter.resize(wide, high).expect("valid size applies");
        let report = stage_kit(&mut presenter, &kit_content_dir()).expect("kit stages");
        let actors = kit_actors().expect("embedded placement parses");
        let sand = actors
            .iter()
            .find(|actor| actor.id == 1)
            .expect("sand floor placed");
        let key = kit_mesh_key(sand);
        assert!(presenter.mesh_keys().contains(&key), "sand mesh resident");
        let draw = report
            .draws
            .iter()
            .find(|draw| draw.key == key)
            .expect("sand draw staged");
        if textured {
            assert!(draw.base_color.is_some(), "sand binds its texture");
        }
        let scene = engine_render::v1::default_scene(wide, high).expect("default scene is valid");
        let mut camera = engine_render::v1::Camera::new(
            [sand.pos_m[0], sand.pos_m[1] + 5.0, sand.pos_m[2]],
            sand.pos_m,
            [0.0, 0.0, -1.0],
            50.0f32.to_radians(),
        );
        camera.aspect = f32::from(WIDTH) / f32::from(HEIGHT);
        camera.validate().expect("close-up camera is valid");
        let list = DrawList {
            camera,
            light: scene.light,
            items: vec![DrawItem {
                mesh: key,
                material: draw.material.clone(),
                base_color: if textured { draw.base_color } else { None },
                normal_map: None,
            }],
        };
        let out = presenter
            .render_offscreen(&list)
            .expect("sand frame renders");
        assert!(out.manifest.is_clean(), "sand frame is validation-clean");
        Some(out.png.expect("sand frame has pixels"))
    }

    // F2P twin: the textured sand region varies at least 4x the untextured
    // run. Skips (passes) with no adapter.
    #[cfg(feature = "app")]
    #[test]
    fn kit_textured_sand_beats_flat_variance_4x() {
        let Some(png_tex) = sand_frame(true) else {
            return;
        };
        let Some(png_flat) = sand_frame(false) else {
            return;
        };
        let var_tex = sand_variance(&png_tex);
        let var_flat = sand_variance(&png_flat);
        eprintln!("VS3 sand variance: textured {var_tex:.1} flat {var_flat:.1}");
        if var_flat == 0.0 {
            assert!(var_tex > 0.0, "textured sand varies");
        } else {
            assert!(
                var_tex >= 4.0 * var_flat,
                "textured sand {var_tex:.1} is 4x flat {var_flat:.1}"
            );
        }
    }
}
