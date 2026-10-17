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
//! upload per body part (4 per fighter) whose 48 B vertices changed since
//! its last upload ([`upload_slot_parts`]).

use std::path::{Path, PathBuf};
use std::time::Instant;

use engine_anim::contract::{skinning_matrices, LocalPose, Skeleton};
use engine_anim::hit_react::{envelope, DIP_M, PEAK_M};
use engine_anim::pose::{blend_poses, sample_pose_with_rots};
use engine_anim::state_machine::FIGHTER_CROSSFADE_TICKS;
use engine_arena::GaitPhase;
use engine_assets::body_import::{
    import_clip_source, import_skinned_body, ClipSource, SkinnedBody,
};
use engine_assets::retarget::{retarget_clip, BoneMap};
use engine_assets::skin_import::ImportedClip;
use engine_core::Fnv64;
use engine_render::present::{DrawItem, DrawList, MeshKey, Presenter, TextureKey, TextureUpload};
use engine_render::v1::{self, Camera, MeshData, PbrMaterial};
use engine_render::v1_skin::{SkinPipeline, SkinWeights};
use sha2::{Digest, Sha256};

use crate::anim_bridge::FighterSlot;
use crate::fight_ring::FightRing;
use crate::fighter_bind::{self, FighterBinding};
use crate::stage_bodies::place_joint;
use crate::view_truth::BodyTruth;
use crate::weapon_trail::{TrailPoint, TrailRing};

/// Fold plus bridge bulk, moved whole into the child module below so this
/// file stays under the size cap: [`FighterLook::observe_tick`] and the act
/// fold live there, on the same paths as before. Presentation only.
#[path = "fighter_look_acts.rs"]
mod fighter_look_acts;

use self::fighter_look_acts::{bridge_act, bridge_slots};

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
/// Single owner of the rate is `engine_core::TICK_HZ`.
const TICK_HZ: f32 = engine_core::TICK_HZ as f32;
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
/// Fall ticks to reach the sand (LF17, view only): 30 at 60 Hz.
const SAND_FALL_TICKS: u32 = 30;
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
/// Shake cap in meters at full trauma (LF11): 0.12 m, the arena director
/// default, so the fight camera matches the staged rig at full trauma.
const SHAKE_MAX_M: f32 = 0.12;

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
    /// Skin radius in meters: the farthest any bind-pose vertex sits from
    /// its nearest bind joint, plus a 10 cm margin. The joints-only pose
    /// pads placed joints by this radius so its bounds hold the skinned
    /// vertex bounds; cold-computed at load, read-only on the hot path.
    radius_m: f32,
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
    prev_clip: usize,
    prev_time: f32,
    fade_left: u32,
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
    /// First tick the additive flinch shows on (`u32::MAX` reads as none).
    /// Presentation only; the sim hash never reads it.
    hit_start: u32,
    /// World-space planar push away from the attacker at `hit_start`,
    /// unit length or zero. Presentation only.
    hit_push: [f32; 2],
}

impl SlotAnim {
    const fn new() -> Self {
        Self {
            act: Act::Idle,
            act_start: 0,
            prev_clip: 0,
            prev_time: 0.0,
            fade_left: 0,
            swings: 0,
            prev_pos: [0.0; 3],
            pos: [0.0; 3],
            prev_yaw: 0.0,
            yaw: 0.0,
            flash_until: 0,
            seen: false,
            run_speed: 0.0,
            balance: GaitPhase::Steady,
            hit_start: u32::MAX,
            hit_push: [0.0, 0.0],
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

/// Full below-sand depth, clamped `0.0..=SAND_LIFT_MAX_M` (LF17, view only).
/// Pure `f32` in/out; non-finite or above sand reads as 0, never lowers.
#[must_use]
fn sand_lift_m(lowest_y: f32) -> f32 {
    if !lowest_y.is_finite() {
        return 0.0;
    }
    (crate::view_truth::GROUND_Y - lowest_y).clamp(0.0, SAND_LIFT_MAX_M)
}

/// Eased down lift (LF17, view only): full depth times smoothstep age over
/// `SAND_FALL_TICKS`. Age 0 gives 0, rest gives full depth within 1 cm.
#[must_use]
fn sand_lift_eased(lowest_y: f32, down_age_ticks: u32) -> f32 {
    let full = sand_lift_m(lowest_y);
    if full <= 0.0 {
        return 0.0;
    }
    if down_age_ticks >= SAND_FALL_TICKS {
        return full;
    }
    let denom = f32::from(u16::try_from(SAND_FALL_TICKS).unwrap_or(u16::MAX));
    let age = f32::from(u16::try_from(down_age_ticks).unwrap_or(u16::MAX));
    let t = (age / denom).clamp(0.0, 1.0);
    full * t * t * (3.0 - 2.0 * t)
}

/// Down-only rest lift for one slot (LF17, view only): eased depth for `Down`
/// or `Fallen`, else 0, so idle hover stays on its own law. View only.
#[must_use]
fn down_rest_lift(slot: &SlotAnim, tick: u32, lowest_y: f32, has_bounds: bool) -> f32 {
    if !has_bounds {
        return 0.0;
    }
    if slot.act == Act::Down {
        sand_lift_eased(lowest_y, tick.saturating_sub(slot.act_start))
    } else if slot.balance == GaitPhase::Fallen {
        sand_lift_m(lowest_y)
    } else {
        0.0
    }
}

/// Whole microseconds of wall clock, saturating. Evidence only: the fighter
/// skin split ([`SkinSplit`]) reports through it and nothing hashed reads it.
fn micros(elapsed: std::time::Duration) -> u64 {
    u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX)
}

/// Body parts left resident for the frame's draws, split into the CPU pose
/// half and the GPU upload half of the fighter skin wall.
///
/// Consumer: [`FighterLook::present_into`] returns it inside
/// [`PresentReport`] and `crates/game/src/bin/arena.rs` folds both halves
/// into the `arena frame-split` line beside the untouched `fighters` lump.
/// Data owner: this module, read-only for its consumers. Units: parts,
/// microseconds of wall clock sampled with [`Instant`] only (no map, poll or
/// wait). Determinism: evidence only; the counters never enter a draw, the
/// view truth or hashed state. A zero means no `SkinnedFrame`s reached the
/// upload path this call, never a second timing law.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SkinSplit {
    /// Parts the frame's draws read this call: one per part with finite
    /// posed vertices, whether it was written or already resident.
    pub uploaded: usize,
    /// `uploaded`'s CPU half: clip sample, skin, pose fold, the down lift and
    /// the upload digest ([`FighterLook::pose_frame`] plus the per-part fold).
    pub pose_us: u64,
    /// `uploaded`'s GPU half: the `v1::upload_mesh` plus
    /// `Presenter::upload_mesh` pair per part (validation, tangent rebuild,
    /// byte pack, buffer write). A part whose digest is unchanged runs no
    /// pair and adds nothing.
    pub upload_us: u64,
}

/// One presented frame's cost report: the draws pushed for the frame and the
/// split of the fighter skin wall behind them.
///
/// Consumer: `crates/game/src/bin/arena.rs` (`ArenaApp::push_fighters`) adds
/// `skin.pose_us`/`skin.upload_us` to its `fighters_pose_us`/
/// `fighters_upload_us` counters, printed as `fighters_pose=`/
/// `fighters_upload=` on the `arena frame-split` line. Units: draws, parts,
/// microseconds of wall clock. Determinism: evidence only; the draws stay the
/// frame's draw list and the counters never enter hashed state.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct PresentReport {
    /// Draws pushed this frame: the staged ring plus the skinned body parts.
    pub draws: usize,
    /// Parts left resident for those draws and the two wall halves.
    pub skin: SkinSplit,
}

/// Upload one slot's skinned parts with the down lift applied (LF17, view
/// only): lifted positions for the draw, normals ride unchanged, tangents
/// recomputed from the lifted positions plus UVs. The GPU vertex is 48 B
/// (pos + normal + uv + tangent, `MeshData::vertex_bytes`); the in-place
/// `refresh_mesh_vertices` path writes 32 B (pos + normal + uv) and leaves
/// the 48 B stride misaligned past the first vertex, stretching every other
/// fighter triangle across the screen (1209Z streaks). Until that path carries
/// tangents, every *changed* skinned frame re-uploads the full mesh (correct
/// stride) so fighters render with limbs and armor.
///
/// The pair is skipped when the frame's lifted positions plus skinned normals
/// hash to the part's resident digest (`digests`, keyed by
/// `slot * MAX_PARTS + part`): the resident bytes already are the frame's
/// bytes, so a frozen body re-uploads nothing and a moved one re-uploads
/// whole. `uploaded` counts the parts left resident for the frame either way.
/// Returns that count beside the two halves they cost: the lift plus digest
/// fold into `pose_us`, the `v1::upload_mesh`/`Presenter::upload_mesh` pair
/// into `upload_us` ([`SkinSplit`]).
/// [found2 | 7.6.2 Rendering with Normal Maps]
fn upload_slot_parts(
    presenter: &mut Presenter,
    slot_index: usize,
    rig: &Rig,
    frames: Vec<engine_render::v1_skin::SkinnedFrame>,
    lift: f32,
    digests: &mut [Option<u64>],
) -> Result<SkinSplit, LookError> {
    let mut split = SkinSplit::default();
    for (part_index, posed) in frames.into_iter().enumerate() {
        let part = &rig.parts[part_index];
        let lifted_started = Instant::now();
        // Digest the exact bytes the pair would upload: the lifted positions
        // and the skinned normals (uvs and indices belong to the rig and
        // never change). Hashing rides the lift pass, so a changed frame pays
        // one pass over the vertices it already touches.
        let mut hasher = Fnv64::new();
        let lifted: Vec<[f32; 3]> = if lift > 0.0 {
            let mut out = Vec::with_capacity(posed.positions.len());
            for position in &posed.positions {
                let moved = [position[0], position[1] + lift, position[2]];
                for component in moved {
                    hasher.write_f32(component);
                }
                out.push(moved);
            }
            out
        } else {
            for position in &posed.positions {
                for component in position {
                    hasher.write_f32(*component);
                }
            }
            posed.positions
        };
        for normal in &posed.normals {
            for component in normal {
                hasher.write_f32(*component);
            }
        }
        let digest = hasher.finish();
        split.pose_us = split
            .pose_us
            .saturating_add(micros(lifted_started.elapsed()));
        let slot_part = slot_index * MAX_PARTS + part_index;
        if digests.get(slot_part) == Some(&Some(digest)) {
            // Resident bytes already are this frame's bytes: no upload.
            split.uploaded += 1;
            continue;
        }
        let key = look_mesh_key(slot_index, part_index);
        // 48 B stride fix: always re-upload with recomputed tangents; never
        // refresh 32 B in place (misaligned past vertex 0, streaks).
        let upload_started = Instant::now();
        let mesh = v1::upload_mesh(&lifted, &posed.normals, &part.mesh.uvs, &part.mesh.indices)
            .map_err(|err| LookError::Skin(format!("{err}")))?;
        presenter
            .upload_mesh(key, &mesh)
            .map_err(|err| LookError::Upload(err.to_string()))?;
        split.upload_us = split
            .upload_us
            .saturating_add(micros(upload_started.elapsed()));
        if let Some(resident) = digests.get_mut(slot_part) {
            *resident = Some(digest);
        }
        split.uploaded += 1;
    }
    Ok(split)
}

/// Note: the authoritative read is `engine-arena` `Sim::balance` (already
/// landed with EG34) via `LiveMatch::balance`; the window polls it each
/// tick through [`FighterLook::observe_live_balances`]. The cue mirror
/// (`Hit` stumbles, `Knockdown` floors) stays for the same-tick swing
/// start, so the same blows lean the same roots.

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
    /// One bridge slot per look slot: the production owner of the act pick
    /// plus the fade driver. Driven in `fold_acts` from the same views, hp
    /// and cues; sampling stays in `slot_pose`. Presentation only.
    bridge: Vec<FighterSlot>,
    /// Blade trail ring, view only: `start_swing` pushes attacking slots,
    /// the next slice's strip reads `pairs`. Never hashed.
    trails: TrailRing,
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
    /// Last uploaded vertex digest per `slot * MAX_PARTS + part`, `None`
    /// before that part's first upload. The upload pair is skipped when a
    /// frame's lifted positions plus skinned normals hash to the resident
    /// entry, so a frozen body re-uploads nothing while a moved one
    /// re-uploads whole (48 B stride, never a 32 B refresh). Presentation
    /// only; never hashed and never drawn.
    uploaded_digest: Vec<Option<u64>>,
}

/// One upload-digest slot per `slot * MAX_PARTS + part`, all empty: the
/// [`FighterLook::uploaded_digest`] shape, one owner for the constructor and
/// the test literals that spell the struct out.
fn empty_upload_digest() -> Vec<Option<u64>> {
    vec![None; MAX_LOOK_FIGHTERS * MAX_PARTS]
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

/// Farthest a bind vertex sits from its nearest bind joint, in meters.
/// Pure `f32` in, same `f32` out; a non-finite vertex reads as infinity so
/// the caller skips it, never a panic.
fn nearest_joint_d(joints: &[[f32; 3]], vertex: [f32; 3]) -> f32 {
    let mut nearest = f32::MAX;
    for joint in joints.iter() {
        let dx = vertex[0] - joint[0];
        let dy = vertex[1] - joint[1];
        let dz = vertex[2] - joint[2];
        nearest = nearest.min(dx * dx + dy * dy + dz * dz);
    }
    nearest.sqrt()
}

/// Skin radius of one rig in meters: the farthest any bind vertex sits from
/// its nearest bind joint, plus 10 cm, floored at 0.5 m. The joints-only
/// pose pads placed joints by this radius so its bounds hold the skinned
/// vertex bounds. Cold path only (once per rig at load); pure `f32` in,
/// same `f32` out; a non-finite vertex never moves the radius.
fn skin_radius_m(body: &SkinnedBody) -> f32 {
    let mut radius = 0.5f32;
    for part in body.parts.iter() {
        for vertex in part.mesh.positions.iter() {
            let d = nearest_joint_d(&body.bind.positions, *vertex);
            if d.is_finite() {
                radius = radius.max(d + 0.10);
            }
        }
    }
    radius
}

/// Posed bound of one skinned body in world meters: the axis-box centre over
/// every posed vertex plus the farthest vertex gap from it, or `None` when no
/// finite vertex folds in. This is the bound a draw carries, so culling
/// follows the pose and a limb the bind pose never had stays inside it
/// (Eberly 3.2); the vertices ride the body's down-rest lift like the drawn
/// mesh does. Meters in, meters out, no heap, and the same frames give the
/// same bound bytes twice.
fn posed_bound_m(body: &PosedBody) -> Option<([f32; 3], f32)> {
    let lift = if body.lift.is_finite() {
        body.lift
    } else {
        0.0
    };
    let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
    let mut folded = false;
    for frame in body.frames.iter() {
        for vertex in frame.positions.iter() {
            if !vertex.iter().all(|v| v.is_finite()) {
                continue;
            }
            folded = true;
            let at = [vertex[0], vertex[1] + lift, vertex[2]];
            for (axis, value) in at.iter().enumerate() {
                lo[axis] = lo[axis].min(*value);
                hi[axis] = hi[axis].max(*value);
            }
        }
    }
    if !folded {
        return None;
    }
    let centre: [f32; 3] = std::array::from_fn(|axis| (lo[axis] + hi[axis]) * 0.5);
    let mut radius2 = 0.0f32;
    for frame in body.frames.iter() {
        for vertex in frame.positions.iter() {
            if !vertex.iter().all(|v| v.is_finite()) {
                continue;
            }
            let (dx, dy, dz) = (
                vertex[0] - centre[0],
                vertex[1] + lift - centre[1],
                vertex[2] - centre[2],
            );
            radius2 = radius2.max(dx * dx + dy * dy + dz * dz);
        }
    }
    Some((centre, radius2.sqrt()))
}

/// Retarget every clip source onto the body skeleton, in file order.
/// Cold path only; a missing clip or a failed map fails loud.
fn retarget_clips(
    sources: &[ClipSource],
    body: &SkinnedBody,
) -> Result<Vec<ImportedClip>, LookError> {
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
    Ok(clips)
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
    let clips = retarget_clips(sources, &body)?;
    Ok(Rig {
        radius_m: skin_radius_m(&body),
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

/// One CPU-posed body with its skinned parts, before any GPU upload.
///
/// Consumer: [`FighterLook::stage_bodies`] uploads `frames` with `lift`;
/// the headless pose step returns these with no `Presenter`. Units: meters
/// for bounds, seconds for clip time. Determinism: pure fold of the tick
/// plus `alpha`; the same inputs pose the same truth twice. Errors: none
/// here; the posing calls fail loud on a missing rig instead of panicking.
struct PosedBody {
    truth: BodyTruth,
    frames: Vec<engine_render::v1_skin::SkinnedFrame>,
    lift: f32,
    /// Bound this draw carries in world meters: the posed-vertex bound
    /// ([`posed_bound_m`]) floored at the rig's bind radius, so a cull reads
    /// the pose and never the bind-pose sphere. `None` when no posed vertex
    /// is finite. Consumer: [`FighterLook::stage_bodies`], the frame's draw
    /// path, replacing the bind sphere a render draw cull would read.
    bound: Option<([f32; 3], f32)>,
}

impl PosedBody {
    fn truth(&self) -> &BodyTruth {
        &self.truth
    }
}

/// Sampled pose shared by the full and joints-only paths: clip pick, world
/// joint matrices and the interpolated root. Both modes fold from here, so
/// their slots, joints, clip and clip time can never drift apart.
struct SlotSample {
    clip: usize,
    time: f32,
    world: Vec<[[f32; 4]; 4]>,
    pos: [f32; 3],
    yaw: f32,
    root: [[f32; 4]; 4],
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
            bridge: bridge_slots(),
            trails: TrailRing::new(),
            tick: 0,
            swing_ticks,
            bundle_shas: bindings.into_iter().map(|b| b.bundle_sha256).collect(),
            truth: Vec::new(),
            shake_trauma_permille: 0,
            uploaded_digest: empty_upload_digest(),
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

    /// Apply one authoritative balance verdict to `slot` (EG45): the gait
    /// read from `Sim::balance` (`engine-arena`) via `LiveMatch::balance`
    /// and polled by [`observe_live_balances`](Self::observe_live_balances).
    /// A bad slot is ignored; a current `Fallen` never downgrades (keeps
    /// flat); every other gait applies (`Steady` resets, `Stumbling` leans
    /// via `balance_lean_m`, `Catching` kneels via `balance_pelvis_y_m`,
    /// `Fallen` floors via `balance_pelvis_y_m` plus the flat `Down` pose
    /// in [`slot_pose`](Self::slot_pose)). Presentation only; the sim
    /// digest never reads this.
    pub fn observe_balance(&mut self, slot: u8, gait: GaitPhase) {
        let Some(anim) = self.slots.get_mut(usize::from(slot)) else {
            return;
        };
        if anim.balance == GaitPhase::Fallen && gait != GaitPhase::Fallen {
            return;
        }
        anim.balance = gait;
    }

    /// Poll the authoritative integer balance gaits into the pose (EG45):
    /// slots `0..fighters.min(MAX_LOOK_FIGHTERS)` read
    /// `LiveMatch::balance` and feed [`observe_balance`](Self::observe_balance)
    /// with `state.gait()`. A `None` (bad slot) is ignored; the Fallen-sticky
    /// rule stays inside `observe_balance` (a current `Fallen` never
    /// downgrades). Presentation only; the sim digest never reads this.
    ///
    /// Consumer: the window (the next slice owns the per-tick call). Data
    /// owner: `engine-arena` owns the per-slot states. Units: slot index.
    /// Determinism: same live plus same fighters gives the same pose twice.
    pub fn observe_live_balances(&mut self, live: &engine_arena::LiveMatch, fighters: usize) {
        for index in 0..fighters.min(MAX_LOOK_FIGHTERS) {
            let Ok(slot) = u8::try_from(index) else {
                continue;
            };
            if let Some(state) = live.balance(slot) {
                self.observe_balance(slot, state.gait());
            }
        }
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

    /// Blade base-plus-tip for `pos` plus `yaw`, view only: the base rides
    /// 1 m above the feet, the tip leads 0.9 m along the facing plus 0.1 m
    /// up. Pure; non-finite input reads as non-finite output, which the
    /// ring drops instead of storing.
    fn blade_points(pos: [f32; 3], yaw: f32) -> ([f32; 3], [f32; 3]) {
        let base = [pos[0], pos[1] + 1.0, pos[2]];
        let (s, c) = yaw.sin_cos();
        let tip = [base[0] + s * 0.9, base[1] + 0.1, base[2] + c * 0.9];
        (base, tip)
    }

    fn start_swing(&mut self, attacker: u8, tick: u32) {
        let index = usize::from(attacker);
        let Some(slot) = self.slots.get(index) else {
            return;
        };
        if slot.act == Act::Down {
            return;
        }
        if let Some(slot) = self.slots.get_mut(index) {
            // A swing already in flight keeps playing; a new one restarts.
            let variant = slot.swings % 3;
            slot.swings = slot.swings.wrapping_add(1);
            slot.act = Act::Attack(variant);
            slot.act_start = tick.saturating_sub(IMPACT_TICKS);
        }
        // Trail samples land in `feed_trails` once per tick, so one swing
        // start never pushes twice. Never hashed.
    }

    /// Feed the blade trail ring once per tick, view only: age once, then
    /// push current blade points for each seen attacking slot (cap 12,
    /// oldest drops) and clear every other slot. Non-finite stores nothing.
    /// Never hashed. Caller: `observe_tick` after the act fold.
    fn feed_trails(&mut self) {
        self.trails.age_tick();
        for index in 0..MAX_LOOK_FIGHTERS {
            let (seen, attacking, pos, yaw) = match self.slots.get(index) {
                Some(slot) => (
                    slot.seen,
                    matches!(slot.act, Act::Attack(_)),
                    slot.pos,
                    slot.yaw,
                ),
                None => continue,
            };
            if seen && attacking {
                let (base_m, tip_m) = Self::blade_points(pos, yaw);
                self.trails.push(index, base_m, tip_m);
            } else {
                self.trails.clear_idle(index);
            }
        }
    }

    /// Trail pairs for `slot`, oldest first; empty when idle. View only,
    /// never hashed. Caller: the next slice's strip draw.
    #[must_use]
    pub fn trail_pairs(&self, slot: usize) -> &[TrailPoint] {
        self.trails.pairs(slot)
    }

    /// Current act for a slot (receipts and tests): the bridge pick mapped
    /// through `bridge_act` for seen slots, `None` otherwise. The
    /// `SlotAnim.act` mirror stays as the sampling cache for `slot_pose`.
    #[must_use]
    pub fn act(&self, slot: usize) -> Option<Act> {
        let seen = self.slots.get(slot).is_some_and(|s| s.seen);
        if !seen {
            return None;
        }
        self.bridge.get(slot).map(|b| bridge_act(b.pick().clip))
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
    /// lowers the pelvis for a fall or a catch (EG45, presentation only):
    /// a `Fallen` gait forces the flat `Down` last frame at once, so the
    /// floored slot lies flat within one frame; `Catching` kneels and
    /// `Stumbling` leans while keeping the act's clip; `Steady` keeps the
    /// pose byte-identical.
    /// A `Down` pose starts here at [`BALANCE_FALLEN_PELVIS_M`]; the LF17
    /// sand rest ([`sand_lift_m`]) only adds lift after the CPU skin folds
    /// the posed bounds, never inside this pose. Last, the additive
    /// flinch (EG18) pushes the pelvis away from the recorded attacker
    /// side and dips it, scaled by the shared react-window
    /// [`envelope`]: full at impact, silent past the window.
    fn slot_pose(&self, rig: &Rig, slot: &SlotAnim, alpha: f32) -> Result<LocalPose, LookError> {
        let (clip_index, time) = if slot.balance == GaitPhase::Fallen {
            let duration = rig.clips.get(5).map_or(0.0, |clip| clip.duration);
            (5, duration * 0.999)
        } else {
            self.clip_time(rig, slot.act, slot.act_start, slot.run_speed, alpha)
        };
        let clip = rig
            .clips
            .get(clip_index)
            .ok_or_else(|| LookError::Skin("clip missing".to_string()))?;
        let current: LocalPose =
            sample_pose_with_rots(&rig.skeleton, &clip.positions, &clip.rotations, time)
                .map_err(|err| LookError::Skin(format!("{err:?}")))?;
        let mut pose = if slot.fade_left == 0 {
            current
        } else if let Some(prev_clip) = rig.clips.get(slot.prev_clip) {
            let prev = sample_pose_with_rots(
                &rig.skeleton,
                &prev_clip.positions,
                &prev_clip.rotations,
                slot.prev_time,
            )
            .map_err(|err| LookError::Skin(format!("{err:?}")))?;
            let left = f32::from(u16::try_from(slot.fade_left).unwrap_or(u16::MAX));
            let total = f32::from(u16::try_from(FIGHTER_CROSSFADE_TICKS).unwrap_or(u16::MAX));
            blend_poses(&prev, &current, 1.0 - left / total)
        } else {
            current
        };
        // In-place clips: the match owns travel, the clip keeps its bob.
        pose.positions[rig.pelvis][0] = rig.bind_pelvis[0];
        pose.positions[rig.pelvis][2] = rig.bind_pelvis[2];
        pose.positions[rig.pelvis][1] =
            balance_pelvis_y_m(slot.balance, pose.positions[rig.pelvis][1]);
        // Additive hit-react (EG18, view only): pelvis rides half the
        // planar peak and half the dip at unit impulse, matching the
        // `engine-anim` shares; the envelope owns the decay so pose and
        // lockstep agree by construction. Zero envelope writes nothing,
        // keeping the past-window pose bit-identical to its base.
        let elapsed = self.tick.wrapping_sub(slot.hit_start);
        let weight = envelope(elapsed);
        if weight > 0.0 && (slot.hit_push[0] != 0.0 || slot.hit_push[1] != 0.0) {
            let planar = 0.5 * PEAK_M * weight;
            let dip = 0.5 * DIP_M * weight;
            pose.positions[rig.pelvis][0] += slot.hit_push[0] * planar;
            pose.positions[rig.pelvis][2] += slot.hit_push[1] * planar;
            pose.positions[rig.pelvis][1] -= dip;
        }
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
    /// their resident vertex buffers, skipping a part whose bytes are already
    /// resident. Returns parts left resident; the wall halves behind them are
    /// [`Self::present_into`]'s report ([`SkinSplit`]). A skinning or upload
    /// refusal is loud.
    pub fn stage_frame(
        &mut self,
        presenter: &mut Presenter,
        fighters: usize,
        alpha: f32,
    ) -> Result<usize, LookError> {
        self.stage_bodies(presenter, fighters, alpha)
            .map(|(split, _)| split.uploaded)
    }

    /// Sample one seen slot at sub-tick `alpha`: clip pick, world joint
    /// matrices and interpolated root. Pure fold of the tick plus `alpha`;
    /// the balance lean (EG45) rides along, presentation only. A `Fallen`
    /// gait forces the flat `Down` last frame (like [`slot_pose`](Self::slot_pose)),
    /// so the floored slot draws flat within one frame.
    fn slot_sample(&self, rig: &Rig, slot: &SlotAnim, alpha: f32) -> Result<SlotSample, LookError> {
        let (clip, time) = if slot.balance == GaitPhase::Fallen {
            let duration = rig.clips.get(5).map_or(0.0, |clip| clip.duration);
            (5, duration * 0.999)
        } else {
            self.clip_time(rig, slot.act, slot.act_start, slot.run_speed, alpha)
        };
        let pose = self.slot_pose(rig, slot, alpha)?;
        let world = skinning_matrices(&rig.skeleton, &pose)
            .map_err(|err| LookError::Skin(format!("{err:?}")))?;
        let mut pos: [f32; 3] =
            std::array::from_fn(|i| slot.prev_pos[i] + (slot.pos[i] - slot.prev_pos[i]) * alpha);
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
        Ok(SlotSample {
            clip,
            time,
            world,
            pos,
            yaw,
            root,
        })
    }

    /// CPU pose step of [`Self::stage_frame`]: clip sample, skin and truth
    /// per seen fighter, with no `Presenter`, so `load`, `observe_tick` and
    /// this run headless. A seen slot with no rig fails loud; unseen slots
    /// pose nothing. The same inputs pose the same bodies twice; the sim
    /// hash never reads them. Each posed body carries its bound from the
    /// posed vertices, floored at the rig's bind radius.
    fn pose_frame(&self, fighters: usize, alpha: f32) -> Result<Vec<PosedBody>, LookError> {
        let alpha = crate::stage_bodies::clamp_alpha(alpha);
        let mut posed = Vec::with_capacity(fighters.min(MAX_LOOK_FIGHTERS));
        for slot_index in 0..fighters.min(MAX_LOOK_FIGHTERS) {
            let slot = self.slots[slot_index];
            if !slot.seen {
                continue;
            }
            let rig = self
                .rigs
                .get(self.rig_for_slot(slot_index))
                .ok_or_else(|| LookError::Skin("rig missing".to_string()))?;
            let sample = self.slot_sample(rig, &slot, alpha)?;
            let pos = sample.pos;
            let yaw = sample.yaw;
            let root = sample.root;
            let matrices: Vec<[[f32; 4]; 4]> = sample
                .world
                .iter()
                .zip(rig.inverse_binds.iter())
                .map(|(joint, inverse)| mul(&mul(inverse, joint), &root))
                .collect();
            let uniform = rig
                .pipe
                .skinning_uniform(&matrices)
                .map_err(|err| LookError::Skin(format!("{err:?}")))?;
            let joints = sample
                .world
                .iter()
                .map(|m| [m[3][0], m[3][1], m[3][2]])
                .collect();
            let mut body = BodyTruth::new(
                u8::try_from(slot_index).unwrap_or(u8::MAX),
                act_label(slot.act),
                pos,
                yaw,
                joints,
            );
            body.clip = u8::try_from(sample.clip).unwrap_or(u8::MAX);
            body.clip_s = sample.time;
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
            // LF17 down rest, view only: eased lift, idle never lifts.
            let lift = down_rest_lift(&slot, self.tick, body.min[1], body.has_bounds());
            if lift > 0.0 {
                body.root[1] += lift;
                body.min[1] += lift;
                body.max[1] += lift;
            }
            let mut drawn = PosedBody {
                truth: body,
                frames,
                lift,
                bound: None,
            };
            // One owner of the bound mint: the posed vertices, floored at
            // the rig's own bind radius so a bound never shrinks under the
            // sphere it replaces.
            drawn.bound =
                posed_bound_m(&drawn).map(|(centre, radius)| (centre, radius.max(rig.radius_m)));
            posed.push(drawn);
        }
        Ok(posed)
    }

    /// Joints-only pose: the same clip sample, root placement and tint as
    /// the full pose, but bounds come from the placed joints padded by the
    /// rig's skin radius, with no vertex skin and no `Presenter`, so a frame
    /// costs a few dozen flops per joint. It poses the same slots, joints,
    /// clip and clip time as the full pose, and its bounds hold the vertex
    /// bounds (see the `bounds_hold` check in tests). A seen slot with no
    /// rig fails loud; unseen slots pose nothing; the same inputs repeat
    /// bit for bit.
    pub fn pose_frame_joints(
        &self,
        fighters: usize,
        alpha: f32,
    ) -> Result<Vec<BodyTruth>, LookError> {
        let alpha = crate::stage_bodies::clamp_alpha(alpha);
        let mut out = Vec::with_capacity(fighters.min(MAX_LOOK_FIGHTERS));
        for slot_index in 0..fighters.min(MAX_LOOK_FIGHTERS) {
            let slot = self.slots[slot_index];
            if !slot.seen {
                continue;
            }
            let rig = self
                .rigs
                .get(self.rig_for_slot(slot_index))
                .ok_or_else(|| LookError::Skin("rig missing".to_string()))?;
            let sample = self.slot_sample(rig, &slot, alpha)?;
            let pos = sample.pos;
            let yaw = sample.yaw;
            let root = sample.root;
            let joints: Vec<[f32; 3]> = sample
                .world
                .iter()
                .map(|m| [m[3][0], m[3][1], m[3][2]])
                .collect();
            let placed: Vec<[f32; 3]> = joints
                .iter()
                .map(|joint| place_joint(*joint, &root))
                .collect();
            let mut body = BodyTruth::new(
                u8::try_from(slot_index).unwrap_or(u8::MAX),
                act_label(slot.act),
                pos,
                yaw,
                joints,
            );
            body.clip = u8::try_from(sample.clip).unwrap_or(u8::MAX);
            body.clip_s = sample.time;
            body.tinted = self.tick < slot.flash_until;
            if let Some((lo, hi)) = crate::stage_bodies::joints_bounds(&placed, rig.radius_m) {
                body.min = lo;
                body.max = hi;
            }
            // LF17 down rest, view only: same eased rule as the full pose.
            let lift = down_rest_lift(
                &self.slots[slot_index],
                self.tick,
                body.min[1],
                body.has_bounds(),
            );
            if lift > 0.0 {
                body.root[1] += lift;
                body.min[1] += lift;
                body.max[1] += lift;
            }
            out.push(body);
        }
        Ok(out)
    }

    /// Headless bout folded into one `arena view:` receipt line, with no
    /// `Presenter` and no window: steps a bout, observes each tick and poses
    /// joints-only. Zero ticks fold no frame and read as `""`; otherwise the
    /// view laws line. The same seed gives the same line twice; the sim hash
    /// never reads it.
    /// Caller: `crates/endpoint/src/packs/view.rs` (SP62 `view.laws` pops this
    /// line, needs SP61); crate-internal until that door wires it.
    pub(crate) fn view_run(seed: u64, ticks: u32, fighters: u8) -> Result<String, LookError> {
        if ticks == 0 {
            return Ok(String::new());
        }
        let config = crate::GameConfig {
            seed,
            ticks,
            fighters,
            half_extent_m: 20.0,
        };
        let mut app = crate::App::new(config).map_err(|err| LookError::Bind(format!("{err:?}")))?;
        let mut look = Self::load(&fighters_content_dir())?;
        let camera = Camera::new(
            [0.0, 5.0, 12.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        );
        let mut laws = crate::view_truth::ViewLaws::default();
        for tick in 0..ticks {
            let outcome = app.tick_empty();
            // LF10: poll the app's mirrored hitstop the same way the window
            // twin does; the look's cue-derived freeze follows the same
            // 4-tick law, so the hold agrees. Presentation only, never hashed.
            let remaining = app.hitstop_remaining();
            let holding = crate::App::hitstop_holds(remaining);
            debug_assert!(
                remaining <= engine_combat::fight::HITSTOP_TICKS,
                "hitstop mirror stays within the sim freeze"
            );
            look.observe_tick(tick, app.fighter_views(), app.hp(), &outcome.cues);
            debug_assert_eq!(
                holding,
                look.bridge.iter().any(|slot| tick < slot.hold_until()),
                "app mirror and look freeze agree at tick {tick}"
            );
            look.observe_live_balances(app.live_match(), usize::from(fighters));
            let joints = look.pose_frame_joints(usize::from(fighters), 0.5)?;
            laws.note(&camera, f64::from(tick) + 0.5, &joints);
        }
        Ok(laws.line().unwrap_or_default())
    }

    /// [`Self::stage_frame`] plus each skinned body as numbers (bounds of
    /// the exact posed vertices, model-space joints, clip, tint): the view
    /// truth costs one pass over vertices the CPU skin already produced.
    fn stage_bodies(
        &mut self,
        presenter: &mut Presenter,
        fighters: usize,
        alpha: f32,
    ) -> Result<(SkinSplit, Vec<BodyTruth>), LookError> {
        // Split: the CPU pose step lives in `pose_frame`; this keeps the
        // upload, so every existing caller poses and draws exactly as before.
        let pose_started = Instant::now();
        let posed = self.pose_frame(fighters, alpha)?;
        let mut split = SkinSplit {
            uploaded: 0,
            pose_us: micros(pose_started.elapsed()),
            upload_us: 0,
        };
        let mut truth = Vec::with_capacity(posed.len());
        for body in posed {
            let slot_index = usize::from(body.truth.slot);
            let rig_index = self.rig_for_slot(slot_index);
            let rig = self
                .rigs
                .get(rig_index)
                .ok_or_else(|| LookError::Skin("rig missing".to_string()))?;
            // The carried bound is what a cull reads, so it must agree with
            // the folded vertex box: one emptiness story for both owners.
            debug_assert_eq!(
                body.bound.is_some(),
                body.truth.has_bounds(),
                "posed bound and folded AABB agree on a finite body"
            );
            // No finite posed vertex means nothing to draw: the slot keeps
            // its resident mesh instead of writing NaN vertices over it.
            if body.bound.is_none() {
                truth.push(body.truth);
                continue;
            }
            let parts = upload_slot_parts(
                presenter,
                slot_index,
                rig,
                body.frames,
                body.lift,
                &mut self.uploaded_digest,
            )?;
            split.uploaded += parts.uploaded;
            split.pose_us = split.pose_us.saturating_add(parts.pose_us);
            split.upload_us = split.upload_us.saturating_add(parts.upload_us);
            truth.push(body.truth);
        }
        Ok((split, truth))
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
                    normal_map: None,
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
    /// textured draws and aim the list camera for the frame's shot. Returns
    /// the frame's draws beside the fighter skin wall split ([`PresentReport`]);
    /// refusals are loud.
    pub fn present_into(
        &mut self,
        presenter: &mut Presenter,
        list: &mut DrawList,
        frame: &LookFrame,
    ) -> Result<PresentReport, LookError> {
        let (skin, truth) = self.stage_bodies(presenter, frame.fighters, frame.alpha)?;
        self.truth = truth;
        let resident = presenter.mesh_keys();
        if let Some(ring) = &self.ring {
            ring.push_draws(list, &resident);
        }
        let draws = self.push_draws(list, &resident, frame.fighters);
        if draws > 0 {
            list.camera = self.fight_camera(&list.camera, frame.shot, frame.fighters, frame.frame);
        }
        Ok(PresentReport { draws, skin })
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
                // Three-to-four seen fall through to the group fit below.
                Some((mid, half, axis)) if seen.len() <= 2 || seen.len() > 4 => (
                    mid,
                    (half * 2.2 + 3.2).clamp(3.5, 7.0),
                    0.0,
                    1.0,
                    axis + std::f32::consts::FRAC_PI_2 + turn * 0.3,
                ),
                _ => (
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

    use engine_audio::FightCue;

    use crate::FighterView;

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
            bridge: bridge_slots(),
            trails: TrailRing::new(),
            tick: 0,
            swing_ticks: 1,
            bundle_shas: Vec::new(),
            truth: Vec::new(),
            shake_trauma_permille: 0,
            uploaded_digest: empty_upload_digest(),
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
    fn fight_camera_frames_four_with_hero_seen() {
        // Two pairs: the duel fit held two, the group fit holds all four.
        let mut slots = vec![SlotAnim::new(); MAX_LOOK_FIGHTERS];
        for (slot, at) in slots.iter_mut().zip([
            [-2.0, 0.0, 0.0],
            [-1.0, 0.0, 0.0],
            [5.0, 0.0, 2.0],
            [6.0, 0.0, 2.0],
        ]) {
            slot.seen = true;
            slot.pos = at;
        }
        let look = FighterLook {
            rigs: Vec::new(),
            ring: None,
            slots,
            bridge: bridge_slots(),
            trails: TrailRing::new(),
            tick: 0,
            swing_ticks: 1,
            bundle_shas: Vec::new(),
            truth: Vec::new(),
            shake_trauma_permille: 0,
            uploaded_digest: empty_upload_digest(),
        };
        let base = Camera::new(
            [0.0, 5.0, 12.0],
            [0.0, 1.0, 0.0],
            [0.0, 1.0, 0.0],
            50.0f32.to_radians(),
        );
        let cam = look.fight_camera(&base, Shot::Action, 4, 0);
        assert_eq!(
            look.fighters_in_view(&cam, 4),
            4,
            "both pairs stay in the 60 deg picture"
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
            bridge: bridge_slots(),
            trails: TrailRing::new(),
            tick: 0,
            swing_ticks: 1,
            bundle_shas: Vec::new(),
            truth: Vec::new(),
            shake_trauma_permille: 0,
            uploaded_digest: empty_upload_digest(),
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

    #[test]
    fn hitstop_hold_freezes_impact_pair_while_others_advance() {
        // LF10: during the sim's hitstop the attacker's and victim's clip
        // clocks stay put while every other slot advances; afterwards all
        // advance. Headless: clip phase reads as `tick - act_start`.
        let mut look = balance_look_two();
        look.slots[2].seen = true;
        look.slots[2].pos = [0.0, 0.0, 2.0];
        look.slots[2].prev_pos = [0.0, 0.0, 2.0];
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0), view(2, 0.0, 2.0)];
        let hp = [1000, 1000, 1000];
        for t in 0..10u32 {
            look.observe_tick(t, &views, &hp, &[]);
        }
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 100,
        };
        look.observe_tick(10, &views, &hp, &[hit]);
        let since =
            |look: &FighterLook, s: usize| look.tick.saturating_sub(look.slots[s].act_start);
        let frozen = [since(&look, 0), since(&look, 1)];
        let free = since(&look, 2);
        // Three quiet ticks of hitstop plus the impact tick (four held ticks
        // matching the App hitstop count 4,3,2,1): the impact pair holds.
        for t in 11..14u32 {
            look.observe_tick(t, &views, &hp, &[]);
            assert_eq!(since(&look, 0), frozen[0], "attacker holds at tick {t}");
            assert_eq!(since(&look, 1), frozen[1], "victim holds at tick {t}");
            assert_eq!(
                since(&look, 2),
                free.saturating_add(t.saturating_sub(10)),
                "bystander advances at tick {t}"
            );
        }
        // Hold spent: every clock advances again.
        look.observe_tick(14, &views, &hp, &[]);
        assert_eq!(
            since(&look, 0),
            frozen[0].saturating_add(1),
            "attacker resumes"
        );
        assert_eq!(
            since(&look, 1),
            frozen[1].saturating_add(1),
            "victim resumes"
        );
        assert_eq!(
            since(&look, 2),
            free.saturating_add(4),
            "bystander keeps advancing"
        );
        // Same script twice gives the same clocks.
        let mut again = balance_look_two();
        again.slots[2].seen = true;
        again.slots[2].pos = [0.0, 0.0, 2.0];
        again.slots[2].prev_pos = [0.0, 0.0, 2.0];
        for t in 0..10u32 {
            again.observe_tick(t, &views, &hp, &[]);
        }
        again.observe_tick(10, &views, &hp, &[hit]);
        for t in 11..15u32 {
            again.observe_tick(t, &views, &hp, &[]);
        }
        assert_eq!(since(&again, 0), since(&look, 0), "attacker replays");
        assert_eq!(since(&again, 1), since(&look, 1), "victim replays");
        assert_eq!(since(&again, 2), since(&look, 2), "bystander replays");
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
            bridge: bridge_slots(),
            trails: TrailRing::new(),
            tick: 0,
            swing_ticks: 1,
            bundle_shas: Vec::new(),
            truth: Vec::new(),
            shake_trauma_permille: 0,
            uploaded_digest: empty_upload_digest(),
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
        // Sim digest stays equal with the look on and off.
        let run_hashes = |with_look: bool| {
            let mut sim =
                LiveMatch::with_rules(&cfg, ArenaRules::v2_free_for_all()).expect("v2 builds");
            let mut look = balance_look_two();
            let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
            let mut hashes = Vec::new();
            for t in 0..10u32 {
                if let Some(tick) = sim.step(&[]) {
                    hashes.push(tick.checkpoint.hash);
                }
                if with_look {
                    look.observe_tick(t, &views, &[1000, 1000], &[]);
                    look.observe_live_balances(&sim, 2);
                }
            }
            hashes
        };
        assert_eq!(run_hashes(false), run_hashes(true));
        assert_eq!(
            run_hashes(true),
            run_hashes(true),
            "same look twice hashes twice"
        );
        let mut sim =
            LiveMatch::with_rules(&cfg, ArenaRules::v2_free_for_all()).expect("v2 builds");
        for _ in 0..10u32 {
            let _ = sim.step(&[]);
        }
        assert_eq!(sim.tick(), 10, "scripted v2 bout runs 10 ticks");
        // Authoritative mirror: both slots read `Sim::balance` via `LiveMatch`.
        let mut look = balance_look_two();
        look.observe_live_balances(&sim, 2);
        for index in 0..2usize {
            let slot = u8::try_from(index).expect("slot fits");
            let want = sim.balance(slot).expect("slot live").gait();
            assert_eq!(look.slots[index].balance, want, "slot {slot} mirrors sim");
        }
        // One floored slot (authoritative `Fallen`): root below 0.4 m within
        // 10 frames, standing slot unchanged.
        let standing_before = look.slots[0];
        look.observe_balance(1, GaitPhase::Fallen);
        assert_eq!(look.slots[1].balance, GaitPhase::Fallen);
        assert!(balance_pelvis_y_m(GaitPhase::Fallen, 0.9) < 0.4);
        assert_eq!(look.slots[0], standing_before, "standing keeps pose");
        // Flat pose with real rigs: `Fallen` forces the `Down` last frame.
        let rigged = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let rig = &rigged.rigs[rigged.rig_for_slot(1)];
        for frame in 0..10u32 {
            let sample = look
                .slot_sample(rig, &look.slots[1], 0.5)
                .expect("floored sample");
            assert_eq!(sample.clip, 5, "floored draws flat on frame {frame}");
            let pose = look
                .slot_pose(rig, &look.slots[1], 0.5)
                .expect("floored pose samples");
            assert!(
                pose.positions[rig.pelvis][1] < 0.4,
                "floored root below 0.4 m on frame {frame}"
            );
        }
        // Standing keeps the act's clip and pelvis.
        let calm_sample = look
            .slot_sample(rig, &look.slots[0], 0.5)
            .expect("standing sample");
        assert_ne!(calm_sample.clip, 5, "standing never draws flat");
        assert_eq!(balance_pelvis_y_m(GaitPhase::Steady, 0.9), 0.9);
        assert_eq!(balance_lean_m(GaitPhase::Steady), 0.0);
        look.stumble_slot(99);
        look.floor_slot(99);
        assert_eq!(look.slots[1].balance, GaitPhase::Fallen, "bad slot ignored");
    }

    #[test]
    fn balance_look_authoritative_verdict_lies_flat_and_kneels() {
        let mut look = balance_look_two();
        look.observe_balance(1, GaitPhase::Fallen);
        assert_eq!(look.slots[1].balance, GaitPhase::Fallen);
        assert!(
            balance_pelvis_y_m(GaitPhase::Fallen, 0.9) < 0.4,
            "fallen lies flat below 0.4 m"
        );
        look.observe_balance(0, GaitPhase::Catching);
        assert_eq!(balance_pelvis_y_m(GaitPhase::Catching, 0.9), 0.45);
        assert_eq!(balance_lean_m(GaitPhase::Catching), 0.0);
        look.observe_balance(1, GaitPhase::Stumbling);
        assert_eq!(
            look.slots[1].balance,
            GaitPhase::Fallen,
            "fallen never downgrades"
        );
        look.observe_balance(99, GaitPhase::Fallen);
        assert_eq!(look.slots[1].balance, GaitPhase::Fallen, "bad slot ignored");
        let mut again = balance_look_two();
        again.observe_balance(1, GaitPhase::Fallen);
        again.observe_balance(0, GaitPhase::Catching);
        again.observe_balance(1, GaitPhase::Stumbling);
        again.observe_balance(99, GaitPhase::Fallen);
        assert_eq!(look.slots[0].balance, again.slots[0].balance);
        assert_eq!(look.slots[1].balance, again.slots[1].balance);
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
    fn balance_poll_mirrors_authoritative_gaits() {
        use engine_arena::{ArenaConfig, ArenaRules, LiveMatch};
        let cfg = ArenaConfig {
            seed: 2040,
            ticks: 20,
            fighters: 2,
            half_extent_m: 10.0,
            table_version: 2,
        };
        let mut live =
            LiveMatch::with_rules(&cfg, ArenaRules::v2_free_for_all()).expect("v2 builds");
        for _ in 0..10u32 {
            let _ = live.step(&[]);
        }
        let mut look = balance_look_two();
        look.observe_live_balances(&live, 2);
        for index in 0..2usize {
            let slot = u8::try_from(index).expect("slot fits");
            let want = live.balance(slot).expect("slot live").gait();
            assert_eq!(look.slots[index].balance, want, "slot {slot} mirrors sim");
        }
        let mut again = balance_look_two();
        again.observe_live_balances(&live, 2);
        assert_eq!(look.slots[0].balance, again.slots[0].balance);
        assert_eq!(look.slots[1].balance, again.slots[1].balance);
        look.observe_balance(1, GaitPhase::Fallen);
        look.observe_balance(99, GaitPhase::Fallen);
        assert_eq!(look.slots[1].balance, GaitPhase::Fallen, "bad slot ignored");
        look.observe_live_balances(&live, 2);
        let want0 = live.balance(0).expect("slot live").gait();
        assert_eq!(look.slots[0].balance, want0, "steady slot still mirrors");
        assert_eq!(
            look.slots[1].balance,
            GaitPhase::Fallen,
            "fallen never downgrades"
        );
        look.observe_live_balances(&live, 99);
        assert_eq!(look.slots[0].balance, want0, "wide poll keeps the mirror");
    }

    #[test]
    fn balance_look_live_poll_lies_flat() {
        use engine_arena::{ArenaConfig, ArenaRules, LiveMatch};
        let cfg = ArenaConfig {
            seed: 2040,
            ticks: 20,
            fighters: 2,
            half_extent_m: 10.0,
            table_version: 2,
        };
        let run = || {
            let mut live =
                LiveMatch::with_rules(&cfg, ArenaRules::v2_free_for_all()).expect("v2 builds");
            for _ in 0..10u32 {
                let _ = live.step(&[]);
            }
            let mut look = balance_look_two();
            look.observe_live_balances(&live, 2);
            for index in 0..2usize {
                let slot = u8::try_from(index).expect("slot fits");
                let want = live.balance(slot).expect("slot live").gait();
                assert_eq!(look.slots[index].balance, want, "slot {slot} mirrors live");
            }
            assert_eq!(
                balance_pelvis_y_m(GaitPhase::Steady, 0.9),
                0.9,
                "standing keeps pose"
            );
            assert_eq!(balance_lean_m(GaitPhase::Steady), 0.0);
            look.observe_balance(1, GaitPhase::Fallen);
            look.observe_live_balances(&live, 2);
            assert_eq!(
                look.slots[1].balance,
                GaitPhase::Fallen,
                "live poll keeps the floor"
            );
            let floored = balance_pelvis_y_m(GaitPhase::Fallen, 0.9);
            assert!(floored < 0.4, "floored live poll lies flat, got {floored}");
            look.observe_balance(0, GaitPhase::Catching);
            assert_eq!(
                balance_pelvis_y_m(GaitPhase::Catching, 0.9),
                0.45,
                "catching kneels"
            );
            assert_eq!(balance_lean_m(GaitPhase::Catching), 0.0, "catching holds");
            look.observe_balance(99, GaitPhase::Fallen);
            assert_eq!(look.slots[1].balance, GaitPhase::Fallen, "bad slot ignored");
            (look.slots[0].balance, look.slots[1].balance, floored)
        };
        let first = run();
        let second = run();
        assert_eq!(first, second, "live poll x2 equal");
        assert!(first.2 < 0.4, "fallen root below 0.4 m twice");
    }

    #[test]
    fn app_live_match_feeds_look_mirror() {
        let config = crate::GameConfig {
            seed: 2040,
            ticks: 20,
            fighters: 2,
            half_extent_m: 20.0,
        };
        let mut app = crate::App::new(config).expect("session builds");
        for _ in 0..10u32 {
            let _ = app.tick_empty();
        }
        let mut look = balance_look_two();
        look.observe_live_balances(app.live_match(), 2);
        for index in 0..2usize {
            let slot = u8::try_from(index).expect("slot fits");
            let want = app.live_match().balance(slot).expect("slot live").gait();
            assert_eq!(look.slots[index].balance, want, "slot {slot} mirrors app");
        }
        let mut again = balance_look_two();
        again.observe_live_balances(app.live_match(), 2);
        assert_eq!(look.slots[0].balance, again.slots[0].balance);
        assert_eq!(look.slots[1].balance, again.slots[1].balance);
    }

    #[test]
    fn sand_lift_rests_the_lowest_vertex_and_never_pushes_down() {
        // The lift reads the pose-skinned bounds of every act. Above sand and
        // non-finite never lift.
        assert_eq!(sand_lift_m(0.05), 0.0, "above sand never pushes down");
        assert_eq!(
            sand_lift_m(0.0),
            0.0,
            "resting exactly on sand needs no lift"
        );
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
    fn product_sunk_56cm_down_rest_reports_green() {
        // Product 05142ee3 frame 169 slot 0 down min -0.561, sunk 56.1 floor 3.
        // Down lift eased over the fall (LF17, view only): 0 at the fall start,
        // full depth at rest, cap 1.0. Pure positions plus view laws, no window.
        // Caller: `FighterLook::pose_frame` plus `pose_frame_joints` (the live
        // down-only rest lift reads this eased depth).
        use crate::view_truth::{BodyTruth, ViewLaws};
        let cam = Camera::new([0.0, 1.5, 6.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.0], 1.0);
        let mut raw = BodyTruth::new(
            0,
            "down",
            [19.185, 0.0, 3.438],
            -2.111,
            vec![[0.0, 1.0, 0.0]],
        );
        raw.fold_vertices(&[[18.672, -0.561, 3.107], [19.493, 1.111, 3.872]]);
        assert!(
            (raw.min[1] + 0.561).abs() < 1e-6,
            "pose min is the sunk depth"
        );
        let mut before = ViewLaws::default();
        before.note(&cam, 169.0, &[raw.clone()]);
        assert!(
            before.line().unwrap().contains("sunk_cm_max=56.1"),
            "unlifted sunk 56.1, got {}",
            before.line().unwrap()
        );
        let start_lift = sand_lift_eased(raw.min[1], 0);
        assert!(
            start_lift.abs() < 1e-6,
            "fall start lifts nothing, got {start_lift}"
        );
        let mid_lift = sand_lift_eased(raw.min[1], SAND_FALL_TICKS / 2);
        assert!(
            mid_lift > 0.0 && mid_lift < 0.561,
            "fall middle lifts partway, got {mid_lift}"
        );
        let lift = sand_lift_eased(raw.min[1], SAND_FALL_TICKS);
        assert!(
            (lift - 0.561).abs() < 1e-6,
            "rest lifts full depth, got {lift}"
        );
        assert!(
            (raw.min[1] + lift).abs() * 100.0 < 1.0,
            "the rested body sits within 1 cm of the sand"
        );
        let mut fixed = raw;
        fixed.root[1] += lift;
        fixed.min[1] += lift;
        fixed.max[1] += lift;
        let mut after = ViewLaws::default();
        after.note(&cam, 169.0, &[fixed]);
        assert!(
            after.line().unwrap().contains("sunk_cm_max=0.0"),
            "lifted sunk 0, got {}",
            after.line().unwrap()
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
    fn fighter_anim_bridge_drives_look_pick() {
        use crate::anim_bridge::{BridgeClip, BridgeTuning};
        // Glue tuning builds from the look's speed constants; bad values refuse.
        let tuning = BridgeTuning::new(super::RUN_SPEED_M_PER_TICK, super::RUN_REF_M_PER_TICK)
            .expect("look tuning builds");
        assert!(
            BridgeTuning::new(0.0, super::RUN_REF_M_PER_TICK).is_err(),
            "zero threshold refuses"
        );
        assert!(
            BridgeTuning::new(super::RUN_SPEED_M_PER_TICK, f32::NAN).is_err(),
            "NaN ref refuses"
        );
        let _ = tuning;
        let run = || {
            let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
            let hp = [1000u16, 1000u16];
            let hit = FightCue::Hit {
                attacker: 0,
                victim: 1,
                damage: 100,
            };
            // Tick 0 still: idle on both sides of the wire.
            let still = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
            look.observe_tick(0, &still, &hp, &[]);
            assert_eq!(look.act(0), Some(Act::Idle));
            assert_eq!(look.bridge[0].pick().clip, BridgeClip::Idle);
            assert_eq!(look.slots[0].fade_left, look.bridge[0].fade().fade_left);
            // Steady 0.10 m per tick: run on both sides, fading together.
            for tick in 1..=4u32 {
                let x = -1.0 + 0.10 * f32::from(u16::try_from(tick).unwrap_or(u16::MAX));
                let views = [view(0, x, 0.0), view(1, 1.0, 0.0)];
                look.observe_tick(tick, &views, &hp, &[]);
            }
            assert_eq!(look.act(0), Some(Act::Run));
            assert_eq!(look.bridge[0].pick().clip, BridgeClip::Run);
            assert!(
                look.bridge[0].fade().blending(),
                "run switch restarts the fade"
            );
            assert_eq!(
                look.slots[0].fade_left,
                look.bridge[0].fade().fade_left,
                "look reads the bridge fade"
            );
            assert_eq!(
                usize::from(look.bridge[0].fade().prev_clip),
                look.slots[0].prev_clip,
                "leaving clip rides the wire"
            );
            // A Hit starts the attack variant on both sides; the look keeps
            // the landed impact backdate while the clip matches.
            let held = [view(0, -0.6, 0.0), view(1, 1.0, 0.0)];
            look.observe_tick(5, &held, &hp, &[hit]);
            assert!(matches!(look.act(0), Some(Act::Attack(0))));
            assert_eq!(look.bridge[0].pick().clip, BridgeClip::Attack0);
            assert_eq!(
                look.slots[0].fade_left,
                look.bridge[0].fade().fade_left,
                "attack restarts the fade through the wire"
            );
            // Zero hp floors both sides.
            look.observe_tick(6, &held, &[1000, 0], &[]);
            assert_eq!(look.act(1), Some(Act::Down));
            assert_eq!(look.bridge[1].pick().clip, BridgeClip::Down);
            (look.act(0), look.bridge[0].pick(), look.slots[0].fade_left)
        };
        assert_eq!(
            run(),
            run(),
            "same cues give the same bridge-driven pick twice"
        );
    }

    #[test]
    fn fighter_anim_bridge_owns_look_act_reads_bridge() {
        use crate::anim_bridge::BridgeClip;
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let hp = [1000u16, 1000u16];
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 100,
        };
        // Idle on a still tick.
        let still = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(0, &still, &hp, &[]);
        assert_eq!(look.act(0), Some(Act::Idle));
        assert_eq!(look.bridge[0].pick().clip, BridgeClip::Idle);
        // Steady motion runs.
        for tick in 1..=4u32 {
            let x = -1.0 + 0.10 * f32::from(u16::try_from(tick).unwrap_or(u16::MAX));
            let views = [view(0, x, 0.0), view(1, 1.0, 0.0)];
            look.observe_tick(tick, &views, &hp, &[]);
        }
        assert_eq!(look.act(0), Some(Act::Run));
        assert_eq!(look.bridge[0].pick().clip, BridgeClip::Run);
        // One attack.
        let held = [view(0, -0.6, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(5, &held, &hp, &[hit]);
        assert!(matches!(look.act(0), Some(Act::Attack(0))));
        assert_eq!(look.bridge[0].pick().clip, BridgeClip::Attack0);
        // A diverged mirror still reports the bridge pick.
        look.slots[0].act = Act::Down;
        assert_eq!(
            look.act(0),
            Some(Act::Attack(0)),
            "act reads the bridge, not the mirror"
        );
        // An unseen slot stays silent even though its bridge idles.
        assert_eq!(look.act(2), None);
        assert_eq!(look.bridge[2].pick().clip, BridgeClip::Idle);
    }

    #[test]
    fn fighter_anim_crossfade_midpoint_blends() {
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let hp = [1000u16, 1000u16];
        // Tick 0 still, then steady 0.05 m per tick: Idle flips to Run at 1.
        for tick in 0..=5u32 {
            let x = -1.0 + 0.05 * f32::from(u16::try_from(tick).unwrap_or(u16::MAX));
            let views = [view(0, x, 0.0), view(1, 1.0, 0.0)];
            look.observe_tick(tick, &views, &hp, &[]);
        }
        let slot = look.slots[0];
        assert_eq!(slot.act, Act::Run, "steady motion runs");
        assert_eq!(slot.fade_left, 5, "switch at 1 plus 4 ticks leaves 5");
        let alpha = 0.5;
        let rig = &look.rigs[look.rig_for_slot(0)];
        let got = look.slot_pose(rig, &slot, alpha).expect("pose samples");
        let left = f32::from(u16::try_from(slot.fade_left).unwrap_or(u16::MAX));
        let total = f32::from(u16::try_from(FIGHTER_CROSSFADE_TICKS).unwrap_or(u16::MAX));
        let w = 1.0 - left / total;
        assert!(
            (w - 4.0 / 9.0).abs() < 1e-6,
            "midpoint weight is 4/9, got {w}"
        );
        assert_ne!(w, 0.5, "9 is odd: no exact half");
        let (cur_clip, cur_time) =
            look.clip_time(rig, slot.act, slot.act_start, slot.run_speed, alpha);
        let cur = sample_pose_with_rots(
            &rig.skeleton,
            &rig.clips[cur_clip].positions,
            &rig.clips[cur_clip].rotations,
            cur_time,
        )
        .expect("current samples");
        let prev_clip = rig.clips.get(slot.prev_clip).expect("prev clip");
        let prev = sample_pose_with_rots(
            &rig.skeleton,
            &prev_clip.positions,
            &prev_clip.rotations,
            slot.prev_time,
        )
        .expect("prev samples");
        let mut expect = blend_poses(&prev, &cur, w);
        expect.positions[rig.pelvis][0] = rig.bind_pelvis[0];
        expect.positions[rig.pelvis][2] = rig.bind_pelvis[2];
        expect.positions[rig.pelvis][1] =
            balance_pelvis_y_m(slot.balance, expect.positions[rig.pelvis][1]);
        assert_eq!(got, expect, "midpoint blends bit for bit");
        let mut cur_pinned = cur;
        cur_pinned.positions[rig.pelvis][0] = rig.bind_pelvis[0];
        cur_pinned.positions[rig.pelvis][2] = rig.bind_pelvis[2];
        cur_pinned.positions[rig.pelvis][1] =
            balance_pelvis_y_m(slot.balance, cur_pinned.positions[rig.pelvis][1]);
        let mut prev_pinned = prev;
        prev_pinned.positions[rig.pelvis][0] = rig.bind_pelvis[0];
        prev_pinned.positions[rig.pelvis][2] = rig.bind_pelvis[2];
        prev_pinned.positions[rig.pelvis][1] =
            balance_pelvis_y_m(slot.balance, prev_pinned.positions[rig.pelvis][1]);
        assert_ne!(got, cur_pinned, "blend differs from the new clip");
        assert_ne!(got, prev_pinned, "blend differs from the old clip");
    }

    #[test]
    fn fighter_anim_crossfade_replays_x2() {
        let run = || {
            let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
            let hp = [1000u16, 1000u16];
            let mut poses = Vec::new();
            for tick in 0..=10u32 {
                let x = -1.0 + 0.05 * f32::from(u16::try_from(tick).unwrap_or(u16::MAX));
                let views = [view(0, x, 0.0), view(1, 1.0, 0.0)];
                look.observe_tick(tick, &views, &hp, &[]);
                let rig = &look.rigs[look.rig_for_slot(0)];
                poses.push(look.slot_pose(rig, &look.slots[0], 0.5).expect("pose"));
                poses.push(look.slot_pose(rig, &look.slots[0], 0.0).expect("pose"));
            }
            poses
        };
        assert_eq!(run(), run(), "same ticks give same blend twice");
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
    fn fighter_anim_hit_react_flinches_and_decays() {
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        let hp = [1000u16, 1000u16];
        let mut hit = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let mut calm = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        for tick in 0..=20u32 {
            hit.observe_tick(tick, &views, &hp, &[]);
            calm.observe_tick(tick, &views, &hp, &[]);
        }
        // Same ticks give the same pose before any hit.
        for alpha in [0.0, 0.5] {
            let slot_h = hit.slots[1];
            let rig_h = &hit.rigs[hit.rig_for_slot(1)];
            let pose_h = hit.slot_pose(rig_h, &slot_h, alpha).expect("pose samples");
            let slot_c = calm.slots[1];
            let rig_c = &calm.rigs[calm.rig_for_slot(1)];
            let pose_c = calm.slot_pose(rig_c, &slot_c, alpha).expect("pose samples");
            assert_eq!(pose_h, pose_c, "pre-hit poses replay at alpha {alpha}");
        }
        let cue = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 100,
        };
        // Same hitstop hold on both sides (LF10): `Parried` freezes the same
        // impact pair without a flinch, so past-window poses compare hold to
        // hold and only the flinch can differ.
        let calm_hold = FightCue::Parried {
            attacker: 0,
            victim: 1,
        };
        hit.observe_tick(21, &views, &hp, &[cue]);
        calm.observe_tick(21, &views, &hp, &[calm_hold]);
        let slot_h = hit.slots[1];
        let rig_h = &hit.rigs[hit.rig_for_slot(1)];
        let flinch = hit.slot_pose(rig_h, &slot_h, 0.5).expect("pose samples");
        let slot_c = calm.slots[1];
        let rig_c = &calm.rigs[calm.rig_for_slot(1)];
        let base = calm.slot_pose(rig_c, &slot_c, 0.5).expect("pose samples");
        let pelvis = rig_h.pelvis;
        let dx = flinch.positions[pelvis][0] - base.positions[pelvis][0];
        let dz = flinch.positions[pelvis][2] - base.positions[pelvis][2];
        let dy = flinch.positions[pelvis][1] - base.positions[pelvis][1];
        assert!(
            dx * dx + dz * dz > 1e-10,
            "a Hit pushes the victim pelvis off its base: dx={dx} dz={dz}"
        );
        assert!(dy < 0.0, "a Hit dips the victim pelvis: dy={dy}");
        // The push points away from the attacker (victim at +X of attacker).
        assert!(dx > 0.0, "flinch runs away from the attacker: dx={dx}");
        // Mid-window the flinch has decayed but still reads.
        for tick in 22..=26u32 {
            hit.observe_tick(tick, &views, &hp, &[]);
            calm.observe_tick(tick, &views, &hp, &[]);
        }
        let slot_h = hit.slots[1];
        let rig_h = &hit.rigs[hit.rig_for_slot(1)];
        let mid = hit.slot_pose(rig_h, &slot_h, 0.5).expect("pose samples");
        let slot_c = calm.slots[1];
        let rig_c = &calm.rigs[calm.rig_for_slot(1)];
        let base_mid = calm.slot_pose(rig_c, &slot_c, 0.5).expect("pose samples");
        let mdx = mid.positions[pelvis][0] - base_mid.positions[pelvis][0];
        let mdz = mid.positions[pelvis][2] - base_mid.positions[pelvis][2];
        assert!(
            mdx * mdx + mdz * mdz < dx * dx + dz * dz,
            "the flinch decays inside the window"
        );
        assert!(
            mdx * mdx + mdz * mdz > 1e-12,
            "the flinch still reads mid-window"
        );
        // Past the window the base returns exactly.
        for tick in 27..=30u32 {
            hit.observe_tick(tick, &views, &hp, &[]);
            calm.observe_tick(tick, &views, &hp, &[]);
        }
        let slot_h = hit.slots[1];
        let rig_h = &hit.rigs[hit.rig_for_slot(1)];
        let past = hit.slot_pose(rig_h, &slot_h, 0.5).expect("pose samples");
        let slot_c = calm.slots[1];
        let rig_c = &calm.rigs[calm.rig_for_slot(1)];
        let base_past = calm.slot_pose(rig_c, &slot_c, 0.5).expect("pose samples");
        // Balance may still read stumbling, but the flinch offset is gone:
        // both sides froze the same hitstop hold (Hit vs Parried), and
        // stumble never moves `slot_pose`, so poses match exactly.
        assert_eq!(past, base_past, "past the window the flinch is silent");
        // Same Hit ticks give the same flinch twice.
        let run = || {
            let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
            for tick in 0..=20u32 {
                look.observe_tick(tick, &views, &hp, &[]);
            }
            look.observe_tick(21, &views, &hp, &[cue]);
            let slot = look.slots[1];
            let rig = &look.rigs[look.rig_for_slot(1)];
            look.slot_pose(rig, &slot, 0.5).expect("pose samples")
        };
        assert_eq!(run(), run(), "same Hit tick gives the same flinch twice");
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
        // A moved pose is what re-sends a part (an upload digest skips only
        // the parts whose bytes are already resident), so every step here
        // steps the tick and both roots: each staged frame is a changed one.
        for step in 1..=5u8 {
            let moved = [
                view(0, -1.0 + f32::from(step) * 0.1, 0.0),
                view(1, 1.0 - f32::from(step) * 0.1, 0.0),
            ];
            look.observe_tick(u32::from(step), &moved, &[1000, 1000], &[]);
            let written = look
                .stage_frame(&mut presenter, 2, 0.5)
                .expect("frame stages");
            assert!(written >= 2, "both fighters write their parts");
        }
        // 48 B stride (pos + normal + uv + tangent) needs a full re-upload
        // with recomputed tangents on every changed skinned frame; the 32 B
        // in-place refresh misaligns past vertex 0 into streaks (1209Z).
        // Creates must grow while every part still writes.
        assert!(
            presenter.gpu_create_totals() > created,
            "tangent re-upload creates buffers: {:?} vs {created:?}",
            presenter.gpu_create_totals()
        );
        // No streak: every posed vertex stays within 5 m of its slot root
        // (a stretched triangle would read as tens of meters).
        for body in look.view_truth() {
            let dx = body.max[0] - body.min[0];
            let dy = body.max[1] - body.min[1];
            let dz = body.max[2] - body.min[2];
            assert!(
                dx < 5.0 && dy < 5.0 && dz < 5.0,
                "fighter bounds stay compact, got [{dx:.2}, {dy:.2}, {dz:.2}]"
            );
        }
        // A fighter that shows up later already owns its buffers.
        let three = [view(0, -1.0, 0.0), view(1, 1.0, 0.0), view(2, 0.0, 3.0)];
        look.observe_tick(6, &three, &[1000, 1000, 1000], &[]);
        look.stage_frame(&mut presenter, 3, 0.5)
            .expect("late fighter stages");
        assert!(
            presenter.gpu_create_totals() > created,
            "late slots re-upload too"
        );
    }

    #[test]
    fn skinned_upload_skips_resident_parts_and_counts_both_halves() {
        // F2P: without the upload digest every part re-runs the
        // `v1::upload_mesh`/`Presenter::upload_mesh` pair on every present, so
        // `second.skin.upload_us` is non-zero and the assertion below fails.
        // The two halves must both count on a staged rig, must sit inside the
        // wall they are cut from, and the half the cut owns must drop on an
        // identical second call while `uploaded` still reports the parts the
        // frame needs - and a moved pose must re-upload whole, never leaving
        // the resident bytes of a previous tick on screen.
        let mut presenter = match Presenter::init_offscreen() {
            Ok(presenter) => presenter,
            Err(engine_render::present::PresentError::NoAdapter) => {
                eprintln!("SKIPPED fighter skin split: no wgpu adapter on this machine");
                return;
            }
            Err(err) => panic!("presenter init failed (must fail loudly): {err}"),
        };
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        look.stage_meshes(&mut presenter).expect("bodies stage");
        let views = [view(0, -1.0, 0.0)];
        look.observe_tick(0, &views, &[1000], &[]);
        let scene = engine_render::v1::default_scene(64, 64).expect("default scene is valid");
        let list = || DrawList {
            camera: scene.camera.clone(),
            light: scene.light.clone(),
            items: Vec::new(),
        };
        let frame = LookFrame {
            fighters: 1,
            alpha: 0.5,
            shot: Shot::Action,
            frame: 0,
        };
        let parts = look.rigs[look.rig_for_slot(0)].parts.len();
        // Warm the skin, the tangent rebuild and the allocator off the clock.
        let warm = LookFrame {
            alpha: 0.25,
            ..frame
        };
        look.present_into(&mut presenter, &mut list(), &warm)
            .expect("warm present");
        // One tick later the fighter stands a step on: the changed placed
        // vertices must re-upload whole, and the identical repeat must not.
        look.observe_tick(1, &[view(0, -0.8, 0.0)], &[1000], &[]);
        let wall = Instant::now();
        let first = look
            .present_into(&mut presenter, &mut list(), &frame)
            .expect("first present");
        let wall_ns = wall.elapsed().as_nanos();
        let second = look
            .present_into(&mut presenter, &mut list(), &frame)
            .expect("repeat present");
        assert!(
            first.skin.pose_us > 0 && first.skin.upload_us > 0,
            "both halves count on a staged rig, got {:?}",
            first.skin
        );
        let split_ns =
            u128::from(first.skin.pose_us) * 1_000 + u128::from(first.skin.upload_us) * 1_000;
        assert!(
            split_ns <= wall_ns,
            "the split never exceeds the wall it was cut from: {split_ns} ns vs {wall_ns} ns"
        );
        assert_eq!(
            first.skin.uploaded, parts,
            "one staged fighter writes its parts"
        );
        assert_eq!(
            second.skin.uploaded, parts,
            "the repeated frame still reports the parts it needs"
        );
        // The cut: an unchanged digest runs no `upload_mesh` pair, so the
        // second identical call adds zero upload time while the first frame
        // of a moved pose still uploads whole.
        assert_eq!(
            second.skin.upload_us, 0,
            "an unchanged digest uploads nothing, got {:?}",
            second.skin
        );
        // A skipped part is never a stale one: the next tick moves the body,
        // so its parts leave the resident digest and the pair runs again.
        look.observe_tick(2, &[view(0, -0.6, 0.0)], &[1000], &[]);
        let moved = look
            .present_into(&mut presenter, &mut list(), &frame)
            .expect("moved present");
        assert!(
            moved.skin.upload_us > 0 && moved.skin.uploaded == parts,
            "a moved pose re-uploads its parts, got {:?}",
            moved.skin
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

    /// True when `inner` sits inside `outer` grown by `pad` on every axis.
    /// A non-finite face reads as outside (loud, never a silent pass).
    fn bounds_hold(
        outer_min: [f32; 3],
        outer_max: [f32; 3],
        inner_min: [f32; 3],
        inner_max: [f32; 3],
        pad: f32,
    ) -> bool {
        outer_min
            .iter()
            .zip(inner_min.iter())
            .all(|(outer, inner)| *outer - pad <= *inner)
            && inner_max
                .iter()
                .zip(outer_max.iter())
                .all(|(inner, outer)| *inner <= *outer + pad)
    }

    /// One synthetic skinned frame over `positions`: the fold input of
    /// [`posed_bound_m`].
    fn bound_frame(positions: &[[f32; 3]]) -> engine_render::v1_skin::SkinnedFrame {
        engine_render::v1_skin::SkinnedFrame {
            positions: positions.to_vec(),
            normals: vec![[0.0, 1.0, 0.0]; positions.len()],
            used_error_material: false,
            diagnostics: Vec::new(),
        }
    }

    /// One synthetic posed body over `positions`, with no bound computed yet.
    fn bound_body(positions: &[[f32; 3]]) -> PosedBody {
        PosedBody {
            truth: BodyTruth::new(0, "idle", [0.0; 3], 0.0, Vec::new()),
            frames: vec![bound_frame(positions)],
            lift: 0.0,
            bound: None,
        }
    }

    /// Gap between two world points in meters.
    fn gap_m(a: [f32; 3], b: [f32; 3]) -> f32 {
        let (dx, dy, dz) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
        (dx * dx + dy * dy + dz * dz).sqrt()
    }

    #[test]
    fn posed_bound_follows_a_posed_arm_outside_the_bind_sphere() {
        // Accept case: the bind sphere is 0.5 m around the trunk and the
        // swing holds the arm 1.5 m out past it. A bind-pose sphere culls
        // that arm; the posed bound holds it.
        let bind_radius_m = 0.5f32;
        let (trunk, arm) = ([0.0, 0.9, 0.0], [1.5, 1.2, 0.0]);
        let body = bound_body(&[[-0.3, 0.0, -0.3], [0.3, 1.8, 0.3], arm]);
        let (centre, radius) = posed_bound_m(&body).expect("finite vertices bind");
        assert!(
            gap_m(centre, arm) <= radius + 1e-6,
            "the posed arm stays inside the bound, got {} over {radius}",
            gap_m(centre, arm)
        );
        assert!(
            gap_m(trunk, arm) > bind_radius_m,
            "the arm leaves the sphere"
        );
        assert!(radius >= bind_radius_m, "the bound clears the sphere");
    }

    #[test]
    fn posed_bound_replays_same_bytes_twice() {
        let body = bound_body(&[[-0.3, 0.0, -0.3], [1.5, 1.2, 0.0], [0.3, 1.8, 0.3]]);
        let first = posed_bound_m(&body).expect("finite vertices bind");
        let second = posed_bound_m(&body).expect("finite vertices bind again");
        assert_eq!(
            (first.0.map(f32::to_bits), first.1.to_bits()),
            (second.0.map(f32::to_bits), second.1.to_bits()),
            "the same posed vertices give the same bound bytes twice"
        );
    }

    #[test]
    fn posed_bound_refuses_a_nan_vertex() {
        // A NaN vertex never widens the bound and never panics; a body whose
        // every vertex is non-finite has no bound at all.
        assert_eq!(
            posed_bound_m(&bound_body(&[[f32::NAN, 0.0, 0.0]])),
            None,
            "no finite vertex, no bound"
        );
        let mixed = bound_body(&[[0.0, 0.0, 0.0], [f32::NAN, 90.0, 0.0], [1.0, 2.0, 0.0]]);
        let (centre, radius) = posed_bound_m(&mixed).expect("finite vertices bind");
        assert_eq!(
            centre,
            [0.5, 1.0, 0.0],
            "the NaN vertex never moves the centre"
        );
        assert!(
            centre.iter().all(|v| v.is_finite()) && radius.is_finite(),
            "the bound stays finite"
        );
        assert!(
            radius <= gap_m(centre, [1.0, 2.0, 0.0]) + 1e-6,
            "the NaN vertex never widens the bound, got {radius}"
        );
    }

    #[test]
    fn posed_bound_never_shrinks_below_the_bind_radius() {
        // The caller floors the posed radius at the rig's bind radius
        // (`Rig::radius_m` = `skin_radius_m` of the bind body, floor 0.5 m),
        // so a bound never shrinks under today's value.
        let bind_radius_m = 0.5f32;
        let tiny = bound_body(&[[-0.01, 0.0, -0.01], [0.01, 0.04, 0.01]]);
        let (_, radius) = posed_bound_m(&tiny).expect("finite vertices bind");
        assert!(radius < bind_radius_m, "a tiny pose sits inside the sphere");
        assert_eq!(radius.max(bind_radius_m), bind_radius_m, "the floor holds");
        // End to end: every slot a real posed frame carries sits at or above
        // its rig's bind radius, never under it.
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(0, &views, &[1000, 1000], &[]);
        let posed = look.pose_frame(2, 0.5).expect("CPU pose");
        assert_eq!(posed.len(), 2, "both seen slots pose");
        for body in &posed {
            let rig = &look.rigs[look.rig_for_slot(usize::from(body.truth.slot))];
            let (centre, radius) = body.bound.expect("a posed body carries a bound");
            assert!(centre.iter().all(|v| v.is_finite()), "centre stays finite");
            assert!(
                radius >= rig.radius_m,
                "slot {} bound {radius} under the bind radius {}",
                body.truth.slot,
                rig.radius_m
            );
        }
    }

    #[test]
    fn view_run_repeats_its_arena_view_line_x2_with_no_gpu() {
        // SP61 accept: a seeded 600-tick 8-fighter run prints the same
        // `arena view:` line twice. No `Presenter` is built anywhere here,
        // so the load plus observe plus CPU pose path runs with no GPU.
        let first = FighterLook::view_run(2040, 600, 8).expect("headless view runs");
        let second = FighterLook::view_run(2040, 600, 8).expect("headless view repeats");
        println!("{first}");
        println!("{second}");
        assert!(!first.is_empty(), "600 ticks fold frames");
        assert!(
            first.starts_with("arena view:"),
            "receipt line, got {first}"
        );
        assert_eq!(first, second, "same seed gives the same view line twice");
    }

    #[test]
    fn view_run_seed_repeats_and_joints_hold_bounds() {
        // Headless run with no `Presenter`: the same seed prints the same
        // `arena view:` line twice, joints-only holds the vertex bounds
        // within 10 cm, and joints-only clears 2000 frames/s.
        let first = FighterLook::view_run(2040, 60, 2).expect("headless view runs");
        let second = FighterLook::view_run(2040, 60, 2).expect("headless view repeats");
        assert!(!first.is_empty(), "ticks fold frames");
        assert!(
            first.starts_with("arena view:"),
            "receipt line, got {first}"
        );
        assert_eq!(first, second, "same seed repeats bit for bit");
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let config = crate::GameConfig {
            seed: 99,
            ticks: 20,
            fighters: 2,
            half_extent_m: 20.0,
        };
        let mut app = crate::App::new(config).expect("bout builds");
        for tick in 0..20u32 {
            let outcome = app.tick_empty();
            look.observe_tick(tick, app.fighter_views(), app.hp(), &outcome.cues);
            let full = look.pose_frame(2, 0.5).expect("CPU pose");
            let joints = look.pose_frame_joints(2, 0.5).expect("joints pose");
            assert_eq!(full.len(), joints.len(), "same slots both ways");
            for (posed, joint) in full.iter().map(PosedBody::truth).zip(&joints) {
                assert!(posed.has_bounds() && joint.has_bounds(), "both modes bound");
                assert!(
                    bounds_hold(joint.min, joint.max, posed.min, posed.max, 0.10),
                    "joints hold vertex within 10 cm"
                );
            }
        }
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(100, &views, &[1000, 1000], &[]);
        for _ in 0..20u32 {
            look.pose_frame_joints(2, 0.5).expect("warm joints pose");
        }
        let frames = 200u32;
        let start = std::time::Instant::now();
        for _ in 0..frames {
            look.pose_frame_joints(2, 0.5).expect("timed joints pose");
        }
        let fps = f64::from(frames) / start.elapsed().as_secs_f64();
        assert!(fps >= 2000.0, "joints-only hits 2000 frames/s");
    }

    #[test]
    fn view_run_joints_bounds_hold_vertex_bounds_within_10cm() {
        // SP61 accept: joints-only bounds (joints padded by the per-rig
        // radius, no vertex skin) hold the full-vertex bounds within 10 cm,
        // over a real bout (idle, run, attack and down acts), and repeat.
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let config = crate::GameConfig {
            seed: 99,
            ticks: 120,
            fighters: 4,
            half_extent_m: 20.0,
        };
        let mut app = crate::App::new(config).expect("bout builds");
        for tick in 0..120u32 {
            let outcome = app.tick_empty();
            look.observe_tick(tick, app.fighter_views(), app.hp(), &outcome.cues);
            let full = look.pose_frame(4, 0.5).expect("CPU pose");
            let joints = look.pose_frame_joints(4, 0.5).expect("joints pose");
            let again = look.pose_frame_joints(4, 0.5).expect("joints pose repeats");
            assert_eq!(joints, again, "same tick gives same joints truth");
            assert_eq!(
                full.len(),
                joints.len(),
                "same slots pose both ways at tick {tick}"
            );
            for (posed, joint) in full.iter().map(PosedBody::truth).zip(&joints) {
                assert_eq!(posed.slot, joint.slot, "slot order matches");
                assert_eq!(posed.joints, joint.joints, "same clip sample feeds both");
                assert_eq!(posed.clip, joint.clip, "same clip feeds both");
                assert_eq!(posed.clip_s, joint.clip_s, "same clip time feeds both");
                assert!(
                    posed.has_bounds() && joint.has_bounds(),
                    "both modes bound tick {tick} slot {}",
                    posed.slot
                );
                assert!(
                    bounds_hold(joint.min, joint.max, posed.min, posed.max, 0.10),
                    "joints bounds hold vertex bounds within 10 cm at tick {tick} slot {}: joints {:?}..{:?} vs vertex {:?}..{:?}",
                    posed.slot,
                    joint.min,
                    joint.max,
                    posed.min,
                    posed.max,
                );
            }
        }
    }

    #[test]
    fn view_run_joints_only_hits_2000_frames_per_second() {
        // SP61 accept: joints-only poses at least 2000 frames/s for 8
        // fighters on the reference CPU. Bounds come from joints, never the
        // vertex skin, so the frame cost stays far under the skin cost.
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let views: Vec<FighterView> = (0..8u8)
            .map(|slot| view(slot, f32::from(slot) - 3.5, 0.0))
            .collect();
        look.observe_tick(0, &views, &[1000u16; 8], &[]);
        // Warm the pose (clip tables, allocator) off the clock.
        for _ in 0..20u32 {
            assert_eq!(
                look.pose_frame_joints(8, 0.5)
                    .expect("warm joints pose")
                    .len(),
                8,
                "every seen slot poses"
            );
        }
        let frames = 400u32;
        let mut posed = 0usize;
        let start = std::time::Instant::now();
        for _ in 0..frames {
            posed += look
                .pose_frame_joints(8, 0.5)
                .expect("timed joints pose")
                .len();
        }
        let fps = f64::from(frames) / start.elapsed().as_secs_f64();
        println!("joints-only 8-fighter pose: {fps:.0} frames/s over {frames} frames");
        assert_eq!(posed, 8 * usize::try_from(frames).unwrap_or(0));
        assert!(
            fps >= 2000.0,
            "joints-only hits 2000 frames/s, got {fps:.0}"
        );
    }

    #[test]
    fn view_run_joints_share_the_vertex_root_placement() {
        // Render-math checklist #1: the AN1 row-vector root maps body +Z
        // onto +X at 90 deg yaw, then moves to the root position. The
        // joints-only bounds ride this same placement as the vertex path.
        let root = root_matrix([1.0, 2.0, 3.0], std::f32::consts::FRAC_PI_2);
        let at = place_joint([0.0, 0.0, 1.0], &root);
        assert!(
            (at[0] - 2.0).abs() < 1e-6 && (at[1] - 2.0).abs() < 1e-6 && (at[2] - 3.0).abs() < 1e-6,
            "row-vector root placement, got {at:?}"
        );
    }

    #[test]
    fn trail_feed_stages_12_frames_oldest_first_and_clears_idle() {
        use crate::weapon_trail::{alpha_for_age, TRAIL_LEN};
        use engine_audio::FightCue;
        // F2P: an attacking slot staged 12 frames holds the last-12 ribbon
        // oldest first; today only `start_swing` pushes once, never ages or
        // clears, so this fails with one unaged pair.
        let mut look = balance_look_two();
        look.swing_ticks = 30;
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 100,
        };
        for tick in 0..12u32 {
            let x = -1.0 + 0.05 * f32::from(u16::try_from(tick).unwrap_or(u16::MAX));
            let views = [view(0, x, 0.0), view(1, 1.0, 0.0)];
            let cues = if tick == 0 { vec![hit] } else { vec![] };
            look.observe_tick(tick, &views, &[1000, 1000], &cues);
        }
        let held = look.trail_pairs(0);
        assert_eq!(held.len(), TRAIL_LEN, "12 staged frames hold 12 pairs");
        assert!(
            held[0].base_m[0] < held[TRAIL_LEN - 1].base_m[0],
            "oldest first with rising x: {:?} vs {:?}",
            held[0].base_m,
            held[TRAIL_LEN - 1].base_m
        );
        let mut ages: Vec<u8> = held.iter().map(|p| p.age_ticks).collect();
        ages.sort_unstable();
        let expect: Vec<u8> = (0..12u8).collect();
        assert_eq!(ages, expect, "ribbon holds ages 0..=11");
        assert_eq!(held[TRAIL_LEN - 1].age_ticks, 0, "newest is age 0");
        assert_eq!(held[0].age_ticks, 11, "oldest is age 11");
        assert!(
            alpha_for_age(0) > alpha_for_age(5) && alpha_for_age(5) > alpha_for_age(11),
            "alpha falls with age"
        );
        assert!(look.trail_pairs(1).is_empty(), "idle slot holds none");
        // Leaving Attack clears the ribbon.
        for tick in 12..60u32 {
            let views = [view(0, -0.4, 0.0), view(1, 1.0, 0.0)];
            look.observe_tick(tick, &views, &[1000, 1000], &[]);
        }
        assert!(
            look.trail_pairs(0).is_empty(),
            "leaving Attack clears the trail"
        );
    }

    #[test]
    fn view_run_pose_without_rigs_fails_loud_never_panics() {
        // Failure paths: a seen slot with no rig fails (never a panic), an
        // unseen bout poses nothing, and zero ticks folds no frame.
        let look = balance_look_two();
        assert!(
            look.pose_frame(2, 0.0).is_err(),
            "seen slot with no rig fails"
        );
        assert!(
            look.pose_frame_joints(2, 0.0).is_err(),
            "joints mode fails too"
        );
        let empty = FighterLook {
            rigs: Vec::new(),
            ring: None,
            slots: vec![SlotAnim::new(); MAX_LOOK_FIGHTERS],
            bridge: bridge_slots(),
            trails: TrailRing::new(),
            tick: 0,
            swing_ticks: 1,
            bundle_shas: Vec::new(),
            truth: Vec::new(),
            shake_trauma_permille: 0,
            uploaded_digest: empty_upload_digest(),
        };
        assert_eq!(
            empty
                .pose_frame(8, 0.0)
                .expect("unseen poses nothing")
                .len(),
            0
        );
        assert_eq!(
            empty
                .pose_frame_joints(8, 0.0)
                .expect("unseen joints pose nothing")
                .len(),
            0
        );
        assert_eq!(
            FighterLook::view_run(2040, 0, 8).expect("zero ticks"),
            "",
            "no frame folded, no line"
        );
    }

    #[test]
    fn skinned_upload_keeps_48b_stride_with_finite_tangents() {
        // 45f7596e streak: `refresh_mesh_vertices` packs 32 B per vertex
        // (pos + normal + uv) into a 48 B buffer (pos + normal + uv +
        // tangent), so every vertex past the first lands misaligned and each
        // fighter triangle stretches across the sand. The frame path must
        // re-upload the full mesh so `vertex_bytes` stays 48 B per vertex
        // with finite tangents. [found2 | 7.6.2 Rendering with Normal Maps]
        let mut look = FighterLook::load(&content_dir()).expect("CT7 bodies load");
        let views = [view(0, -1.0, 0.0), view(1, 1.0, 0.0)];
        look.observe_tick(0, &views, &[1000, 1000], &[]);
        let posed = look.pose_frame(2, 0.5).expect("CPU pose");
        assert_eq!(posed.len(), 2, "both seen slots pose");
        for body in &posed {
            let slot = usize::from(body.truth.slot);
            let rig = &look.rigs[look.rig_for_slot(slot)];
            assert!(body.lift.is_finite(), "lift stays finite");
            for (part_index, frame) in body.frames.iter().enumerate() {
                let part = &rig.parts[part_index];
                assert_eq!(
                    frame.positions.len(),
                    part.mesh.positions.len(),
                    "skin keeps vertex count"
                );
                let lifted: Vec<[f32; 3]> = if body.lift > 0.0 {
                    frame
                        .positions
                        .iter()
                        .map(|p| [p[0], p[1] + body.lift, p[2]])
                        .collect()
                } else {
                    frame.positions.clone()
                };
                assert!(
                    lifted.iter().flatten().all(|c| c.is_finite()),
                    "lifted positions stay finite"
                );
                let mesh =
                    v1::upload_mesh(&lifted, &frame.normals, &part.mesh.uvs, &part.mesh.indices)
                        .expect("skinned part re-uploads");
                assert_eq!(
                    mesh.tangents.len(),
                    lifted.len(),
                    "tangents ride every vertex"
                );
                assert!(
                    mesh.tangents.iter().flatten().all(|c| c.is_finite()),
                    "tangents stay finite"
                );
                assert_eq!(
                    mesh.vertex_bytes().len(),
                    lifted.len() * 48,
                    "48 B stride holds every vertex"
                );
                let refresh_len = lifted.len() * 32;
                assert_ne!(
                    refresh_len,
                    mesh.vertex_bytes().len(),
                    "32 B refresh would misalign past vertex 0"
                );
            }
        }
    }
}
