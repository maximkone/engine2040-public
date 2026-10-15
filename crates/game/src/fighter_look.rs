//! fighter_look: the CT7 fighters as skinned, textured bodies in the presenter.
//!
//! Consumer: the `arena` binary (window frames plus their offscreen
//! captures, free play and the staged fight). Data owner: this module owns
//! the presentation anim state per slot (act, clip clock, hit flash, facing)
//! and the fight camera; bodies and clips come from
//! `engine_assets::body_import`, clip retarget from `engine_assets::retarget`,
//! linear blend skinning from `engine_render::v1_skin`, bundle identity from
//! [`crate::fighter_bind`] (body bytes re-hashed against the CT7 pins).
//! Units: meters, radians, fixed ticks at 60 Hz, clip seconds.
//! Determinism: presentation only. It reads views, hp and cues after each
//! tick and never writes match state, so the tick hash cannot move.
//! Errors: typed [`LookError`]; a missing or re-cooked body fails at load,
//! a refused upload fails the frame.
//! Budget: cold load about 1 s (4 bodies, 6 clips, 16 PNG decodes); per
//! frame CPU skinning of about 3.9k vertices per fighter plus one mesh
//! upload per body part (4 per fighter).

use std::path::{Path, PathBuf};

use engine_anim::contract::{skinning_matrices, LocalPose, Skeleton};
use engine_anim::pose::sample_pose_with_rots;
use engine_arena::GaitPhase;
use engine_assets::body_import::{import_clip_source, import_skinned_body, ClipSource};
use engine_assets::retarget::{retarget_clip, BoneMap};
use engine_assets::skin_import::ImportedClip;
use engine_audio::FightCue;
use engine_render::present::{DrawItem, DrawList, MeshKey, Presenter, TextureKey, TextureUpload};
use engine_render::v1::{self, Camera, MeshData, PbrMaterial};
use engine_render::v1_skin::{SkinPipeline, SkinWeights};
use sha2::{Digest, Sha256};

use crate::fight_ring::FightRing;
use crate::fighter_bind::{self, FighterBinding};
use crate::view_truth::BodyTruth;
use crate::FighterView;

/// Mesh id base for skinned body parts (clear of the cube, dressing, kit
/// and staged-box keys): `base + slot * 16 + part`.
pub const LOOK_MESH_ID_BASE: u64 = 0xF16B_0000;
/// Texture id base for body base-color maps: `base + rig * 16 + part`.
pub const LOOK_TEXTURE_ID_BASE: u64 = 0xF16C_0000;
/// Most fighters drawn (the match cap).
pub const MAX_LOOK_FIGHTERS: usize = 8;
/// Steepest fight-camera look down: rise per metre of eye-to-focus reach.
const MAX_PITCH_SLOPE: f32 = 0.75;
/// Fixed ticks per second (clip clocks advance with match ticks).
const TICK_HZ: f32 = 60.0;
/// Ticks between swing start and the impact cue (about mid-swing).
const IMPACT_TICKS: u32 = 12;
/// Ticks a hit victim stays tinted (about 100 ms: a flash, not a strobe).
const FLASH_TICKS: u32 = 6;
/// Farthest drawn attacker-to-victim gap that still tints the victim.
/// Rules v1 lands every swing on every living foe at any range, so without
/// this gate all eight bodies strobe red from the first exchange on.
const TINT_REACH_M: f32 = 3.0;
/// Planar speed above which a fighter reads as running (meters per tick).
const RUN_SPEED_M_PER_TICK: f32 = 0.012;
/// Reference planar speed where the run clip plays at its authored rate
/// (meters per tick; presentation only, never in the tick hash).
const RUN_REF_M_PER_TICK: f32 = 0.05;
/// Largest facing turn per tick in radians (about 14 degrees).
const MAX_TURN_PER_TICK: f32 = 0.25;
/// Most parts per body (atlas quadrants); keys reserve 16 per slot.
const MAX_PARTS: usize = 16;
/// Pelvis height for a fallen slot: lies flat, below 0.4 m (EG45).
/// Presentation only; the sim hash never reads it.
const BALANCE_FALLEN_PELVIS_M: f32 = 0.25;
/// Pelvis height for a catching slot: kneels (EG45). Presentation only.
const BALANCE_CATCH_PELVIS_M: f32 = 0.45;
/// Forward lean for a stumbling slot in meters (EG45). Presentation only.
const BALANCE_STUMBLE_LEAN_M: f32 = 0.18;
/// Largest sand lift in meters (LF17, view only): the fallen pelvis (EG45)
/// puts the upright start of the death clip about 56 cm under the sand, so
/// the cap sits above any body height and never clips a real sink.
const SAND_LIFT_MAX_M: f32 = 1.0;
/// View-only shake added per in-reach Hit, in permille (LF11, view only).
/// Presentation only; the sim hash never reads it.
const SHAKE_HIT_PERMILLE: u32 = 400;
/// View-only shake added per in-reach Knockdown, in permille (LF11).
/// Presentation only; the sim hash never reads it.
const SHAKE_KNOCKDOWN_PERMILLE: u32 = 700;
/// Clamp for the fight-camera trauma, in permille (LF11).
const SHAKE_MAX_PERMILLE: u32 = 1000;
/// Trauma decayed per tick: 1000 per second at 60 Hz needs 16.67 per tick,
/// so 17 (the ceiling) clears full trauma in 59 ticks, inside 1.0 s.
/// Integer-only, so the same ticks give the same trauma twice.
const SHAKE_DECAY_PERMILLE_PER_TICK: u32 = 17;
/// Shake cap in meters at full trauma (LF11): the arena director default,
/// so the fight camera matches the staged rig at full trauma.
const SHAKE_MAX_M: f32 = 0.35;

/// Clip files under `glb/clips/`, in [`ClipRole`] order.
const CLIP_FILES: [&str; 6] = [
    "human_male_idle_combat_main_hand_melee.glb",
    "human_male_run.glb",
    "human_male_cast_main_hand_melee.glb",
    "human_male_cast_main_hand_melee_2.glb",
    "human_male_cast_main_hand_melee_3.glb",
    "human_male_death.glb",
];

/// Load and frame failures.
#[derive(Clone, PartialEq, Debug)]
pub enum LookError {
    /// Bundle bind refused (manifest or body file).
    Bind(String),
    /// A body's bytes differ from the pinned CT7 SHA-256.
    ShaMismatch(String),
    /// Body or clip import failed.
    Import(String),
    /// Skinning or mesh validation failed.
    Skin(String),
    /// The presenter refused an upload.
    Upload(String),
    /// The fight ring (floor, tiers, textures) refused to build or stage.
    Ring(String),
}

impl core::fmt::Display for LookError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Bind(msg) => write!(f, "fighter look bind: {msg}"),
            Self::ShaMismatch(name) => write!(f, "fighter look: {name} is not the CT7 bytes"),
            Self::Import(msg) => write!(f, "fighter look import: {msg}"),
            Self::Skin(msg) => write!(f, "fighter look skin: {msg}"),
            Self::Upload(msg) => write!(f, "fighter look upload: {msg}"),
            Self::Ring(msg) => write!(f, "fighter look ring: {msg}"),
        }
    }
}

impl std::error::Error for LookError {}

/// What a fighter is doing on screen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Act {
    /// Guard stance, looping.
    Idle,
    /// Moving across the sand, looping.
    Run,
    /// One sword swing (variant cycles per swing).
    Attack(u8),
    /// Knocked down: death clip, held on its last frame.
    Down,
}

/// An act's name in the view truth (`idle`, `run`, `attack`, `down`).
fn act_label(act: Act) -> &'static str {
    match act {
        Act::Idle => "idle",
        Act::Run => "run",
        Act::Attack(_) => "attack",
        Act::Down => "down",
    }
}

/// One body part ready to skin: bind mesh, rows, texture and factors.
struct RigPart {
    mesh: MeshData,
    skin: SkinWeights,
    texture: Option<(TextureKey, TextureUpload)>,
    material: PbrMaterial,
}

/// One variant: skeleton, inverse binds, parts and its retargeted clips.
struct Rig {
    variant: String,
    skeleton: Skeleton,
    inverse_binds: Vec<[[f32; 4]; 4]>,
    pelvis: usize,
    bind_pelvis: [f32; 3],
    pipe: SkinPipeline,
    parts: Vec<RigPart>,
    /// Clips in [`CLIP_FILES`] order, on this rig's skeleton.
    clips: Vec<ImportedClip>,
}

/// Presentation state for one slot.
#[derive(Clone, Copy, PartialEq, Debug)]
struct SlotAnim {
    act: Act,
    act_start: u32,
    swings: u8,
    prev_pos: [f32; 3],
    pos: [f32; 3],
    prev_yaw: f32,
    yaw: f32,
    flash_until: u32,
    seen: bool,
    /// Planar meters per tick, set beside the speed pick in `fold_acts`.
    /// Presentation only; never enters the tick hash.
    run_speed: f32,
    /// Integer balance gait for the look (EG45): steady reads as before,
    /// stumbling leans, fallen lies flat, catching kneels. Presentation
    /// only; the sim digest never reads this.
    balance: GaitPhase,
}

impl SlotAnim {
    const fn new() -> Self {
        Self {
            act: Act::Idle,
            act_start: 0,
            swings: 0,
            prev_pos: [0.0; 3],
            pos: [0.0; 3],
            prev_yaw: 0.0,
            yaw: 0.0,
            flash_until: 0,
            seen: false,
            run_speed: 0.0,
            balance: GaitPhase::Steady,
        }
    }
}

/// Playback scale for the run clip from planar speed: speed over
/// [`RUN_REF_M_PER_TICK`], clamped to half through double rate. Pure
/// function of speed, so the same tick plus the same speed gives the
/// same clip time. Non-finite speeds hold the authored rate.
fn run_phase_scale(speed_m_per_tick: f32) -> f32 {
    if !speed_m_per_tick.is_finite() {
        return 1.0;
    }
    (speed_m_per_tick / RUN_REF_M_PER_TICK).clamp(0.5, 2.0)
}

/// Pelvis height for one balance gait (EG45): steady and stumbling keep
/// the clip height, fallen lies flat below 0.4 m, catching kneels.
/// Pure, no allocation; non-finite clip heights fall back to the gait
/// target (steady falls back to 0.0) so no NaN ever reaches the skin.
#[must_use]
fn balance_pelvis_y_m(gait: GaitPhase, clip_y: f32) -> f32 {
    match gait {
        GaitPhase::Fallen => BALANCE_FALLEN_PELVIS_M,
        GaitPhase::Catching => BALANCE_CATCH_PELVIS_M,
        GaitPhase::Steady | GaitPhase::Stumbling => {
            if clip_y.is_finite() {
                clip_y
            } else {
                0.0
            }
        }
    }
}

/// Forward lean for one balance gait in meters (EG45): only stumbling
/// leans its root; every other gait holds its ground. Pure.
#[must_use]
fn balance_lean_m(gait: GaitPhase) -> f32 {
    match gait {
        GaitPhase::Stumbling => BALANCE_STUMBLE_LEAN_M,
        GaitPhase::Steady | GaitPhase::Fallen | GaitPhase::Catching => 0.0,
    }
}

/// Lift to rest a body on the sand (LF17, view only): the below-sand depth
/// of the posed bounds, clamped to `0.0..=SAND_LIFT_MAX_M`. The full depth
/// applies from the first tick: the fallen pelvis moves the whole upright
/// start pose down at once, so any ease would show a body sunk into the sand.
///
/// Consumer: [`FighterLook::stage_bodies`] (the frame's CPU skin folds every
/// posed vertex into `BodyTruth::min`, so the fix reads the pose-skinned
/// bounds, never the clip file: the skeleton pose owns the vertices).
/// Data owner: this module owns the present lift; the sim hash never reads
/// it. Units: meters in, meters out. Determinism: pure `f32` in, `f32` out,
/// no clock, no allocation. Errors: none; non-finite or above-sand bounds
/// read as 0, and the helper only adds lift, never lowers.
#[must_use]
fn sand_lift_m(lowest_y: f32) -> f32 {
    if !lowest_y.is_finite() {
        return 0.0;
    }
    (crate::view_truth::GROUND_Y - lowest_y).clamp(0.0, SAND_LIFT_MAX_M)
}

/// Upload one slot's skinned parts with the down lift applied (LF17, view
/// only): lifted positions for the draw, normals ride unchanged. The body
/// part owns one fixed-size vertex buffer for its whole life: every frame
/// rewrites it in place. Returns parts written.
fn upload_slot_parts(
    presenter: &mut Presenter,
    slot_index: usize,
    rig: &Rig,
    frames: Vec<engine_render::v1_skin::SkinnedFrame>,
    lift: f32,
) -> Result<usize, LookError> {
    let mut uploaded = 0usize;
    for (part_index, posed) in frames.into_iter().enumerate() {
        let part = &rig.parts[part_index];
        let lifted: Vec<[f32; 3]> = if lift > 0.0 {
            posed
                .positions
                .iter()
                .map(|p| [p[0], p[1] + lift, p[2]])
                .collect()
        } else {
            posed.positions
        };
        let key = look_mesh_key(slot_index, part_index);
        let rewritten = presenter
            .refresh_mesh_vertices(key, &lifted, &posed.normals, &part.mesh.uvs)
            .map_err(|err| LookError::Upload(err.to_string()))?;
        if !rewritten {
            let mesh = v1::upload_mesh(&lifted, &posed.normals, &part.mesh.uvs, &part.mesh.indices)
                .map_err(|err| LookError::Skin(format!("{err}")))?;
            presenter
                .upload_mesh(key, &mesh)
                .map_err(|err| LookError::Upload(err.to_string()))?;
        }
        uploaded += 1;
    }
    Ok(uploaded)
}

/// Note: the authoritative read is `engine-arena` `Sim::balance` (already
/// landed with EG34); the window polls it once `LiveMatch` exposes the
/// sim (a later row owns that edge). Until then the look mirrors the same
/// integer verdicts from the cues that feed the sim (Hit stumbles,
/// Knockdown floors), so the same blows lean the same roots.

/// Camera shot for the fight camera.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shot {
    /// Close on the hero (slot 0).
    Hero,
    /// Wide establishing view of the arena with the fight in it.
    Wide,
    /// Mid shot fitting every fighter.
    Action,
}

/// One presented frame's look inputs.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LookFrame {
    /// Fighters in the match.
    pub fighters: usize,
    /// Sub-tick interpolation in `0..=1`.
    pub alpha: f32,
    /// Camera shot for this frame.
    pub shot: Shot,
    /// Presented frame index (slow camera orbit).
    pub frame: u32,
}

/// Shot for presented frame `frame` of a `frames`-long capture run: a
/// directed run (capture camera path) splits into thirds, hero close-up,
/// then the wide arena, then the fight; free play keeps the fight shot.
#[must_use]
pub fn shot_for_frame(frame: u32, frames: Option<u32>, directed: bool) -> Shot {
    match frames {
        Some(total) if directed && total >= 3 => {
            let third = total / 3;
            if frame < third {
                Shot::Hero
            } else if frame < third * 2 {
                Shot::Wide
            } else {
                Shot::Action
            }
        }
        _ => Shot::Action,
    }
}

/// Whether a world point lands in `camera`'s picture: in front of the near
/// plane and inside the field of view. False for a camera that cannot build
/// a view.
fn in_picture(camera: &Camera, at: [f32; 3]) -> bool {
    let Ok(view) = camera.view_matrix() else {
        return false;
    };
    let mut point = camera.eye;
    (point.x, point.y, point.z) = (at[0], at[1], at[2]);
    let local = view.transform_point3(point);
    let depth = -local.z;
    let half = (camera.fov_y * 0.5).tan();
    depth > camera.near
        && local.y.abs() <= depth * half
        && local.x.abs() <= depth * half * camera.aspect
}

/// Degrees `camera` looks down below the horizon (negative looks up).
#[must_use]
pub fn look_down_deg(camera: &Camera) -> f32 {
    let dir = camera.target - camera.eye;
    (-dir.y).atan2(dir.x.hypot(dir.z)).to_degrees()
}

/// Metres the eye can back away from `focus` along `azimuth` before it
/// leaves the square pit of half-width `limit`.
fn pit_room(focus: [f32; 3], azimuth: f32, limit: f32) -> f32 {
    let along = |at: f32, dir: f32| {
        if dir > 1e-4 {
            (limit - at) / dir
        } else if dir < -1e-4 {
            (-limit - at) / dir
        } else {
            f32::MAX
        }
    };
    along(focus[0], azimuth.sin())
        .min(along(focus[2], azimuth.cos()))
        .max(0.0)
}

/// Orbit side and reach for a fight eye `distance` from `focus`: the wanted
/// azimuth while the pit has room for it, else the mirror side-on view or a
/// diagonal that does, else the roomiest of them pulled in along its own
/// line. Clamping x and z apart once parked the eye over a duel by the wall,
/// looking straight down at the sand (perf window, 2026-10-11).
fn pit_orbit(focus: [f32; 3], azimuth: f32, distance: f32, limit: f32) -> (f32, f32) {
    use std::f32::consts::{FRAC_PI_4, PI};
    let mut best = (azimuth, 0.0f32);
    for turn in [
        0.0,
        PI,
        FRAC_PI_4,
        -FRAC_PI_4,
        3.0 * FRAC_PI_4,
        -3.0 * FRAC_PI_4,
    ] {
        let side = azimuth + turn;
        let room = pit_room(focus, side, limit);
        if room >= distance {
            return (side, distance);
        }
        if room > best.1 {
            best = (side, room);
        }
    }
    best
}

/// The fighter look: rigs, per-slot anim state and the last tick seen.
pub struct FighterLook {
    rigs: Vec<Rig>,
    /// Sand pit plus stone tiers drawn under the fighters once staged.
    ring: Option<FightRing>,
    slots: Vec<SlotAnim>,
    tick: u32,
    /// Ticks one swing clip plays (from the hero's first swing clip).
    swing_ticks: u32,
    bundle_shas: Vec<String>,
    /// Every body the last presented frame drew, as numbers.
    truth: Vec<BodyTruth>,
    /// View-only camera trauma in permille `0..=1000` (LF11): in-reach Hits
    /// add 400, in-reach Knockdowns add 700, clamped at 1000, decayed 1000
    /// per second. Presentation only; the sim hash never reads it.
    shake_trauma_permille: u32,
}

/// Fighters content dir (`content/fighters`), resolved like the kit dir:
/// walk up from the executable to the tree holding the manifest, else the
/// launch directory.
#[must_use]
pub fn fighters_content_dir() -> PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        let mut anchor = exe.parent().map(Path::to_path_buf);
        while let Some(dir) = anchor {
            let candidate = dir.join("content/fighters");
            if candidate.join(fighter_bind::MANIFEST_NAME).is_file() {
                return candidate;
            }
            anchor = dir.parent().map(Path::to_path_buf);
        }
    }
    PathBuf::from("content/fighters")
}

fn sha_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

fn digest32(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// Same-name bone pairs from the target rig onto the clip rig.
fn same_name_map(source: &Skeleton, target: &Skeleton) -> Result<BoneMap, LookError> {
    let pairs: Vec<(&str, &str)> = target
        .joints
        .iter()
        .filter(|joint| source.joints.iter().any(|other| other.name == joint.name))
        .map(|joint| (joint.name.as_str(), joint.name.as_str()))
        .collect();
    BoneMap::build(source, target, &pairs).map_err(|err| LookError::Import(format!("{err:?}")))
}

fn load_rig(
    binding: &FighterBinding,
    rig_index: usize,
    sources: &[ClipSource],
) -> Result<Rig, LookError> {
    let bytes = std::fs::read(&binding.body_glb)
        .map_err(|err| LookError::Bind(format!("{}: {err}", binding.variant)))?;
    if sha_hex(&bytes) != binding.body_glb_sha256 {
        return Err(LookError::ShaMismatch(binding.variant.clone()));
    }
    let body = import_skinned_body(&bytes)
        .map_err(|err| LookError::Import(format!("{}: {err}", binding.variant)))?;
    if body.parts.len() > MAX_PARTS {
        return Err(LookError::Import(format!(
            "{}: {} parts over {MAX_PARTS}",
            binding.variant,
            body.parts.len()
        )));
    }
    let mut pipe = SkinPipeline::new();
    pipe.upload_skeleton(body.skeleton.len())
        .map_err(|err| LookError::Skin(format!("{err:?}")))?;
    let mut parts = Vec::with_capacity(body.parts.len());
    for (index, part) in body.parts.iter().enumerate() {
        let mesh = v1::upload_mesh(
            &part.mesh.positions,
            &part.mesh.normals,
            &part.mesh.uv0,
            &part.mesh.indices,
        )
        .map_err(|err| LookError::Skin(format!("{} part {index}: {err}", binding.variant)))?;
        let skin = pipe
            .upload_weights(&part.skin.joints, &part.skin.weights)
            .map_err(|err| LookError::Skin(format!("{err:?}")))?;
        let texture = match &part.base_color_image {
            Some(image) => {
                let upload = TextureUpload::from_encoded(image)
                    .map_err(|err| LookError::Import(format!("{err}")))?;
                let id = LOOK_TEXTURE_ID_BASE
                    + u64::try_from(rig_index * MAX_PARTS + index)
                        .map_err(|_| LookError::Import("texture id".to_string()))?;
                Some((TextureKey::new(id, digest32(image)), upload))
            }
            None => None,
        };
        let factor = part.material.base_color_factor;
        let material = PbrMaterial {
            base_color: [factor[0], factor[1], factor[2]],
            metallic: part.material.metallic_factor,
            roughness: part.material.roughness_factor,
            exposure: 1.0,
        };
        material
            .validate()
            .map_err(|err| LookError::Import(format!("material: {err}")))?;
        parts.push(RigPart {
            mesh,
            skin,
            texture,
            material,
        });
    }
    let pelvis = body
        .skeleton
        .joints
        .iter()
        .position(|joint| joint.name == "pelvis")
        .ok_or_else(|| LookError::Import(format!("{}: no pelvis joint", binding.variant)))?;
    let mut clips = Vec::with_capacity(sources.len());
    for source in sources {
        let map = same_name_map(&source.skeleton, &body.skeleton)?;
        let clip = source
            .clips
            .first()
            .ok_or_else(|| LookError::Import("clip file holds no clip".to_string()))?;
        let moved = retarget_clip(
            clip,
            &source.skeleton,
            &source.bind,
            &body.skeleton,
            &body.bind,
            &map,
        )
        .map_err(|err| LookError::Import(format!("retarget {}: {err:?}", clip.name)))?;
        clips.push(moved);
    }
    Ok(Rig {
        variant: binding.variant.clone(),
        bind_pelvis: body.bind.positions[pelvis],
        skeleton: body.skeleton,
        inverse_binds: body.inverse_binds,
        pelvis,
        pipe,
        parts,
        clips,
    })
}

/// Multiply two AN1 row-vector matrices: `p * a * b`.
fn mul(a: &[[f32; 4]; 4], b: &[[f32; 4]; 4]) -> [[f32; 4]; 4] {
    let mut out = [[0.0f32; 4]; 4];
    for (i, row) in out.iter_mut().enumerate() {
        for (k, cell) in row.iter_mut().enumerate() {
            *cell = a[i][0] * b[0][k] + a[i][1] * b[1][k] + a[i][2] * b[2][k] + a[i][3] * b[3][k];
        }
    }
    out
}

/// Root placement in the AN1 row-vector layout: turn about +Y by `yaw`
/// (body forward +Z maps to `(sin yaw, 0, cos yaw)`), then move to `pos`.
fn root_matrix(pos: [f32; 3], yaw: f32) -> [[f32; 4]; 4] {
    let (s, c) = yaw.sin_cos();
    [
        [c, 0.0, -s, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [s, 0.0, c, 0.0],
        [pos[0], pos[1], pos[2], 1.0],
    ]
}

/// Shortest signed angle from `from` to `to`, in `(-PI, PI]`.
fn angle_delta(from: f32, to: f32) -> f32 {
    let tau = std::f32::consts::TAU;
    let mut d = (to - from) % tau;
    if d > std::f32::consts::PI {
        d -= tau;
    } else if d <= -std::f32::consts::PI {
        d += tau;
    }
    d
}

/// Mesh key for one slot's body part.
#[must_use]
pub fn look_mesh_key(slot: usize, part: usize) -> MeshKey {
    let index = slot * MAX_PARTS + part;
    let id = LOOK_MESH_ID_BASE + u64::try_from(index).unwrap_or(u64::MAX - LOOK_MESH_ID_BASE);
    let mut digest = [0x5Bu8; 32];
    digest[0] = u8::try_from(slot).unwrap_or(u8::MAX);
    digest[1] = u8::try_from(part).unwrap_or(u8::MAX);
    MeshKey::new(id, digest)
}

/// GPU skin availability probe behind the window path.
///
/// Consumer: [`FighterLook::load_and_stage`] (cold window entry) calls this
/// so the built `GpuSkinned` path stays constructed in the live frame. Data
/// owner: `engine-render` owns the GPU device and pipeline; this file owns
/// the call only. Units: none. Determinism: presentation only; the answer
/// never enters hashed state and the CPU skin vectors below stay the single
/// render path. Errors: never fails; a missing adapter or a device refusal
/// reads as `None` (headless skips, never fails). Budget: one adapter
/// request per staged match, zero per tick or frame.
fn gpu_skin_adapter() -> Option<String> {
    match engine_render::v1_skin::GpuSkinned::init() {
        Ok(gpu) => Some(gpu.adapter_name().to_string()),
        Err(_) => None,
    }
}

impl FighterLook {
    /// Bind the CT7 bundles under `content_dir`, re-hash every body against
    /// its pin, import bodies plus clips, retarget the clips per rig and
    /// decode the base-color maps. Cold path; loud on any refusal.
    pub fn load(content_dir: &Path) -> Result<Self, LookError> {
        let bindings =
            fighter_bind::bind_all(content_dir).map_err(|err| LookError::Bind(err.to_string()))?;
        let mut sources = Vec::with_capacity(CLIP_FILES.len());
        for file in CLIP_FILES {
            let path = content_dir.join("glb").join("clips").join(file);
            let bytes =
                std::fs::read(&path).map_err(|err| LookError::Import(format!("{file}: {err}")))?;
            sources.push(
                import_clip_source(&bytes)
                    .map_err(|err| LookError::Import(format!("{file}: {err}")))?,
            );
        }
        let mut rigs = Vec::with_capacity(bindings.len());
        for (index, binding) in bindings.iter().enumerate() {
            rigs.push(load_rig(binding, index, &sources)?);
        }
        let swing_s = rigs
            .first()
            .and_then(|rig| rig.clips.get(2))
            .map_or(0.0, |clip| clip.duration);
        // Whole ticks covering the swing; clip lengths are a few seconds.
        let mut swing_ticks = 1u32;
        while f32::from(u16::try_from(swing_ticks).unwrap_or(u16::MAX)) < swing_s * TICK_HZ
            && swing_ticks < 600
        {
            swing_ticks += 1;
        }
        Ok(Self {
            rigs,
            ring: None,
            slots: vec![SlotAnim::new(); MAX_LOOK_FIGHTERS],
            tick: 0,
            swing_ticks,
            bundle_shas: bindings.into_iter().map(|b| b.bundle_sha256).collect(),
            truth: Vec::new(),
            shake_trauma_permille: 0,
        })
    }

    /// Pinned bundle SHAs of the bound variants, manifest order.
    #[must_use]
    pub fn bundle_shas(&self) -> &[String] {
        &self.bundle_shas
    }

    /// Variant drawn for a slot: the hero on slot 0, opponents cycle after.
    #[must_use]
    pub fn rig_for_slot(&self, slot: usize) -> usize {
        if slot == 0 || self.rigs.len() < 2 {
            return 0;
        }
        1 + (slot - 1) % (self.rigs.len() - 1)
    }

    /// Variant name drawn for a slot.
    #[must_use]
    pub fn variant_for_slot(&self, slot: usize) -> &str {
        self.rigs
            .get(self.rig_for_slot(slot))
            .map_or("", |rig| rig.variant.as_str())
    }

    /// Upload every base-color map. Cold path; a refused upload is loud.
    pub fn stage_textures(&self, presenter: &mut Presenter) -> Result<usize, LookError> {
        let mut staged = 0usize;
        for rig in &self.rigs {
            for part in &rig.parts {
                if let Some((key, upload)) = &part.texture {
                    presenter
                        .upload_texture(*key, upload)
                        .map_err(|err| LookError::Upload(err.to_string()))?;
                    staged += 1;
                }
            }
        }
        Ok(staged)
    }

    /// Fold one simulated tick into the presentation state: positions,
    /// facing toward the nearest standing rival, swings from cues (the
    /// attacker starts its swing so the impact lands on the cue tick), hit
    /// tint on victims, knockdown on zero hp. Never touches match state.
    pub fn observe_tick(
        &mut self,
        tick: u32,
        views: &[FighterView],
        hp: &[u16],
        cues: &[FightCue],
    ) {
        // View-only shake decay (LF11): 1000 permille per second, integer
        // only, before the tick's impacts land. Same ticks give the same
        // trauma twice; presentation only, never the sim hash.
        let elapsed = tick.saturating_sub(self.tick);
        self.shake_trauma_permille = self
            .shake_trauma_permille
            .saturating_sub(elapsed.saturating_mul(SHAKE_DECAY_PERMILLE_PER_TICK));
        self.tick = tick;
        self.fold_positions(views);
        self.fold_cues(tick, cues);
        self.fold_acts(tick, views, hp);
    }

    /// Shift each seen slot's position (previous, current) for the frame lerp.
    fn fold_positions(&mut self, views: &[FighterView]) {
        for view in views {
            let Some(slot) = self.slots.get_mut(usize::from(view.slot)) else {
                continue;
            };
            if !view.pos_m.iter().all(|v| v.is_finite()) {
                continue;
            }
            if slot.seen {
                slot.prev_pos = slot.pos;
            } else {
                slot.prev_pos = view.pos_m;
                slot.seen = true;
            }
            slot.pos = view.pos_m;
        }
    }

    /// Swings for attackers (impact on the cue tick), tint for victims.
    /// Balance mirror (EG45): the same blows that feed the sim's integer
    /// balance lean the look (Hit stumbles, Knockdown floors). Read-only;
    /// never writes match state. A fall never downgrades to a stumble.
    fn fold_cues(&mut self, tick: u32, cues: &[FightCue]) {
        for cue in cues {
            match *cue {
                FightCue::Hit {
                    attacker, victim, ..
                } => {
                    self.start_swing(attacker, tick);
                    if self.in_reach(attacker, victim) {
                        if let Some(slot) = self.slots.get_mut(usize::from(victim)) {
                            slot.flash_until = tick + FLASH_TICKS;
                        }
                        self.add_shake_trauma(SHAKE_HIT_PERMILLE);
                    }
                    self.stumble_slot(victim);
                }
                FightCue::Knockdown { attacker, victim } => {
                    self.start_swing(attacker, tick);
                    if self.in_reach(attacker, victim) {
                        if let Some(slot) = self.slots.get_mut(usize::from(victim)) {
                            slot.flash_until = tick + FLASH_TICKS;
                        }
                        self.add_shake_trauma(SHAKE_KNOCKDOWN_PERMILLE);
                    }
                    self.floor_slot(victim);
                }
                FightCue::Blocked { attacker, .. }
                | FightCue::Parried { attacker, .. }
                | FightCue::Dodged { attacker, .. } => self.start_swing(attacker, tick),
                FightCue::StaminaOut { .. } => {}
            }
        }
    }

    /// Mark `slot` stumbling unless it already lies flat or kneels: impacts
    /// never downgrade an in-flight fall, matching the sim. Bad slots are
    /// ignored. Presentation only.
    fn stumble_slot(&mut self, slot: u8) {
        let Some(anim) = self.slots.get_mut(usize::from(slot)) else {
            return;
        };
        match anim.balance {
            GaitPhase::Fallen | GaitPhase::Catching => {}
            GaitPhase::Steady | GaitPhase::Stumbling => {
                anim.balance = GaitPhase::Stumbling;
            }
        }
    }

    /// Mark `slot` fallen (lies flat). Bad slots are ignored.
    /// Presentation only.
    fn floor_slot(&mut self, slot: u8) {
        let Some(anim) = self.slots.get_mut(usize::from(slot)) else {
            return;
        };
        anim.balance = GaitPhase::Fallen;
    }

    /// Fold view-only shake trauma (LF11): saturating add, clamped at
    /// [`SHAKE_MAX_PERMILLE`]. Presentation only; the sim hash never reads it.
    fn add_shake_trauma(&mut self, amount_permille: u32) {
        self.shake_trauma_permille = self
            .shake_trauma_permille
            .saturating_add(amount_permille)
            .min(SHAKE_MAX_PERMILLE);
    }

    /// Drawn slots under `fighters` that wear the hit tint now, and all
    /// drawn slots: the arena receipt's share of red bodies.
    #[must_use]
    pub fn tinted(&self, fighters: usize) -> (usize, usize) {
        let drawn = self
            .slots
            .iter()
            .take(fighters.min(MAX_LOOK_FIGHTERS))
            .filter(|s| s.seen);
        drawn.fold((0, 0), |(red, all), slot| {
            (red + usize::from(self.tick < slot.flash_until), all + 1)
        })
    }

    /// Both slots drawn and their feet within [`TINT_REACH_M`] on the sand.
    fn in_reach(&self, attacker: u8, victim: u8) -> bool {
        let (Some(a), Some(v)) = (
            self.slots.get(usize::from(attacker)),
            self.slots.get(usize::from(victim)),
        ) else {
            return false;
        };
        a.seen && v.seen && (a.pos[0] - v.pos[0]).hypot(a.pos[2] - v.pos[2]) <= TINT_REACH_M
    }

    /// Knockdowns, facing toward the nearest standing rival, run or idle
    /// once a swing has played out.
    fn fold_acts(&mut self, tick: u32, views: &[FighterView], hp: &[u16]) {
        let standing: Vec<(usize, [f32; 3])> = views
            .iter()
            .filter(|view| hp.get(usize::from(view.slot)).copied().unwrap_or(1) > 0)
            .map(|view| (usize::from(view.slot), view.pos_m))
            .collect();
        for view in views {
            let index = usize::from(view.slot);
            let down = hp.get(index).copied().unwrap_or(1) == 0;
            let Some(slot) = self.slots.get_mut(index) else {
                continue;
            };
            slot.prev_yaw = slot.yaw;
            if down {
                slot.run_speed = 0.0;
                if slot.act != Act::Down {
                    slot.act = Act::Down;
                    slot.act_start = tick;
                }
                continue;
            }
            if slot.act == Act::Down {
                slot.act = Act::Idle;
                slot.act_start = tick;
            }
            // Face the nearest standing rival; with none, face the motion.
            let rival = standing
                .iter()
                .filter(|(other, _)| *other != index)
                .map(|(_, pos)| (pos[0] - slot.pos[0], pos[2] - slot.pos[2]))
                .min_by(|a, b| (a.0 * a.0 + a.1 * a.1).total_cmp(&(b.0 * b.0 + b.1 * b.1)));
            let step = (
                slot.pos[0] - slot.prev_pos[0],
                slot.pos[2] - slot.prev_pos[2],
            );
            let speed = (step.0 * step.0 + step.1 * step.1).sqrt();
            slot.run_speed = if speed.is_finite() { speed } else { 0.0 };
            let aim = match rival {
                Some(dir) if dir.0 * dir.0 + dir.1 * dir.1 > 1e-6 => Some(dir),
                _ if speed > RUN_SPEED_M_PER_TICK => Some(step),
                _ => None,
            };
            if let Some((dx, dz)) = aim {
                let want = dx.atan2(dz);
                let turn = angle_delta(slot.yaw, want).clamp(-MAX_TURN_PER_TICK, MAX_TURN_PER_TICK);
                slot.yaw += turn;
            }
            let swinging = matches!(slot.act, Act::Attack(_))
                && tick.saturating_sub(slot.act_start) < self.swing_ticks;
            if swinging {
                continue;
            }
            let next = if speed > RUN_SPEED_M_PER_TICK {
                Act::Run
            } else {
                Act::Idle
            };
            if next != slot.act {
                slot.act = next;
                slot.act_start = tick;
            }
        }
    }

    fn start_swing(&mut self, attacker: u8, tick: u32) {
        let Some(slot) = self.slots.get_mut(usize::from(attacker)) else {
            return;
        };
        if slot.act == Act::Down {
            return;
        }
        // A swing already in flight keeps playing; a new one restarts.
        let variant = slot.swings % 3;
        slot.swings = slot.swings.wrapping_add(1);
        slot.act = Act::Attack(variant);
        slot.act_start = tick.saturating_sub(IMPACT_TICKS);
    }

    /// Current act for a slot (receipts and tests).
    #[must_use]
    pub fn act(&self, slot: usize) -> Option<Act> {
        self.slots.get(slot).filter(|s| s.seen).map(|s| s.act)
    }

    /// Clip index plus clip time for an act at the frame's sub-tick `alpha`;
    /// a finished swing falls back to idle or run (resolved here, so the
    /// frame never shows a frozen last swing frame). The run clip scales
    /// with planar speed through [`run_phase_scale`], so the same tick
    /// plus the same speed gives the same time; other acts ignore speed.
    fn clip_time(
        &self,
        rig: &Rig,
        act: Act,
        act_start: u32,
        run_speed: f32,
        alpha: f32,
    ) -> (usize, f32) {
        let since =
            f32::from(u16::try_from(self.tick.saturating_sub(act_start)).unwrap_or(u16::MAX));
        let t = (since + alpha) / TICK_HZ;
        let duration = |index: usize| rig.clips.get(index).map_or(0.0, |clip| clip.duration);
        match act {
            Act::Idle => (0, t),
            Act::Run => (1, t * run_phase_scale(run_speed)),
            Act::Attack(variant) => {
                let index = 2 + usize::from(variant.min(2));
                if t < duration(index) {
                    (index, t)
                } else {
                    (0, t - duration(index))
                }
            }
            // Hold the last death frame (sampling wraps past the end).
            Act::Down => (5, t.min(duration(5) * 0.999)),
        }
    }

    /// Pose for one slot at sub-tick `alpha`: the current act's clip
    /// sampled at the speed-scaled run time. Pure function of the tick
    /// plus the recorded speed; the pelvis stays pinned in place (the
    /// match owns travel, the clip keeps its bob). The balance gait then
    /// lowers the pelvis for a fall or a catch (EG45, presentation only).
    /// A `Down` pose starts here at [`BALANCE_FALLEN_PELVIS_M`]; the LF17
    /// sand rest ([`sand_lift_m`]) only adds lift after the CPU skin folds
    /// the posed bounds, never inside this pose.
    fn slot_pose(&self, rig: &Rig, slot: &SlotAnim, alpha: f32) -> Result<LocalPose, LookError> {
        let (clip_index, time) =
            self.clip_time(rig, slot.act, slot.act_start, slot.run_speed, alpha);
        let clip = rig
            .clips
            .get(clip_index)
            .ok_or_else(|| LookError::Skin("clip missing".to_string()))?;
        let mut pose: LocalPose =
            sample_pose_with_rots(&rig.skeleton, &clip.positions, &clip.rotations, time)
                .map_err(|err| LookError::Skin(format!("{err:?}")))?;
        // In-place clips: the match owns travel, the clip keeps its bob.
        pose.positions[rig.pelvis][0] = rig.bind_pelvis[0];
        pose.positions[rig.pelvis][2] = rig.bind_pelvis[2];
        pose.positions[rig.pelvis][1] =
            balance_pelvis_y_m(slot.balance, pose.positions[rig.pelvis][1]);
        Ok(pose)
    }

    /// Create every slot's body-part vertex buffers once, from the bind
    /// pose, so no fighter ever creates a GPU buffer inside a frame: a
    /// slot that appears late already has its buffers and just rewrites
    /// them. Returns parts staged. Cold path; loud on refusal.
    pub fn stage_meshes(&self, presenter: &mut Presenter) -> Result<usize, LookError> {
        let mut staged = 0usize;
        for slot_index in 0..MAX_LOOK_FIGHTERS {
            let rig = &self.rigs[self.rig_for_slot(slot_index)];
            for (part_index, part) in rig.parts.iter().enumerate() {
                presenter
                    .upload_mesh(look_mesh_key(slot_index, part_index), &part.mesh)
                    .map_err(|err| LookError::Upload(err.to_string()))?;
                staged += 1;
            }
        }
        Ok(staged)
    }

    /// Skin every seen fighter at sub-tick `alpha` and write its parts into
    /// their resident vertex buffers. Returns parts written. A skinning or
    /// upload refusal is loud.
    pub fn stage_frame(
        &self,
        presenter: &mut Presenter,
        fighters: usize,
        alpha: f32,
    ) -> Result<usize, LookError> {
        self.stage_bodies(presenter, fighters, alpha)
            .map(|(uploaded, _)| uploaded)
    }

    /// [`Self::stage_frame`] plus each skinned body as numbers (bounds of
    /// the exact posed vertices, model-space joints, clip, tint): the view
    /// truth costs one pass over vertices the CPU skin already produced.
    fn stage_bodies(
        &self,
        presenter: &mut Presenter,
        fighters: usize,
        alpha: f32,
    ) -> Result<(usize, Vec<BodyTruth>), LookError> {
        let alpha = if alpha.is_finite() {
            alpha.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let mut uploaded = 0usize;
        let mut truth = Vec::with_capacity(fighters.min(MAX_LOOK_FIGHTERS));
        for slot_index in 0..fighters.min(MAX_LOOK_FIGHTERS) {
            let slot = self.slots[slot_index];
            if !slot.seen {
                continue;
            }
            let rig = &self.rigs[self.rig_for_slot(slot_index)];
            let (clip_index, time) =
                self.clip_time(rig, slot.act, slot.act_start, slot.run_speed, alpha);
            let pose: LocalPose = self.slot_pose(rig, &slot, alpha)?;
            let world = skinning_matrices(&rig.skeleton, &pose)
                .map_err(|err| LookError::Skin(format!("{err:?}")))?;
            let mut pos: [f32; 3] = std::array::from_fn(|i| {
                slot.prev_pos[i] + (slot.pos[i] - slot.prev_pos[i]) * alpha
            });
            let yaw = slot.prev_yaw + angle_delta(slot.prev_yaw, slot.yaw) * alpha;
            // Balance lean (EG45): a stumbling root steps forward along its
            // facing; every other gait holds. Presentation only.
            let lean = balance_lean_m(slot.balance);
            if lean != 0.0 && yaw.is_finite() {
                let (s, c) = yaw.sin_cos();
                if s.is_finite() && c.is_finite() {
                    pos[0] += lean * s;
                    pos[2] += lean * c;
                }
            }
            let root = root_matrix(pos, yaw);
            let matrices: Vec<[[f32; 4]; 4]> = world
                .iter()
                .zip(rig.inverse_binds.iter())
                .map(|(joint, inverse)| mul(&mul(inverse, joint), &root))
                .collect();
            let uniform = rig
                .pipe
                .skinning_uniform(&matrices)
                .map_err(|err| LookError::Skin(format!("{err:?}")))?;
            let joints = world.iter().map(|m| [m[3][0], m[3][1], m[3][2]]).collect();
            let mut body = BodyTruth::new(
                u8::try_from(slot_index).unwrap_or(u8::MAX),
                act_label(slot.act),
                pos,
                yaw,
                joints,
            );
            body.clip = u8::try_from(clip_index).unwrap_or(u8::MAX);
            body.clip_s = time;
            body.tinted = self.tick < slot.flash_until;
            let mut frames = Vec::with_capacity(rig.parts.len());
            for part in rig.parts.iter() {
                let posed = rig
                    .pipe
                    .render_skinned(&part.mesh, Some(&part.skin), &uniform)
                    .map_err(|err| LookError::Skin(format!("{err:?}")))?;
                body.fold_vertices(&posed.positions);
                if posed.used_error_material {
                    body.error_parts = body.error_parts.saturating_add(1);
                }
                frames.push(posed);
            }
            // LF17 sand rest, view only: lift a body by its below-sand depth
            // so the lowest vertex rests on GROUND_Y. The bounds come
            // from the CPU skin of the pose `slot_pose` returned (after the
            // `FALLEN_PELVIS_M` clamp), the same vertices the frame draws, so
            // the fix reads the pose, never the clip file. The lift is a +y
            // translation of the root and every posed vertex, at full depth
            // from the first tick. Every act rests on the sand (the fallen
            // pelvis sinks a start pose 56 cm, a fast run dips the feet a few
            // cm); above sand never pushes down; the sim hash never reads
            // this.
            let lift = if body.has_bounds() {
                sand_lift_m(body.min[1])
            } else {
                0.0
            };
            if lift > 0.0 {
                pos[1] += lift;
                body.root[1] += lift;
                body.min[1] += lift;
                body.max[1] += lift;
            }
            uploaded += upload_slot_parts(presenter, slot_index, rig, frames, lift)?;
            truth.push(body);
        }
        Ok((uploaded, truth))
    }

    /// Every body the last [`Self::present_into`] drew, as numbers.
    #[must_use]
    pub fn view_truth(&self) -> &[BodyTruth] {
        &self.truth
    }

    /// Sim time the last presented frame showed, in ticks: the last tick
    /// folded plus the interpolation `alpha`.
    #[must_use]
    pub fn sim_time(&self, alpha: f32) -> f64 {
        f64::from(self.tick) + f64::from(alpha)
    }

    /// Push one textured draw per resident body part; hit victims draw
    /// tinted for [`FLASH_TICKS`]. Missing keys draw nothing. Returns draws.
    pub fn push_draws(&self, list: &mut DrawList, resident: &[MeshKey], fighters: usize) -> usize {
        let mut pushed = 0usize;
        for slot_index in 0..fighters.min(MAX_LOOK_FIGHTERS) {
            let slot = &self.slots[slot_index];
            if !slot.seen {
                continue;
            }
            let rig = &self.rigs[self.rig_for_slot(slot_index)];
            let flash = self.tick < slot.flash_until;
            for (part_index, part) in rig.parts.iter().enumerate() {
                let key = look_mesh_key(slot_index, part_index);
                if !resident.contains(&key) {
                    continue;
                }
                let mut material = part.material.clone();
                if flash {
                    material.base_color = [
                        material.base_color[0],
                        material.base_color[1] * 0.35,
                        material.base_color[2] * 0.3,
                    ];
                    material.exposure = 1.6;
                }
                list.items.push(DrawItem {
                    mesh: key,
                    material,
                    base_color: part.texture.as_ref().map(|(key, _)| *key),
                });
                pushed += 1;
            }
        }
        pushed
    }

    /// Window and capture entry: load from [`fighters_content_dir`],
    /// upload every base-color map, then build and stage the fight ring
    /// for a match clamped at `half_extent_m`. Cold path; loud on refusal.
    pub fn load_and_stage(
        presenter: &mut Presenter,
        half_extent_m: f32,
    ) -> Result<Self, LookError> {
        // Keep the built GPU skin path constructed in the live window frame;
        // the CPU skin below stays the single render path.
        let _ = gpu_skin_adapter();
        let mut look = Self::load(&fighters_content_dir())?;
        look.stage_textures(presenter)?;
        look.stage_meshes(presenter)?;
        let ring = FightRing::load(&crate::arena_kit::kit_content_dir(), half_extent_m)
            .map_err(|err| LookError::Ring(err.to_string()))?;
        ring.stage(presenter)
            .map_err(|err| LookError::Ring(err.to_string()))?;
        look.ring = Some(ring);
        Ok(look)
    }

    /// One frame for the presenter: skin and upload every fighter, push the
    /// textured draws and aim the list camera for the frame's shot.
    /// Returns draws pushed; refusals are loud.
    pub fn present_into(
        &mut self,
        presenter: &mut Presenter,
        list: &mut DrawList,
        frame: &LookFrame,
    ) -> Result<usize, LookError> {
        let (_, truth) = self.stage_bodies(presenter, frame.fighters, frame.alpha)?;
        self.truth = truth;
        let resident = presenter.mesh_keys();
        if let Some(ring) = &self.ring {
            ring.push_draws(list, &resident);
        }
        let draws = self.push_draws(list, &resident, frame.fighters);
        if draws > 0 {
            list.camera = self.fight_camera(&list.camera, frame.shot, frame.fighters, frame.frame);
        }
        Ok(draws)
    }

    /// Seen fighters whose chest (1 m up) is in `camera`'s picture: the
    /// window's framing check, 0 when the shot shows nobody.
    #[must_use]
    pub fn fighters_in_view(&self, camera: &Camera, fighters: usize) -> usize {
        self.slots
            .iter()
            .take(fighters.min(MAX_LOOK_FIGHTERS))
            .filter(|slot| slot.seen && in_picture(camera, [slot.pos[0], 1.0, slot.pos[2]]))
            .count()
    }

    /// The hero's duel: midpoint, half separation and axis yaw toward the
    /// nearest other fighter (standing rivals first). `None` without a
    /// seen hero plus rival.
    fn duel(&self, fighters: usize) -> Option<([f32; 3], f32, f32)> {
        let hero = self.slots.first().filter(|slot| slot.seen)?;
        let rival = self
            .slots
            .iter()
            .take(fighters.min(MAX_LOOK_FIGHTERS))
            .skip(1)
            .filter(|slot| slot.seen)
            .min_by(|a, b| {
                let key = |slot: &SlotAnim| {
                    let (dx, dz) = (slot.pos[0] - hero.pos[0], slot.pos[2] - hero.pos[2]);
                    (slot.act == Act::Down, dx * dx + dz * dz)
                };
                let (ka, kb) = (key(a), key(b));
                ka.0.cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
            })?;
        let (dx, dz) = (rival.pos[0] - hero.pos[0], rival.pos[2] - hero.pos[2]);
        let half = 0.5 * (dx * dx + dz * dz).sqrt();
        let mid = [hero.pos[0] + 0.5 * dx, 0.0, hero.pos[2] + 0.5 * dz];
        Some((mid, half, dx.atan2(dz)))
    }

    /// View-only shake sample for the fight camera (LF11): one
    /// [`crate::camera::shake_offset`] sample for this tick's trauma, as an
    /// `[x, y, z]` vector added to both eye and target. Pure; zero trauma
    /// gives `[0.0; 3]`, so calm shots pass through untouched.
    fn fight_shake_vec(&self) -> [f32; 3] {
        let shake = crate::camera::shake_offset(self.shake_trauma_permille, self.tick, SHAKE_MAX_M);
        [shake.x, shake.y, shake.z]
    }

    /// Wide establishing camera aimed at `center` (LF11 shake included):
    /// keeps the base eye, aims at the fight, then moves both by the shake
    /// sample before validate. Pure presentation.
    fn wide_camera(&self, base: &Camera, center: [f32; 3]) -> Camera {
        let shake = self.fight_shake_vec();
        let mut camera = base.clone();
        camera.target = [center[0], 1.0, center[2]].into();
        let eye_arr = camera.eye.to_array();
        camera.eye = [
            eye_arr[0] + shake[0],
            eye_arr[1] + shake[1],
            eye_arr[2] + shake[2],
        ]
        .into();
        let tgt_arr = camera.target.to_array();
        camera.target = [
            tgt_arr[0] + shake[0],
            tgt_arr[1] + shake[1],
            tgt_arr[2] + shake[2],
        ]
        .into();
        if camera.validate().is_ok() {
            camera
        } else {
            base.clone()
        }
    }

    /// Fight camera for `shot`: aims at the hero, the whole fight, or a
    /// wide arena view, orbiting slowly with `frame`. Pure presentation.
    /// View-only shake (LF11): the same [`crate::camera::shake_offset`]
    /// offset moves both eye and target, so the frame translates while the
    /// view direction holds; zero trauma leaves the shot untouched.
    #[must_use]
    pub fn fight_camera(&self, base: &Camera, shot: Shot, fighters: usize, frame: u32) -> Camera {
        let seen: Vec<[f32; 3]> = self
            .slots
            .iter()
            .take(fighters.min(MAX_LOOK_FIGHTERS))
            .filter(|slot| slot.seen)
            .map(|slot| slot.pos)
            .collect();
        let Some(first) = seen.first().copied() else {
            return base.clone();
        };
        let count = f32::from(u8::try_from(seen.len()).unwrap_or(u8::MAX));
        let mut center = [0.0f32; 3];
        for pos in &seen {
            center[0] += pos[0] / count;
            center[2] += pos[2] / count;
        }
        let mut radius = 0.0f32;
        for pos in &seen {
            let (dx, dz) = (pos[0] - center[0], pos[2] - center[2]);
            radius = radius.max((dx * dx + dz * dz).sqrt());
        }
        let turn = f32::from(u16::try_from(frame % 3600).unwrap_or(0)) * 0.0025;
        let (focus, distance, height, look_y, azimuth) = match shot {
            Shot::Hero => {
                let hero = self.slots[0];
                let at = if hero.seen { hero.pos } else { first };
                (at, 3.4, 1.55, 1.15, hero.yaw + 0.55 + turn * 0.2)
            }
            Shot::Action => match self.duel(fighters) {
                // Side-on to the hero's duel: both fighters across frame.
                Some((mid, half, axis)) => (
                    mid,
                    (half * 2.2 + 3.2).clamp(3.5, 7.0),
                    0.0,
                    1.0,
                    axis + std::f32::consts::FRAC_PI_2 + turn * 0.3,
                ),
                None => (
                    center,
                    (radius * 1.9 + 4.5).clamp(5.0, 16.0),
                    0.0,
                    1.0,
                    0.9 + turn,
                ),
            },
            Shot::Wide => {
                return self.wide_camera(base, center);
            }
        };
        let height = if height > 0.0 {
            height
        } else {
            1.6 + distance * 0.22
        };
        // Keep the eye inside the pit so the wall never blocks the shot.
        let limit = self
            .ring
            .as_ref()
            .map_or(f32::MAX, |ring| ring.wall_m - 0.6);
        let (azimuth, reach) = pit_orbit(focus, azimuth, distance, limit);
        // View-only shake (LF11): one sample moves both eye and target
        // before validate, so the same ticks give the same offsets.
        let shake = self.fight_shake_vec();
        let eye = [
            focus[0] + azimuth.sin() * reach + shake[0],
            height.min(look_y + reach * MAX_PITCH_SLOPE) + shake[1],
            focus[2] + azimuth.cos() * reach + shake[2],
        ];
        let target = [focus[0] + shake[0], look_y + shake[1], focus[2] + shake[2]];
        let mut camera = base.clone();
        camera.eye = eye.into();
        camera.target = target.into();
        camera.up = [0.0, 1.0, 0.0].into();
        camera.fov_y = 60.0f32.to_radians();
        camera.near = 0.1;
        camera.far = camera.far.max(120.0);
        if camera.validate().is_ok() {
            camera
        } else {
            base.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn content_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../content/fighters")
    }

    fn view(slot: u8, x: f32, z: f32) -> FighterView {
        FighterView {
            slot,
            pos_m: [x, 0.0, z],
            yaw: 0.0,
        }
    }

    #[test]
    fn root_matrix_turns_forward_onto_yaw() {
        let m = root_matrix([1.0, 0.0, 2.0], std::f32::consts::FRAC_PI_2);
        // Forward +Z at yaw 90 degrees points along +X; translation last row.
        let forward = [m[2][0], m[2][1], m[2][2]];
        assert!((forward[0] - 1.0).abs() < 1e-6 && forward[2].abs() < 1e-6);
        assert_eq!(m[3], [1.0, 0.0, 2.0, 1.0]);
        let id = mul(&m, &root_matrix([0.0; 3], 0.0));
        assert_eq!(id, m);
    }

    #[test]
    fn directed_runs_split_into_hero_wide_action_thirds() {
        assert_eq!(shot_for_frame(0, Some(120), true), Shot::Hero);
        assert_eq!(shot_for_frame(39, Some(120), true), Shot::Hero);
        assert_eq!(shot_for_frame(40, Some(120), true), Shot::Wide);
        assert_eq!(shot_for_frame(80, Some(120), true), Shot::Action);
        assert_eq!(shot_for_frame(0, Some(120), false), Shot::Action);
        assert_eq!(shot_for_frame(0, None, true), Shot::Action);
    }

    #[test]
    fn fight_camera_eye_backs_off_a_wall_instead_of_hovering() {
        // A duel 1 m off the +z wall with the side-on eye wanted toward it:
        // per-axis clamping parked that eye 1 m from the focus and 4.5 m up.
        let (limit, focus) = (9.4, [0.0, 0.0, 8.4]);
        let (azimuth, reach) = pit_orbit(focus, 0.0, 9.0, limit);
        assert!((reach - 9.0).abs() < 1e-4, "full distance on the open side");
        let eye_z = focus[2] + azimuth.cos() * reach;
        assert!(eye_z.abs() <= limit + 1e-3, "eye inside the pit: {eye_z}");
        assert!(focus[2] - eye_z > 8.0, "eye backs into the pit: {eye_z}");
        assert_eq!(pit_orbit([0.0; 3], 0.3, 9.0, f32::MAX), (0.3, 9.0));
    }

    #[test]
    fn fight_camera_framing_sees_a_side_on_duel_and_not_a_top_down_one() {
        let side = Camera::new(
            [0.0, 4.5, 9.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        );
        assert!(in_picture(&side, [-3.0, 1.0, 0.0]) && in_picture(&side, [3.0, 1.0, 0.0]));
        assert!(!in_picture(&side, [0.0, 1.0, 12.0]), "behind the eye");
        assert!(look_down_deg(&side) > 20.0 && look_down_deg(&side) < 25.0);
        // The 2026-10-11 shot: eye 1 m off the duel's middle, 4.5 m up, fighters 5 m either side.
        let top = Camera::new(
            [0.0, 4.5, 1.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        );
        assert!(!in_picture(&top, [-5.0, 1.0, 0.0]) && !in_picture(&top, [5.0, 1.0, 0.0]));
        assert!(look_down_deg(&top) > 70.0);
    }

    /// Group-shot camera state without bodies: seven seen slots across
    /// +-6 m, hero unseen so `Shot::Action` frames the whole bout through
    /// [`FighterLook::fight_camera`], the path the widening changed.
    fn spread_look() -> FighterLook {
        let mut slots = vec![SlotAnim::new(); MAX_LOOK_FIGHTERS];
        for (slot, x) in slots
            .iter_mut()
            .skip(1)
            .zip([-6.0, -4.0, -2.0, 0.0, 2.0, 4.0, 6.0])
        {
            slot.seen = true;
            slot.pos = [x, 0.0, 0.0];
        }
        FighterLook {
            rigs: Vec::new(),
            ring: None,
            slots,
            tick: 0,
            swing_ticks: 1,
            bundle_shas: Vec::new(),
            truth: Vec::new(),
            shake_trauma_permille: 0,
        }
    }

    #[test]
    fn fight_camera_widens_group_shot_to_60deg_and_frames_the_bout() {
        let mut look = spread_look();
        let base = Camera::new(
            [0.0, 5.0, 12.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        );
        let cam = look.fight_camera(&base, Shot::Action, 8, 0);
        assert!(
            (cam.fov_y.to_degrees() - 60.0).abs() < 1e-4,
            "group shot rides the widened lens, got {} deg",
            cam.fov_y.to_degrees()
        );
        assert_eq!(
            look.fighters_in_view(&cam, 8),
            7,
            "the whole spread stays in the 60 deg picture"
        );
        // The duel branch rides the same lens: hero plus nearest rival
        // framed side-on at 60 deg.
        look.slots[0].seen = true;
        look.slots[0].pos = [20.0, 0.0, 0.0];
        let duel_cam = look.fight_camera(&base, Shot::Action, 8, 0);
        assert!(
            (duel_cam.fov_y.to_degrees() - 60.0).abs() < 1e-4,
            "duel shot rides the widened lens, got {} deg",
            duel_cam.fov_y.to_degrees()
        );
        assert!(
            in_picture(&duel_cam, [20.0, 1.0, 0.0]) && in_picture(&duel_cam, [7.0, 1.0, 0.0]),
            "hero plus nearest rival stay in the duel picture"
        );
    }

    #[test]
    fn fight_camera_without_seen_fighters_returns_base_untouched() {
        // Failure path: nobody seen, the base camera (lens included)
        // passes through instead of framing an empty bout.
        let look = FighterLook {
            rigs: Vec::new(),
            ring: None,
            slots: vec![SlotAnim::new(); MAX_LOOK_FIGHTERS],
            tick: 0,
            swing_ticks: 1,
            bundle_shas: Vec::new(),
            truth: Vec::new(),
            shake_trauma_permille: 0,
        };
        let base = Camera::new(
            [0.0, 5.0, 12.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        );
        let cam = look.fight_camera(&base, Shot::Action, 8, 0);
        assert_eq!(cam, base, "empty bout keeps the base camera");
    }

    #[test]
    fn fight_camera_eye_stays_in_the_pit_from_every_corner() {
        for limit in [9.4f32, 4.0] {
            for focus in [
                [limit, 0.0, limit],
                [-limit, 0.0, limit],
                [limit, 0.0, -limit],
                [-limit, 0.0, -limit],
            ] {
                for step in 0..16u8 {
                    let wanted = f32::from(step) * std::f32::consts::FRAC_PI_8;
                    let (azimuth, reach) = pit_orbit(focus, wanted, 9.0, limit);
                    let (x, z) = (
                        focus[0] + azimuth.sin() * reach,
                        focus[2] + azimuth.cos() * reach,
                    );
                    assert!(
                        x.abs() <= limit + 1e-3 && z.abs() <= limit + 1e-3,
                        "eye ({x}, {z}) left the pit"
                    );
                    // The roomiest side always reaches across most of the pit.
                    assert!(
                        reach >= 9.0f32.min(limit * 1.4),
                        "reach {reach} at {wanted} in {limit}"
                    );
                    let rise = 4.5f32.min(1.0 + reach * MAX_PITCH_SLOPE) - 1.0;
                    assert!(
                        rise / reach <= MAX_PITCH_SLOPE + 1e-4,
                        "never a top-down look"
                    );
                }
            }
        }
    }

    #[test]
    fn angle_delta_takes_the_short_way() {
        let d = angle_delta(3.0, -3.0);
        assert!(d > 0.0 && d < 0.3, "wraps through PI, got {d}");
        assert!((angle_delta(0.0, 1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn look_keys_never_collide_across_slots_and_parts() {
        let mut keys = Vec::new();
        for slot in 0..MAX_LOOK_FIGHTERS {
            for part in 0..4 {
                keys.push(look_mesh_key(slot, part));
            }
        }
        let total = keys.len();
        keys.sort();
        keys.dedup();
        assert_eq!(keys.len(), total);
        assert!(keys.iter().all(|key| key.id >= LOOK_MESH_ID_BASE));
    }

    #[test]
    fn ct7_bodies_load_with_clips_and_textures() {
        let look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        assert_eq!(look.rigs.len(), 4);
        assert_eq!(
            look.bundle_shas(),
            &fighter_bind::EXPECTED_BUNDLE_SHA256[..]
        );
        assert_eq!(look.variant_for_slot(0), "hero");
        assert_ne!(look.variant_for_slot(1), "hero");
        for rig in &look.rigs {
            assert_eq!(rig.clips.len(), CLIP_FILES.len());
            assert!(rig.parts.iter().all(|part| part.texture.is_some()));
        }
    }

    #[test]
    fn rest_pose_skins_onto_the_bind_mesh_at_the_root() {
        let look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let rig = &look.rigs[0];
        // Bind pose through skinning must give the bind mesh moved by root.
        let bind = engine_anim::contract::LocalPose {
            positions: (0..rig.skeleton.len())
                .map(|j| rig.clips[0].positions.tracks[j].values[0])
                .collect(),
            rotations: (0..rig.skeleton.len())
                .map(|j| rig.clips[0].rotations[j].values[0])
                .collect(),
        };
        let world = skinning_matrices(&rig.skeleton, &bind).expect("pose");
        let root = root_matrix([3.0, 0.0, -2.0], 0.0);
        let matrices: Vec<[[f32; 4]; 4]> = world
            .iter()
            .zip(rig.inverse_binds.iter())
            .map(|(joint, inverse)| mul(&mul(inverse, joint), &root))
            .collect();
        let uniform = rig.pipe.skinning_uniform(&matrices).expect("uniform");
        let part = &rig.parts[0];
        let posed = rig
            .pipe
            .render_skinned(&part.mesh, Some(&part.skin), &uniform)
            .expect("skin");
        // Idle frame 0 is close to bind: every vertex lands near its bind
        // spot plus the root offset, and the body stays on its feet.
        let mut min_y = f32::MAX;
        let mut max_y = f32::MIN;
        for p in &posed.positions {
            min_y = min_y.min(p[1]);
            max_y = max_y.max(p[1]);
        }
        assert!(max_y > 1.0 && max_y < 2.2, "standing height, got {max_y}");
        assert!(min_y > -0.2, "feet stay above the sand, got {min_y}");
        let (min_x, max_x) = posed
            .positions
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), p| {
                (lo.min(p[0]), hi.max(p[0]))
            });
        assert!(
            min_x > 2.0 && max_x < 4.0,
            "root moved the body to x=3, got {min_x}..{max_x}"
        );
    }

    #[test]
    fn cues_drive_swings_tint_and_knockdown() {
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(0, &views, &[1000, 1000], &[]);
        assert_eq!(look.act(0), Some(Act::Idle));
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 100,
        };
        look.observe_tick(20, &views, &[1000, 900], &[hit]);
        assert!(matches!(look.act(0), Some(Act::Attack(_))));
        assert!(look.slots[1].flash_until > 20);
        // Facing turns toward the rival (+X from slot 0).
        assert!(look.slots[0].yaw > 0.0);
        look.observe_tick(21, &views, &[1000, 0], &[]);
        assert_eq!(look.act(1), Some(Act::Down));
        // Running: a slot that moves faster than the run floor runs.
        let moved = [view(0, -0.9, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(80, &moved, &[1000, 0], &[]);
        assert!(matches!(look.act(0), Some(Act::Attack(_)) | Some(Act::Run)));
    }

    fn balance_look_two() -> FighterLook {
        let mut slots = vec![SlotAnim::new(); MAX_LOOK_FIGHTERS];
        for (s, x) in slots.iter_mut().take(2).zip([-1.0, 1.0]) {
            s.seen = true;
            s.pos = [x, 0.0, 0.0];
            s.prev_pos = s.pos;
        }
        FighterLook {
            rigs: Vec::new(),
            ring: None,
            slots,
            tick: 0,
            swing_ticks: 1,
            bundle_shas: Vec::new(),
            truth: Vec::new(),
            shake_trauma_permille: 0,
        }
    }

    #[test]
    fn balance_look_floored_lies_flat() {
        use engine_arena::{ArenaConfig, ArenaRules, LiveMatch};
        let cfg = ArenaConfig {
            seed: 2040,
            ticks: 20,
            fighters: 2,
            half_extent_m: 10.0,
            table_version: 2,
        };
        let mut sim =
            LiveMatch::with_rules(&cfg, ArenaRules::v2_free_for_all()).expect("v2 builds");
        for _ in 0..10u32 {
            let _ = sim.step(&[]);
        }
        assert_eq!(sim.tick(), 10, "scripted v2 bout runs 10 ticks");
        let mut look = balance_look_two();
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        let kd = FightCue::Knockdown {
            attacker: 0,
            victim: 1,
        };
        look.observe_tick(0, &views, &[1000, 1000], &[kd]);
        for t in 1..10u32 {
            look.observe_tick(t, &views, &[1000, 1000], &[]);
        }
        assert_eq!(look.slots[1].balance, GaitPhase::Fallen);
        assert!(balance_pelvis_y_m(GaitPhase::Fallen, 0.9) < 0.4);
        look.stumble_slot(99);
        look.floor_slot(99);
        assert_eq!(look.slots[1].balance, GaitPhase::Fallen, "bad slot ignored");
    }

    #[test]
    fn balance_look_standing_keeps_pose() {
        assert_eq!(balance_pelvis_y_m(GaitPhase::Steady, 0.9), 0.9);
        assert_eq!(balance_lean_m(GaitPhase::Steady), 0.0);
        assert_eq!(balance_pelvis_y_m(GaitPhase::Steady, f32::NAN), 0.0);
        let mut look = balance_look_two();
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(0, &views, &[1000, 1000], &[]);
        assert_eq!(look.slots[0].balance, GaitPhase::Steady);
        assert_eq!(look.slots[0].pos, [-1.0, 0.0, 0.0]);
    }

    #[test]
    fn sand_lift_rests_the_lowest_vertex_and_never_pushes_down() {
        // The lift reads the pose-skinned bounds of every act. Above sand and
        // non-finite never lift.
        assert_eq!(sand_lift_m(0.05), 0.0, "above sand never pushes down");
        assert_eq!(sand_lift_m(0.0), 0.0, "resting exactly on sand needs no lift");
        assert_eq!(sand_lift_m(f32::NAN), 0.0, "non-finite never lifts");
        assert_eq!(sand_lift_m(f32::INFINITY), 0.0, "infinite never lifts");
        assert_eq!(sand_lift_m(f32::NEG_INFINITY), 0.0, "infinite never lifts");
    }

    #[test]
    fn view_truth_down_lift_rests_the_fallen_start_pose_at_once() {
        // The product floor: the upright start of the death clip with the
        // fallen pelvis sank 56.1 cm; the lift must bring it back to the
        // sand on the very first tick (it was 0 there before, then 20 cm).
        let sunk = -0.561;
        let lift = sand_lift_m(sunk);
        assert!(
            (lift - 0.561).abs() < 1e-6,
            "full depth lifts at once, got {lift}"
        );
        assert!(
            (sunk + lift).abs() * 100.0 < 3.0,
            "the lifted body sits within the 3 cm sunk floor"
        );
    }

    #[test]
    fn view_truth_sand_lift_matches_sink_and_caps_at_one_meter() {
        assert!((sand_lift_m(-0.13) - 0.13).abs() < 1e-6);
        assert_eq!(sand_lift_m(-5.0), SAND_LIFT_MAX_M, "a bad pose never flies");
    }

    #[test]
    fn view_truth_down_lift_stable_x2() {
        assert_eq!(sand_lift_m(-0.13), sand_lift_m(-0.13));
        assert_eq!(sand_lift_m(-0.561), sand_lift_m(-0.561));
    }

    #[test]
    fn balance_look_digest_equal_x2() {
        use engine_arena::{ArenaConfig, ArenaRules, LiveMatch};
        let cfg = ArenaConfig {
            seed: 2040,
            ticks: 10,
            fighters: 2,
            half_extent_m: 10.0,
            table_version: 1,
        };
        let run = |with_look: bool| {
            let mut sim = LiveMatch::with_rules(&cfg, ArenaRules::V1).expect("v1 builds");
            let mut look = balance_look_two();
            let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
            let mut hashes = Vec::new();
            for t in 0..10u32 {
                if let Some(tick) = sim.step(&[]) {
                    hashes.push(tick.checkpoint.hash);
                }
                if with_look {
                    look.observe_tick(t, &views, &[1000, 1000], &[]);
                }
            }
            hashes
        };
        assert_eq!(run(false), run(true));
        assert_eq!(run(true), run(true), "same look twice hashes twice");
    }

    #[test]
    fn a_hit_from_across_the_pit_swings_but_never_tints_the_victim() {
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let far = [view(0, -4.0, 0.0), view(1, 4.0, 0.0)];
        look.observe_tick(0, &far, &[1000, 1000], &[]);
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 100,
        };
        look.observe_tick(20, &far, &[1000, 900], &[hit]);
        assert!(matches!(look.act(0), Some(Act::Attack(_))));
        assert_eq!(look.slots[1].flash_until, 0, "8 m away: no tint");
        assert_eq!(look.tinted(2), (0, 2));
        let near = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(40, &near, &[1000, 800], &[hit]);
        assert_eq!(look.slots[1].flash_until, 40 + FLASH_TICKS);
        assert_eq!(look.tinted(2), (1, 2));
    }

    #[test]
    fn fighter_anim_same_tick_gives_same_clip_and_phase() {
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        let hp = [1000u16, 1000u16];
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 100,
        };
        let mut first = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let mut second = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        for tick in 0..=10u32 {
            let cues = if tick == 5 { vec![hit] } else { vec![] };
            first.observe_tick(tick, &views, &hp, &cues);
            second.observe_tick(tick, &views, &hp, &cues);
        }
        for alpha in [0.25, 0.5] {
            let rig = &first.rigs[first.rig_for_slot(0)];
            let other = &second.rigs[second.rig_for_slot(0)];
            let a = first
                .slot_pose(rig, &first.slots[0], alpha)
                .expect("pose samples");
            let b = second
                .slot_pose(other, &second.slots[0], alpha)
                .expect("pose samples");
            assert_eq!(a, b);
        }
    }

    #[test]
    fn fighter_anim_run_phase_scales_with_speed() {
        let mut slow = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let mut fast = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        for tick in 0..=20u32 {
            let slow_x = -1.0 + 0.025 * f32::from(u16::try_from(tick).unwrap_or(u16::MAX));
            let fast_x = -1.0 + 0.10 * f32::from(u16::try_from(tick).unwrap_or(u16::MAX));
            let slow_views = [view(0, slow_x, 0.0), view(1, 5.0, 0.0)];
            let fast_views = [view(0, fast_x, 0.0), view(1, 5.0, 0.0)];
            slow.observe_tick(tick, &slow_views, &[1000, 1000], &[]);
            fast.observe_tick(tick, &fast_views, &[1000, 1000], &[]);
        }
        assert_eq!(slow.act(0), Some(Act::Run));
        assert_eq!(fast.act(0), Some(Act::Run));
        let slow_slot = slow.slots[0];
        let fast_slot = fast.slots[0];
        assert!((slow_slot.run_speed - 0.025).abs() < 1e-6);
        assert!((fast_slot.run_speed - 0.10).abs() < 1e-6);
        let slow_rig = &slow.rigs[slow.rig_for_slot(0)];
        let fast_rig = &fast.rigs[fast.rig_for_slot(0)];
        let (_, slow_t) = slow.clip_time(
            slow_rig,
            slow_slot.act,
            slow_slot.act_start,
            slow_slot.run_speed,
            0.5,
        );
        let (_, fast_t) = fast.clip_time(
            fast_rig,
            fast_slot.act,
            fast_slot.act_start,
            fast_slot.run_speed,
            0.5,
        );
        assert!(
            fast_t > slow_t,
            "fast {fast_t} must lead slow {slow_t} at the same tick"
        );
        let (_, slow_again) = slow.clip_time(
            slow_rig,
            slow_slot.act,
            slow_slot.act_start,
            slow_slot.run_speed,
            0.5,
        );
        let (_, fast_again) = fast.clip_time(
            fast_rig,
            fast_slot.act,
            fast_slot.act_start,
            fast_slot.run_speed,
            0.5,
        );
        assert_eq!(slow_t, slow_again);
        assert_eq!(fast_t, fast_again);
    }

    #[test]
    fn staged_bodies_rewrite_their_buffers_in_place_every_frame() {
        let mut presenter = match Presenter::init_offscreen() {
            Ok(presenter) => presenter,
            Err(engine_render::present::PresentError::NoAdapter) => {
                eprintln!("SKIPPED fighter buffer test: no wgpu adapter on this machine");
                return;
            }
            Err(err) => panic!("presenter init failed (must fail loudly): {err}"),
        };
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let staged = look.stage_meshes(&mut presenter).expect("bodies stage");
        assert!(staged >= MAX_LOOK_FIGHTERS, "every slot owns its parts");
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(0, &views, &[1000, 1000], &[]);
        let created = presenter.gpu_create_totals();
        for step in 0..5u8 {
            let written = look
                .stage_frame(&mut presenter, 2, f32::from(step) * 0.2)
                .expect("frame stages");
            assert!(written >= 2, "both fighters write their parts");
        }
        assert_eq!(
            presenter.gpu_create_totals(),
            created,
            "five skinned frames rewrite the staged buffers and create none"
        );
        // A fighter that shows up later already owns its buffers.
        let three = [view(0, -1.0, 0.0), view(1, 1.0, 0.0), view(2, 0.0, 3.0)];
        look.observe_tick(1, &three, &[1000, 1000, 1000], &[]);
        look.stage_frame(&mut presenter, 3, 0.5)
            .expect("late fighter stages");
        assert_eq!(
            presenter.gpu_create_totals(),
            created,
            "a late slot creates no buffer either"
        );
    }

    #[test]
    fn gpu_skin_probe_runs_twice_with_same_answer() {
        // Behavior plus headless failure path: on a machine without a GPU
        // the probe reads as `None` (skip, never fail); with a GPU it reads
        // as `Some` adapter name. Either way it never panics and repeats.
        let first = super::gpu_skin_adapter();
        let second = super::gpu_skin_adapter();
        assert_eq!(first, second, "probe is stable across calls");
    }

    #[test]
    fn gpu_skin_probe_leaves_cpu_skin_vectors_unchanged() {
        // The live window entry probes the GPU path while the CPU skin stays
        // the single render path: the skinned vertex digest below must read
        // the same before and after the probe.
        use sha2::{Digest, Sha256};
        let look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let rig = &look.rigs[0];
        let bind = engine_anim::contract::LocalPose {
            positions: (0..rig.skeleton.len())
                .map(|j| rig.clips[0].positions.tracks[j].values[0])
                .collect(),
            rotations: (0..rig.skeleton.len())
                .map(|j| rig.clips[0].rotations[j].values[0])
                .collect(),
        };
        let world = skinning_matrices(&rig.skeleton, &bind).expect("pose");
        let root = root_matrix([0.0, 0.0, 0.0], 0.0);
        let matrices: Vec<[[f32; 4]; 4]> = world
            .iter()
            .zip(rig.inverse_binds.iter())
            .map(|(joint, inverse)| mul(&mul(inverse, joint), &root))
            .collect();
        let digest_of = |matrices: &[[[f32; 4]; 4]]| {
            let uniform = rig.pipe.skinning_uniform(matrices).expect("uniform");
            let part = &rig.parts[0];
            let posed = rig
                .pipe
                .render_skinned(&part.mesh, Some(&part.skin), &uniform)
                .expect("skin");
            let mut hasher = Sha256::new();
            for p in &posed.positions {
                for c in p {
                    hasher.update(c.to_le_bytes());
                }
            }
            hasher.finalize().to_vec()
        };
        let before = digest_of(&matrices);
        let _ = super::gpu_skin_adapter();
        let after = digest_of(&matrices);
        assert_eq!(before, after, "probe must not move the skin vectors");
    }

    #[test]
    fn shake_trauma_hit_adds_and_settles_within_one_second() {
        // View-only shake: an in-reach Hit adds 400 permille, then the
        // 1000-per-second decay clears it within 60 ticks (1.0 s at 60 Hz).
        let mut look = balance_look_two();
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(0, &views, &[1000, 1000], &[]);
        assert_eq!(look.shake_trauma_permille, 0, "no impacts yet: no shake");
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 100,
        };
        look.observe_tick(10, &views, &[1000, 900], &[hit]);
        assert_eq!(
            look.shake_trauma_permille, SHAKE_HIT_PERMILLE,
            "one in-reach Hit adds 400 permille"
        );
        // The fight camera carries the shake: same offset `shake_offset`
        // reports for this trauma plus tick, on both eye and target.
        let base = Camera::new(
            [0.0, 5.0, 12.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        );
        let shaken = look.fight_camera(&base, Shot::Action, 2, 0);
        let expected = crate::camera::shake_offset(SHAKE_HIT_PERMILLE, look.tick, SHAKE_MAX_M);
        assert_ne!(
            (expected.x, expected.y, expected.z),
            (0.0, 0.0, 0.0),
            "400 permille at tick 10 shakes (non-zero offset)"
        );
        // A calm twin (same slots, same tick, no Hit) frames the same shot
        // without the offset: both eye and target move by `expected`.
        let mut calm = balance_look_two();
        calm.observe_tick(0, &views, &[1000, 1000], &[]);
        calm.observe_tick(10, &views, &[1000, 900], &[]);
        let unshaken = calm.fight_camera(&base, Shot::Action, 2, 0);
        let eye_delta = shaken.eye - unshaken.eye;
        let target_delta = shaken.target - unshaken.target;
        for (got, want, axis) in [
            (eye_delta.x, expected.x, "eye x"),
            (eye_delta.y, expected.y, "eye y"),
            (eye_delta.z, expected.z, "eye z"),
            (target_delta.x, expected.x, "target x"),
            (target_delta.y, expected.y, "target y"),
            (target_delta.z, expected.z, "target z"),
        ] {
            assert!(
                (got - want).abs() < 1e-6,
                "{axis} carries the shake offset: got {got}, want {want}"
            );
        }
        for t in 11..=70u32 {
            look.observe_tick(t, &views, &[1000, 900], &[]);
        }
        assert_eq!(
            look.shake_trauma_permille, 0,
            "400 permille decays to 0 within 60 ticks (1.0 s)"
        );
        let rest = crate::camera::shake_offset(0, look.tick, SHAKE_MAX_M);
        assert_eq!((rest.x, rest.y, rest.z), (0.0, 0.0, 0.0));
        // Settled: the shaken shot matches the calm shot exactly.
        let settled = look.fight_camera(&base, Shot::Action, 2, 0);
        let mut calm_late = balance_look_two();
        for t in 0..=70u32 {
            calm_late.observe_tick(t, &views, &[1000, 900], &[]);
        }
        let calm_cam = calm_late.fight_camera(&base, Shot::Action, 2, 0);
        assert_eq!(settled.eye, calm_cam.eye, "settled eye matches calm");
        assert_eq!(
            settled.target, calm_cam.target,
            "settled target matches calm"
        );
    }

    #[test]
    fn shake_trauma_knockdown_clamps_and_repeats() {
        // Two in-reach Knockdowns (700 + 700) clamp at 1000, and the same
        // cue script plus the same ticks give the same camera twice.
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        let kd = FightCue::Knockdown {
            attacker: 0,
            victim: 1,
        };
        let mut first = balance_look_two();
        let mut second = balance_look_two();
        for look in [&mut first, &mut second] {
            look.observe_tick(0, &views, &[1000, 1000], &[]);
            look.observe_tick(10, &views, &[1000, 0], &[kd]);
            look.observe_tick(11, &views, &[1000, 0], &[kd]);
        }
        assert_eq!(
            first.shake_trauma_permille, SHAKE_MAX_PERMILLE,
            "700 + 700 clamps at 1000, got {}",
            first.shake_trauma_permille
        );
        assert_eq!(first.shake_trauma_permille, second.shake_trauma_permille);
        let base = Camera::new(
            [0.0, 5.0, 12.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        );
        let a = first.fight_camera(&base, Shot::Action, 2, 7);
        let b = second.fight_camera(&base, Shot::Action, 2, 7);
        assert_eq!(a.eye, b.eye, "same ticks give same shaken eye");
        assert_eq!(a.target, b.target, "same ticks give same shaken target");
    }

    #[test]
    fn shake_trauma_out_of_reach_and_non_impacts_never_shake() {
        // Failure paths: a Hit from 8 m away never shakes (same gate as the
        // hit tint), and Blocked/Parried/Dodged/StaminaOut never shake even
        // in reach.
        let mut look = balance_look_two();
        let far = [view(0, -4.0, 0.0), view(1, 4.0, 0.0)];
        look.observe_tick(0, &far, &[1000, 1000], &[]);
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 100,
        };
        look.observe_tick(20, &far, &[1000, 900], &[hit]);
        assert_eq!(
            look.shake_trauma_permille, 0,
            "8 m away: no shake (out of reach)"
        );
        let near = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(40, &near, &[1000, 1000], &[]);
        for cue in [
            FightCue::Blocked {
                attacker: 0,
                victim: 1,
                chip: 10,
            },
            FightCue::Parried {
                attacker: 0,
                victim: 1,
            },
            FightCue::Dodged {
                attacker: 0,
                victim: 1,
            },
            FightCue::StaminaOut { fighter: 0 },
        ] {
            look.observe_tick(41, &near, &[1000, 1000], &[cue]);
            assert_eq!(
                look.shake_trauma_permille, 0,
                "non-impact {cue:?} never shakes"
            );
        }
    }
}
