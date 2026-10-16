//! windowed dressing: torch/banner staging for the window twin.
//!
//! Consumer: `windowed` twin plus bins via its re-export. Data owner: this
//! module owns the parse plus mesh/material/camera staging; the JSON file
//! owns the coordinates. Units: meters, radians. Determinism: the same file
//! gives the same actors and meshes; pixels never enter the sim hash.
//! Errors: typed `super::WinError`, never panics.

use super::WinError;
use engine_assets::lod_swap::{distance_mm_between, target_lod};
use engine_assets::mesh_cook::MAX_LOD_LEVELS;
use engine_render::present::{DrawList, MeshKey, Presenter, SizeClass};

/// Dressing staging: `content/arena/dressing.json` torch/banner actors
/// bound into the window twin forward path (0 new tris: transformed cubes).
/// Consumer: the `arena` twin (`frame_list`, `init_twin`, headless smoke).
/// Data owner: this section owns the parse plus mesh/material/camera staging;
/// the JSON file owns the coordinates. Units: meters, radians. Determinism:
/// the same file gives the same actors and meshes; pixels never enter the
/// sim hash. Errors: typed [`WinError`], never panics.
pub const DRESSING_JSON: &str = include_str!("../../../content/arena/dressing.json");
/// Dressing actors staged from the file (2 torches + 4 banners).
pub const DRESSING_ACTOR_COUNT: usize = 6;
/// Mesh id base for dressing draws (the central cube keeps id 1).
pub const DRESSING_MESH_ID_BASE: u64 = 0xD2E5_5100;
/// Twin establishing eye in meters (west of the walls, down the axis).
pub const DRESSING_EYE_M: [f32; 3] = [-14.0, 5.0, 0.0];
/// Twin establishing target in meters (the gate court).
pub const DRESSING_TARGET_M: [f32; 3] = [14.0, 1.0, 0.0];
/// Twin establishing vertical fov in degrees.
pub const DRESSING_FOV_DEG: f32 = 50.0;

/// Dressing asset kind: torch approach pair or banner court/far pairs.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DressingAsset {
    /// Approach torch flanking the tunnel mouth.
    Torch,
    /// Court or far banner pairing the gate axis.
    Banner,
}

/// One staged dressing actor: box center plus full extents in meters.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DressingActor {
    /// Stable actor id from the file (101..=106).
    pub id: u32,
    /// Which kit prop this stages.
    pub asset: DressingAsset,
    /// Box center in meters `[x, y, z]`.
    pub pos_m: [f32; 3],
    /// Full extents in meters `[sx, sy, sz]`.
    pub size_m: [f32; 3],
}

/// Raw field slice after `"needle":` in one actor block. No heap; the
/// caller trims the token. Missing needle or colon fails loudly.
fn dressing_field_raw<'a>(block: &'a str, needle: &str) -> Result<&'a str, WinError> {
    let at = block
        .find(needle)
        .ok_or_else(|| WinError::Render(format!("dressing missing {needle}")))?;
    let rest = block
        .get(at + needle.len()..)
        .ok_or_else(|| WinError::Render(format!("dressing missing {needle}")))?;
    let colon = rest
        .find(':')
        .ok_or_else(|| WinError::Render(format!("dressing missing {needle}")))?;
    rest.get(colon + 1..)
        .ok_or_else(|| WinError::Render(format!("dressing missing {needle}")))
}

/// Parse one finite `f32` field (`"x"`, `"sy"` ...). Rejects missing,
/// unparsable and non-finite values loudly.
fn dressing_parse_f32(block: &str, needle: &str) -> Result<f32, WinError> {
    let rest = dressing_field_raw(block, needle)?;
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    let token = rest.get(..end).unwrap_or("").trim();
    let value: f32 = token
        .parse()
        .map_err(|_| WinError::Render(format!("dressing bad {needle}")))?;
    if !value.is_finite() {
        return Err(WinError::Render(format!("dressing bad {needle}")));
    }
    Ok(value)
}

/// Parse one `u32` id field. Rejects missing and unparsable values loudly.
fn dressing_parse_id(block: &str) -> Result<u32, WinError> {
    let rest = dressing_field_raw(block, "\"id\"")?;
    let end = rest.find([',', '}']).unwrap_or(rest.len());
    let token = rest.get(..end).unwrap_or("").trim();
    token
        .parse::<u32>()
        .map_err(|_| WinError::Render("dressing bad \"id\"".to_string()))
}

/// Parse one asset label (`"torch"` or `"banner"`). Unknown labels fail.
fn dressing_parse_asset(block: &str) -> Result<DressingAsset, WinError> {
    let rest = dressing_field_raw(block, "\"asset\"")?;
    let quote = rest
        .find('"')
        .ok_or_else(|| WinError::Render("dressing bad \"asset\"".to_string()))?;
    let tail = rest.get(quote + 1..).unwrap_or("");
    let end = tail
        .find('"')
        .ok_or_else(|| WinError::Render("dressing bad \"asset\"".to_string()))?;
    match tail.get(..end).unwrap_or("") {
        "torch" => Ok(DressingAsset::Torch),
        "banner" => Ok(DressingAsset::Banner),
        _ => Err(WinError::Render("dressing bad \"asset\"".to_string())),
    }
}

/// Parse one actor object block (no nested objects inside). Validates
/// ranges: positions inside +/-99 m, sizes in `(0, 10]`, ids nonzero.
fn dressing_actor_from_block(block: &str) -> Result<DressingActor, WinError> {
    let id = dressing_parse_id(block)?;
    if id == 0 {
        return Err(WinError::Render("dressing bad \"id\"".to_string()));
    }
    let asset = dressing_parse_asset(block)?;
    let x = dressing_parse_f32(block, "\"x\"")?;
    let y = dressing_parse_f32(block, "\"y\"")?;
    let z = dressing_parse_f32(block, "\"z\"")?;
    let sx = dressing_parse_f32(block, "\"sx\"")?;
    let sy = dressing_parse_f32(block, "\"sy\"")?;
    let sz = dressing_parse_f32(block, "\"sz\"")?;
    for (name, v) in [("x", x), ("y", y), ("z", z)] {
        let _ = name;
        if !v.is_finite() {
            return Err(WinError::Render("dressing bad pos".to_string()));
        }
    }
    if !(-99.0..=99.0).contains(&x) || !(-99.0..=99.0).contains(&z) || !(0.0..=10.0).contains(&y) {
        return Err(WinError::Render("dressing actor out of bounds".to_string()));
    }
    for size in [sx, sy, sz] {
        if !(0.0 < size && size <= 10.0) {
            return Err(WinError::Render("dressing bad size".to_string()));
        }
    }
    Ok(DressingActor {
        id,
        asset,
        pos_m: [x, y, z],
        size_m: [sx, sy, sz],
    })
}

/// Parse the dressing actor table from JSON text. Expects exactly
/// [`DRESSING_ACTOR_COUNT`] objects inside the `"actors"` array; any shape
/// drift (missing array, wrong count, bad actor) fails loudly so a stale
/// binary never silently renders stale dressing.
pub fn dressing_actors_from_json(
    text: &str,
) -> Result<[DressingActor; DRESSING_ACTOR_COUNT], WinError> {
    let anchor = text
        .find("\"actors\"")
        .ok_or_else(|| WinError::Render("dressing missing \"actors\"".to_string()))?;
    let tail = text
        .get(anchor..)
        .ok_or_else(|| WinError::Render("dressing missing \"actors\"".to_string()))?;
    let open = tail
        .find('[')
        .ok_or_else(|| WinError::Render("dressing missing \"actors\"".to_string()))?;
    let mut cursor = anchor + open + 1;
    let mut out: [DressingActor; DRESSING_ACTOR_COUNT] = [DressingActor {
        id: 1,
        asset: DressingAsset::Torch,
        pos_m: [0.0, 0.0, 0.0],
        size_m: [1.0, 1.0, 1.0],
    }; DRESSING_ACTOR_COUNT];
    let mut count = 0usize;
    while count < DRESSING_ACTOR_COUNT {
        let rest = text
            .get(cursor..)
            .ok_or_else(|| WinError::Render("dressing actor missing".to_string()))?;
        let open_rel = rest
            .find('{')
            .ok_or_else(|| WinError::Render("dressing actor missing".to_string()))?;
        let body = rest.get(open_rel + 1..).unwrap_or("");
        let close_rel = body
            .find('}')
            .ok_or_else(|| WinError::Render("dressing actor missing".to_string()))?;
        let block = body.get(..close_rel).unwrap_or("");
        out[count] = dressing_actor_from_block(block)?;
        count += 1;
        cursor += open_rel + 1 + close_rel + 1;
    }
    Ok(out)
}

/// Parse the embedded `content/arena/dressing.json` table. Same bytes give
/// the same actors; a drifted file fails loudly at staging time.
pub fn dressing_actors() -> Result<[DressingActor; DRESSING_ACTOR_COUNT], WinError> {
    dressing_actors_from_json(DRESSING_JSON)
}

/// Stable mesh key per dressing actor. Never collides with the central
/// cube key: a distinct base id plus the actor id baked into the digest.
#[must_use]
pub fn dressing_mesh_key(actor: &DressingActor) -> MeshKey {
    MeshKey::new(
        DRESSING_MESH_ID_BASE + u64::from(actor.id),
        [actor.id.to_le_bytes()[0]; 32],
    )
}

/// Transformed cube mesh for one actor: the unit cube scaled by the full
/// extents and translated to the box center. Normals stay axis-aligned
/// (boxes only); UVs and indices ride through. Validation failures are
/// typed, never panics.
pub fn dressing_mesh(actor: &DressingActor) -> Result<engine_render::v1::MeshData, WinError> {
    let cube = engine_render::v1::cube();
    let mut positions = Vec::with_capacity(cube.positions.len());
    for p in &cube.positions {
        positions.push([
            actor.pos_m[0] + p[0] * actor.size_m[0],
            actor.pos_m[1] + p[1] * actor.size_m[1],
            actor.pos_m[2] + p[2] * actor.size_m[2],
        ]);
    }
    engine_render::v1::upload_mesh(&positions, &cube.normals, &cube.uvs, &cube.indices)
        .map_err(|err| WinError::Render(format!("dressing mesh: {err}")))
}

/// PBR factors per asset: torches read warm and bright, banners deep red;
/// both rough dielectrics against the central copper cube. Always valid.
#[must_use]
pub fn dressing_material(asset: DressingAsset) -> engine_render::v1::PbrMaterial {
    match asset {
        DressingAsset::Torch => engine_render::v1::PbrMaterial {
            base_color: [0.95, 0.55, 0.20],
            metallic: 0.0,
            roughness: 0.6,
            exposure: 1.0,
        },
        DressingAsset::Banner => engine_render::v1::PbrMaterial {
            base_color: [0.55, 0.08, 0.08],
            metallic: 0.0,
            roughness: 0.8,
            exposure: 1.0,
        },
    }
}

/// Twin establishing camera: west of the walls looking down the
/// walls-to-gate axis at the gate court. Aspect follows the frame; bad
/// sizes fail loudly instead of touching the GPU.
pub fn dressing_camera(width: u32, height: u32) -> Result<engine_render::v1::Camera, WinError> {
    if SizeClass::classify(width, height) != SizeClass::Sizes {
        return Err(WinError::Render(format!(
            "dressing frame size {width}x{height} rejected"
        )));
    }
    let mut camera = engine_render::v1::Camera::new(
        DRESSING_EYE_M,
        DRESSING_TARGET_M,
        [0.0, 1.0, 0.0],
        DRESSING_FOV_DEG.to_radians(),
    );
    let wide =
        u16::try_from(width).map_err(|_| WinError::Render("dressing camera aspect".to_string()))?;
    let high = u16::try_from(height)
        .map_err(|_| WinError::Render("dressing camera aspect".to_string()))?;
    camera.aspect = f32::from(wide) / f32::from(high);
    camera.far = 120.0;
    camera
        .validate()
        .map_err(|err| WinError::Render(format!("dressing camera: {err}")))?;
    Ok(camera)
}

/// Upload every dressing mesh into the twin. Cold init path only (heap per
/// mesh here, zero per frame later). Parse failures are loud; a refused
/// GPU upload skips that actor (like the central cube) so one bad actor
/// never blanks the frame. Returns staged actors uploaded.
pub fn stage_dressing(presenter: &mut Presenter) -> Result<usize, WinError> {
    let actors = dressing_actors()?;
    let mut uploaded = 0usize;
    for actor in &actors {
        let key = dressing_mesh_key(actor);
        let mesh = dressing_mesh(actor)?;
        if presenter.upload_mesh(key, &mesh).is_ok() {
            uploaded += 1;
        }
    }
    Ok(uploaded)
}

/// LOD plus camera distance of one dressing actor for an eye position: the
/// distance in whole millimeters through the asset LOD bands ([`target_lod`]
/// from LOD0, so the band pick is exact). A refused distance (non-finite
/// position) measures 0 mm and holds LOD0 instead of refusing the frame.
fn dressing_lod_and_mm(actor: &DressingActor, eye_m: [f32; 3]) -> (usize, u32) {
    let distance_mm = distance_mm_between(eye_m, actor.pos_m).unwrap_or(0);
    (target_lod(distance_mm, 0, MAX_LOD_LEVELS), distance_mm)
}

/// Resident LOD of one dressing actor for an eye position, through the
/// asset LOD bands. The twin frame path calls this per actor per frame, so
/// the asset LOD module has a live production caller outside its own tests.
#[must_use]
pub fn dressing_lod(actor: &DressingActor, eye_m: [f32; 3]) -> usize {
    dressing_lod_and_mm(actor, eye_m).0
}

/// Push one draw per resident dressing mesh, nearest band first: the asset
/// LOD bands order submission front-to-back, so tiled GPUs shade less
/// twice. Same set as before (missing keys still draw nothing); order is
/// the only change, and opaque depth-tested boxes paint identical pixels
/// in any order. Cold per-frame tail (stack parse, no mesh rebuild).
/// Returns draws pushed.
pub fn push_dressing_draws(list: &mut DrawList, resident: &[MeshKey]) -> Result<usize, WinError> {
    let actors = dressing_actors()?;
    let eye = list.camera.eye.to_array();
    // Bounded stack rank, nearest band first; ties keep file order (the
    // slot breaks them), so the same frame always submits the same order.
    let mut rank: [(usize, u32, usize); DRESSING_ACTOR_COUNT] = [(0, 0, 0); DRESSING_ACTOR_COUNT];
    for (slot, actor) in actors.iter().enumerate() {
        let (lod, distance_mm) = dressing_lod_and_mm(actor, eye);
        rank[slot] = (lod, distance_mm, slot);
    }
    for i in 1..DRESSING_ACTOR_COUNT {
        let key = rank[i];
        let mut j = i;
        while j > 0 && rank[j - 1] > key {
            rank[j] = rank[j - 1];
            j -= 1;
        }
        rank[j] = key;
    }
    let mut pushed = 0usize;
    for (_, _, slot) in rank {
        let actor = &actors[slot];
        let key = dressing_mesh_key(actor);
        if resident.contains(&key) {
            let material = dressing_material(actor.asset);
            material
                .validate()
                .map_err(|err| WinError::Render(format!("dressing material: {err}")))?;
            list.items.push(engine_render::present::DrawItem {
                mesh: key,
                material,
                base_color: None,
                normal_map: None,
            });
            pushed += 1;
        }
    }
    Ok(pushed)
}
