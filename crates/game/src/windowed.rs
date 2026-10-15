//! windowed: windowed play behind the `app` feature.
//!
//! Consumer: operator play on the reference GPU; the editor launches the `arena`
//! binary as a child process. Data owner: this module owns the windowed
//! session driver (winit event loop, fixed-tick accumulator, input bindings,
//! device audio, presenter HUD); match authority stays in `engine-arena`,
//! envelope/storage in `engine-save`. Units: fixed ticks at the arena rate
//! (60 Hz), presented frames per redraw, wall elapsed in milliseconds.
//! Determinism scope: the same [`GameConfig`](super::GameConfig) plus the
//! same bound drives the same checkpoints as the headless leg (replay
//! equivalence is tested, not claimed). Errors: typed [`WinError`]; a missing
//! display fails loudly, a lost audio device degrades with a notice while the
//! headless mixer keeps rendering voices.

use std::sync::{Arc, Mutex};

use engine_assets::lod_swap::{distance_mm_between, target_lod};
use engine_assets::mesh_cook::MAX_LOD_LEVELS;
use engine_audio::{device::DeviceOutput, Bank, Mixer};
use engine_input::{Bindings, FighterId, FighterInput, PhysicalButton};
use engine_render::{
    backend::{BackendChoice, BackendError},
    present::{DrawList, MeshKey, PresentError, Presenter, SizeClass},
    window::{WindowShell, FIXED_DT},
};
use engine_ui::{
    text::{emit_quads, feed_lines, hp_text, layout, Align, Atlas},
    DuelOverlay, Hud,
};

use super::audio_mix::{Material, MixBridge, MixVoice, VoiceSeed, Wall, MAX_MIX_VOICES};
use super::camera::DirectorParams;
use super::camera_feed::PresentationFeed;
use super::{App, Checkpoint, GameConfig, GameError, TickOutcome, HUMAN_SLOT};

#[path = "windowed_smoke.rs"]
mod windowed_smoke;
pub use windowed_smoke::{
    check_exit_deadline_with_limit, format_timed_smoke, surface_error_is_loud,
};

#[path = "staged_fight_frames.rs"]
mod staged_fight_frames;
pub use staged_fight_frames::{
    fight_body_material, fight_body_mesh, fight_mesh_key, push_fight_draws,
    run_staged_fight_frames, split_staged_flag, stage_fight_bodies, staged_ct7_body_shas,
    staged_ct7_bundle_shas, staged_fight_frame_list, staged_fight_frame_png,
    staged_fight_frame_rgba, staged_fight_input, staged_frame_manifest, staged_session_config,
    StagedFightFrame, StagedFightReport, StagedFighterDot, StagedTally, FIGHT_BODY_SIZE_M,
    FIGHT_MESH_ID_BASE, STAGED_FIGHT_FIGHTERS, STAGED_FIGHT_FLAG, STAGED_FIGHT_FRAMES,
    STAGED_FIGHT_HALF_EXTENT_M, STAGED_FIGHT_HITS, STAGED_FIGHT_KNOCKDOWNS, STAGED_FIGHT_SEED,
    STAGED_FIGHT_TICKS,
};

/// Mixer rate for the windowed leg (the bank bakes at this rate too).
pub const WINDOWED_SAMPLE_RATE: u32 = 48_000;
/// Mixer frames rendered per fixed tick (one second of audio per 60 ticks).
pub const FRAMES_PER_TICK: usize = 800;
/// Windowed smoke length in ticks (2 s at 60 Hz).
pub const SMOKE_TICKS: u32 = 120;
/// Window size opened for the smoke.
pub const WINDOW_SIZE: (u32, u32) = (1280, 720);
/// Fixed step in seconds (60 Hz); mirrors the shell accumulator.
pub const WIN_FIXED_DT_S: f64 = 1.0 / 60.0;
/// Fixed step in milliseconds (the accumulator input unit).
pub const WIN_FIXED_DT_MS: f64 = 1000.0 / 60.0;
/// Most fixed steps per presented frame (spiral-of-death clamp).
pub const WIN_MAX_STEPS_PER_FRAME: u32 = 5;
/// Offscreen smoke frame size (small for fast CI; windowed play uses
/// [`WINDOW_SIZE`]).
pub const SMOKE_FRAME_SIZE: (u32, u32) = (128, 96);
/// Own wall-clock bound for automated `--exit` runs in milliseconds (5 min):
/// a hung smoke fails itself loudly within minutes instead of burning a core
/// for hours. Overridable per run with `--deadline-ms`.
pub const SMOKE_WALL_LIMIT_MS: u32 = 300_000;
/// Cap for display/audio backend messages carried in errors and notices.
pub const MAX_BACKEND_MSG: usize = 256;

/// Windowed failures: bounded diagnostics, never panics. Audio loss is not an
/// error: opening the device returns a notice string and the headless mixer
/// keeps rendering voices offline.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum WinError {
    /// CLI row rejected (`--frames` missing/not a number/out of range,
    /// unknown flag); culprit carried.
    BadArgs(String),
    /// Config row outside its range, row name carried.
    BadConfig(&'static str),
    /// Session failure from the headless leg.
    Game(GameError),
    /// No display: event-loop or window creation failed with the backend
    /// message. Callers fall back to the headless smoke, never panic.
    NoDisplay(String),
    /// Presenter failure (device/surface/frame).
    Present(PresentError),
    /// Automated `--exit` smoke overran its own wall-clock bound; the
    /// process ends loudly instead of burning a core behind an open window.
    Timeout(String),
    /// HUD scene build failure (the default scene is valid, so this is the
    /// loud unreachable path); message carried.
    Render(String),
}

impl core::fmt::Display for WinError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadArgs(row) => write!(f, "bad arena arg ({row})"),
            Self::BadConfig(row) => write!(f, "bad arena config ({row})"),
            Self::Game(err) => write!(f, "arena game error ({err})"),
            Self::NoDisplay(msg) => write!(f, "no display ({msg})"),
            Self::Present(err) => write!(f, "presenter error ({err})"),
            Self::Timeout(msg) => write!(f, "exit timeout ({msg})"),
            Self::Render(msg) => write!(f, "render error ({msg})"),
        }
    }
}

impl std::error::Error for WinError {}

impl From<GameError> for WinError {
    fn from(err: GameError) -> Self {
        Self::Game(err)
    }
}

impl From<PresentError> for WinError {
    fn from(err: PresentError) -> Self {
        Self::Present(err)
    }
}

/// Bound a backend message so errors and notices stay small and printable.
pub fn cap_message(msg: &str) -> String {
    let out: String = msg.chars().take(MAX_BACKEND_MSG).collect();
    if out.len() < msg.len() {
        format!("{out}...")
    } else {
        out
    }
}

/// Canonical capture scene id the review path can select (the sand arena).
pub const ARENA_SCENE_ID: &str = "arena";
/// Fighter count for the 8-fighter capture (arena plus 8).
pub const ARENA8_FIGHTERS: u8 = 8;
/// Arena half-extent for the capture scene in meters.
pub const ARENA_HALF_EXTENT_M: f32 = 20.0;

/// Capture scene selector: today only the arena exists; unknown names fail
/// loudly at parse so a typo never silently captures the wrong scene.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SceneSelect {
    /// 20x20 m sand arena with the gate tunnel (capture fixture).
    Arena,
}

impl SceneSelect {
    /// Parse `--scene <name>`; only `arena` is accepted.
    pub fn parse(raw: &str) -> Result<Self, WinError> {
        match raw {
            "arena" => Ok(Self::Arena),
            _ => Err(WinError::BadArgs(format!("--scene {raw}"))),
        }
    }

    /// Canonical scene id string.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Arena => ARENA_SCENE_ID,
        }
    }
}

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
            });
            pushed += 1;
        }
    }
    Ok(pushed)
}

/// Fixed seed for the arena600 camera path (mirrors
/// [`engine_render::backend::ARENA600_SEED`]; the value is repeated here so
/// the CLI layer reads without a dependency edge beyond `BackendChoice`).
pub const ARENA600_SEED: u64 = 2040;
/// Fixed frame count for the arena600 camera path (mirrors
/// [`engine_render::backend::ARENA600_FRAMES`]).
pub const ARENA600_FRAMES: u32 = 600;

/// Capture camera-path selector: today only the fixed arena600 orbit exists;
/// unknown names fail loudly at parse so a typo never silently captures the
/// wrong path. Pure and headless-safe.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CameraPath {
    /// Fixed seed-2040 600-frame arena orbit.
    Arena600,
}

impl CameraPath {
    /// Parse `--camera-path <name>`; only `arena600` is accepted.
    pub fn parse(raw: &str) -> Result<Self, WinError> {
        match raw {
            "arena600" => Ok(Self::Arena600),
            _ => Err(WinError::BadArgs(format!("--camera-path {raw}"))),
        }
    }

    /// Canonical path id string.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Arena600 => "arena600",
        }
    }

    /// Session seed pinned by this path.
    #[must_use]
    pub fn seed(self) -> u64 {
        match self {
            Self::Arena600 => ARENA600_SEED,
        }
    }

    /// Frame count pinned by this path.
    #[must_use]
    pub fn frames(self) -> u32 {
        match self {
            Self::Arena600 => ARENA600_FRAMES,
        }
    }
}

/// Pin the session coordinates for a capture camera path: `Arena600` fixes
/// the seed to [`ARENA600_SEED`] and floors the tick budget at
/// [`ARENA600_FRAMES`] unless an explicit `--ticks` wins; `--frames` still
/// caps the run later in the binary. `None` keeps free play. Pure.
#[must_use]
pub fn pin_session_for_camera_path(
    seed: u64,
    ticks: u32,
    path: Option<CameraPath>,
    ticks_flag: Option<u32>,
) -> (u64, u32) {
    let Some(path) = path else {
        return (seed, ticks);
    };
    match path {
        CameraPath::Arena600 => {
            let floor = ticks_flag.unwrap_or_else(|| ticks.max(ARENA600_FRAMES));
            (path.seed(), floor)
        }
    }
}

/// Orbit eye in meters for one captured frame under a camera path:
/// `Arena600` flies the fixed seed-2040 orbit from
/// [`engine_render::backend::arena600_eye`]; `None` keeps free play with no
/// forced eye. Pure presentation math; the tick never sees it.
#[must_use]
pub fn orbit_eye_for_frame(frame: u32, path: Option<CameraPath>) -> Option<[f32; 3]> {
    path.map(|selected| match selected {
        CameraPath::Arena600 => engine_render::backend::arena600_eye(frame),
    })
}

/// Steer one HUD draw list onto the capture orbit: `Arena600` parks the
/// camera eye on the fixed seed-2040 orbit for `frame`; `None` leaves free
/// play untouched. Pure presentation math; the tick never sees it.
pub fn apply_camera_orbit(list: &mut DrawList, frame: u32, path: Option<CameraPath>) {
    if let Some(eye) = orbit_eye_for_frame(frame, path) {
        list.camera.eye = eye.into();
    }
}

/// Map a backend-label failure into the typed CLI refusal.
fn backend_arg(context: &str, err: BackendError) -> WinError {
    WinError::BadArgs(format!("{context} {err}"))
}

/// Fetch the value after a flag at `rest[index]`; a missing value fails
/// loudly naming the flag. Split out of [`WinArgs::parse`] (fn-length bar).
fn flag_value<'rest>(
    rest: &'rest [String],
    index: usize,
    flag: &str,
) -> Result<&'rest str, WinError> {
    rest.get(index)
        .map(String::as_str)
        .ok_or_else(|| WinError::BadArgs(format!("{flag} needs a value")))
}

/// Parse one `--backend` label: unknown labels fail loudly, never defaulted
/// silently. Split out of [`WinArgs::parse`] (fn-length bar).
fn parse_backend_flag(raw: &str) -> Result<BackendChoice, WinError> {
    BackendChoice::from_name(raw).map_err(|err| backend_arg("--backend", err))
}

/// Parsed `arena` CLI: `--frames N` caps presented frames (and fixed ticks),
/// `--exit` exits after the cap instead of idling with the window open.
/// `--seed S` and `--ticks T` override the session coordinates.
/// `--fighters N` selects the fighter count (8-fighter captures use 8),
/// `--scene arena` selects the capture scene, `--timed-smoke` prints wall
/// elapsed plus fps on the smoke path so fps is measurable. `--backend
/// <auto|dx12|vulkan|gl>` selects the GPU backend (reuses
/// [`engine_render::backend::BackendChoice`]; overrides `WGPU_BACKEND`) and
/// `--camera-path arena600` pins the fixed seed-2040 600-frame orbit. Pure.
/// `--mute` never opens an audio device (automated smoke stays silent).
/// `--deadline-ms N` overrides the `--exit` wall-clock bound; `--present-every
/// N` presents every Nth headless tick and `--present-skip` presents none
/// (ticks still play every tick, hashes unchanged; 1 is today's behavior).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct WinArgs {
    /// Fixed ticks (and frames) to run when `Some`; unbounded window play
    /// when `None`.
    pub frames: Option<u32>,
    /// Exit after `frames` instead of keeping the window open.
    pub exit: bool,
    /// Session seed override (`None` keeps the default).
    pub seed: Option<u64>,
    /// Session tick-count override (`None` keeps the default).
    pub ticks: Option<u32>,
    /// Fighter-count override (`None` keeps the default duel).
    pub fighters: Option<u8>,
    /// Capture scene override (`None` keeps the arena default).
    pub scene: Option<SceneSelect>,
    /// Print wall elapsed plus fps on the smoke path.
    pub timed_smoke: bool,
    /// GPU backend override (`None` keeps env/compiled-in default).
    pub backend: Option<BackendChoice>,
    /// Capture camera-path override (`None` keeps free play).
    pub camera_path: Option<CameraPath>,
    /// Never open an audio device; mixer voices render offline.
    /// Every automated smoke and capture passes this, so tests never make sound.
    pub mute: bool,
    /// `--exit` wall-clock bound override in milliseconds (`None` keeps
    /// [`SMOKE_WALL_LIMIT_MS`]).
    pub deadline_ms: Option<u32>,
    /// Ticks per presented frame (`1` presents every tick, today's behavior).
    pub present_every: u32,
    /// Present no frame at all; ticks still play every tick.
    pub present_skip: bool,
}

impl WinArgs {
    /// Effective `--exit` wall-clock bound in milliseconds: the
    /// `--deadline-ms` override when present, else [`SMOKE_WALL_LIMIT_MS`].
    /// Pure, so the smoke loop and the tests share it exactly.
    #[must_use]
    pub fn wall_limit_ms(self) -> u32 {
        self.deadline_ms.unwrap_or(SMOKE_WALL_LIMIT_MS)
    }
}

/// Measured fps for a timed smoke: presented frames per wall second.
/// Non-finite or non-positive elapsed and zero frames fail loudly.
pub fn smoke_fps(frames: u32, elapsed_ms: f64) -> Result<f64, WinError> {
    if frames == 0 {
        return Err(WinError::BadArgs(
            "timed smoke needs frames > 0".to_string(),
        ));
    }
    if !elapsed_ms.is_finite() || elapsed_ms <= 0.0 {
        return Err(WinError::BadArgs(format!(
            "timed smoke elapsed_ms {elapsed_ms} is not finite positive"
        )));
    }
    Ok(f64::from(frames) / (elapsed_ms / 1000.0))
}

/// Wall-clock bound for one automated `--exit` run in milliseconds (5 min):
/// a hung smoke fails itself with a loud line instead of burning a core
/// behind an open window (the operator reaper stays the backstop, never the
/// first gate).
pub const EXIT_WALL_LIMIT_MS: f64 = 300_000.0;

/// Muted-smoke notice: no audio device is opened on the automated path or
/// under `--mute`; mixer voices render offline. One constant so the string
/// can never drift from the guarantee.
pub const MUTED_AUDIO_NOTICE: &str = "audio muted: no device opened; mixer voices render offline";

/// True while this run is automated smoke: `--mute` mutes explicitly, while
/// `--exit` and `--timed-smoke` runs are automated by construction, so they
/// mute too. Pure so the binary and tests share it.
#[must_use]
pub fn smoke_muted(args: &WinArgs) -> bool {
    args.mute || args.exit || args.timed_smoke
}

/// Check one `--exit` elapsed reading against the wall-clock bound.
/// Over-limit, negative and non-finite readings fail loudly; pure so tests
/// force the deadline case without waiting out the bound.
pub fn check_exit_deadline(elapsed_ms: f64) -> Result<(), WinError> {
    if !elapsed_ms.is_finite() || elapsed_ms < 0.0 {
        return Err(WinError::Timeout(format!(
            "exit elapsed_ms {elapsed_ms} is not finite nonnegative"
        )));
    }
    if elapsed_ms > EXIT_WALL_LIMIT_MS {
        return Err(WinError::Timeout(format!(
            "exit ran {elapsed_ms:.0}ms past the {EXIT_WALL_LIMIT_MS:.0}ms wall-clock bound"
        )));
    }
    Ok(())
}

/// What the window loop does with one `render_surface` outcome. Transient
/// acquire states skip the frame (Lost/Outdated already reconfigured inside
/// the call); everything else hard fails loudly so an unpainted window can
/// never report green.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SurfaceFrameAction {
    /// A visible frame reached the surface.
    Presented,
    /// Nothing reached the surface this redraw (suspended, or a transient
    /// acquire state); retry next frame.
    Skipped,
    /// No visible frame can be presented; end loudly.
    Failed,
}

/// Map one `render_surface` outcome to the window-loop action: `Ok` with
/// `presented` is the only green path, `Ok` suspended plus the transient
/// acquire errors skip, every other error fails. Pure so tests pin it.
#[must_use]
pub fn surface_frame_action(result: Result<bool, &PresentError>) -> SurfaceFrameAction {
    match result {
        Ok(true) => SurfaceFrameAction::Presented,
        Ok(false) => SurfaceFrameAction::Skipped,
        Err(
            PresentError::SurfaceTimeout
            | PresentError::SurfaceOccluded
            | PresentError::SurfaceLost { .. }
            | PresentError::SurfaceOutdated { .. },
        ) => SurfaceFrameAction::Skipped,
        Err(_) => SurfaceFrameAction::Failed,
    }
}

/// Apply one CLI flag to the in-progress [`WinArgs`]: value flags consume
/// the next token, bare flags flip a bool, unknown flags fail loudly.
/// Split out of [`WinArgs::parse`] (fn-length bar).
fn apply_win_flag(
    out: &mut WinArgs,
    flag: &str,
    rest: &[String],
    index: &mut usize,
) -> Result<(), WinError> {
    match flag {
        "--frames" => {
            *index += 1;
            let raw = flag_value(rest, *index, "--frames")?;
            let value: u32 = raw
                .parse()
                .map_err(|_| WinError::BadArgs(format!("--frames {raw}")))?;
            if value == 0 || value > super::MAX_TICKS {
                return Err(WinError::BadArgs(format!("--frames {raw}")));
            }
            out.frames = Some(value);
        }
        "--exit" => out.exit = true,
        "--seed" => {
            *index += 1;
            let raw = flag_value(rest, *index, "--seed")?;
            let value: u64 = raw
                .parse()
                .map_err(|_| WinError::BadArgs(format!("--seed {raw}")))?;
            out.seed = Some(value);
        }
        "--ticks" => {
            *index += 1;
            let raw = flag_value(rest, *index, "--ticks")?;
            let value: u32 = raw
                .parse()
                .map_err(|_| WinError::BadArgs(format!("--ticks {raw}")))?;
            if value == 0 || value > super::MAX_TICKS {
                return Err(WinError::BadArgs(format!("--ticks {raw}")));
            }
            out.ticks = Some(value);
        }
        "--fighters" => {
            *index += 1;
            let raw = flag_value(rest, *index, "--fighters")?;
            out.fighters = Some(windowed_smoke::parse_fighter_count(raw)?);
        }
        "--scene" => {
            *index += 1;
            let raw = flag_value(rest, *index, "--scene")?;
            out.scene = Some(SceneSelect::parse(raw)?);
        }
        "--timed-smoke" => out.timed_smoke = true,
        "--backend" => {
            *index += 1;
            let raw = flag_value(rest, *index, "--backend")?;
            out.backend = Some(parse_backend_flag(raw)?);
        }
        "--camera-path" => {
            *index += 1;
            let raw = flag_value(rest, *index, "--camera-path")?;
            out.camera_path = Some(CameraPath::parse(raw)?);
        }
        "--mute" => out.mute = true,
        "--deadline-ms" => {
            *index += 1;
            let raw = flag_value(rest, *index, "--deadline-ms")?;
            out.deadline_ms = Some(windowed_smoke::parse_deadline_value(raw)?);
        }
        "--present-every" => {
            *index += 1;
            let raw = flag_value(rest, *index, "--present-every")?;
            out.present_every = super::parse_present_every(raw)
                .map_err(|_| WinError::BadArgs(format!("--present-every {raw}")))?;
        }
        "--present-skip" => out.present_skip = true,
        other => return Err(WinError::BadArgs(other.to_string())),
    }
    Ok(())
}

impl WinArgs {
    /// Parse the `arena` CLI (program name at index 0 is skipped when
    /// present). Unknown flags, a missing `--frames`/`--seed`/`--ticks`/
    /// `--fighters`/`--scene`/`--backend`/`--camera-path` value, unparsable
    /// numbers or backend/camera-path labels, a zero `--frames`
    /// and a fighter count outside `2..=16` fail loudly.
    /// `--deadline-ms` takes the same missing/zero/unparsable path, as does
    /// `--present-every` (needs `1..=MAX_TICKS`).
    pub fn parse(args: &[String]) -> Result<Self, WinError> {
        let mut out = Self {
            frames: None,
            exit: false,
            seed: None,
            ticks: None,
            fighters: None,
            scene: None,
            timed_smoke: false,
            backend: None,
            camera_path: None,
            mute: false,
            deadline_ms: None,
            present_every: super::DEFAULT_PRESENT_EVERY,
            present_skip: false,
        };
        // Skip argv[0] when it does not look like a flag (the program name).
        let rest: &[String] = match args.first() {
            Some(first) if !first.starts_with("--") => &args[1.min(args.len())..],
            _ => args,
        };
        let mut index = 0usize;
        while index < rest.len() {
            apply_win_flag(&mut out, &rest[index], rest, &mut index)?;
            index += 1;
        }
        Ok(out)
    }
}

/// Fixed-tick accumulator with interpolation: wall milliseconds in, fixed
/// ticks out, `alpha` for render blending. Pure math, no clock inside, so
/// tests drive it exactly. NaN/non-positive elapsed steps nothing.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TickAccumulator {
    /// Buffered wall seconds awaiting fixed steps.
    acc_s: f64,
}

impl TickAccumulator {
    /// Empty accumulator (nothing buffered).
    pub fn new() -> Self {
        Self { acc_s: 0.0 }
    }

    /// Push wall `elapsed_ms` and drain whole fixed steps, at most
    /// [`WIN_MAX_STEPS_PER_FRAME`]. Non-finite or non-positive input steps
    /// nothing and buffers nothing.
    pub fn push_elapsed_ms(&mut self, elapsed_ms: f64) -> u32 {
        if !elapsed_ms.is_finite() || elapsed_ms <= 0.0 {
            return 0;
        }
        let clamped_s =
            (elapsed_ms / 1000.0).min(WIN_FIXED_DT_S * f64::from(WIN_MAX_STEPS_PER_FRAME));
        self.acc_s += clamped_s;
        let mut steps = 0u32;
        while self.acc_s >= WIN_FIXED_DT_S && steps < WIN_MAX_STEPS_PER_FRAME {
            self.acc_s -= WIN_FIXED_DT_S;
            steps += 1;
        }
        steps
    }

    /// Interpolation fraction into the next fixed tick (`acc / dt`,
    /// clamped `0.0..=1.0`); the renderer blends the last two ticks with it.
    pub fn alpha(self) -> f32 {
        ((self.acc_s / WIN_FIXED_DT_S) as f32).clamp(0.0, 1.0)
    }

    /// Buffered wall milliseconds awaiting steps (diagnostics only).
    pub fn pending_ms(self) -> f64 {
        (self.acc_s * 1000.0).max(0.0)
    }
}

impl Default for TickAccumulator {
    /// Empty accumulator (see [`TickAccumulator::new`]).
    fn default() -> Self {
        Self::new()
    }
}

/// Build one validated [`FighterInput`] from the currently pressed buttons
/// through the live bindings table: wishes from the movement rows, button
/// bits from the action rows, facing pinned to 0. The bindings rows only ever
/// emit wishes in `-100..=100` and legal button bits, so this always validates.
pub fn fighter_input_from_pressed(
    tick: u32,
    fighter: FighterId,
    pressed: &[PhysicalButton],
    bindings: &Bindings,
) -> FighterInput {
    let (mx, mz) = bindings.wish_from_pressed(pressed);
    FighterInput {
        tick,
        fighter,
        mx,
        mz,
        facing: 0,
        buttons: bindings.button_bits(pressed),
    }
}

/// Open the default cpal output feeding the shared mixer, unless `muted`:
/// muted runs never touch the audio device and keep the [`MUTED_AUDIO_NOTICE`]
/// string while mixer voices render offline. `Ok` keeps the
/// stream handle alive (dropping it stops playback); `Err` is not a failure:
/// the caller keeps the notice string (`audio degraded: ...`) and renders
/// mixer voices offline instead. Never panics.
pub fn open_audio(mixer: Arc<Mutex<Mixer>>, muted: bool) -> (Option<DeviceOutput>, String) {
    if muted {
        return (None, MUTED_AUDIO_NOTICE.to_string());
    }
    match DeviceOutput::open_default(mixer) {
        Ok(output) => {
            let notice = format!(
                "audio live: {}ch {}Hz",
                output.channels(),
                output.sample_rate()
            );
            (Some(output), notice)
        }
        Err(err) => (
            None,
            format!("audio degraded: {err}; mixer voices render offline"),
        ),
    }
}

/// Windowed mix listener in integer cm (arena center floor plane): inside the
/// tunnel send radius, so the played mix hears tails until per-fighter
/// seats land.
pub const WIN_LISTENER_CM: [i32; 2] = [0, 0];
/// Same seat in 3D for the reverb send query (`[x east, y up, z south]`).
pub const WIN_LISTENER_3CM: [i32; 3] = [0, 0, 0];

/// Placeholder arena occluder for the windowed mix: one charted stone wall at
/// x = 250 cm spanning z -500..500 (the audio muffle fixture). The live wall
/// set plus per-fighter voice positions are not wired in yet; until then
/// [`mix_tick_audio`] rings even-indexed voices at `[500, 0]` behind this
/// wall and odd-indexed ones at the listener's ears. Stack value, no heap.
fn win_walls() -> [Wall; 1] {
    [Wall::new(250, -500, 250, 500, Material::Stone)]
}

/// Mix stats for one tick's voices: how many played plus the summed reverb
/// send (the caller folds the mean into its report).
pub struct TickMixStats {
    /// Voices handed to the mixer, cap overflow included.
    pub voices: usize,
    /// Summed tunnel send over the capped mixed voices.
    pub send_sum: f64,
}

/// Play one tick's voices through the mix bridge on the game thread: query
/// the walls per voice, slew each slot, floor the sfx path at full muffle
/// and stamp the tunnel send, then hand the occluded gains to the mixer.
/// Borrows plus stack buffers only, no heap per call. A refused query (only
/// an over-cap wall set, never the single charted wall here) plays the tick
/// dry; voices past [`MAX_MIX_VOICES`] play dry rather than dropping.
pub fn mix_tick_audio(
    mixer: &mut Mixer,
    bank: &Bank,
    bridge: &mut MixBridge,
    walls: &[Wall],
    outcome: &TickOutcome,
) -> TickMixStats {
    let seed = VoiceSeed {
        pos_cm: WIN_LISTENER_CM,
        voice_bus: false,
        base_gain: 1.0,
    };
    let mut seeds = [seed; MAX_MIX_VOICES];
    let mut mixed = [MixVoice {
        gain: 1.0,
        send: 0.0,
    }; MAX_MIX_VOICES];
    let n = outcome.audio.len().min(MAX_MIX_VOICES);
    for (index, command) in outcome.audio.iter().take(n).enumerate() {
        seeds[index] = VoiceSeed {
            pos_cm: if index % 2 == 0 {
                [500, 0]
            } else {
                WIN_LISTENER_CM
            },
            voice_bus: false,
            base_gain: command.gain,
        };
    }
    // A refused query (only an over-cap wall set, never the single charted
    // wall here) plays the tick dry via the zero default.
    let got = bridge
        .update(
            &seeds[..n],
            WIN_LISTENER_CM,
            WIN_LISTENER_3CM,
            walls,
            &mut mixed[..n],
        )
        .unwrap_or_default();
    let mut send_sum = 0f64;
    for (command, voice) in outcome.audio.iter().take(got).zip(mixed.iter()) {
        mixer.play(bank.get(command.sound), command.gain * voice.gain);
        send_sum += f64::from(voice.send);
    }
    for command in outcome.audio.iter().skip(got) {
        mixer.play(bank.get(command.sound), command.gain);
    }
    TickMixStats {
        voices: outcome.audio.len(),
        send_sum,
    }
}

/// HUD text cells plus quads for one snapshot (hp line plus kill-feed lines
/// through the UI text path: layout cells, then one quad per cell). Pure.
pub fn hud_cells_quads(hud: &Hud) -> (usize, usize) {
    let mut lines = vec![hp_text(hud)];
    lines.extend(feed_lines(&hud.feed));
    let atlas = Atlas::new();
    let mut cells = 0usize;
    let mut quads = 0usize;
    for line in &lines {
        let laid = layout(line, 32, Align::Left);
        let (_, _, indices) = emit_quads(&laid, atlas, 1.0);
        cells += laid.len();
        quads += indices.len() / 6;
    }
    (cells, quads)
}

/// Build the live duel overlay for one app snapshot: tick from the HUD clock,
/// one named row per live fighter slot with permille health from the match.
/// Consumer: the headless smoke draws this after each presented twin frame so
/// the built `DuelOverlay` has a live caller outside its own tests; the tick
/// never sees it (presentation only, pixels never enter the sim hash). Heap on
/// the presentation path only (row names inside the overlay), zero per tick.
#[must_use]
pub fn duel_overlay_for_app(app: &App) -> DuelOverlay {
    const NAMES: [&str; 16] = [
        "F0", "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "F13",
        "F14", "F15",
    ];
    let mut overlay = DuelOverlay::new(app.hud().tick);
    let live = usize::from(app.config().fighters);
    for (slot, name) in NAMES.iter().enumerate().take(live) {
        let hp = app.hp().get(slot).copied().unwrap_or(0);
        if let Ok(byte) = u8::try_from(slot) {
            if let Some(id) = FighterId::new(byte) {
                let _ = overlay.upsert_fighter(id, *name, hp);
            }
        }
    }
    overlay
}

/// Clear-only draw list for one HUD frame: shared camera plus light from
/// the default scene at `width` x `height`, no draws (the HUD text/quads from
/// [`hud_cells_quads`] ride the frame receipt, not the draw list; fighter
/// meshes land with the asset bundle loader). Over-bound or zero sizes fail
/// loudly instead of touching the GPU.
pub fn hud_draw_list(width: u32, height: u32) -> Result<DrawList, WinError> {
    if SizeClass::classify(width, height) != SizeClass::Sizes {
        return Err(WinError::Render(format!(
            "hud frame size {width}x{height} rejected"
        )));
    }
    let scene = engine_render::v1::default_scene(width, height)
        .map_err(|err| WinError::Render(cap_message(&format!("{err}"))))?;
    Ok(DrawList {
        camera: scene.camera,
        light: scene.light,
        items: Vec::new(),
    })
}

/// Windowed session report: everything the smoke ran, as data. Units: ticks
/// are fixed 60 Hz steps, frames are presented (or offscreen twin) frames,
/// audio counts offline mixer frames (0 while a device stream is live).
#[derive(Clone, PartialEq, Debug)]
pub struct WindowedReport {
    /// Fixed ticks simulated.
    pub ticks_run: u32,
    /// Checkpoint of the last simulated tick, if any ran.
    pub last_checkpoint: Option<Checkpoint>,
    /// HUD text cells laid out (hp line plus kill-feed lines).
    pub hud_cells: usize,
    /// HUD quads emitted for the laid-out cells.
    pub hud_quads: usize,
    /// Mixer frames rendered offline across the session.
    pub mixed_frames: usize,
    /// Mean tunnel reverb send over the mixed voices (`0.0` when the session
    /// rang no voices; `Eq` stays off this report because this is a float).
    pub mean_send: f32,
    /// Audio device status: always a headless skip with its reason on this
    /// machine (no device is opened in CI); the mixer path is fully rendered.
    pub audio_device: &'static str,
    /// Window size opened.
    pub window_size: (u32, u32),
    /// Shell frames pumped.
    pub frames_pumped: u64,
    /// Camera frames presented through the camera feed (one per fixed tick).
    pub camera_frames: u32,
}

/// Windowed smoke report: fixed ticks plus presented frames plus HUD plus audio
/// and presenter notices. The same idle tape gives the headless checkpoints
/// (tested, not claimed).
#[derive(Clone, PartialEq, Debug)]
pub struct WinReport {
    /// Fixed ticks simulated.
    pub ticks_run: u32,
    /// Frames presented (offscreen twin in the headless smoke, swapchain in
    /// the winit loop; 0 with a skip notice when no adapter exists).
    pub frames_presented: u32,
    /// Checkpoint of the last simulated tick, if any ran.
    pub last_checkpoint: Option<Checkpoint>,
    /// HUD text cells laid out (hp line plus kill-feed lines).
    pub hud_cells: usize,
    /// HUD quads emitted for the laid-out cells.
    pub hud_quads: usize,
    /// Mixer frames rendered offline (0 while a device stream is live).
    pub mixed_frames: usize,
    /// Mean tunnel reverb send over the mixed voices (`0.0` when the smoke
    /// rang no voices).
    pub mean_send: f32,
    /// Audio status: live device line or degraded-with-reason notice.
    pub audio_notice: String,
    /// Presenter status: adapter/driver line or skip-with-reason notice.
    pub presenter_notice: String,
    /// Camera frames presented through the camera feed (one per fixed tick).
    pub camera_frames: u32,
}

/// Run windowed play headless-safe: open the render window shell, feed one
/// idle human input per fixed tick through the default input bindings (AI
/// brains fill every other slot, same as the headless leg), play each tick's
/// audio commands through the headless mixer, and lay out the HUD text plus
/// quads. Bounded by `ticks` (and the session length); device audio is
/// skipped with a reason. Never panics on missing display/audio hardware:
/// there is none to miss yet (see the module docs).
pub fn run_windowed(config: GameConfig, ticks: u32) -> Result<WindowedReport, GameError> {
    let mut app = App::new(config)?.into_duel()?;
    let mut shell = WindowShell::new();
    shell.open(WINDOW_SIZE.0, WINDOW_SIZE.1);
    let bindings = Bindings::new();
    let bank = Bank::placeholder(WINDOWED_SAMPLE_RATE);
    let mut mixer = Mixer::new(WINDOWED_SAMPLE_RATE);
    let human = FighterId::new(HUMAN_SLOT).ok_or(GameError::BadConfig("fighters"))?;

    let ticks_run = ticks.min(config.ticks);
    let mut mixed_frames = 0usize;
    // Mix state: the tunnel preset validates once here (48 kHz is a
    // proven local invariant, see `audio_mix` tests), then every tick's
    // voices run through it before the mixer.
    let mut bridge = MixBridge::new(WINDOWED_SAMPLE_RATE).expect("48 kHz tunnel params validate");
    let walls = win_walls();
    let mut send_sum = 0f64;
    let mut send_voices = 0u64;
    // Reused offline mix buffer (one heap alloc here, zero per tick): the
    // per-tick path fills it via `render_into` instead of `render`.
    let mut scratch = vec![0.0f32; FRAMES_PER_TICK];
    // Presentation feed: the director follows every accepted tick.
    // Trauma folds from the tick's live cues (in-sim record); shake stays
    // presentation-side inside the director, never in the checkpoints.
    let mut feed = PresentationFeed::new(HUMAN_SLOT, DirectorParams::default())
        .expect("arena camera table is valid");
    for tick in 0..ticks_run {
        // Idle wish through the live bindings table (no keys pressed): this
        // is the bindings-to-FighterInput path windowed play uses, with an
        // empty frame rather than a scripted tape.
        let pressed: &[_] = &[];
        let (mx, mz) = bindings.wish_from_pressed(pressed);
        let input = FighterInput {
            tick,
            fighter: human,
            mx,
            mz,
            facing: 0,
            buttons: bindings.button_bits(pressed),
        };
        let outcome = app.tick(input);
        debug_assert!(outcome.input_accepted);
        let _ = feed.win_frame(&outcome);
        let stats = mix_tick_audio(&mut mixer, &bank, &mut bridge, &walls, &outcome);
        send_sum += stats.send_sum;
        send_voices += stats.voices as u64;
        mixer.render_into(&mut scratch);
        mixed_frames += scratch.len();
        shell.pump(FIXED_DT, u64::from(tick), |_| {});
    }

    // HUD presentation: hp line plus kill-feed lines through the UI text
    // path (layout cells, then one quad per cell for the renderer).
    let (hud_cells, hud_quads) = hud_cells_quads(app.hud());

    Ok(WindowedReport {
        ticks_run,
        last_checkpoint: app.checkpoints().last().copied(),
        hud_cells,
        hud_quads,
        mixed_frames,
        mean_send: if send_voices > 0 {
            (send_sum / send_voices as f64) as f32
        } else {
            0.0
        },
        audio_device:
            "skipped: no audio device opened in headless CI; mixer voices rendered offline",
        window_size: shell.size(),
        frames_pumped: shell.draw_count(),
        camera_frames: feed.len() as u32,
    })
}

/// Open the smoke twin when an adapter exists: cube plus dressing plus the
/// arena kit staged, skip with `None` when no adapter exists. A refused kit
/// stage keeps the cube/dressing frame (the cube is the visible error mesh),
/// never fails the smoke. Never panics without hardware.
fn smoke_presenter() -> Result<Option<(Presenter, MeshKey, super::arena_kit::KitReport)>, WinError>
{
    match Presenter::init_offscreen() {
        Ok(mut presenter) => {
            presenter.resize(SMOKE_FRAME_SIZE.0, SMOKE_FRAME_SIZE.1)?;
            // One lit cube proves the forward pass; a failed upload falls
            // back to clear-only frames rather than failing the smoke.
            let key = MeshKey::new(1, [0xA1; 32]);
            let _ = presenter.upload_mesh(key, &engine_render::v1::cube());
            // Dressing: stage the torch/banner boxes beside the cube;
            // a refused upload skips those draws, never the smoke.
            let _ = stage_dressing(&mut presenter);
            // Arena kit: the 8 glTF files fitted to the placement boxes;
            // a refused stage keeps the cube frame, never fails the smoke.
            let kit = super::arena_kit::stage_kit_or_default(&mut presenter);
            Ok(Some((presenter, key, kit)))
        }
        Err(PresentError::NoAdapter) => Ok(None),
        Err(err) => Err(WinError::Present(err)),
    }
}

/// Build one smoke twin draw list: HUD clear plus the lit cube plus the
/// staged dressing boxes plus the staged kit draws under the establishing
/// camera (or the capture orbit eye for `frame` under `path`; the tick never
/// sees it). Missing keys draw nothing; bad sizes fail loudly instead of
/// touching the GPU.
fn smoke_twin_list(
    presenter: &Presenter,
    key: MeshKey,
    kit: &super::arena_kit::KitReport,
    frame: u32,
    path: Option<CameraPath>,
) -> Result<DrawList, WinError> {
    let mut list = hud_draw_list(SMOKE_FRAME_SIZE.0, SMOKE_FRAME_SIZE.1)?;
    // Establishing view down the walls-to-gate axis: the same camera the
    // window twin presents, so both legs frame dressing.
    list.camera = dressing_camera(SMOKE_FRAME_SIZE.0, SMOKE_FRAME_SIZE.1)?;
    apply_camera_orbit(&mut list, frame, path);
    if presenter.mesh_keys().contains(&key) {
        let scene = engine_render::v1::default_scene(SMOKE_FRAME_SIZE.0, SMOKE_FRAME_SIZE.1)
            .map_err(|err| WinError::Render(cap_message(&format!("{err}"))))?;
        list.items.push(engine_render::present::DrawItem {
            mesh: key,
            material: scene.material,
            base_color: None,
        });
    }
    let resident = presenter.mesh_keys();
    let _ = push_dressing_draws(&mut list, &resident)?;
    // Kit draws in place of nothing: the cube stays lit beside them (the
    // visible error mesh), so a refused stage still paints a valid frame.
    let _ = super::arena_kit::push_kit_draws(&mut list, &resident, kit);
    // Kit prop rows ride the twin frame: physical placement boxes for the
    // brief (presentation only, hash untouched).
    let _ = super::arena_kit::kit_prop_rows().len();
    Ok(list)
}

/// Headless smoke: the `--frames N --exit` path with no display. Feeds
/// the same idle bindings inputs as [`run_windowed`] (same tape, same
/// checkpoints), renders mixer voices offline, lays out the HUD, and presents
/// one offscreen twin frame per tick when an adapter exists (skip with a
/// notice when [`PresentError::NoAdapter`]). `camera_path` flies the capture
/// orbit over the twin frames (presentation only); `None` keeps the
/// establishing view. `muted` opens no audio device (automated smoke always
/// passes true, so tests never make sound). `wall_limit_ms` bounds the run
/// ([`SMOKE_WALL_LIMIT_MS`] by default, `--deadline-ms` on the binary): past
/// it the smoke fails loudly instead of hanging. `present_every` presents
/// every Nth tick and `present_skip` none (hashes unchanged either way).
/// Never panics without hardware.
pub fn run_headless_smoke_with_options(
    config: GameConfig,
    frames: u32,
    camera_path: Option<CameraPath>,
    muted: bool,
    wall_limit_ms: u32,
    present_every: u32,
    present_skip: bool,
) -> Result<WinReport, WinError> {
    if frames == 0 || frames > super::MAX_TICKS {
        return Err(WinError::BadArgs(format!("--frames {frames}")));
    }
    let started = std::time::Instant::now();
    let mut app = App::new(config)?.into_duel()?;
    let bindings = Bindings::new();
    let bank = Bank::placeholder(WINDOWED_SAMPLE_RATE);
    let mut mixer = Mixer::new(WINDOWED_SAMPLE_RATE);
    let human = FighterId::new(HUMAN_SLOT).ok_or(GameError::BadConfig("fighters"))?;

    // Audio degrades with a notice: probe once when unmuted, keep rendering
    // offline either way so the smoke never depends on hardware. Automated
    // smoke passes muted, so no device is ever opened there.
    let probe = Arc::new(Mutex::new(Mixer::new(WINDOWED_SAMPLE_RATE)));
    let (_device, audio_notice) = open_audio(Arc::clone(&probe), muted);
    drop(_device);

    // Presenter twin when an adapter exists; skip-not-green otherwise.
    let mut presenter = smoke_presenter()?;
    let presenter_notice = match presenter {
        Some((ref presenter, _, _)) => format!(
            "presenter twin: {} ({})",
            presenter.adapter_name(),
            presenter.adapter_driver()
        ),
        None => "presenter skipped: no wgpu adapter on this machine".to_string(),
    };

    let ticks_run = frames.min(config.ticks);
    let mut mixed_frames = 0usize;
    // Mix state, same wiring as `run_windowed` (tunnel preset on, stone
    // wall muffle per voice, game thread only).
    let mut bridge = MixBridge::new(WINDOWED_SAMPLE_RATE)
        .map_err(|err| WinError::Render(format!("tunnel params invalid: {err}")))?;
    let walls = win_walls();
    let mut send_sum = 0f64;
    let mut send_voices = 0u64;
    // Reused offline mix buffer (one heap alloc here, zero per tick).
    let mut scratch = vec![0.0f32; FRAMES_PER_TICK];
    let mut frames_presented = 0u32;
    // Presentation feed, same wiring as `run_windowed`: orbit framing
    // per tick with live-cue trauma, presentation-side shake only.
    let mut feed = PresentationFeed::new(HUMAN_SLOT, DirectorParams::default())
        .map_err(|err| WinError::Render(format!("camera table invalid: {err}")))?;
    for tick in 0..ticks_run {
        // Own wall-clock bound: a wedged device fails the smoke loudly
        // instead of hanging the run past its limit.
        check_exit_deadline_with_limit(started.elapsed().as_secs_f64() * 1000.0, wall_limit_ms)?;
        let input = fighter_input_from_pressed(tick, human, &[], &bindings);
        let outcome = app.tick(input);
        debug_assert!(outcome.input_accepted);
        let _ = feed.win_frame(&outcome);
        let stats = mix_tick_audio(&mut mixer, &bank, &mut bridge, &walls, &outcome);
        send_sum += stats.send_sum;
        send_voices += u64::try_from(stats.voices).unwrap_or(u64::MAX);
        mixer.render_into(&mut scratch);
        mixed_frames += scratch.len();
        if present_skip || !super::should_present(tick, ticks_run, present_every) {
            continue;
        }
        if let Some((ref mut presenter, key, ref kit)) = presenter {
            let list = smoke_twin_list(presenter, key, kit, frames_presented, camera_path)?;
            let out = presenter.render_offscreen(&list)?;
            if out.receipt.presented {
                frames_presented += 1;
            }
            // Duel overlay rides the presented frame: live names plus HP pips
            // for this tick (presentation only, hash untouched).
            let presented_overlay = duel_overlay_for_app(&app);
            let _ = presented_overlay.to_bytes();
        }
    }

    let (hud_cells, hud_quads) = hud_cells_quads(app.hud());
    Ok(WinReport {
        ticks_run,
        frames_presented,
        last_checkpoint: app.checkpoints().last().copied(),
        hud_cells,
        hud_quads,
        mixed_frames,
        mean_send: if send_voices > 0 {
            (send_sum / f64::from(u32::try_from(send_voices).unwrap_or(u32::MAX))) as f32
        } else {
            0.0
        },
        audio_notice,
        presenter_notice,
        camera_frames: u32::try_from(feed.len()).unwrap_or(u32::MAX),
    })
}

/// Headless smoke: the `--frames N --exit` path with no display. Feeds
/// the same idle bindings inputs as [`run_windowed`] (same tape, same
/// checkpoints), renders mixer voices offline, lays out the HUD, and presents
/// one offscreen twin frame per tick when an adapter exists (skip with a
/// notice when [`PresentError::NoAdapter`]). `camera_path` flies the capture
/// orbit over the twin frames (presentation only); `None` keeps the
/// establishing view. Never panics without hardware.
pub fn run_headless_smoke(
    config: GameConfig,
    frames: u32,
    camera_path: Option<CameraPath>,
) -> Result<WinReport, WinError> {
    if frames == 0 || frames > super::MAX_TICKS {
        return Err(WinError::BadArgs(format!("--frames {frames}")));
    }
    let mut app = App::new(config)?.into_duel()?;
    let bindings = Bindings::new();
    let bank = Bank::placeholder(WINDOWED_SAMPLE_RATE);
    let mut mixer = Mixer::new(WINDOWED_SAMPLE_RATE);
    let human = FighterId::new(HUMAN_SLOT).ok_or(GameError::BadConfig("fighters"))?;

    // Audio never opens a device on the automated path: muted by
    // construction, so the smoke stays silent on machines with hardware
    // while mixer voices render offline either way.
    let audio_notice = MUTED_AUDIO_NOTICE.to_string();

    // Presenter twin when an adapter exists; skip-not-green otherwise.
    let mut presenter = smoke_presenter()?;
    let presenter_notice = match presenter {
        Some((ref presenter, _, _)) => format!(
            "presenter twin: {} ({})",
            presenter.adapter_name(),
            presenter.adapter_driver()
        ),
        None => "presenter skipped: no wgpu adapter on this machine".to_string(),
    };

    let ticks_run = frames.min(config.ticks);
    let mut mixed_frames = 0usize;
    // The automated smoke owns its wall-clock bound: a hung run fails
    // itself loudly instead of burning a core.
    let started = std::time::Instant::now();
    // Mix state, same wiring as `run_windowed` (tunnel preset on, stone
    // wall muffle per voice, game thread only).
    let mut bridge = MixBridge::new(WINDOWED_SAMPLE_RATE).expect("48 kHz tunnel params validate");
    let walls = win_walls();
    let mut send_sum = 0f64;
    let mut send_voices = 0u64;
    // Reused offline mix buffer (one heap alloc here, zero per tick).
    let mut scratch = vec![0.0f32; FRAMES_PER_TICK];
    let mut frames_presented = 0u32;
    // Presentation feed, same wiring as `run_windowed`: orbit framing
    // per tick with live-cue trauma, presentation-side shake only.
    let mut feed = PresentationFeed::new(HUMAN_SLOT, DirectorParams::default())
        .expect("arena camera table is valid");
    for tick in 0..ticks_run {
        check_exit_deadline(started.elapsed().as_secs_f64() * 1000.0)?;
        let input = fighter_input_from_pressed(tick, human, &[], &bindings);
        let outcome = app.tick(input);
        debug_assert!(outcome.input_accepted);
        let _ = feed.win_frame(&outcome);
        let stats = mix_tick_audio(&mut mixer, &bank, &mut bridge, &walls, &outcome);
        send_sum += stats.send_sum;
        send_voices += stats.voices as u64;
        mixer.render_into(&mut scratch);
        mixed_frames += scratch.len();
        if let Some((ref mut presenter, key, ref kit)) = presenter {
            let list = smoke_twin_list(presenter, key, kit, frames_presented, camera_path)?;
            let out = presenter.render_offscreen(&list)?;
            if out.receipt.presented {
                frames_presented += 1;
            }
        }
    }

    let (hud_cells, hud_quads) = hud_cells_quads(app.hud());
    Ok(WinReport {
        ticks_run,
        frames_presented,
        last_checkpoint: app.checkpoints().last().copied(),
        hud_cells,
        hud_quads,
        mixed_frames,
        mean_send: if send_voices > 0 {
            (send_sum / send_voices as f64) as f32
        } else {
            0.0
        },
        audio_notice,
        presenter_notice,
        camera_frames: feed.len() as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> GameConfig {
        GameConfig {
            seed: 2040,
            ticks: SMOKE_TICKS,
            fighters: 2,
            half_extent_m: 20.0,
        }
    }

    #[test]
    fn windowed_smoke_runs_the_full_data_path() {
        let report = run_windowed(config(), SMOKE_TICKS).unwrap();
        assert_eq!(report.ticks_run, SMOKE_TICKS);
        assert_eq!(report.window_size, WINDOW_SIZE);
        assert_eq!(report.frames_pumped, u64::from(SMOKE_TICKS));
        assert_eq!(report.mixed_frames, SMOKE_TICKS as usize * FRAMES_PER_TICK);
        // HP line always lays out; one quad per cell.
        assert!(report.hud_cells >= "HP 1000/1000".len());
        assert_eq!(report.hud_quads, report.hud_cells);
        assert!(report.last_checkpoint.is_some());
        // Honest device report: skipped with a reason, never silent success.
        assert!(report.audio_device.contains("skipped"));
        // The frame loop presents one camera frame per fixed tick.
        assert_eq!(report.camera_frames, SMOKE_TICKS);

        // Replay equivalence across legs: the windowed idle run matches a
        // headless run fed the same idle inputs tick for tick.
        let mut headless = App::new(config()).unwrap().into_duel().unwrap();
        let mut last = None;
        for tick in 0..SMOKE_TICKS {
            let outcome = headless.tick(FighterInput {
                tick,
                fighter: FighterId::new(HUMAN_SLOT).unwrap(),
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: engine_input::Buttons::NONE,
            });
            last = outcome.checkpoint;
        }
        assert_eq!(report.last_checkpoint, last);
    }

    #[test]
    fn windowed_smoke_bounds_and_rejects() {
        // Zero ticks: empty but valid report.
        let report = run_windowed(config(), 0).unwrap();
        assert_eq!((report.ticks_run, report.last_checkpoint), (0, None));
        assert_eq!(report.mixed_frames, 0);
        assert_eq!(report.camera_frames, 0);
        // Ticks past the session end clamp to the session.
        let report = run_windowed(config(), SMOKE_TICKS + 1000).unwrap();
        assert_eq!(report.ticks_run, SMOKE_TICKS);
        // Bad config fails instead of opening anything.
        assert!(run_windowed(
            GameConfig {
                ticks: 0,
                ..config()
            },
            10
        )
        .is_err());
    }

    #[test]
    fn net_host_matches_headless_tick_for_tick_under_app() {
        // The LAN host leg (headless std::net code) simulates the same match
        // as the windowed idle path: same config plus same inputs gives the
        // same checkpoints on both legs.
        use super::super::NetGame;
        use engine_input::Buttons;

        let (code, mut host) = NetGame::host(config()).unwrap();
        assert!(code.starts_with(super::super::SESSION_CODE_PREFIX));
        let mut headless = super::super::App::new(config()).unwrap();
        for tick in 0..SMOKE_TICKS {
            let input = FighterInput {
                tick,
                fighter: FighterId::new(HUMAN_SLOT).unwrap(),
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: Buttons::NONE,
            };
            let net_outcome = host.tick_local(input);
            let headless_outcome = headless.tick(input);
            assert!(net_outcome.input_accepted);
            assert_eq!(net_outcome.checkpoint, headless_outcome.checkpoint);
        }
        assert_eq!(host.app().checkpoints(), headless.checkpoints());
    }

    #[test]
    fn win_args_parse_frames_exit_and_overrides() {
        let args = [
            "arena".to_string(),
            "--frames".to_string(),
            "60".to_string(),
            "--exit".to_string(),
        ];
        let parsed = WinArgs::parse(&args).unwrap();
        assert_eq!(
            parsed,
            WinArgs {
                frames: Some(60),
                exit: true,
                seed: None,
                ticks: None,
                fighters: None,
                scene: None,
                timed_smoke: false,
                backend: None,
                camera_path: None,
                mute: false,
                deadline_ms: None,
                present_every: 1,
                present_skip: false,
            }
        );
        // Flags without the program name parse too.
        let parsed = WinArgs::parse(&["--frames".to_string(), "7".to_string()]).unwrap();
        assert_eq!(parsed.frames, Some(7));
        assert!(!parsed.exit);
        // Seed and ticks overrides.
        let parsed = WinArgs::parse(
            &["--seed", "9", "--ticks", "64"]
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!((parsed.seed, parsed.ticks), (Some(9), Some(64)));
        // Failures are typed, never panics.
        for bad in [
            vec!["arena".to_string(), "--frames".to_string()],
            vec!["arena".to_string(), "--frames".to_string(), "0".to_string()],
            vec![
                "arena".to_string(),
                "--frames".to_string(),
                "abc".to_string(),
            ],
            vec!["arena".to_string(), "--nope".to_string()],
            vec!["arena".to_string(), "--seed".to_string()],
            vec!["arena".to_string(), "--fighters".to_string()],
            vec![
                "arena".to_string(),
                "--fighters".to_string(),
                "1".to_string(),
            ],
            vec![
                "arena".to_string(),
                "--fighters".to_string(),
                "17".to_string(),
            ],
            vec![
                "arena".to_string(),
                "--scene".to_string(),
                "dungeon".to_string(),
            ],
            vec!["arena".to_string(), "--scene".to_string()],
            vec!["arena".to_string(), "--backend".to_string()],
        ] {
            assert!(WinArgs::parse(&bad).is_err(), "args accepted: {bad:?}");
        }
        // Every backend label parses, including the Metal label the render
        // backend owns (`BackendChoice::from_name("metal")` is valid there);
        // only unknown labels fail.
        let parsed = WinArgs::parse(&["--backend".to_string(), "metal".to_string()]).unwrap();
        assert_eq!(parsed.backend, Some(BackendChoice::Metal));
        for bad in [
            vec!["arena".to_string(), "--camera-path".to_string()],
            vec![
                "arena".to_string(),
                "--camera-path".to_string(),
                "dungeon".to_string(),
            ],
            vec!["arena".to_string(), "--deadline-ms".to_string()],
            vec![
                "arena".to_string(),
                "--deadline-ms".to_string(),
                "0".to_string(),
            ],
            vec![
                "arena".to_string(),
                "--deadline-ms".to_string(),
                "abc".to_string(),
            ],
        ] {
            assert!(WinArgs::parse(&bad).is_err(), "args accepted: {bad:?}");
        }
    }

    #[test]
    fn win_args_parse_mute_and_deadline() {
        // Automated smoke passes `--mute` (no audio device) and may override
        // the `--exit` wall-clock bound; interactive defaults stay loud with
        // the 5-minute bound.
        let parsed = WinArgs::parse(
            &["arena", "--mute", "--deadline-ms", "1000"]
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert!(parsed.mute);
        assert_eq!(parsed.deadline_ms, Some(1000));
        assert_eq!(parsed.wall_limit_ms(), 1000);
        let plain = WinArgs::parse(&["arena".to_string()]).unwrap();
        assert!(!plain.mute);
        assert_eq!(plain.deadline_ms, None);
        assert_eq!(plain.wall_limit_ms(), SMOKE_WALL_LIMIT_MS);
        assert_eq!(SMOKE_WALL_LIMIT_MS, 300_000);
    }

    #[test]
    fn exit_deadline_fails_loudly_past_the_limit() {
        // Under the limit passes; at or past it fails with the limit named;
        // bad readings and a zero limit are refused just as loudly.
        assert!(check_exit_deadline_with_limit(0.0, 1000).is_ok());
        assert!(check_exit_deadline_with_limit(999.9, 1000).is_ok());
        let err = check_exit_deadline_with_limit(1000.0, 1000).unwrap_err();
        assert!(format!("{err}").contains("limit_ms=1000"), "{err}");
        let err = check_exit_deadline_with_limit(300_001.0, SMOKE_WALL_LIMIT_MS).unwrap_err();
        assert!(format!("{err}").contains("deadline exceeded"), "{err}");
        assert!(check_exit_deadline_with_limit(f64::NAN, 1000).is_err());
        assert!(check_exit_deadline_with_limit(f64::INFINITY, 1000).is_err());
        assert!(check_exit_deadline_with_limit(-1.0, 1000).is_err());
        assert!(check_exit_deadline_with_limit(0.0, 0).is_err());
    }

    #[test]
    fn surface_errors_split_transient_skips_from_loud_failures() {
        // Transient acquire states skip the frame and retry next frame;
        // everything else (no surface, lost device, validation, caller
        // input) fails loudly so an unpainted window never reports green.
        for transient in [
            PresentError::SurfaceTimeout,
            PresentError::SurfaceOccluded,
            PresentError::SurfaceLost { generation: 3 },
            PresentError::SurfaceOutdated { generation: 3 },
        ] {
            assert!(!surface_error_is_loud(&transient), "{transient}");
        }
        for loud in [
            PresentError::NoAdapter,
            PresentError::Device("gone".into()),
            PresentError::DeviceLost("gone".into()),
            PresentError::Validation("scope".into()),
            PresentError::NoSurface,
            PresentError::InvalidSize {
                width: 9999,
                height: 1,
            },
            PresentError::Readback("map".into()),
        ] {
            assert!(surface_error_is_loud(&loud), "{loud}");
        }
    }

    #[test]
    fn win_args_parse_backend_and_camera_path() {
        // Harness flags: an explicit backend plus the fixed arena600
        // orbit; labels are validated, never defaulted silently.
        let parsed = WinArgs::parse(
            &["arena", "--backend", "vulkan", "--camera-path", "arena600"]
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(parsed.backend, Some(BackendChoice::Vulkan));
        assert_eq!(parsed.camera_path, Some(CameraPath::Arena600));
        assert_eq!(parsed.camera_path.unwrap().id(), "arena600");
        assert_eq!(parsed.camera_path.unwrap().seed(), ARENA600_SEED);
        assert_eq!(parsed.camera_path.unwrap().frames(), ARENA600_FRAMES);
        assert_eq!((ARENA600_SEED, ARENA600_FRAMES), (2040, 600));
        // Every backend label parses; the orbit pins the harness coordinates.
        for (label, want) in [
            ("auto", BackendChoice::Auto),
            ("dx12", BackendChoice::Dx12),
            ("vulkan", BackendChoice::Vulkan),
            ("gl", BackendChoice::Gl),
        ] {
            let parsed = WinArgs::parse(&["--backend".to_string(), label.to_string()]).unwrap();
            assert_eq!(parsed.backend, Some(want));
        }
        assert_eq!(CameraPath::parse("arena600").unwrap(), CameraPath::Arena600);
        assert!(CameraPath::parse("orbit").is_err());
        // Defaults stay free-play with no backend override.
        let plain = WinArgs::parse(&["arena".to_string()]).unwrap();
        assert_eq!((plain.backend, plain.camera_path), (None, None));
    }

    #[test]
    fn camera_path_pins_seed_and_floors_ticks() {
        // The orbit pins seed 2040 and floors the budget at 600 ticks;
        // an explicit `--ticks` still wins; free play is untouched.
        assert_eq!(
            pin_session_for_camera_path(9, 600, Some(CameraPath::Arena600), None),
            (ARENA600_SEED, ARENA600_FRAMES)
        );
        assert_eq!(
            pin_session_for_camera_path(9, 30, Some(CameraPath::Arena600), Some(30)),
            (ARENA600_SEED, 30)
        );
        assert_eq!(pin_session_for_camera_path(9, 120, None, None), (9, 120));
    }

    #[test]
    fn camera_orbit_steers_the_draw_list_eye() {
        // The orbit parks the camera eye on the fixed seed-2040 turn: frame
        // 0 and frame 150 differ, the same frame repeats, and free play
        // leaves the default scene eye alone.
        assert_eq!(orbit_eye_for_frame(0, None), None);
        let first = orbit_eye_for_frame(0, Some(CameraPath::Arena600)).expect("orbit has an eye");
        let quarter =
            orbit_eye_for_frame(150, Some(CameraPath::Arena600)).expect("orbit has an eye");
        assert_eq!(first, engine_render::backend::arena600_eye(0));
        assert_ne!(first, quarter);
        assert_eq!(
            first,
            orbit_eye_for_frame(0, Some(CameraPath::Arena600)).expect("orbit repeats")
        );
        let mut list = hud_draw_list(128, 96).expect("hud list builds");
        let free_eye = list.camera.eye;
        apply_camera_orbit(&mut list, 0, None);
        assert_eq!(list.camera.eye, free_eye);
        apply_camera_orbit(&mut list, 150, Some(CameraPath::Arena600));
        assert_ne!(list.camera.eye, free_eye);
        list.validate().expect("orbited list still validates");
    }

    #[test]
    fn win_args_parse_rl5_capture_selectors() {
        // The 8-fighter arena capture the review path selects plus
        // the timed-smoke fps flag on the arena binary.
        let parsed = WinArgs::parse(
            &[
                "arena",
                "--frames",
                "60",
                "--exit",
                "--fighters",
                "8",
                "--scene",
                "arena",
                "--timed-smoke",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(parsed.fighters, Some(ARENA8_FIGHTERS));
        assert_eq!(parsed.scene, Some(SceneSelect::Arena));
        assert_eq!(parsed.scene.unwrap().id(), ARENA_SCENE_ID);
        assert!(parsed.timed_smoke);
        assert_eq!(SceneSelect::parse("arena").unwrap(), SceneSelect::Arena);
        assert!(SceneSelect::parse("dungeon").is_err());
        // Defaults stay duel + untimed.
        let plain = WinArgs::parse(&["arena".to_string()]).unwrap();
        assert_eq!(
            (plain.fighters, plain.scene, plain.timed_smoke),
            (None, None, false)
        );
    }

    #[test]
    fn timed_smoke_fps_is_measurable_and_loud() {
        // 60 frames in one wall second is 60 fps; the rate scales linearly.
        assert!((smoke_fps(60, 1000.0).unwrap() - 60.0).abs() < 1e-9);
        assert!((smoke_fps(120, 500.0).unwrap() - 240.0).abs() < 1e-9);
        // Zero frames, zero time and non-finite time fail loudly.
        assert!(smoke_fps(0, 1000.0).is_err());
        assert!(smoke_fps(60, 0.0).is_err());
        assert!(smoke_fps(60, -5.0).is_err());
        assert!(smoke_fps(60, f64::NAN).is_err());
        assert!(smoke_fps(60, f64::INFINITY).is_err());
    }

    #[test]
    fn accumulator_steps_at_60hz_with_interpolation() {
        let mut acc = TickAccumulator::new();
        // Sub-frame elapsed buffers; two halves make one step.
        assert_eq!(acc.push_elapsed_ms(WIN_FIXED_DT_MS / 2.0), 0);
        assert!(acc.alpha() > 0.0 && acc.alpha() < 1.0);
        assert_eq!(acc.push_elapsed_ms(WIN_FIXED_DT_MS / 2.0 + 0.001), 1);
        // One frame is one tick.
        assert_eq!(acc.push_elapsed_ms(WIN_FIXED_DT_MS), 1);
        // 10 s clamps to the per-frame cap, never 600.
        assert_eq!(acc.push_elapsed_ms(10_000.0), WIN_MAX_STEPS_PER_FRAME);
        // Bad wall readings step nothing and buffer nothing.
        for bad in [f64::NAN, f64::INFINITY, -5.0, 0.0] {
            let before = acc.pending_ms();
            assert_eq!(acc.push_elapsed_ms(bad), 0);
            assert_eq!(acc.pending_ms(), before);
        }
        assert!((0.0..=1.0).contains(&acc.alpha()));
    }

    #[test]
    fn bindings_map_keys_and_mouse_to_fighter_input() {
        use crate::input::{map_key, map_mouse, KeyCode as C, MouseButton as MB};
        use engine_input::map::Key as K;
        // WASD plus attack/block/dodge rows map through the engine codes.
        assert_eq!(map_key(C::W), PhysicalButton::Key(K::W.0));
        assert_eq!(map_key(C::F), PhysicalButton::Key(K::F.0));
        assert_eq!(map_key(C::Space), PhysicalButton::Key(K::SPACE.0));
        assert_eq!(map_mouse(MB::Left), PhysicalButton::Mouse(0));
        assert_eq!(map_mouse(MB::Other(300)), PhysicalButton::Mouse(255));
        // Pressed rows build a validated input through the live table.
        let bindings = Bindings::new();
        let fwd = bindings.button(engine_input::BoundAction::MoveForward);
        let input =
            fighter_input_from_pressed(3, FighterId::new(HUMAN_SLOT).unwrap(), &[fwd], &bindings);
        assert_eq!((input.mx, input.mz), (100, 0));
        input.validate().unwrap();
        assert_eq!(input.tick, 3);
    }

    #[test]
    fn audio_mute_opens_no_device_and_names_it() {
        // Automated smoke never touches the audio device: muted runs keep
        // the shared notice string while the mixer still renders offline.
        let mixer = Arc::new(Mutex::new(Mixer::new(WINDOWED_SAMPLE_RATE)));
        let (stream, notice) = open_audio(Arc::clone(&mixer), true);
        assert!(stream.is_none(), "muted smoke opens no audio device");
        assert_eq!(notice, MUTED_AUDIO_NOTICE);
        // WinError prints every variant without panicking.
        for err in [
            WinError::BadArgs("x".to_string()),
            WinError::BadConfig("ticks"),
            WinError::Game(GameError::BadConfig("ticks")),
            WinError::NoDisplay("headless".to_string()),
            WinError::Present(PresentError::NoAdapter),
            WinError::Render("scene".to_string()),
            WinError::Timeout("bound".to_string()),
        ] {
            assert!(!format!("{err}").is_empty());
        }
        assert!(format!("{}", WinError::Timeout("bound".to_string())).contains("timeout"));
    }

    #[test]
    fn mute_flag_parses_and_mutes_automated_runs() {
        // `--mute` parses; `--exit` and `--timed-smoke` runs mute by
        // construction, so every automated smoke stays silent.
        let muted = WinArgs::parse(&["arena".to_string(), "--mute".to_string()]).unwrap();
        assert!(muted.mute);
        assert!(smoke_muted(&muted));
        let automated = WinArgs::parse(&[
            "arena".to_string(),
            "--frames".to_string(),
            "7".to_string(),
            "--exit".to_string(),
        ])
        .unwrap();
        assert!(smoke_muted(&automated));
        let timed = WinArgs::parse(&["arena".to_string(), "--timed-smoke".to_string()]).unwrap();
        assert!(smoke_muted(&timed));
        // Free play stays live: no flag, no exit, no timed line.
        let plain = WinArgs::parse(&["arena".to_string()]).unwrap();
        assert!(!plain.mute);
        assert!(!smoke_muted(&plain));
    }

    #[test]
    fn exit_deadline_fails_loud_when_forced() {
        // Zero and in-bound readings pass; the bound itself still passes.
        check_exit_deadline(0.0).unwrap();
        check_exit_deadline(EXIT_WALL_LIMIT_MS).unwrap();
        check_exit_deadline(EXIT_WALL_LIMIT_MS - 1.0).unwrap();
        // Past the bound fails loudly and names the wall-clock bound.
        let err = check_exit_deadline(EXIT_WALL_LIMIT_MS + 1.0).unwrap_err();
        let msg = format!("{err}");
        assert!(msg.contains("wall-clock"), "{msg}");
        // Non-finite and negative readings fail loudly, never clamp.
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            assert!(check_exit_deadline(bad).is_err(), "elapsed accepted: {bad}");
        }
        assert!(matches!(
            check_exit_deadline(EXIT_WALL_LIMIT_MS + 1.0),
            Err(WinError::Timeout(_))
        ));
    }

    #[test]
    fn surface_policy_presents_only_visible_frames() {
        // The only green path is a presented surface frame.
        assert_eq!(
            surface_frame_action(Ok(true)),
            SurfaceFrameAction::Presented
        );
        // Suspended frames and transient acquire states skip to next redraw.
        assert_eq!(surface_frame_action(Ok(false)), SurfaceFrameAction::Skipped);
        for transient in [
            PresentError::SurfaceTimeout,
            PresentError::SurfaceOccluded,
            PresentError::SurfaceLost { generation: 3 },
            PresentError::SurfaceOutdated { generation: 3 },
        ] {
            assert_eq!(
                surface_frame_action(Err(&transient)),
                SurfaceFrameAction::Skipped,
                "transient must skip: {transient}"
            );
        }
        // Every other failure ends loudly: an unpainted window never passes.
        for hard in [
            PresentError::NoAdapter,
            PresentError::Device("x".into()),
            PresentError::DeviceLost("x".into()),
            PresentError::OutOfMemory { generation: 3 },
            PresentError::Validation("x".into()),
            PresentError::Readback("x".into()),
            PresentError::InvalidSize {
                width: 9999,
                height: 1,
            },
            PresentError::InvalidDraw {
                index: 0,
                reason: "x".into(),
            },
            PresentError::InvalidMesh("x".into()),
            PresentError::InvalidTexture("x".into()),
            PresentError::MissingTexture {
                id: 7,
                reason: "gone".into(),
            },
            PresentError::NoSurface,
            PresentError::BadDigest("x".into()),
        ] {
            assert_eq!(
                surface_frame_action(Err(&hard)),
                SurfaceFrameAction::Failed,
                "hard failure must fail: {hard}"
            );
        }
    }

    #[test]
    fn audio_open_never_panics_and_names_its_state() {
        let mixer = Arc::new(Mutex::new(Mixer::new(WINDOWED_SAMPLE_RATE)));
        let (_stream, notice) = open_audio(Arc::clone(&mixer), false);
        assert!(!notice.is_empty(), "audio state is always reported");
        assert!(notice.contains("audio live") || notice.contains("audio degraded"));
        // WinError prints every variant without panicking.
        for err in [
            WinError::BadArgs("x".to_string()),
            WinError::BadConfig("ticks"),
            WinError::Game(GameError::BadConfig("ticks")),
            WinError::NoDisplay("headless".to_string()),
            WinError::Present(PresentError::NoAdapter),
            WinError::Render("scene".to_string()),
        ] {
            assert!(!format!("{err}").is_empty());
        }
    }

    #[test]
    fn audio_mute_opens_no_device() {
        // Automated smoke passes `--mute`: no device is opened and the notice
        // names the muted state, so tests never make sound.
        let mixer = Arc::new(Mutex::new(Mixer::new(WINDOWED_SAMPLE_RATE)));
        let (device, notice) = open_audio(Arc::clone(&mixer), true);
        assert!(device.is_none(), "muted runs open no audio device");
        assert_eq!(notice, MUTED_AUDIO_NOTICE);
    }

    #[test]
    fn hud_draw_list_validates_sizes() {
        hud_draw_list(128, 96).unwrap().validate().unwrap();
        assert!(hud_draw_list(0, 96).is_err());
        assert!(hud_draw_list(SMOKE_FRAME_SIZE.0, 0).is_err());
        assert!(hud_draw_list(9999, 64).is_err());
    }

    #[test]
    fn headless_smoke_matches_the_same_tape() {
        let frames = 30u32;
        let report = run_headless_smoke(config(), frames, None).unwrap();
        assert_eq!(report.ticks_run, frames);
        assert_eq!(report.mixed_frames, frames as usize * FRAMES_PER_TICK);
        assert!(report.hud_cells >= "HP 1000/1000".len());
        assert_eq!(report.hud_quads, report.hud_cells);
        assert!(!report.audio_notice.is_empty());
        // Automated smoke opens no audio device, by construction.
        assert!(
            report.audio_notice.contains("muted"),
            "smoke must stay muted: {}",
            report.audio_notice
        );
        // Configurable smoke with the default wall limit matches the same tape.
        let report_options = run_headless_smoke_with_options(
            config(),
            frames,
            None,
            true,
            SMOKE_WALL_LIMIT_MS,
            1,
            false,
        )
        .unwrap();
        assert_eq!(report_options.ticks_run, frames);
        assert_eq!(report_options.last_checkpoint, report.last_checkpoint);
        assert!(!report.presenter_notice.is_empty());
        // The smoke loop presents one camera frame per fixed tick.
        assert_eq!(report.camera_frames, frames);
        // Same idle tape on the headless leg gives the same checkpoints.
        let mut headless = App::new(config()).unwrap().into_duel().unwrap();
        let bindings = Bindings::new();
        let human = FighterId::new(HUMAN_SLOT).unwrap();
        let mut last = None;
        for tick in 0..frames {
            last = headless
                .tick(fighter_input_from_pressed(tick, human, &[], &bindings))
                .checkpoint;
        }
        assert_eq!(report.last_checkpoint, last);
        // Bounds and rejections are typed.
        assert!(run_headless_smoke(config(), 0, None).is_err());
        assert!(run_headless_smoke(config(), super::super::MAX_TICKS + 1, None).is_err());
        assert!(run_headless_smoke(
            GameConfig {
                ticks: 0,
                ..config()
            },
            4,
            None
        )
        .is_err());
        // Configurable smoke rejects the same bounds under the default limit.
        assert!(run_headless_smoke_with_options(
            config(),
            0,
            None,
            true,
            SMOKE_WALL_LIMIT_MS,
            1,
            false
        )
        .is_err());
        // A zero wall limit fails the smoke loudly instead of hanging it.
        assert!(
            run_headless_smoke_with_options(config(), frames, None, true, 0, 1, false).is_err(),
            "zero deadline must fail loudly"
        );
        // The orbit flag parses and flows without touching the sim: the same
        // idle tape gives the same checkpoints under the orbit too.
        let orbit = run_headless_smoke(config(), frames, Some(CameraPath::Arena600)).unwrap();
        assert_eq!(orbit.ticks_run, frames);
        assert_eq!(orbit.last_checkpoint, last);
        // Configurable orbit matches too.
        let orbit_options = run_headless_smoke_with_options(
            config(),
            frames,
            Some(CameraPath::Arena600),
            true,
            SMOKE_WALL_LIMIT_MS,
            1,
            false,
        )
        .unwrap();
        assert_eq!(orbit_options.last_checkpoint, last);
    }

    #[test]
    fn win_mix_wires_tunnel_send_and_wall_muffle() {
        use engine_audio::BankSound;

        let bank = Bank::placeholder(WINDOWED_SAMPLE_RATE);
        let mut mixer = Mixer::new(WINDOWED_SAMPLE_RATE);
        let mut bridge = MixBridge::new(WINDOWED_SAMPLE_RATE).unwrap();
        let walls = win_walls();
        // Two voices: index 0 behind the stone wall, index 1 at the ears.
        let mut outcome = TickOutcome::default();
        for _ in 0..2 {
            outcome.audio.push(super::super::AudioCommand {
                tick: 0,
                sound: BankSound::Hit,
                gain: 1.0,
            });
        }
        let stats = mix_tick_audio(&mut mixer, &bank, &mut bridge, &walls, &outcome);
        assert_eq!(stats.voices, 2);
        assert_eq!(mixer.voice_count(), 2);
        // Both voices carry the tunnel send; the listener sits inside it.
        let send = bridge.send_for(WIN_LISTENER_3CM);
        assert!((stats.send_sum - 2.0 * f64::from(send)).abs() < 1e-6);
        assert!(send > 0.0, "tunnel preset stays on in the windowed mix");
        // The walled voice already ducks while the open one holds unity.
        let (walled, open) = (
            bridge.state_q16(0).expect("slot 0"),
            bridge.state_q16(1).expect("slot 1"),
        );
        assert!(walled < open, "walled {walled} vs open {open}");
        // Empty ticks mix nothing and fail nothing.
        let empty = TickOutcome::default();
        let stats = mix_tick_audio(&mut mixer, &bank, &mut bridge, &walls, &empty);
        assert_eq!((stats.voices, stats.send_sum), (0, 0.0));
    }

    #[test]
    fn dressing_binds_the_file_table() {
        // The embedded file is the staging table, not a copy.
        assert!(DRESSING_JSON.contains("torch_approach_w"));
        assert!(DRESSING_JSON.contains("banner_far_e"));
        let actors = dressing_actors().expect("dressing parses");
        assert_eq!(actors.len(), DRESSING_ACTOR_COUNT);
        let ids: Vec<u32> = actors.iter().map(|actor| actor.id).collect();
        assert_eq!(ids, vec![101, 102, 103, 104, 105, 106]);
        assert_eq!(actors[0].asset, DressingAsset::Torch);
        assert_eq!(actors[2].asset, DressingAsset::Banner);
        // Torches flank the approach, banners pair the court and far end.
        assert_eq!(actors[0].pos_m, [5.5, 1.0, -2.0]);
        assert_eq!(actors[3].pos_m, [17.0, 1.5, 2.0]);
        assert_eq!(actors[5].size_m, [1.6, 3.0, 0.6]);
        // Keys never collide with the central cube or each other.
        let cube = MeshKey::new(1, [0xA1; 32]);
        let mut keys = Vec::with_capacity(actors.len());
        for actor in &actors {
            let key = dressing_mesh_key(actor);
            assert_ne!(key, cube);
            assert!(!keys.contains(&key));
            keys.push(key);
            // Meshes validate and keep the staged extents.
            let mesh = dressing_mesh(actor).expect("dressing mesh builds");
            assert_eq!(mesh.positions.len(), 24);
            assert_eq!(mesh.indices.len(), 36);
            let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
            for p in &mesh.positions {
                for axis in 0..3 {
                    lo[axis] = lo[axis].min(p[axis]);
                    hi[axis] = hi[axis].max(p[axis]);
                }
            }
            for axis in 0..3 {
                let extent = hi[axis] - lo[axis];
                assert!((extent - actor.size_m[axis]).abs() < 1e-4);
                let center = 0.5 * (hi[axis] + lo[axis]);
                assert!((center - actor.pos_m[axis]).abs() < 1e-4);
            }
            dressing_material(actor.asset)
                .validate()
                .expect("material valid");
        }
    }

    #[test]
    fn dressing_camera_frames_the_axis() {
        let camera = dressing_camera(128, 96).expect("camera builds");
        let cols = camera
            .view_proj()
            .expect("camera validates")
            .to_cols_array();
        let project = |point: &[f32; 3]| {
            let (x, y, z) = (point[0], point[1], point[2]);
            [
                cols[0] * x + cols[4] * y + cols[8] * z + cols[12],
                cols[1] * x + cols[5] * y + cols[9] * z + cols[13],
                cols[2] * x + cols[6] * y + cols[10] * z + cols[14],
                cols[3] * x + cols[7] * y + cols[11] * z + cols[15],
            ]
        };
        let actors = dressing_actors().expect("dressing parses");
        // Every actor center plus the central cube land inside the frustum.
        let mut points: Vec<[f32; 3]> = actors.iter().map(|a| a.pos_m).collect();
        points.push([0.0, 0.0, 0.0]);
        for point in &points {
            let clip = project(point);
            assert!(clip[3] > 1e-6, "point behind eye: {point:?}");
            let ndc = [clip[0] / clip[3], clip[1] / clip[3], clip[2] / clip[3]];
            assert!(
                (-1.0..=1.0).contains(&ndc[0]) && (-1.0..=1.0).contains(&ndc[1]),
                "outside frame {point:?} -> {ndc:?}"
            );
        }
        assert!(dressing_camera(0, 96).is_err());
        assert!(dressing_camera(9999, 64).is_err());
    }

    /// Numbers of `"key": [..]` (or the one number after `"key":`) in the
    /// establishing file's camera block.
    fn establishing_numbers(text: &str, key: &str) -> Vec<f32> {
        let tail = &text[text.find("\"camera\"").expect("camera block")..];
        let rest = &tail[tail.find(key).expect("camera key") + key.len()..];
        let value = rest[rest.find(':').expect("colon") + 1..].trim_start();
        let (body, end) = match value.strip_prefix('[') {
            Some(list) => (list, ']'),
            None => (value, ','),
        };
        body[..body.find(end).expect("value end")]
            .split(',')
            .map(|token| token.trim().parse().expect("number"))
            .collect()
    }

    #[test]
    fn establishing_json_camera_matches_the_code_pins() {
        let json = include_str!("../../../content/arena/establishing.json");
        assert_eq!(establishing_numbers(json, "\"eye_m\""), DRESSING_EYE_M);
        assert_eq!(establishing_numbers(json, "\"target_m\""), DRESSING_TARGET_M);
        assert_eq!(establishing_numbers(json, "\"fov_deg\""), [DRESSING_FOV_DEG]);
        // A 1e-3 drift in the file must fail the pin.
        let drifted = json.replace("\"fov_deg\": 50.0", "\"fov_deg\": 50.001");
        assert_ne!(
            establishing_numbers(&drifted, "\"fov_deg\""),
            [DRESSING_FOV_DEG]
        );
    }

    #[test]
    fn dressing_rejects_bad_tables() {
        assert!(dressing_actors_from_json("{}").is_err());
        assert!(dressing_actors_from_json("{\"actors\": []}").is_err());
        // Unknown asset never stages silently.
        let bad_asset = DRESSING_JSON.replacen("\"torch\"", "\"lamp\"", 1);
        assert!(dressing_actors_from_json(&bad_asset).is_err());
        // Five actors is a torn table, not a smaller frame.
        let short = DRESSING_JSON.replacen("\"id\": 106", "\"id\": 99999", 1);
        let actors = dressing_actors_from_json(&short).expect("ids still parse");
        assert!(actors.iter().any(|actor| actor.id == 99999));
        // NaN coordinates fail instead of reaching the GPU.
        let nan = DRESSING_JSON.replacen("5.5", "NaN", 1);
        assert!(dressing_actors_from_json(&nan).is_err());
        // Draws only cover resident keys; an empty cache draws nothing.
        let mut list = hud_draw_list(128, 96).expect("hud builds");
        let pushed = push_dressing_draws(&mut list, &[]).expect("empty draws");
        assert_eq!(pushed, 0);
        assert!(list.items.is_empty());
    }

    #[test]
    fn dressing_lod_bands_and_order() {
        // Establishing eye (-14, 5, 0): torches at ~20 m read LOD1, court
        // and far banners at ~31-40 m read LOD2, through the asset bands.
        let actors = dressing_actors().expect("dressing parses");
        let lods: Vec<usize> = actors
            .iter()
            .map(|actor| dressing_lod(actor, DRESSING_EYE_M))
            .collect();
        assert_eq!(lods, vec![1, 1, 2, 2, 2, 2]);
        // A far eye pushes every actor past the last band: LOD3, no refusal.
        for actor in &actors {
            assert_eq!(dressing_lod(actor, [200.0, 5.0, 0.0]), 3);
        }
        // Full-resident frame under the establishing camera pushes all six,
        // nearer band first (torches, then court banners, then far banners:
        // file order here).
        let mut list = hud_draw_list(128, 96).expect("hud builds");
        list.camera = dressing_camera(128, 96).expect("camera builds");
        let resident: Vec<MeshKey> = actors.iter().map(dressing_mesh_key).collect();
        let pushed = push_dressing_draws(&mut list, &resident).expect("draws push");
        assert_eq!(pushed, 6);
        let order: Vec<MeshKey> = list
            .items
            .iter()
            .map(|item| item.mesh)
            .filter(|key| resident.contains(key))
            .collect();
        assert_eq!(order, resident);
    }

    #[test]
    fn kit_twin_stages_11_draws_over_5000_tris() {
        // Twin staging path: cube plus dressing plus the 8-file kit. No
        // adapter on this machine skips (never fails): the smoke does the
        // same, so CI without a GPU stays green.
        let Some((mut presenter, key, kit)) = smoke_presenter().expect("twin opens or skips")
        else {
            return;
        };
        assert_eq!(
            kit.draw_count(),
            crate::arena_kit::KIT_ACTOR_COUNT,
            "twin stages one draw per placed actor"
        );
        assert_eq!(kit.assets_staged, crate::arena_kit::KIT_ASSET_COUNT);
        assert!(
            kit.triangles >= 5_000,
            "twin stages {} tris, want at least 5000",
            kit.triangles
        );
        let list = smoke_twin_list(&presenter, key, &kit, 0, None).expect("twin list builds");
        let actors = crate::arena_kit::kit_actors().expect("placement parses");
        for actor in &actors {
            let want = crate::arena_kit::kit_mesh_key(actor);
            assert!(
                presenter.mesh_keys().contains(&want),
                "kit actor {} is resident",
                actor.id
            );
            assert!(
                list.items.iter().any(|item| item.mesh == want),
                "kit actor {} is drawn",
                actor.id
            );
        }
        // Frame 0 presents: the picture is no longer the cube-only frame.
        // The bare list (empty kit report) repaints exactly the old
        // cube-plus-dressing picture; its PNG hash must differ from the
        // kit frame hash, or the kit draws nothing visible.
        use sha2::{Digest, Sha256};
        let out = presenter.render_offscreen(&list).expect("twin renders");
        assert!(out.receipt.presented, "frame 0 presents");
        let png_kit = out.png.expect("kit frame has bytes");
        let bare = smoke_twin_list(
            &presenter,
            key,
            &crate::arena_kit::KitReport::default(),
            0,
            None,
        )
        .expect("bare list builds");
        let out_bare = presenter.render_offscreen(&bare).expect("bare renders");
        assert!(out_bare.receipt.presented, "bare frame 0 presents");
        let png_bare = out_bare.png.expect("bare frame has bytes");
        let mut digest = Sha256::new();
        digest.update(&png_kit);
        let sha_kit = format!("{:x}", digest.finalize_reset());
        digest.update(&png_bare);
        let sha_bare = format!("{:x}", digest.finalize());
        assert_ne!(sha_kit, sha_bare, "kit frame matches the cube frame");
    }

    #[test]
    fn duel_overlay_tracks_live_hp_and_pips() {
        // Behavior: one named row per live slot, tick plus permille health
        // from the match, pips lit per `hp_pips` so the frame shows health.
        let mut app = App::new(config()).unwrap();
        for tick in 0..3u32 {
            let input = fighter_input_from_pressed(
                tick,
                FighterId::new(HUMAN_SLOT).unwrap(),
                &[],
                &Bindings::new(),
            );
            let _ = app.tick(input);
        }
        let overlay = super::duel_overlay_for_app(&app);
        assert_eq!(overlay.len(), 2);
        assert_eq!(overlay.tick(), app.hud().tick);
        for slot in 0..2u8 {
            let id = FighterId::new(slot).unwrap();
            let row = overlay.row(id).expect("live slot tracked");
            let hp = app.hp()[usize::from(slot)];
            assert_eq!(row.hp_permille(), hp);
            assert_eq!(row.pips(), engine_ui::hp_pips(hp));
        }
        let decoded =
            engine_ui::DuelOverlay::from_bytes(&overlay.to_bytes()).expect("bytes decode");
        assert_eq!(decoded, overlay);
    }

    #[test]
    fn duel_overlay_caps_at_sixteen_with_stable_names() {
        // Corner: a full 16-fighter match still builds 16 distinct rows with
        // stable short names; the same snapshot repeats exactly.
        let full = GameConfig {
            fighters: 16,
            ..config()
        };
        let app = App::new(full).unwrap();
        let overlay = super::duel_overlay_for_app(&app);
        assert_eq!(overlay.len(), 16);
        assert_eq!(overlay.rows().len(), 16);
        let again = super::duel_overlay_for_app(&app);
        assert_eq!(overlay, again, "same snapshot repeats");
        for (slot, row) in overlay.rows().iter().enumerate() {
            let byte = u8::try_from(slot).expect("slot fits");
            assert_eq!(row.id(), FighterId::new(byte).unwrap());
            assert_eq!(row.name(), format!("F{slot}"));
        }
    }
}
