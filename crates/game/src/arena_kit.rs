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
use engine_render::material_kind::MaterialKind;
use engine_render::present::MeshKey;
#[cfg(feature = "app")]
use engine_render::present::{TextureKey, TextureSource, TextureUpload};
#[cfg(feature = "app")]
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

use crate::kit_assets::{kit_logical_path, KitAssets};

/// Placement table loading (parse `placement.json` into actors): the
/// `kit_load` child module owns it, re-exported here so the twin and the
/// window keep calling `arena_kit::` paths.
#[path = "kit_load.rs"]
mod kit_load;
pub use kit_load::{
    kit_actors, kit_actors_from_json, KitActor, KitAsset, KIT_ACTOR_COUNT, KIT_JSON,
};

/// VS7 proof: the frame light list holds every torch actor, and the sand
/// near a torch is far brighter than the far sand. The CPU mirror of the
/// shader's torch falloff, so the numbers are the drawn ones.
#[path = "torch_acceptance.rs"]
pub mod torch_acceptance;
/// VS7: the placement's torch actors become lights. The feed from the
/// placement to the presenter's torch list, with a bounded flicker.
/// Presentation only, never hashed.
#[path = "torch_feed.rs"]
pub mod torch_feed;

/// VR2 material sets (cooked CC0 base, normal and ORM per kit file, tiling
/// UVs, glTF fallback maps): the `kit_pbr` child module, `app` leg only.
#[cfg(feature = "app")]
#[path = "kit_pbr.rs"]
pub mod kit_pbr;

/// GS15: the imported kit's measured mesh cost against the potato caps, with
/// the files and meshes over one. A child module, `app` leg only.
#[cfg(feature = "app")]
#[path = "kit_budget.rs"]
pub mod kit_budget;
#[cfg(feature = "app")]
pub use kit_budget::{KitBudget, KitBudgetFinding};

/// Hand-placed zone table parsing (the design gate's rows): the `kit_zones`
/// child module, re-exported so callers keep `arena_kit::` paths.
#[path = "kit_zones.rs"]
mod kit_zones;
pub use kit_zones::{kit_zone_rows, KitZoneRow, KIT_ZONE_GATE_ID, KIT_ZONE_ROWS};

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
/// Look probe: `E2040_KIT_TEXTURES=off` stages every kit draw flat, so one
/// capture pair can show the textures on and off. Read at import, so the
/// staged report and the window twin list agree; any other value (unset
/// included) keeps the shipped look.
pub const KIT_TEXTURES_ENV: &str = "E2040_KIT_TEXTURES";

/// Whether kit draws bind their textures: false only for `off` after
/// trimming, so a typo never silently drops them.
///
/// Caller: `import_kit_from` reads it once per import, never per frame.
#[must_use]
pub fn kit_textures_enabled(value: Option<&str>) -> bool {
    !value.is_some_and(|value| value.trim().eq_ignore_ascii_case("off"))
}
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
    /// A base-color image a file names is missing or does not decode; never
    /// falls back to a white draw.
    Texture(String),
    /// A mesh cannot be fitted to its placement box; bounded reason.
    Fit(String),
    /// The content patch beside the kit root cannot be read, parsed or
    /// matched against the directory; names the patch path and the reason.
    Patch(String),
}

impl core::fmt::Display for KitError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Parse(detail) => write!(f, "kit placement: {detail}"),
            Self::Io(detail) => write!(f, "kit file: {detail}"),
            Self::Import(detail) => write!(f, "kit import: {detail}"),
            Self::Texture(detail) => write!(f, "kit texture: {detail}"),
            Self::Fit(detail) => write!(f, "kit fit: {detail}"),
            Self::Patch(detail) => write!(f, "kit patch: {detail}"),
        }
    }
}

impl std::error::Error for KitError {}

/// Bound a diagnostic string so errors stay small and printable.
pub(crate) fn bound_diag(text: &str) -> String {
    text.chars().take(MAX_KIT_DIAG).collect()
}

fn parse_err(text: &str) -> KitError {
    KitError::Parse(bound_diag(text))
}

/// Import failure with bounded detail; keeps headless parsing lines short.
fn import_err(text: &str) -> KitError {
    KitError::Import(bound_diag(text))
}

/// Base-color image failure with bounded detail, never a white draw.
#[cfg(feature = "app")]
fn texture_err(text: &str) -> KitError {
    KitError::Texture(bound_diag(text))
}

/// Buffer length as `u64` without truncation; a merged kit mesh that would
/// not fit in `u32` indices fails loudly instead of wrapping.
#[cfg(feature = "app")]
fn len_u64(len: usize, file: &'static str) -> Result<u64, KitError> {
    let words = u32::try_from(len).map_err(|_| import_err(file))?;
    Ok(u64::from(words))
}

/// Merged mesh buffers plus source triangles: positions, normals, uvs,
/// rebased indices, triangle count.
#[cfg(feature = "app")]
type MergedBuffers = (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>, u64);

/// One staged source file: name, merged buffers, first material, base-color
/// texture key (`None` only when the file names no image; a named image that
/// is missing or does not decode refuses), the cooked set it wears and the
/// key of its normal-map set.
#[cfg(feature = "app")]
type KitSource = (
    &'static str,
    Vec<[f32; 3]>,
    Vec<[f32; 3]>,
    Vec<[f32; 2]>,
    Vec<u32>,
    engine_assets::cooked::CookedMaterial,
    Option<TextureKey>,
    Option<kit_pbr::KitPbr>,
    Option<TextureKey>,
);

// Placement loading lives in the `kit_load` child module (re-exported above).

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

/// Match collision world for the embedded placement: every actor whose
/// `physical` flag is set, as floor boxes so walls stop being pictures (the
/// shipped placement collides six: stands, both walls, the gate, crate_a,
/// debris_field; the sand floor and the torches/banners are dressing).
///
/// Consumer: the arena session that plays the staged kit (a later caller
/// reads this world into the spine step); data owner stays this module
/// (placement parse). Units: meters. Determinism: same bytes give the same
/// sorted world; insertion order never moves the hash. Errors:
/// [`KitError::Parse`] when the placement or any box is bad. Budget: one box
/// per physical actor, built on demand at init; zero per tick.
///
/// Caller: [`kit_prop_rows`] (same file) keeps the brief in lockstep with
/// collision (see its body).
pub fn kit_match_world() -> Result<MatchWorld, KitError> {
    let actors = kit_actors()?;
    kit_match_world_for_actors(&actors)
}

/// [`kit_match_world`] with the live session's refusal path: an unbuildable
/// placement maps to [`MatchWorld::empty()`] (the legacy open arena) instead
/// of failing the session, and [`kit_prop_rows`] already reports zero rows
/// for the same placement, so view and sim stay in lockstep.
///
/// Caller: `crates/game/src/windowed.rs` (`run_windowed`) and
/// `crates/game/src/bin/arena.rs` build the live match over this.
/// Units: meters. Determinism: same bytes give the same world; the empty
/// world keeps the legacy checkpoints. Budget: parsed once per call at
/// session load; zero per tick.
#[must_use]
pub fn kit_match_world_or_empty() -> MatchWorld {
    kit_match_world().unwrap_or_else(|_| MatchWorld::empty())
}

/// One view row per staged kit draw: kind plus bounds in `kit_actors` order.
///
/// Kind is the actor asset file name; bounds are the placement box
/// (`pos_m +/- size_m / 2`) passed through [`aabb_of`] so the pair keeps
/// `[min, max]` order. Pure reader: embedded placement only, no presenter,
/// no file IO. View only; feeds the brief text, never the sim hash. The view
/// draws every staged actor, `physical` or not: the flag is the collision
/// gate ([`kit_obstacles`]), never the draw gate.
///
/// Consumer: the `--view-brief` prop list. Units: meters. Errors: none; an
/// unparsable placement or a bad box reads as zero rows, never a panic.
/// Budget: one entry per staged actor, built on demand for the brief;
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

/// LF8: the shading family of one kit mesh. Only the torch (iron) and the
/// banner (cloth) leave the stone default; the sand, walls, stands, gate,
/// crate and debris keep the stone path. `None` means "do not touch".
#[must_use]
pub fn kit_material_kind(asset: KitAsset) -> Option<MaterialKind> {
    match asset {
        KitAsset::Torch => Some(MaterialKind::Metal),
        KitAsset::Banner => Some(MaterialKind::Cloth),
        _ => None,
    }
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
    /// Material-set key (the normal-map key a draw names; `None` draws with
    /// the geometric normal and the material factors alone).
    pub normal_map: Option<TextureKey>,
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
    pub textures: Vec<(TextureKey, TextureSource)>,
    /// Material sets (normal map, ORM, base they sit over): uploaded once by
    /// [`upload_kit`], named by the baked actors' `normal_map` key.
    pub sets: Vec<kit_pbr::KitMaterialSet>,
    /// Cooked sets loaded from `cooked/` (5 on the shipping content). A
    /// source without `cooked/` stages the glTF textures and reports fewer.
    pub cooked_sets: usize,
    /// Measured mesh cost of the imported source files against the potato
    /// caps, with the files and meshes that exceed one.
    pub budget: KitBudget,
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

/// Base-color image one kit import names: the first material slot with
/// [`TextureSlot::BaseColor`](engine_assets::cooked::TextureSlot) over all
/// materials (a file may dress parts in an untextured material first, like
/// the banner cloth). `Ok(None)` when the file names no image. A named image
/// the import does not carry, or bytes that do not decode, refuse by file
/// name instead of staging a white draw. Cold path only.
#[cfg(feature = "app")]
pub(crate) fn kit_texture(
    import: &engine_assets::mesh_import::GltfImport,
    file: &'static str,
    file_index: usize,
) -> Result<Option<(TextureKey, TextureSource)>, KitError> {
    for material in &import.materials {
        let Some(asset) = material
            .textures
            .iter()
            .find(|slot| slot.slot == engine_assets::cooked::TextureSlot::BaseColor)
            .map(|slot| slot.asset)
        else {
            continue;
        };
        let bytes = import.images.get(&asset).ok_or_else(|| {
            texture_err(&format!("{file}: base-color image {asset:#018x} missing"))
        })?;
        let upload = TextureUpload::from_encoded(bytes)
            .map_err(|err| texture_err(&format!("{file}: image does not decode: {err}")))?;
        let id = KIT_TEXTURE_ID_BASE + u64::try_from(file_index).unwrap_or(u64::MAX);
        return Ok(Some((
            TextureKey::new(id, digest32(bytes)),
            TextureSource::Rgba8(upload),
        )));
    }
    Ok(None)
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
            let rebased32 = u32::try_from(rebased).map_err(|_| import_err(file))?;
            indices.push(rebased32);
        }
        triangles += len_u64(mesh.indices.len(), file)? / 3;
    }
    if positions.is_empty() {
        return Err(import_err(file));
    }
    Ok((positions, normals, uvs, indices, triangles))
}

/// Procedural crowd rows and columns on the west stands: 3 tiers by 8 seats.
#[cfg(feature = "app")]
const CROWD_ROWS: usize = 3;
/// Seats per stands tier, spaced along the stands length.
#[cfg(feature = "app")]
const CROWD_COLS: usize = 8;
/// Stone top sits near 3.0 m; heads must clear this bound.
#[cfg(feature = "app")]
const CROWD_MIN_TOP_M: f32 = 3.2;
/// Lift target when the fitted stone runs low, so heads always clear stone.
#[cfg(feature = "app")]
const CROWD_LIFT_TOP_M: f32 = 3.35;

/// True for the west stands actor that carries the procedural crowd.
///
/// Pure over the placement row; the same actor gives the same bool.
/// Caller: `import_kit` in this file merges crowd only for this actor.
#[cfg(feature = "app")]
fn crowd_is_stands(actor: &KitActor) -> bool {
    actor.id == 2 && actor.asset == KitAsset::Stands
}

/// Append one axis-aligned box (24 verts, 36 indices) with outward normals.
///
/// Cold init only; the staging buffers grow here, zero per tick.
/// Caller: `append_stands_crowd` builds bodies plus heads from this box.
#[cfg(feature = "app")]
fn push_crowd_box(
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
    center: [f32; 3],
    size: [f32; 3],
) {
    let hx = size[0] * 0.5;
    let hy = size[1] * 0.5;
    let hz = size[2] * 0.5;
    let cx = center[0];
    let cy = center[1];
    let cz = center[2];
    // Six faces with outward normals plus four corners each, matching the
    // procedural cube winding (culling stays off, so order is informational).
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        (
            [1.0, 0.0, 0.0],
            [
                [cx + hx, cy - hy, cz - hz],
                [cx + hx, cy - hy, cz + hz],
                [cx + hx, cy + hy, cz + hz],
                [cx + hx, cy + hy, cz - hz],
            ],
        ),
        (
            [-1.0, 0.0, 0.0],
            [
                [cx - hx, cy - hy, cz + hz],
                [cx - hx, cy - hy, cz - hz],
                [cx - hx, cy + hy, cz - hz],
                [cx - hx, cy + hy, cz + hz],
            ],
        ),
        (
            [0.0, 1.0, 0.0],
            [
                [cx - hx, cy + hy, cz - hz],
                [cx - hx, cy + hy, cz + hz],
                [cx + hx, cy + hy, cz + hz],
                [cx + hx, cy + hy, cz - hz],
            ],
        ),
        (
            [0.0, -1.0, 0.0],
            [
                [cx - hx, cy - hy, cz + hz],
                [cx - hx, cy - hy, cz - hz],
                [cx + hx, cy - hy, cz - hz],
                [cx + hx, cy - hy, cz + hz],
            ],
        ),
        (
            [0.0, 0.0, 1.0],
            [
                [cx - hx, cy - hy, cz + hz],
                [cx + hx, cy - hy, cz + hz],
                [cx + hx, cy + hy, cz + hz],
                [cx - hx, cy + hy, cz + hz],
            ],
        ),
        (
            [0.0, 0.0, -1.0],
            [
                [cx + hx, cy - hy, cz - hz],
                [cx - hx, cy - hy, cz - hz],
                [cx - hx, cy + hy, cz - hz],
                [cx + hx, cy + hy, cz - hz],
            ],
        ),
    ];
    let base = u32::try_from(positions.len()).unwrap_or(u32::MAX);
    let mut face_base = base;
    for (normal, corners) in &faces {
        for (corner_index, corner) in corners.iter().enumerate() {
            positions.push(*corner);
            normals.push(*normal);
            let uv = match corner_index {
                0 => [0.0, 0.0],
                1 => [1.0, 0.0],
                2 => [1.0, 1.0],
                _ => [0.0, 1.0],
            };
            uvs.push(uv);
        }
        indices.extend_from_slice(&[
            face_base,
            face_base + 1,
            face_base + 2,
            face_base,
            face_base + 2,
            face_base + 3,
        ]);
        face_base += 4;
    }
}

/// Merge 24 spectators (body plus head boxes) into fitted stands buffers.
///
/// Feet rest on tier fractions of the fitted height, rising from the arena
/// side east to the back west; heads clear [`CROWD_MIN_TOP_M`]. Returns
/// false when the fitted mesh has no bounds, staging stone without crowd
/// instead of skipping the actor. Same fitted mesh gives the same crowd;
/// cold init only, zero per tick.
/// Caller: `import_kit` in this file calls this for the stands actor only.
#[cfg(feature = "app")]
fn append_stands_crowd(
    positions: &mut Vec<[f32; 3]>,
    normals: &mut Vec<[f32; 3]>,
    uvs: &mut Vec<[f32; 2]>,
    indices: &mut Vec<u32>,
) -> bool {
    let [min, max] = match aabb_of(positions) {
        Ok(bounds) => bounds,
        Err(_) => return false,
    };
    let dx = max[0] - min[0];
    let dy = max[1] - min[1];
    let dz = max[2] - min[2];
    if !(dx > 1e-6 && dy > 1e-6 && dz > 1e-6) {
        return false;
    }
    let x0 = min[0] + dx * 0.08;
    let x1 = max[0] - dx * 0.08;
    let z0 = min[2] + dz * 0.08;
    let z1 = max[2] - dz * 0.08;
    let span_x = x1 - x0;
    let span_z = z1 - z0;
    if !(span_x > 1e-6 && span_z > 1e-6) {
        return false;
    }
    let stone_len = positions.len();
    let rows_f = f32::from(u8::try_from(CROWD_ROWS).unwrap_or(3));
    let cols_f = f32::from(u8::try_from(CROWD_COLS).unwrap_or(8));
    for row in 0..CROWD_ROWS {
        let row_f = f32::from(u8::try_from(row).unwrap_or(0));
        // East near the arena sits low, west back sits high like the tiers.
        let frac_x = (row_f + 0.5) / rows_f;
        let x = x1 - frac_x * span_x;
        let tread = min[1] + ((row_f + 0.75) / rows_f) * dy;
        for col in 0..CROWD_COLS {
            let col_f = f32::from(u8::try_from(col).unwrap_or(0));
            let frac_z = (col_f + 0.5) / cols_f;
            let z = z0 + frac_z * span_z;
            let variant = u8::try_from((row * CROWD_COLS + col) % 3).unwrap_or(0);
            let body_h = 0.50 + 0.05 * f32::from(variant);
            let body_center = [x, tread + body_h * 0.5, z];
            push_crowd_box(
                positions,
                normals,
                uvs,
                indices,
                body_center,
                [0.34, body_h, 0.24],
            );
            let head_side = 0.20;
            let head_center = [x, tread + body_h + 0.04 + head_side * 0.5, z];
            push_crowd_box(
                positions,
                normals,
                uvs,
                indices,
                head_center,
                [head_side, head_side, head_side],
            );
        }
    }
    // The fitted stone may run low; lift only the crowd so heads clear it.
    // Stone stays where the fit put it, so tiers never move.
    let top = positions
        .iter()
        .skip(stone_len)
        .map(|point| point[1])
        .fold(f32::NEG_INFINITY, f32::max);
    if top <= CROWD_MIN_TOP_M {
        let lift = CROWD_LIFT_TOP_M - top;
        for point in positions.iter_mut().skip(stone_len) {
            point[1] += lift;
        }
    }
    true
}

/// Positions, normals, uvs and indices of one fitted actor.
#[cfg(feature = "app")]
type FittedBuffers = (Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<u32>);

/// Fitted buffers for one actor, with crowd merged on the stands.
///
/// Pure over the source plus fit; the same inputs give the same buffers.
/// Cold init only, zero per tick.
/// Caller: `import_kit` in this file bakes one actor through this helper.
#[cfg(feature = "app")]
fn fitted_buffers(source: &KitSource, actor: &KitActor, params: &FitParams) -> FittedBuffers {
    let (mut pos, mut nor) = apply_fit(&source.1, &source.2, params);
    // A file in a cooked set tiles by meters (box projection of the fitted
    // stone); the crowd boxes appended below keep their own UVs.
    let mut uv = match source.7 {
        Some(pbr) if source.8.is_some() => kit_pbr::box_uvs(&pos, &nor, pbr.tile_m),
        _ => source.3.clone(),
    };
    let mut idx = source.4.clone();
    if crowd_is_stands(actor) {
        // A refused crowd keeps stone without crowd, never skips the actor.
        let _ = append_stands_crowd(&mut pos, &mut nor, &mut uv, &mut idx);
    }
    (pos, nor, uv, idx)
}

/// Import plus fit every kit actor mesh from `root/src/<asset>.gltf` (CPU
/// only, no GPU): one import per distinct file, one baked mesh per actor.
/// A bad actor is skipped with its reason and counted; an empty result fails
/// loudly so a missing kit never renders as nothing.
#[cfg(feature = "app")]
pub fn import_kit(root: &Path) -> Result<ImportedKit, KitError> {
    import_kit_from(&engine_assets::FsSource::new(root))
}

/// [`import_kit`] in a chosen map shape: a frame test imports the same kit as
/// RGBA8 to measure what the block upload changed. The shipping path is
/// [`CookedShape::Block`](kit_pbr::CookedShape::Block), which [`import_kit`]
/// uses.
#[cfg(feature = "app")]
pub fn import_kit_shape(root: &Path, shape: kit_pbr::CookedShape) -> Result<ImportedKit, KitError> {
    import_kit_from_shape(&engine_assets::FsSource::new(root), shape)
}

/// One merged source mesh plus its first material and maps per distinct kit
/// file, with the summed source triangles.
#[cfg(feature = "app")]
fn load_kit_sources(
    source: &dyn engine_assets::AssetSource,
    assets: &KitAssets<'_>,
    files: &[&'static str],
    maps: &mut kit_pbr::KitMaps,
    textures_on: bool,
    shape: kit_pbr::CookedShape,
) -> Result<(Vec<KitSource>, u64, KitBudget), KitError> {
    let mut sources: Vec<KitSource> = Vec::with_capacity(files.len());
    let mut triangles: u64 = 0;
    let mut budget = KitBudget::default();
    for (file_index, file) in files.iter().enumerate() {
        let bytes = assets.bytes(&kit_logical_path(file))?;
        let import = engine_assets::mesh_import::import_gltf_path(file, bytes)
            .map_err(|err| KitError::Import(bound_diag(&format!("{file}: {err}"))))?;
        // The base image decodes once here: the budget reads its pixels and
        // the stage uploads the same upload. The cooked set's base, when the
        // file wears one, is the map the tier draws instead.
        let base = kit_texture(&import, file, file_index)?;
        let cooked_px = kit_pbr::cooked_base_px(source, file);
        let gltf_px = base
            .as_ref()
            .map(|(_, texture)| texture.width().saturating_mul(texture.height()));
        for (mesh, cooked) in import.meshes.iter().enumerate() {
            let names_base = import.materials.get(mesh).is_some_and(|material| {
                material
                    .textures
                    .iter()
                    .any(|slot| slot.slot == engine_assets::cooked::TextureSlot::BaseColor)
            });
            let tex_px = match cooked_px {
                Some(px) => px,
                None if names_base => gltf_px.unwrap_or(0),
                None => 0,
            };
            budget.add(
                file,
                mesh,
                engine_assets::budget::MeshStats {
                    tris: u32::try_from(cooked.indices.len() / 3).unwrap_or(u32::MAX),
                    bones: 0,
                    influences: 0,
                    materials_per_skin: u32::from(import.materials.get(mesh).is_some()),
                    tex_px,
                },
            );
        }
        let (pos, nor, uv, idx, tris) = merge_import(file, &import)?;
        triangles += tris;
        let material = import.materials.first().cloned().unwrap_or_default();
        let (texture, pbr, normal_key) =
            maps.stage(source, file, &import, file_index, base, textures_on, shape)?;
        sources.push((file, pos, nor, uv, idx, material, texture, pbr, normal_key));
    }
    Ok((sources, triangles, budget))
}

/// [`import_kit`] over any content source: the files are read as
/// `src/<asset>.gltf` by logical path, so a browser build hands in a fetched
/// bundle and gets the same kit as the native directory read. The bytes are
/// held by generational handles and unloaded when the import is done.
///
/// Cooked sets load in [`CookedShape::Block`](kit_pbr::CookedShape::Block): the
/// presenter decides whether the GPU takes blocks or decodes to RGBA8.
#[cfg(feature = "app")]
pub fn import_kit_from(source: &dyn engine_assets::AssetSource) -> Result<ImportedKit, KitError> {
    import_kit_from_shape(source, kit_pbr::CookedShape::Block)
}

/// [`import_kit_from`] in a chosen map shape: a frame test imports the same kit
/// as RGBA8 to measure what the block upload changed.
#[cfg(feature = "app")]
pub fn import_kit_from_shape(
    source: &dyn engine_assets::AssetSource,
    shape: kit_pbr::CookedShape,
) -> Result<ImportedKit, KitError> {
    let actors = kit_actors()?;
    let files = kit_asset_files(&actors);
    let mut assets = KitAssets::load(source, &files)?;
    let mut maps = kit_pbr::KitMaps::default();
    let textures_on = kit_textures_enabled(std::env::var(KIT_TEXTURES_ENV).ok().as_deref());
    let (sources, triangles, budget) =
        load_kit_sources(source, &assets, &files, &mut maps, textures_on, shape)?;
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
        // Crowd rides the stands draw, so draws stay at 11 and files at 8.
        let (fit_pos, fit_nor, fit_uv, fit_idx) = fitted_buffers(source, actor, &params);
        let mesh = match engine_render::v1::upload_mesh_tiled(&fit_pos, &fit_nor, &fit_uv, &fit_idx)
        {
            Ok(mesh) => mesh,
            Err(_) => {
                skipped.push((actor.id, "mesh"));
                continue;
            }
        };
        // First material of the file dresses all its actors.
        let (mut material, fallback) = kit_material(&source.5);
        if fallback {
            material_fallbacks += 1;
        }
        // A cooked set carries the look in its maps: factors pass the ORM
        // through (white, rough 1) and only iron turns the metallic channel on.
        if let (Some(wear), Some(_)) = (source.7, source.8) {
            material.base_color = [1.0, 1.0, 1.0];
            material.roughness = 1.0;
            material.metallic = wear.metallic;
        }
        // The file's base-color texture dresses all its actors; `None`
        // stages this actor untextured.
        let texture = source.6;
        baked.push(BakedKitMesh {
            actor: *actor,
            key: kit_mesh_key(actor),
            mesh,
            material,
            texture,
            normal_map: source.8,
        });
    }
    if !kit_meets_tris_floor(triangles) {
        return Err(KitError::Import(bound_diag("kit triangles below floor")));
    }
    if baked.is_empty() {
        return Err(KitError::Import("no kit actor baked".to_string()));
    }
    // The source bytes are baked into meshes and maps by now: the handles go
    // stale and the bytes go back to the queue's budget.
    assets.unload();
    let cooked_sets = maps.cooked_loaded();
    let kit_pbr::KitMaps { textures, sets, .. } = maps;
    Ok(ImportedKit {
        baked,
        triangles,
        files: files.len(),
        skipped,
        material_fallbacks,
        textures,
        sets,
        cooked_sets,
        budget,
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
    /// Resident material-set key (`None` draws with the geometric normal and
    /// the material factors alone).
    pub normal_map: Option<TextureKey>,
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
    /// Measured mesh cost of the imported kit against the potato caps.
    pub budget: KitBudget,
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
        if presenter.upload_texture_source(*key, upload).is_ok() {
            tex_ok.push(*key);
        }
    }
    // One material set per distinct normal map (base resident first); a
    // refused set drops its actors back to the flat-normal draw.
    let mut sets_ok: Vec<TextureKey> = Vec::with_capacity(kit.sets.len());
    for set in &kit.sets {
        let base = set.base.filter(|key| tex_ok.contains(key));
        if presenter
            .upload_material_set_source(set.key, base, &set.normal, set.orm.as_ref())
            .is_ok()
        {
            sets_ok.push(set.key);
        }
    }
    for baked in &kit.baked {
        if presenter.upload_mesh(baked.key, &baked.mesh).is_ok() {
            let base_color = baked.texture.filter(|key| tex_ok.contains(key));
            let normal_map = baked.normal_map.filter(|key| sets_ok.contains(key));
            if let Some(kind) = kit_material_kind(baked.actor.asset) {
                presenter.set_material_kind(baked.key, kind);
            }
            // RL51: only a mesh whose cooked set rode in tiles by meters.
            if normal_map.is_some() {
                let detail = kit_pbr::surface_detail_for_file(baked.actor.asset.file_name());
                presenter.set_surface_detail(baked.key, detail);
            }
            draws.push(KitDraw {
                key: baked.key,
                material: baked.material.clone(),
                base_color,
                normal_map,
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
        budget: kit.budget.clone(),
    }
}

/// The patch file the shipped run applies when it sits beside the kit root:
/// `content/arena` reads `content/patch.e2patch`, the file `e2patch diff`
/// writes. A patch carries logical `src/...` paths, so it is built from the
/// kit root and applied there, never from an absolute tree path.
#[cfg(feature = "app")]
pub const KIT_PATCH_FILE: &str = "patch.e2patch";

/// Read the patch beside the kit root. Native only: the browser stages its kit
/// from a fetched bundle ([`stage_kit_from`]), so no sibling file exists there.
/// `Ok(None)` means no patch file; a read error keeps the path in the message.
#[cfg(all(feature = "app", not(target_arch = "wasm32")))]
fn read_kit_patch(path: &Path) -> Result<Option<Vec<u8>>, KitError> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(KitError::Patch(bound_diag(&format!(
            "{}: {error}",
            path.display()
        )))),
    }
}

/// The browser has no filesystem; a patch path beside the root does not exist.
#[cfg(all(feature = "app", target_arch = "wasm32"))]
fn read_kit_patch(_path: &Path) -> Result<Option<Vec<u8>>, KitError> {
    Ok(None)
}

/// Run `f` over the kit's content: the directory itself, or the directory
/// under a verified patch when [`KIT_PATCH_FILE`] sits beside `root`. The
/// patch's base hashes are checked first, so a patch for another build is
/// refused by path instead of serving stale or mixed bytes. No patch file is
/// the plain directory read, exactly as before.
#[cfg(feature = "app")]
fn with_kit_source<T>(
    root: &Path,
    f: impl FnOnce(&dyn engine_assets::AssetSource) -> Result<T, KitError>,
) -> Result<T, KitError> {
    let base = engine_assets::FsSource::new(root);
    let patch_path = root.with_file_name(KIT_PATCH_FILE);
    match read_kit_patch(&patch_path)? {
        Some(bytes) => {
            let patch = engine_assets::asset_source::PatchSet::parse(&bytes).map_err(|error| {
                KitError::Patch(bound_diag(&format!("{}: {error}", patch_path.display())))
            })?;
            patch.verify_base(&base).map_err(|error| {
                KitError::Patch(bound_diag(&format!("{}: {error}", patch_path.display())))
            })?;
            let patched: engine_assets::asset_source::PatchedSource<'_> = patch.apply(&base);
            f(&patched)
        }
        None => f(&base),
    }
}

/// Import from `root` and upload into the presenter in one call (the twin
/// staging path). A content patch beside the root is applied first; see
/// [`with_kit_source`].
#[cfg(feature = "app")]
pub fn stage_kit(
    presenter: &mut engine_render::present::Presenter,
    root: &Path,
) -> Result<KitReport, KitError> {
    with_kit_source(root, |source| stage_kit_from(presenter, source))
}

/// [`stage_kit`] over any content source (a fetched bundle in the browser).
#[cfg(feature = "app")]
pub fn stage_kit_from(
    presenter: &mut engine_render::present::Presenter,
    source: &dyn engine_assets::AssetSource,
) -> Result<KitReport, KitError> {
    let kit = import_kit_from(source)?;
    Ok(upload_kit(presenter, &kit))
}

/// Stage the content-dir kit, drawing the cube only when staging is refused
/// (twin fallback path); never fails, never panics. A refusal surfaces as
/// one skipped draw in the receipt (use [`stage_kit`] for the typed reason).
#[cfg(feature = "app")]
pub fn stage_kit_or_default(presenter: &mut engine_render::present::Presenter) -> KitReport {
    stage_kit(presenter, &kit_content_dir()).unwrap_or_else(|_| KitReport {
        skipped: 1,
        ..Default::default()
    })
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
            // The draw's material set: the forward pass maps the normal and
            // scales roughness, metallic and ambient occlusion by its ORM.
            normal_map: draw.normal_map,
            // NS31b2 slots: the kit imports no ORM or emissive map yet, so
            // both bind the fallbacks.
            orm: None,
            emissive: None,
        });
        pushed += 1;
    }
    pushed
}

// ---------------------------------------------------------------------------
// Headless leg: CPU-only kit gate without `app`. Default tests prove 8 files
// and 5,000+ tris with no `gltf` in the default graph. Same order and counts
// as the `app` import; GPU upload stays on the `app` leg.
// ---------------------------------------------------------------------------

/// One baked actor on the headless leg: placement plus stable key.
///
/// Consumer: default gate tests keep order and counts identical to `app`.
/// Units: meters for the actor box. Determinism: same files, same keys in
/// placement order. Errors: none; bad files fail the import. Budget: 11
/// entries at init; zero per tick.
///
/// Caller: crates/game/src/arena_kit.rs `import_kit` builds one per actor.
#[cfg(not(feature = "app"))]
pub struct BakedKitMesh {
    /// Placement actor this entry was fitted to.
    pub actor: KitActor,
    /// Stable presenter key for this actor.
    pub key: MeshKey,
}

/// CPU-side kit import on the headless leg: fit plus triangles, no GPU.
///
/// Consumer: default gate tests. Units: triangles from glTF accessors.
/// Determinism: same bytes, same counts and keys. Errors: [`KitError`]
/// when a file cannot be read or parsed or the floor fails. Budget: files
/// read once at init; zero per tick.
///
/// Caller: crates/game/src/arena_kit.rs `import_kit` is called by tests.
#[cfg(not(feature = "app"))]
pub struct ImportedKit {
    /// One entry per placed actor (11 on the shipping placement).
    pub baked: Vec<BakedKitMesh>,
    /// Source triangles over distinct files, not per-actor copies.
    pub triangles: u64,
    /// Distinct kit files imported.
    pub files: usize,
    /// Actors skipped with reason (`"import"` or `"fit"`).
    pub skipped: Vec<(u32, &'static str)>,
    /// Actors on fallback materials.
    pub material_fallbacks: usize,
}

// Every `u64` after `"needle":` in order (counts, indices).
#[cfg(not(feature = "app"))]
fn all_u64_after(text: &str, quoted: &str, file: &'static str) -> Result<Vec<u64>, KitError> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(quoted) {
        let with_needle = rest.get(at..).ok_or_else(|| import_err(file))?;
        let after_needle = with_needle
            .strip_prefix(quoted)
            .ok_or_else(|| import_err(file))?;
        let colon_at = after_needle.find(':').ok_or_else(|| import_err(file))?;
        let with_colon = after_needle
            .get(colon_at..)
            .ok_or_else(|| import_err(file))?;
        let after_colon = with_colon
            .strip_prefix(':')
            .ok_or_else(|| import_err(file))?;
        let trimmed = after_colon.trim_start();
        let mut end = 0usize;
        for c in trimmed.chars() {
            if c.is_ascii_digit() {
                end = end
                    .checked_add(c.len_utf8())
                    .ok_or_else(|| import_err(file))?;
            } else {
                break;
            }
        }
        if end == 0 {
            return Err(import_err(file));
        }
        let digits = trimmed.get(..end).ok_or_else(|| import_err(file))?;
        let value = digits.parse::<u64>().map_err(|_| import_err(file))?;
        out.push(value);
        rest = trimmed.get(end..).ok_or_else(|| import_err(file))?;
    }
    Ok(out)
}

// Triangles for one file: index counts over primitives, no `gltf`.
#[cfg(not(feature = "app"))]
fn file_triangles(text: &str, file: &'static str) -> Result<u64, KitError> {
    let counts = all_u64_after(text, "\"count\"", file)?;
    let indices = all_u64_after(text, "\"indices\"", file)?;
    let mut triangles = 0u64;
    for idx in indices {
        let ai = usize::try_from(idx).map_err(|_| import_err(file))?;
        let count = counts.get(ai).copied().ok_or_else(|| import_err(file))?;
        triangles = triangles
            .checked_add(count / 3)
            .ok_or_else(|| import_err(file))?;
    }
    if triangles == 0 {
        return Err(import_err(file));
    }
    Ok(triangles)
}

/// Import every kit actor from `root/src/<asset>.gltf` without `gltf`
/// (headless leg): std-only JSON counts over the same files the `app` leg
/// parses, same order and keys. A missing file or floor miss fails loudly.
///
/// The files are read through the content seam (a file source natively, a
/// fetched bundle in the browser) and held by generational handles, so the
/// headless leg has no file system of its own.
///
/// Caller: default gate tests call `import_kit` with [`kit_content_dir`].
#[cfg(not(feature = "app"))]
pub fn import_kit(root: &Path) -> Result<ImportedKit, KitError> {
    import_kit_from(&engine_assets::FsSource::new(root))
}

/// [`import_kit`] over any content source: the files are read as
/// `src/<asset>.gltf` by logical path, so the browser hands in a fetched
/// bundle and gets the same kit as the native directory read.
#[cfg(not(feature = "app"))]
pub fn import_kit_from(source: &dyn engine_assets::AssetSource) -> Result<ImportedKit, KitError> {
    let actors = kit_actors()?;
    let files = kit_asset_files(&actors);
    let mut assets = KitAssets::load(source, &files)?;
    let mut triangles = 0u64;
    for file in &files {
        let bytes = assets.bytes(&kit_logical_path(file))?;
        let text = std::str::from_utf8(bytes).map_err(|_| import_err(file))?;
        if !text.contains("\"baseColorFactor\"") || !text.contains("\"metallicFactor\"") {
            return Err(import_err(file));
        }
        let tris = file_triangles(text, file)?;
        triangles = triangles
            .checked_add(tris)
            .ok_or_else(|| import_err(file))?;
    }
    // The counts are in: the handles go stale and the bytes go back to the
    // queue's budget.
    assets.unload();
    let mut baked = Vec::with_capacity(actors.len());
    for actor in &actors {
        if !files.contains(&actor.asset.file_name()) {
            return Err(import_err(actor.asset.file_name()));
        }
        baked.push(BakedKitMesh {
            actor: *actor,
            key: kit_mesh_key(actor),
        });
    }
    if !kit_meets_tris_floor(triangles) {
        return Err(KitError::Import(bound_diag("kit triangles below floor")));
    }
    if baked.is_empty() {
        return Err(KitError::Import("no kit actor baked".to_string()));
    }
    Ok(ImportedKit {
        baked,
        triangles,
        files: files.len(),
        skipped: Vec::new(),
        material_fallbacks: 0,
    })
}

#[cfg(test)]
mod tests {
    use super::kit_load::kit_actor_from_block;
    use super::*;
    use engine_arena::{ArenaConfig, ArenaRules, LiveMatch, FIGHTER_RADIUS_M};
    use engine_input::{Buttons, FighterId, FighterInput, FighterTape};

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

    /// The `physical` flag is real data, not a constant: the six walls collide
    /// and the five dressing actors do not. Ids follow the placement file
    /// order (1 `sand_floor`, 2 `stands`, 3 `wall_north`, 4 `wall_south`,
    /// 5 `gate_tunnel`, 6/7 `torch_*`, 8/9 `banner_*`, 10 `crate_a`,
    /// 11 `debris_field`), so a flip on either side fails by name.
    #[test]
    fn physical_pins_the_six_walls_and_drops_the_dressing() {
        let actors = kit_actors().expect("embedded placement parses");
        let collides: Vec<(u32, KitAsset)> = actors
            .iter()
            .filter(|actor| actor.physical)
            .map(|actor| (actor.id, actor.asset))
            .collect();
        assert_eq!(
            collides,
            vec![
                (2, KitAsset::Stands),
                (3, KitAsset::Walls),
                (4, KitAsset::Walls),
                (5, KitAsset::GateTunnel),
                (10, KitAsset::Crate),
                (11, KitAsset::Debris),
            ],
            "the stands, both walls, the gate and the two cover boxes collide"
        );
        let dressing: Vec<u32> = actors
            .iter()
            .filter(|actor| !actor.physical)
            .map(|actor| actor.id)
            .collect();
        assert_eq!(
            dressing,
            vec![1, 6, 7, 8, 9],
            "the sand floor, both torches and both banners are dressing"
        );
        // The data drives the sim: the collision world is exactly those six
        // boxes, while the view keeps drawing every staged actor.
        let world = kit_match_world().expect("embedded placement collides");
        assert_eq!(world.obstacles().len(), 6);
        assert_eq!(kit_prop_rows().len(), KIT_ACTOR_COUNT);
    }

    /// The embedded placement stages 11 actors and collides 6 of them: the
    /// stands, both walls, the gate and the two cover boxes (the name counts
    /// the staged actors, the world holds the physical set).
    #[test]
    fn kit_world_holds_11_colliders() {
        assert_eq!(
            kit_actors().expect("embedded placement parses").len(),
            KIT_ACTOR_COUNT,
            "11 staged actors"
        );
        let world = kit_match_world().expect("embedded placement collides");
        assert_eq!(world.obstacles().len(), 6, "6 physical boxes collide");
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
        // Index 1 is `stands`, a physical actor, so its box reaches the world
        // and a zero size must refuse (the sand floor is dressing and builds
        // no box at all).
        let mut actors = kit_actors().expect("embedded placement parses");
        actors[1].size_m = [0.0, 1.0, 1.0];
        assert!(kit_match_world_for_actors(&actors).is_err());
        let mut actors = kit_actors().expect("embedded placement parses");
        actors[2].yaw = core::f32::consts::FRAC_PI_2;
        assert!(kit_match_world_for_actors(&actors).is_err());
        // Dropping one `physical` flag drops exactly one box from the six.
        let mut actors = kit_actors().expect("embedded placement parses");
        actors[10].physical = false;
        let world = kit_match_world_for_actors(&actors).expect("five walls still collide");
        assert_eq!(world.obstacles().len(), 5);
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
    fn lf8_kit_kinds_name_the_torch_iron_and_the_banner_cloth() {
        assert_eq!(
            kit_material_kind(KitAsset::Torch),
            Some(MaterialKind::Metal)
        );
        assert_eq!(
            kit_material_kind(KitAsset::Banner),
            Some(MaterialKind::Cloth)
        );
        for asset in [
            KitAsset::SandFloor,
            KitAsset::Stands,
            KitAsset::Walls,
            KitAsset::GateTunnel,
            KitAsset::Crate,
            KitAsset::Debris,
        ] {
            assert_eq!(kit_material_kind(asset), None, "{asset:?} stays stone");
        }
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
            ..KitReport::default()
        };
        assert_eq!(
            live_kit_notice(&report),
            "arena: staged 0 kit draws (2 skipped)"
        );
    }

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

    // The import gate reads the real kit: every mesh is measured, and the
    // banner cloth base now sits under the potato cap, so the kit passes.
    #[cfg(feature = "app")]
    #[test]
    fn the_shipping_kit_measures_meshes_against_the_potato_caps() {
        let kit = import_kit(&kit_content_dir()).expect("kit imports from content");
        let budget = &kit.budget;
        assert_eq!(budget.meshes, 38, "one measured mesh per primitive");
        assert_eq!(budget.max_tris, 512, "the floor is the largest mesh");
        assert_eq!(budget.max_bones, 0, "the kit is static");
        assert_eq!(budget.max_influences, 0, "no mesh is skinned");
        assert_eq!(budget.max_materials, 1, "one material per mesh");
        assert_eq!(
            budget.max_tex_px, 1_048_576,
            "the biggest base is a 1024x1024 cooked set, exactly at the cap"
        );
        assert_eq!(budget.status(), "PASS");
        assert_eq!(
            budget.findings,
            Vec::new(),
            "no mesh is over a potato cap: the banner cloth base is 1024x986"
        );
        let line = budget.line();
        assert!(line.contains("meshes=38"), "{line}");
        assert!(line.contains("caps=POTATO budget=PASS"), "{line}");
    }

    // Same bytes give the same meshes: one import per run, keys stable x2.
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

    // Every kit actor draws a base-color image: one texture per distinct
    // material (five cooked sets plus the banner cloth), every actor keyed,
    // keys stable across reimports.
    #[cfg(feature = "app")]
    #[test]
    fn kit_import_carries_base_color_textures() {
        let root = kit_content_dir();
        let kit = import_kit(&root).expect("kit imports from content");
        let materials = kit_pbr::COOKED_SETS.len() + 1;
        assert_eq!(
            kit.textures.len(),
            materials,
            "one base-color texture per distinct kit material"
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
        assert_eq!(keys.len(), materials, "texture keys distinct");
        for (key, upload) in &kit.textures {
            assert!(
                upload.width() >= 4 && upload.height() >= 4,
                "texture {key:?} is a real image, not a stub"
            );
        }
    }

    // A file that names a base-color image and does not carry it refuses by
    // file name, never a white draw; bytes that are not an image refuse the
    // same way.
    #[cfg(feature = "app")]
    #[test]
    fn kit_refuses_a_missing_base_color_image() {
        use engine_assets::cooked::{CookedMaterial, TextureRef, TextureSlot};
        let mut import = engine_assets::mesh_import::GltfImport {
            materials: vec![CookedMaterial {
                textures: vec![TextureRef {
                    asset: 0xDEAD_BEEF,
                    slot: TextureSlot::BaseColor,
                }],
                ..Default::default()
            }],
            ..Default::default()
        };
        let err =
            kit_texture(&import, "gate_tunnel.gltf", 0).expect_err("a named image must be carried");
        assert!(matches!(err, KitError::Texture(_)), "{err:?}");
        assert!(err.to_string().contains("gate_tunnel.gltf"), "{err}");
        import.images.insert(0xDEAD_BEEF, b"not an image".to_vec());
        let err = kit_texture(&import, "gate_tunnel.gltf", 0).expect_err("garbage does not decode");
        assert!(matches!(err, KitError::Texture(_)), "{err:?}");
    }

    // The capture probe turns textures off only on `off`, so a typo never
    // silently changes the shipped look.
    #[test]
    fn kit_texture_switch_turns_off_only_on_off() {
        assert!(kit_textures_enabled(None));
        assert!(kit_textures_enabled(Some("1")));
        assert!(!kit_textures_enabled(Some("off")));
        assert!(!kit_textures_enabled(Some(" OFF ")));
    }

    // VR2: every arena-kit material has base, normal and ORM. The five cooked
    // sets plus the banner cloth give six material sets, each with an ORM map
    // and a base it sits over; every actor names one set and one base, and
    // the cooked files supply the five CC0 sets. Skips (passes) when the
    // content is still Git LFS pointers.
    #[cfg(feature = "app")]
    #[test]
    fn kit_every_material_has_base_normal_and_orm() {
        let kit = import_kit(&kit_content_dir()).expect("kit imports from content");
        if kit.cooked_sets == 0 {
            return;
        }
        assert_eq!(
            kit.cooked_sets,
            kit_pbr::COOKED_SETS.len(),
            "all five cooked sets load"
        );
        assert_eq!(
            kit.sets.len(),
            kit_pbr::COOKED_SETS.len() + 1,
            "five cooked sets plus the cloth"
        );
        for set in &kit.sets {
            assert!(set.base.is_some(), "set {:?} sits over a base", set.key);
            assert!(set.orm.is_some(), "set {:?} carries an ORM map", set.key);
            assert!(
                set.normal.width() >= 512,
                "set {:?} normal is a real map",
                set.key
            );
        }
        for baked in &kit.baked {
            let set = baked
                .normal_map
                .and_then(|key| kit.sets.iter().find(|set| set.key == key))
                .unwrap_or_else(|| panic!("actor {} names a material set", baked.actor.id));
            assert_eq!(
                baked.texture, set.base,
                "actor {} base matches its set",
                baked.actor.id
            );
        }
    }

    // The kit loads the same from a packed bundle as from the directory
    // (meshes, materials and textures of every actor), and a bundle that lost
    // or damaged a kit file fails with that file's name.
    #[cfg(feature = "app")]
    #[test]
    fn kit_import_from_a_bundle_matches_the_directory() {
        use engine_assets::asset_source::{pack_to, PackRules};
        use engine_assets::{BundleSource, FsSource};
        let root = kit_content_dir();
        let fs = FsSource::new(&root);
        let bundle_of = |rules: &PackRules| {
            let mut bytes = Vec::new();
            pack_to(&fs, rules, &mut bytes).expect("kit packs");
            bytes
        };
        let all_bytes = bundle_of(&PackRules::new().include("src/").include("cooked/"));
        let bundle = BundleSource::open(all_bytes.clone()).expect("bundle opens");
        let from_dir = import_kit(&root).expect("kit imports from content");
        let from_bundle = import_kit_from(&bundle).expect("kit imports from the bundle");
        assert_eq!(from_bundle.files, from_dir.files);
        assert_eq!(from_bundle.triangles, from_dir.triangles);
        assert_eq!(from_bundle.skipped, from_dir.skipped);
        assert_eq!(from_bundle.baked.len(), from_dir.baked.len());
        for (packed, loose) in from_bundle.baked.iter().zip(&from_dir.baked) {
            assert_eq!(packed.actor.id, loose.actor.id);
            assert_eq!(packed.key, loose.key);
            assert_eq!(packed.mesh, loose.mesh, "actor {} mesh", loose.actor.id);
            assert_eq!(packed.material, loose.material);
            assert_eq!(packed.texture, loose.texture);
        }
        assert_eq!(from_bundle.textures, from_dir.textures);
        assert_eq!(from_bundle.sets, from_dir.sets);

        let partial = BundleSource::open(bundle_of(&PackRules::new().include("src/banner.gltf")))
            .expect("partial bundle opens");
        match import_kit_from(&partial) {
            Err(KitError::Io(text)) => {
                assert!(
                    text.contains("not found") && text.contains(".gltf"),
                    "{text}"
                );
            }
            other => panic!(
                "a bundle without kit files must fail by name, got {:?}",
                other.err()
            ),
        }

        let mut damaged = all_bytes;
        let crate_blob = bundle
            .entries_in_file_order()
            .find(|entry| entry.path == "src/crate.gltf")
            .expect("crate.gltf packed");
        damaged[crate_blob.offset + crate_blob.len / 2] ^= 0xFF;
        let damaged = BundleSource::open(damaged).expect("framing intact");
        match import_kit_from(&damaged) {
            Err(KitError::Io(text)) => {
                assert!(
                    text.contains("crate.gltf") && text.contains("hash mismatch"),
                    "{text}"
                );
            }
            other => panic!("a damaged blob must fail by name, got {:?}", other.err()),
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
                orm: None,
                emissive: None,
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

    // Crowd on the stands: the west stands draw carries procedural
    // spectators (body plus head boxes) above its treads, so tiers read as
    // spectators instead of bare stone. Fails before the crowd merge lands.
    #[cfg(feature = "app")]
    #[test]
    fn stands_mesh_carries_procedural_crowd() {
        let kit = import_kit(&kit_content_dir()).expect("kit imports from content");
        let stands = kit
            .baked
            .iter()
            .find(|baked| baked.actor.id == 2)
            .expect("stands actor staged");
        // 24 spectators (3 rows x 8 cols) add body plus head boxes.
        assert!(
            stands.mesh.positions.len() >= 2_000,
            "stands mesh holds crowd, got {} verts",
            stands.mesh.positions.len()
        );
        let top = stands
            .mesh
            .positions
            .iter()
            .map(|point| point[1])
            .fold(f32::NEG_INFINITY, f32::max);
        assert!(top > 3.2, "crowd heads rise above the stone top, got {top}");
    }

    #[cfg(feature = "app")]
    #[test]
    fn crowd_helpers_name_stands_only() {
        let actors = kit_actors().expect("embedded placement parses");
        let sand = actors
            .iter()
            .find(|actor| actor.id == 1)
            .expect("sand floor placed");
        let stands = actors
            .iter()
            .find(|actor| actor.id == 2)
            .expect("stands placed");
        assert!(!crowd_is_stands(sand), "sand never carries crowd");
        assert!(crowd_is_stands(stands), "west stands carries crowd");
        for actor in &actors {
            if actor.id != 2 {
                assert!(
                    !crowd_is_stands(actor),
                    "actor {} is stone, not crowd",
                    actor.id
                );
            }
        }
        // One box is 24 verts plus 36 indices with unit normals in UV range.
        let mut pos = Vec::new();
        let mut nor = Vec::new();
        let mut uv = Vec::new();
        let mut idx = Vec::new();
        push_crowd_box(
            &mut pos,
            &mut nor,
            &mut uv,
            &mut idx,
            [0.0, 1.0, 0.0],
            [0.34, 0.55, 0.24],
        );
        assert_eq!(pos.len(), 24, "one box holds 24 verts");
        assert_eq!(nor.len(), 24);
        assert_eq!(uv.len(), 24);
        assert_eq!(idx.len(), 36);
        for normal in &nor {
            let len2 = normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2];
            assert!((len2 - 1.0).abs() < 1e-6, "crowd normals stay unit");
        }
        for coord in &uv {
            assert!(
                (0.0..=1.0).contains(&coord[0]) && (0.0..=1.0).contains(&coord[1]),
                "crowd uvs sample the texture"
            );
        }
        // Empty mesh refuses crowd instead of dividing by zero.
        let mut empty_pos = Vec::new();
        let mut empty_nor = Vec::new();
        let mut empty_uv = Vec::new();
        let mut empty_idx = Vec::new();
        assert!(
            !append_stands_crowd(
                &mut empty_pos,
                &mut empty_nor,
                &mut empty_uv,
                &mut empty_idx
            ),
            "empty mesh takes no crowd"
        );
    }

    /// Observed pose series for slot 0 over `config`, played by an
    /// `LiveMatch` over `world` with a taped walk: slot 0 wishes `mz` every
    /// tick and slot 1 idles, so both slots are taped and no brain decides.
    /// The sample pushed for each tick is that tick's dataset row, which
    /// holds the pre-step pose (tick 0 is the settled spawn), so the series
    /// covers every pose the match reached at a tick boundary.
    fn kit_wall_walk(config: &ArenaConfig, world: &MatchWorld, mz: i8) -> Vec<[f32; 3]> {
        let mut inputs = Vec::new();
        for tick in 0..config.ticks {
            inputs.push(FighterInput {
                tick,
                fighter: FighterId(0),
                mx: 0,
                mz,
                facing: 0,
                buttons: Buttons::NONE,
            });
            inputs.push(FighterInput {
                tick,
                fighter: FighterId(1),
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: Buttons::NONE,
            });
        }
        let tape = FighterTape(inputs);
        let mut live = LiveMatch::with_rules_world(config, ArenaRules::V1, world.clone())
            .expect("match builds");
        let mut out = Vec::new();
        while live.tick() < config.ticks {
            let tick = live.tick();
            let stepped = live.step(kit_tick_inputs(&tape, tick)).expect("tick steps");
            out.push(stepped.rows[0].observation.fighters[0].pos);
        }
        out
    }

    /// The input slice staged for one tick of a `(tick, fighter)`-sorted tape.
    fn kit_tick_inputs(tape: &FighterTape, tick: u32) -> &[FighterInput] {
        let start = tape.0.partition_point(|input| input.tick < tick);
        let end = tape.0.partition_point(|input| input.tick <= tick);
        &tape.0[start..end]
    }

    /// F2P: a fighter driven straight at `wall_north` (centre `[-2, -10]`,
    /// half `[4, 0.5]`) for 600 ticks never reaches its near face `z = -9.5`,
    /// while the same drive over the empty world walks through it. Mirrors
    /// `arena::spine_step::walled_fights_never_put_a_fighter_inside_the_wall`.
    #[test]
    fn kit_walls_stop_a_fighter() {
        let world = kit_match_world().expect("embedded placement collides");
        let face_z = -10.0 + 0.5;
        // A seed whose settled slot-0 spawn stands south of the wall and well
        // inside its 8 m x band, so a straight -z walk meets the flat face
        // (no corner slide): the walk is then deterministic.
        let mut chosen = None;
        for seed in 0..64u64 {
            let config = ArenaConfig::try_new(seed, 600, 2, 12.0).expect("valid config");
            let mut probe =
                LiveMatch::with_rules_world(&config, ArenaRules::V1, world.clone()).expect("probe");
            let stepped = probe.step(&[]).expect("probe tick steps");
            let spawn = stepped.rows[0].observation.fighters[0].pos;
            if (-5.0..=1.0).contains(&spawn[0]) && spawn[2] > -8.0 {
                chosen = Some((config, spawn));
                break;
            }
        }
        let (config, spawn) = chosen.expect("a seed spawns in front of the wall");
        assert!(spawn[2] > face_z, "the walk starts clear of the wall");

        let walled = kit_wall_walk(&config, &world, -100);
        assert_eq!(walled.len(), 600, "one pose per tick");
        for pose in &walled {
            assert!(
                !world.overlaps([pose[0], pose[2]], FIGHTER_RADIUS_M),
                "fighter inside a kit box at {pose:?}"
            );
            assert!(
                pose[2] > face_z,
                "the drive reached the wall's near face at {pose:?}"
            );
        }
        let reached = walled.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min);
        assert!(
            reached < -8.5,
            "the drive must reach the wall, got {reached}"
        );

        // F2P oracle: the same drive over the open arena walks through the
        // wall, so the world is what stops the fighter.
        let open = kit_wall_walk(&config, &MatchWorld::empty(), -100);
        let through = open.iter().map(|p| p[2]).fold(f32::INFINITY, f32::min);
        assert!(
            through < -10.5,
            "the empty world must let the fighter through the wall, got {through}"
        );
    }

    /// Accept, second half: 1,000 seeded ticks of two bots (20 seeds x 50
    /// ticks) see 0 fighters inside a kit box.
    #[test]
    fn kit_walls_never_crossed_by_two_bots_x1000() {
        let world = kit_match_world().expect("embedded placement collides");
        let mut ticks = 0u32;
        let mut moved = false;
        for seed in 0..20u64 {
            let config = ArenaConfig::try_new(seed, 50, 2, 12.0).expect("valid config");
            let mut live = LiveMatch::with_rules_world(&config, ArenaRules::V1, world.clone())
                .expect("bot match builds");
            let mut first = None;
            while live.tick() < config.ticks {
                let stepped = live.step(&[]).expect("bot tick steps");
                for pose in stepped.rows[0].observation.fighters.iter() {
                    if let Some(start) = first {
                        moved |= pose.pos != start;
                    } else {
                        first = Some(pose.pos);
                    }
                    assert!(
                        !world.overlaps([pose.pos[0], pose.pos[2]], FIGHTER_RADIUS_M),
                        "seed {seed}: fighter inside a kit box at {:?}",
                        pose.pos
                    );
                }
                ticks += 1;
            }
        }
        assert_eq!(ticks, 1000, "20 seeds x 50 ticks of two bots");
        assert!(moved, "the bots must walk, or the crossing test is vacuous");
    }

    /// F2P: the kit reads the hand-placed zone table (`content/design/
    /// arena-zones.rows`), so the game's zone reading and `tools/level-check.mjs`
    /// come from one file. The flag columns had no Rust reader (only test
    /// fixtures and win-rate prose named them).
    #[test]
    fn kit_zone_rows_reads_the_hand_placed_layout() {
        let rows = kit_zone_rows().expect("embedded zone rows parse");
        assert_eq!(
            rows.len(),
            kit_zones::KIT_ZONE_COUNT,
            "11 actor rows plus centre"
        );
        let gate = rows
            .iter()
            .find(|row| row.id == KIT_ZONE_GATE_ID)
            .expect("gate row present");
        // 10 m on the centre lane, 2 m half-edge: the shipped gate anchor.
        assert_eq!((gate.class, gate.cx_cm, gate.cz_cm), (2, 1000, 0));
        assert_eq!(gate.radius_cm, 200);
        // The flags are shipped data: two advantage bumper rows and exactly
        // one holdable dominance point (the duel centre).
        assert_eq!(rows.iter().filter(|r| r.class == 4).count(), 2);
        assert_eq!(rows.iter().filter(|r| r.dominance).count(), 1);
        assert!(rows.iter().any(|r| r.advantage && r.class == 4));
        assert!(rows.iter().any(|r| r.risk));
    }

    /// The reader refuses what `tools/level-check.mjs` refuses, with the line
    /// number or the row count: a float, an out-of-band class, and a table
    /// missing an actor row (1 row of the shipped 12).
    #[test]
    fn kit_zone_rows_refuse_bad_rows_by_line_and_count() {
        let float = "table arena-zones version 1\nsand_floor class=0 cx_cm=1.5 cz_cm=0 radius_cm=100 height_cm=20 advantage=0 dominance=0 risk=0\n";
        let err = kit_zones::kit_zone_rows_from_text(float).expect_err("a float cx_cm refuses");
        assert!(err.to_string().contains("line 2"), "{err}");
        let class = "table arena-zones version 1\nsand_floor class=9 cx_cm=0 cz_cm=0 radius_cm=100 height_cm=20 advantage=0 dominance=0 risk=0\n";
        let err = kit_zones::kit_zone_rows_from_text(class).expect_err("class 9 refuses");
        assert!(err.to_string().contains("line 2"), "{err}");
        let short = "table arena-zones version 1\nsand_floor class=0 cx_cm=-6000 cz_cm=0 radius_cm=1500 height_cm=20 advantage=0 dominance=0 risk=0\n";
        let err =
            kit_zones::kit_zone_rows_from_text(short).expect_err("a missing actor row refuses");
        assert!(err.to_string().contains("12 rows, saw 1"), "{err}");
    }

    /// F24 fixture: a kit root with two logical blobs and the tree one patch
    /// ahead of it (only `a.txt` changes; `b.txt` rides the manifest as the
    /// unchanged base row the verifier checks).
    #[cfg(feature = "app")]
    fn patch_fixture(tag: &str) -> std::path::PathBuf {
        let root =
            std::env::temp_dir().join(format!("e2040_kit_patch_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let base = root.join("content").join("arena").join("src");
        let new = root.join("new").join("arena").join("src");
        std::fs::create_dir_all(&base).expect("base src");
        std::fs::create_dir_all(&new).expect("new src");
        std::fs::write(base.join("a.txt"), b"base-a").expect("base a");
        std::fs::write(base.join("b.txt"), b"same-b").expect("base b");
        std::fs::write(new.join("a.txt"), b"patch-a").expect("new a");
        std::fs::write(new.join("b.txt"), b"same-b").expect("new b");
        root
    }

    /// F24: the shipped stage wraps the content dir in a patch when
    /// `patch.e2patch` sits beside it; the carried blob wins, every other
    /// path still reads the directory, and removing the file is the plain
    /// directory read again.
    #[cfg(feature = "app")]
    #[test]
    fn kit_source_applies_a_patch_beside_the_root() {
        let root = patch_fixture("applied");
        let arena = root.join("content").join("arena");
        let new = root.join("new").join("arena");
        let base = engine_assets::asset_source::Manifest::from_source(
            &engine_assets::FsSource::new(&arena),
        )
        .expect("base manifest");
        let patch = engine_assets::asset_source::PatchSet::build(
            &base,
            &engine_assets::FsSource::new(&new),
        )
        .expect("patch builds")
        .0;
        let patch_path = arena.with_file_name(KIT_PATCH_FILE);
        std::fs::write(&patch_path, patch.to_bytes().expect("patch bytes")).expect("patch writes");

        with_kit_source(&arena, |source| {
            assert_eq!(
                source.read("src/a.txt").expect("patched reads").as_ref(),
                b"patch-a"
            );
            assert_eq!(
                source.read("src/b.txt").expect("kept reads").as_ref(),
                b"same-b"
            );
            Ok(())
        })
        .expect("the patch applies");

        std::fs::remove_file(&patch_path).expect("patch removed");
        with_kit_source(&arena, |source| {
            assert_eq!(
                source.read("src/a.txt").expect("disk reads").as_ref(),
                b"base-a"
            );
            Ok(())
        })
        .expect("the directory reads");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// F24: a patch built for another base is refused by the path it names,
    /// so a shipped run never mixes a stale blob with the new tree.
    #[cfg(feature = "app")]
    #[test]
    fn kit_source_refuses_a_patch_for_another_base_by_path() {
        let root = patch_fixture("refused");
        let arena = root.join("content").join("arena");
        let new = root.join("new").join("arena");
        let base = engine_assets::asset_source::Manifest::from_source(
            &engine_assets::FsSource::new(&arena),
        )
        .expect("base manifest");
        let patch = engine_assets::asset_source::PatchSet::build(
            &base,
            &engine_assets::FsSource::new(&new),
        )
        .expect("patch builds")
        .0;
        std::fs::write(
            arena.with_file_name(KIT_PATCH_FILE),
            patch.to_bytes().expect("bytes"),
        )
        .expect("patch writes");
        // The base moved under the patch: b.txt keeps its manifest row but
        // not its bytes, so the verifier must name it.
        std::fs::write(arena.join("src").join("b.txt"), b"changed-b").expect("base edit");

        let err = with_kit_source(&arena, |_| Ok(())).expect_err("the base changed");
        let text = err.to_string();
        assert!(text.contains("src/b.txt"), "{text}");
        assert!(text.contains("patch.e2patch"), "{text}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
