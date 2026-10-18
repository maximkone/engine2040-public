//! game: packaged arena app skeleton.
//!
//! Consumer: multiplayer, packaging and quality review. Data owner: this
//! crate owns the session loop (window/input bindings feeding the arena match,
//! HUD/audio/presentation observations) and the versioned progression payload
//! ([`progress`]); `engine-save` owns envelope/storage, `engine-arena` owns
//! match authority. Units: meters, radians, fixed ticks at the arena rate;
//! health in permille (`0..=1000`). Determinism scope: same [`GameConfig`]
//! plus same human inputs gives identical checkpoints, HUD, audio data and
//! progression; no wall clock, no device calls and no RNG in the default leg.
//! Errors: [`GameError`] at every boundary; invalid input is rejected loudly
//! (dropped tick or `Err`), never skipped silently and never panics.
//!
//! Headless-first: the default build is std-only. All presentation (the
//! render window, device audio) sits behind the `app` feature in
//! [`windowed`]; shipped play uses `app`, and both legs simulate the same
//! kinematic arena match so headless checkpoints replay exactly under the
//! window. The imported skinned fighter is deferred to the follow-up art
//! hook ([`PoseSource::ImportedFighter`]): presentation poses come from the
//! arena movement path ([`App::presentation`]); authored content is the visual gate.

/// Sim state to clip cues with a per-slot crossfade: the act pick plus the
/// fade driver the fighter look samples. Std-only, presentation only.
pub mod anim_bridge;
/// Arena glTF kit: placement parse, AABB fit and presenter staging.
/// Std-only parse plus fit on the default leg; file import plus upload join
/// the `app` leg so headless builds pull no `gltf`.
pub mod arena_kit;
/// Fight-driven music/tension bridge: maps each tick's fight cues onto
/// the ringside director and serves the mix layers on every outcome.
pub mod audio_director;
/// Reverb-send plus occlusion-gain bridge: game-thread mix over
/// the frozen audio brains, tunnel preset on, voice floor at -12 dB.
pub mod audio_mix;
pub mod balance_verdict;
/// Balance harness: falsifiable spacing experiments over
/// the arena fork path for balance tuning. Std-only.
pub mod bench;
/// Break look: authority break events painted as debris through the float physical layer,
/// the intact piece hidden on the break tick. Presentation only, never hashed. App leg only.
#[cfg(feature = "app")]
pub mod break_look;
/// Break window: the center crate in the playable window, whole until a sword hit breaks it, then
/// debris that falls and settles (RZ4w). Presentation only, never hashed. App leg only.
#[cfg(feature = "app")]
pub mod break_window;
/// Arena camera director: lock-on, tick-decay trauma shake,
/// pure framing fn, presentation only. Std-only.
pub mod camera;
/// Camera boom collision: pillar-aware arm length (Godot SpringArm3D shape),
/// pure presentation math, never hashed. Std-only.
pub mod camera_arm;
/// Presentation feed: drives the camera director from the frame
/// loop with in-sim trauma and presentation-side shake. Std-only.
pub mod camera_feed;
/// WASD and the stick relative to the camera: turns the bindings' local wish
/// into the world wish the sim takes. Input mapping only, never hashed. Std-only.
pub mod camera_wish;
/// Showcase clip capture: tape-bound PNG sequence plus manifest.
/// Headless-safe: flags, tape codec, std-only PNG writer and frame sink.
pub mod capture;
/// Crash reports to a local file: version, adapter, tape tail,
/// checkpoint, loud exit. Std-only.
pub mod crash;
/// Reacting crowd sim: 20k-seat integer state feeding the instanced
/// stands through the render staging. Std-only; consumer is [`App`].
pub mod crowd_gpu;
/// Dev-only hot-patch seam for one gameplay point (practice tuning).
/// Std-only; consumer is the window `arena.rs::poll_hot_reload`.
pub mod dev_hotpatch;
/// Auto spectator camera (trailer cuts from fight beats): shot choice over
/// fixed-tick cues with continuity rules, presentation only. Std-only.
pub mod director;
/// Feel flow view: hit-stop, slow-mo and camera shake from fight cues,
/// applied by the window. Presentation only. App leg only.
#[cfg(feature = "app")]
pub mod feel_view;
/// Fight ring: sand pit plus stone tiers under the fighter look. App leg.
#[cfg(feature = "app")]
pub mod fight_ring;
/// Staged fight: binds the content bundles to fighters and runs the
/// scripted bout headless (fighter_bind + staged_fight bin). Std-only.
pub mod fighter_bind;
/// Fighter look: CT7 bodies skinned and textured in the presenter, posed by
/// presentation anim state read after each tick. App leg only.
#[cfg(feature = "app")]
pub mod fighter_look;
/// Window match shell: Title, Match, Paused, Result over the arena match.
/// Std-only, pure transitions; pause holds the tick cursor.
pub mod flow;
/// First-person eye pose beside camera: look clamp plus interp eye.
pub mod fps_view;
/// Sim freeze mirror for the look: the hitstop count combat sets on impact,
/// read-only for clip-clock holds. Std-only.
pub mod hitstop;
/// Engine-owned window input codes (game-agnostic key/mouse maps). Std-only.
pub mod input;
/// Ladder, glory ranks, repair-craft hooks LIGHT slice: integer
/// fold, replayable, nothing CUT. Std-only.
pub mod ladder;
/// The human fighter's soft lock-on: the `facing` toward the nearest living
/// rival that the window puts on the live input (and the spear demo script
/// aims with). Input mapping only, never hashed. Std-only.
pub mod lock_on;
/// Loop rows text loader: designers edit numbers, no Rust edit.
/// Std-only, cold path at a progression boundary.
pub mod loop_text;
/// Living ludus voices — trainer, blacksmith and crowd: fight memories
/// with goals over the district yard, scripted fallback offline. Std-only.
pub mod ludus_npcs;
/// Display mode list plus validated swap keeping last-good: adapter modes
/// in, one staged swap out at a frame boundary. Std-only, presentation only.
pub mod modes;
/// Mode framework: arena, siege and district free-roam over the
/// ladder. Std-only.
pub mod modes_world;
/// One seat of a two-human match over UDP (LAN or relay), driven per frame by
/// the window or the headless proof loop. Std-only.
pub mod net_seat;
pub mod netgame;
/// Gamepad sticks plus buttons through the shared bindings table. Std-only.
pub mod pad;
/// Clip-clock hold: which fighter slots freeze while sim hitstop is above
/// zero. Std-only.
pub mod pose_hold;
/// Practice swap: staged combat tuning applied at a tick boundary. Std-only.
pub mod practice;
pub mod progress;
/// Hot-reload gate: staged generations applied at frame boundaries. Std-only.
pub mod reload;
/// Best-of-5 duel rounds over one-round arena sessions. Pure folds,
/// no hash input. Std-only.
pub mod rounds;
/// Siege mode: 32v32 over four arena shards on a dedicated server (WD4).
pub mod siege;
/// Spear grapple picture: the spear in flight, stuck or dropped, the verlet rope between the
/// two fighters and the carry the ragdoll layer follows. Presentation only, never hashed. App
/// leg only.
#[cfg(feature = "app")]
pub mod spear_look;
/// Spear grapple in the window: meshes for the spear, trail and rope, the camera follow and the
/// scene the arena binary drives. Presentation only, never hashed. App leg only.
#[cfg(feature = "app")]
pub mod spear_window;
/// CPU pose step of the fighter look: clip sample plus joint bounds, no GPU.
pub mod stage_bodies;
/// Steamworks backend: achievements, stats, cloud generations and presence.
/// Std-only until the orchestrator lands the `steam` feature.
pub mod steam;
/// View truth: every drawn body as numbers and a run's view laws, no image.
/// `app` only, beside the look that fills it.
#[cfg(feature = "app")]
pub mod view_truth;
/// Virtual time: a speed that eases toward a target and scales the wall
/// milliseconds the tick accumulator sees; locked at real time in a net
/// match. Presentation only, never hashed. App leg only.
#[cfg(feature = "app")]
pub mod virt;
/// Weapon trail: per-slot blade base-plus-tip ring, view only.
/// Holds the last 12 pairs per slot; idle slots stay empty.
/// Age ticks fade alpha from 1 at age 0 to 0 at age 11.
pub mod weapon_trail;
/// What the `arena` window tells the audio feed each tick: the listener at
/// the camera, placed fight and footstep plays, tension, the crowd bed.
/// Presentation only.
pub mod window_audio_space;
#[cfg(feature = "app")]
pub mod windowed;

use crate::audio_director::{drive, Director, DirectorError, DirectorState, Layers, TensionTable};
use crate::crowd_gpu::CrowdGpuSim;
use crate::director::{Director as CutDirector, DirectorConfig, FighterLateral, MIN_SHOT_TICKS};
use engine_anim::contract::{LocalPose, RootTable};
use engine_arena::recorder::Row;
use engine_arena::world::MatchWorld;
use engine_arena::{
    run_loco_poses, ArenaConfig, ArenaRules, Checkpoint, LiveMatch, LivePresentation, MatchSpec,
};
use engine_audio::{BankSound, Crowd, FightCue, Reaction};
use engine_combat::{fight::MAX_HP_PERMILLE, FightEvent};
use engine_destruction::bonded::BreakEvent;
use engine_input::{FighterId, FighterInput, FighterTape, MAX_PARTICIPANTS};
use engine_save::{migrate, open, save_generation, slot_name, SaveErr};
use engine_ui::{Hud, KillCause, KillEntry, CELLS_PER_METER};
pub use netgame::{
    decode_session_code, encode_session_code, first_divergence, DesyncReport, NetApplied, NetGame,
    REMOTE_VIEW_DELAY_TICKS, SESSION_CODE_PREFIX,
};
pub use progress::{GameProgress, ProgressError};

/// Human fighter slot: single player vs AI. The tape carries only this
/// slot's inputs; the arena brains fill every other slot.
pub const HUMAN_SLOT: u8 = 0;

/// Session configuration: one human vs `fighters - 1` AI brains.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GameConfig {
    /// Arena initialization seed; also stored as the save envelope seed.
    pub seed: u64,
    /// Fixed steps for the session, `1..=MAX_TICKS`.
    pub ticks: u32,
    /// Fighters, `2..=MAX_PARTICIPANTS`; slot 0 is human, the rest are AI.
    pub fighters: u8,
    /// Arena half-extent in meters, `5.0..=100.0`.
    pub half_extent_m: f32,
}

/// Bound replay work per session (mirrors the arena bound).
pub const MAX_TICKS: u32 = 100_000;
/// Default present cadence: every tick presents (today's behavior).
pub const DEFAULT_PRESENT_EVERY: u32 = 1;

/// Parse `--present-every N` (`1..=MAX_TICKS`).
pub fn parse_present_every(raw: &str) -> Result<u32, GameError> {
    let every: u32 = raw
        .parse()
        .map_err(|_| GameError::BadConfig("present_every"))?;
    if every == 0 || every > MAX_TICKS {
        return Err(GameError::BadConfig("present_every"));
    }
    Ok(every)
}

/// True when tick `tick` (0-based, `ticks` total) presents under cadence
/// `every`: every Nth tick plus the final tick; `every <= 1` is today's
/// behavior (every tick). Pure.
#[must_use]
pub fn should_present(tick: u32, ticks: u32, present_every: u32) -> bool {
    if present_every <= 1 {
        return true;
    }
    let one_based = tick.saturating_add(1);
    one_based == ticks || one_based.is_multiple_of(present_every)
}

impl GameConfig {
    /// Range-check every field; NaN or out-of-range fails with the culprit row.
    pub fn validate(self) -> Result<(), GameError> {
        if self.ticks == 0 || self.ticks > MAX_TICKS {
            return Err(GameError::BadConfig("ticks"));
        }
        if self.fighters < 2 || self.fighters > MAX_PARTICIPANTS {
            return Err(GameError::BadConfig("fighters"));
        }
        if !self.half_extent_m.is_finite() || self.half_extent_m < 5.0 || self.half_extent_m > 100.0
        {
            return Err(GameError::BadConfig("half_extent_m"));
        }
        Ok(())
    }

    /// Lower into the arena match config (same seed, ticks, fighters, extent).
    /// Table version 1: every session starts on the frozen rules; the live
    /// duel opts into v2 with [`App::into_duel`] (GP32).
    fn arena_config(self) -> ArenaConfig {
        ArenaConfig {
            seed: self.seed,
            ticks: self.ticks,
            fighters: self.fighters,
            half_extent_m: self.half_extent_m,
            table_version: engine_combat::TABLES_VERSION_V1,
        }
    }
}

// Layered config: Figment shaped per row fallback, std only.
// Donor: SergioBenitez/Figment, Apache-2.0 (src/figment.rs:569 extract_inner,
// :291 merge); shape only port reimplemented here, no bytes vendored.
// https://github.com/SergioBenitez/Figment/blob/master/src/figment.rs
// Precedence is env rows over file rows over defaults; one bad row falls
// back to the next layer instead of failing the whole extract.
impl Default for GameConfig {
    // Canonical defaults: seed 2040, 600 ticks, 2 fighters, 20 m extent.
    fn default() -> Self {
        Self {
            seed: 2040,
            ticks: 600,
            fighters: 2,
            half_extent_m: 20.0,
        }
    }
}

/// Look up one named row in a layer; `None` when absent. Figment
/// `extract_inner` shape: single key read used per row by the merge. Pure.
#[must_use]
pub fn extract_inner(rows: &[(&str, &str)], key: &str) -> Option<String> {
    rows.iter()
        .find(|row| row.0 == key)
        .map(|row| row.1.to_string())
}

// Parse one seed row; any u64 goes. Pure.
fn parse_layer_seed(raw: &str) -> Option<u64> {
    raw.parse().ok()
}

// Parse one ticks row inside 1..=MAX_TICKS; else None. Pure.
fn parse_layer_ticks(raw: &str) -> Option<u32> {
    raw.parse()
        .ok()
        .filter(|value| (1..=MAX_TICKS).contains(value))
}

// Parse one fighters row inside 2..=MAX_PARTICIPANTS; else None. Pure.
fn parse_layer_fighters(raw: &str) -> Option<u8> {
    raw.parse()
        .ok()
        .filter(|value| (2..=MAX_PARTICIPANTS).contains(value))
}

// Parse one extent row inside 5.0..=100.0 finite meters; else None. Pure.
fn parse_layer_extent(raw: &str) -> Option<f32> {
    raw.parse()
        .ok()
        .filter(|value: &f32| value.is_finite() && (5.0..=100.0).contains(value))
}

// Resolve one layered row: env wins, then file, then the default.
// Returns the winning value plus true when a present row refused to parse.
// Pure.
fn resolve_layer_row<T: Copy>(
    env_raw: Option<&str>,
    file_raw: Option<&str>,
    parse: fn(&str) -> Option<T>,
    default: T,
) -> (T, bool) {
    if let Some(raw) = env_raw {
        if let Some(value) = parse(raw) {
            return (value, false);
        }
        if let Some(raw) = file_raw {
            if let Some(value) = parse(raw) {
                return (value, true);
            }
        }
        return (default, true);
    }
    if let Some(raw) = file_raw {
        if let Some(value) = parse(raw) {
            return (value, false);
        }
        return (default, true);
    }
    (default, false)
}

// True when a layer holds a key outside the four known rows. Pure.
fn has_unknown_layer_key(rows: &[(&str, &str)]) -> bool {
    rows.iter().any(|row| {
        row.0 != "seed" && row.0 != "ticks" && row.0 != "fighters" && row.0 != "half_extent_m"
    })
}

/// Merge file plus env rows over defaults and collect every refused row name.
/// Precedence per row is env over file over defaults with fall-through; a
/// present but unparsable row falls back one layer and its name is collected.
/// Units: seed is an opaque `u64`, ticks and fighters are counts, extent is
/// meters. Determinism: same rows give the same merge and the same report, no
/// IO and no clock. The report names rows in refusal order: `seed`, `ticks`,
/// `fighters`, `half_extent_m`, then `unknown` once when either layer holds
/// an outside key. Caller: [`layered_extract`] (production wrapper below).
#[must_use]
pub fn layered_extract_report(
    defaults: GameConfig,
    file_rows: &[(&str, &str)],
    env_rows: &[(&str, &str)],
) -> (GameConfig, Vec<&'static str>) {
    let mut refused: Vec<&'static str> = Vec::new();
    let (seed, seed_refused) = resolve_layer_row(
        extract_inner(env_rows, "seed").as_deref(),
        extract_inner(file_rows, "seed").as_deref(),
        parse_layer_seed,
        defaults.seed,
    );
    if seed_refused {
        refused.push("seed");
    }
    let (ticks, ticks_refused) = resolve_layer_row(
        extract_inner(env_rows, "ticks").as_deref(),
        extract_inner(file_rows, "ticks").as_deref(),
        parse_layer_ticks,
        defaults.ticks,
    );
    if ticks_refused {
        refused.push("ticks");
    }
    let (fighters, fighters_refused) = resolve_layer_row(
        extract_inner(env_rows, "fighters").as_deref(),
        extract_inner(file_rows, "fighters").as_deref(),
        parse_layer_fighters,
        defaults.fighters,
    );
    if fighters_refused {
        refused.push("fighters");
    }
    let (half_extent_m, extent_refused) = resolve_layer_row(
        extract_inner(env_rows, "half_extent_m").as_deref(),
        extract_inner(file_rows, "half_extent_m").as_deref(),
        parse_layer_extent,
        defaults.half_extent_m,
    );
    if extent_refused {
        refused.push("half_extent_m");
    }
    if has_unknown_layer_key(file_rows) || has_unknown_layer_key(env_rows) {
        refused.push("unknown");
    }
    let merged = GameConfig {
        seed,
        ticks,
        fighters,
        half_extent_m,
    };
    if merged.validate().is_ok() {
        (merged, refused)
    } else {
        (defaults, refused)
    }
}

/// Merge file plus env rows over defaults, one row at a time: env wins, then
/// file, then the default; a missing or unparsable row falls back to the next
/// layer. A fully unusable stack keeps the valid defaults. Pure, never fails.
#[must_use]
pub fn layered_extract(
    defaults: GameConfig,
    file_rows: &[(&str, &str)],
    env_rows: &[(&str, &str)],
) -> GameConfig {
    layered_extract_report(defaults, file_rows, env_rows).0
}

/// Game failures: bounded diagnostics, never panics.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GameError {
    /// Config row outside its range, row name carried.
    BadConfig(&'static str),
    /// Save slot `>= SLOTS`, culprit carried.
    BadSlot(u8),
    /// Filesystem failure, kind carried (paths stay in the caller).
    Io(std::io::ErrorKind),
    /// Envelope/storage failure from `engine-save`.
    Save(SaveErr),
    /// Progression payload failure (owned here, see [`progress`]).
    Progress(ProgressError),
    /// Imported skinned fighter pose requested before authored content lands (the
    /// follow-up art hook; see [`PoseSource::ImportedFighter`]).
    ArtDeferred,
    /// Remote (LAN peer) input rejected: row names are `slot` (self slot,
    /// unknown slot or duplicate for this tick), `range` (out-of-range
    /// fields) and `over` (session already ended).
    BadRemote(&'static str),
    /// LAN transport/session failure from `engine-net` (version mismatch,
    /// bad wire bytes, late input); nothing was applied.
    Net(engine_net::session::NetError),
    /// The host and the guest would play different matches (rules, spear,
    /// world or config): both match identities carried, nothing was paired.
    SpecMismatch {
        /// The host's match identity.
        host: u64,
        /// The guest's match identity.
        guest: u64,
    },
}

impl core::fmt::Display for GameError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadConfig(row) => write!(f, "bad game config ({row})"),
            Self::BadSlot(slot) => write!(f, "bad save slot ({slot})"),
            Self::Io(kind) => write!(f, "io error ({kind:?})"),
            Self::Save(err) => write!(f, "save error ({err:?})"),
            Self::Progress(err) => write!(f, "{err}"),
            Self::ArtDeferred => write!(
                f,
                "imported fighter art deferred to the GM1 follow-up (O7 content)"
            ),
            Self::BadRemote(row) => write!(f, "bad remote input ({row})"),
            Self::Net(err) => write!(f, "net error ({err:?})"),
            Self::SpecMismatch { host, guest } => write!(
                f,
                "the host plays another match (host identity {host:016x}, this code {guest:016x}): \
                 use the host's fresh code on the same build"
            ),
        }
    }
}

impl std::error::Error for GameError {}

impl From<ProgressError> for GameError {
    fn from(err: ProgressError) -> Self {
        Self::Progress(err)
    }
}

/// One audio voice request as data: no device calls in the default leg. The
/// windowed leg plays these through the headless mixer; device playback is a
/// presentation concern there, never simulation state.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AudioCommand {
    /// Fixed tick the command was emitted on.
    pub tick: u32,
    /// Which bank voice to play.
    pub sound: BankSound,
    /// Per-voice gain, `0.0..=1.0`.
    pub gain: f32,
}

/// Outcome of one fixed tick: checkpoint plus HUD/audio/presentation data.
/// All fields are plain data; nothing here touches a device.
#[derive(Clone, PartialEq, Debug)]
pub struct TickOutcome {
    /// Fixed tick just simulated.
    pub tick: u32,
    /// False when the input was rejected (wrong slot, out of range, or the
    /// match already ended): the cursor did not advance and the remaining
    /// fields repeat the last accepted tick (or defaults before tick 0).
    pub input_accepted: bool,
    /// Match checkpoint for [`TickOutcome::tick`] (`None` only when the very
    /// first input is rejected before any tick ran).
    pub checkpoint: Option<Checkpoint>,
    /// HUD snapshot after the tick (kill feed, human hp, ladder score).
    pub hud: Hud,
    /// Audio voice requests for the tick, in combat emission order.
    pub audio: Vec<AudioCommand>,
    /// Fight cues mapped 1:1 from the tick's combat events.
    pub cues: Vec<FightCue>,
    /// Slots that began an attack on this tick (TABLES_V2 press tell, EG19b);
    /// empty under v1 rows, on quiet ticks and past the press tick.
    /// Presentation only: the swing clip starts from these (EG19c), so no
    /// hp, audio, crowd or hash contact. Fighter slots in press order.
    pub started: Vec<u8>,
    /// Destruction break events of the tick as the authority emitted them (bond snaps, then
    /// freed chunks); empty on quiet ticks and on a rejected tick. The window's break picture
    /// (`break_window`) reads them after the tick ran. Presentation only: the events are
    /// already hashed by the arena, nothing here is read back.
    pub breaks: Vec<BreakEvent>,
    /// Crowd reaction per cue, in the same order as [`TickOutcome::cues`].
    pub reactions: Vec<Reaction>,
    /// Crowd-bed gain multiplier `0.0..=1.0` after the tick.
    pub bed_gain: f32,
    /// Tension-director mix answer for the tick (intensity, music scalar,
    /// crowd beds); the window mix plays these, headless tests assert them.
    pub layers: Layers,
}

impl Default for TickOutcome {
    /// Empty outcome shaped like a rejection before tick 0: no checkpoint
    /// yet, session-start HUD, full bed gain and the quiet-room mix. Hot
    /// loops hold one of these and refill it through
    /// [`App::observe_into`], which reuses its buffers instead of
    /// allocating per tick.
    fn default() -> Self {
        Self {
            tick: 0,
            input_accepted: false,
            checkpoint: None,
            hud: Hud::new(
                0,
                u32::from(MAX_HP_PERMILLE),
                u32::from(MAX_HP_PERMILLE),
                0,
                0,
            ),
            audio: Vec::new(),
            cues: Vec::new(),
            started: Vec::new(),
            breaks: Vec::new(),
            reactions: Vec::new(),
            bed_gain: 1.0,
            layers: Layers {
                intensity: 0.0,
                music: 0.0,
                murmur: 1.0,
                cheer: 0.0,
                gasp: 0.0,
                boo: 0.0,
            },
        }
    }
}

/// Presentation tracks for the simulated prefix: the arena movement path.
/// Read-only trajectory data for the renderer/replay; never hashed.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct PresentationData {
    /// One locomotion pose vector per fighter, one pose per simulated tick
    /// (see [`run_loco_poses`](engine_arena::run_loco_poses)).
    pub pose_tracks: Vec<Vec<LocalPose>>,
    /// One root-motion table per fighter over the same prefix.
    pub root_tables: Vec<RootTable>,
}

/// Presentation pose source: the arena movement path now, the imported
/// skinned fighter after authored content lands.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum PoseSource {
    /// Poses from [`App::presentation`] (locomotion state machine tracks).
    LocoPresentation,
    /// Imported skinned fighter clip pose (follow-up art hook; authored
    /// content is the visual gate). Always [`GameError::ArtDeferred`] today.
    ImportedFighter,
}

/// Loaded progression: the payload plus the envelope coordinates it was
/// sealed under, so callers can check the match seed survived the roundtrip.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct LoadedProgress {
    /// Decoded progression payload.
    pub progress: GameProgress,
    /// Envelope seed (the arena seed at save time).
    pub envelope_seed: u64,
    /// Envelope tick (the fixed-tick cursor at save time).
    pub envelope_tick: u64,
}

/// Packaged arena session: window/input bindings feed the arena match in the
/// windowed leg; the headless leg drives the same match from recorded
/// [`FighterInput`]s. One human (slot 0) vs AI-filled slots; LAN play
/// ([`NetGame`]) seats a second human via [`App::offer_remote`] and
/// [`App::new_for_slot`].
pub struct App {
    config: GameConfig,
    arena: ArenaConfig,
    /// Local human slot: [`HUMAN_SLOT`] for single player, 0 (host) or 1
    /// (guest) for LAN. Every other human slot stages through
    /// [`App::offer_remote`]; the rest are AI-filled by the arena.
    human_slot: u8,
    tape: FighterTape,
    cursor: u32,
    /// Incremental authority (EG1): one arena sim step per accepted tick,
    /// byte-equal to the old whole-prefix replays. Rebuilt from the tape
    /// only on the rare stale path (a failed step poisons it).
    live: LiveMatch,
    /// Fight rules every live sim of this session runs (EG3): the ones the
    /// arena config selects ([`ArenaConfig::rules`]). v1 until
    /// [`App::into_duel`]. [`App::spec`] names this match for the net server
    /// and client, so LAN play can build the same one.
    rules: ArenaRules,
    /// Spear grapple on (`docs/plan/spear-grapple.md`): set by [`App::into_spear`] before the
    /// first tick. Every live and presentation sim of the session is built with it.
    spear: bool,
    /// Sim steps spent by retired live matches (rebuilds), so
    /// [`App::sim_steps`] counts every step the session ever paid.
    retired_steps: u64,
    checkpoints: Vec<Checkpoint>,
    hp: [u16; MAX_PARTICIPANTS as usize],
    /// Sim hitstop ticks left, mirrored from the live tick events (Hit or
    /// Parried sets 4, anything else counts down). Presentation only, never
    /// hashed; the look holds clip clocks while it is above zero.
    hitstop_remaining: u32,
    /// Best-of-5 fold over closed rounds; pure, never hashed.
    rounds: crate::rounds::MatchState,
    hud: Hud,
    crowd: Crowd,
    /// Reacting crowd visual state: 20k-seat integer sim driven once per
    /// accepted tick beside the audio clock. Presentation only, never hashed.
    crowd_gpu: CrowdGpuSim,
    /// Tension director: driven once per accepted tick from the tick's
    /// fight cues (presentation only, never hashed). The window plays
    /// `director_layers` through the crowd/music buses.
    director: Director,
    /// Auto spectator camera: driven once per accepted tick from the tick's
    /// cues, lateral geometry and health (presentation only, never hashed).
    /// The headless capture files its digest beside the frames.
    cut_director: CutDirector,
    /// Latest tick's fighter laterals from the authoritative dataset, fed to
    /// the spectator camera above; reused across direct `observe_into` calls.
    laterals: Vec<FighterLateral>,
    /// Latest tick's fighter views (slot, feet position, yaw) from the same
    /// authoritative dataset rows as [`App::laterals`]: VS4 serves them to
    /// the fighter-draw leg through [`App::fighter_views`]. Presentation
    /// only, never hashed.
    views: Vec<FighterView>,
    /// Scratch cue buffer reused by [`App::simulate_tick`]: cleared and
    /// extended every accepted tick, so the tick path allocates nothing at
    /// steady state. Never read across ticks.
    tick_cues: Vec<FightCue>,
    /// Scratch starter-slot buffer, same reuse discipline as
    /// [`App::tick_cues`].
    tick_started: Vec<u8>,
    /// The tick's authority break events, filled by [`App::simulate_tick`] and moved into
    /// [`TickOutcome::breaks`] by [`App::observe_into`]; empty between ticks.
    tick_breaks: Vec<BreakEvent>,
    /// Last mix answer, served on every outcome (rejected ticks repeat it).
    director_layers: Layers,
    kills: Vec<KillEntry>,
    matches_completed: u32,
    wins: u32,
    losses: u32,
    decided: bool,
    /// Rolling presentation cursor: cached movement-path tracks over
    /// `0..present_ticks`, extended by one tick on every accepted tick.
    /// Frames read [`App::pose_cursor`] with no re-simulation; the data is
    /// presentation only, never hashed.
    present: PresentationData,
    /// Ticks covered by [`App::present`]: equals the cursor after every
    /// accepted tick, `0` before tick 0.
    present_ticks: u32,
    /// Latest locomotion pose per fighter in slot order: the tail of
    /// [`App::present`], served by [`App::pose_cursor`] with no work.
    /// Empty before tick 0.
    present_latest: Vec<LocalPose>,
    /// When false, accepted ticks skip the rolling presentation cursor
    /// (one presentation sim step per tick); [`App::presentation`] then
    /// serves the full legs on demand. Presentation only, never hashed.
    present_live: bool,
    /// Incremental presentation (EG1b): one recording sim step per accepted
    /// tick appends to [`App::present`]. `None` until the first extension
    /// and while the cursor is dropped; rebuilt from the tape when stale.
    present_sim: Option<LivePresentation>,
    /// Presentation sim steps spent by retired presentation sims.
    present_retired_steps: u64,
    /// Static obstacle world this session's match and pose cursor run over
    /// (default [`MatchWorld::empty`], the legacy open arena). The live
    /// window loads the arena kit's physical placement here, so kit walls
    /// stop being pictures; every other caller keeps the empty default and
    /// its byte-frozen checkpoints. Units: meters. Determinism: the same
    /// world gives the same checkpoints (the digest rides every one).
    world: MatchWorld,
}

impl App {
    /// Start a session: validates the config, seats every fighter at full hp
    /// and seeds an empty human tape. AI fills every non-human slot on any
    /// tick without a tape input for it.
    pub fn new(config: GameConfig) -> Result<Self, GameError> {
        Self::new_for_slot(config, HUMAN_SLOT)
    }

    /// [`App::new`] over a static obstacle world: the arena kit's physical
    /// placement ([`crate::arena_kit::kit_match_world`]) so kit walls stop
    /// being pictures. [`MatchWorld::empty`] is the legacy open arena, so
    /// every other caller stays byte-frozen.
    ///
    /// Caller: `crates/game/src/windowed.rs` (`run_windowed`) and
    /// `crates/game/src/bin/arena.rs` build the live window here.
    pub fn with_world(config: GameConfig, world: MatchWorld) -> Result<Self, GameError> {
        Self::new_for_slot_world(config, HUMAN_SLOT, world)
    }

    /// Move a fresh session to the live duel's combat: tables v2 under rules
    /// v2 (GP32: 15-tick startups, input buffer, commit lock). Only before
    /// the first tick; later calls refuse with `BadConfig("duel_tables")`.
    /// The proof scene and LAN play never call this and stay on v1.
    pub fn into_duel(mut self) -> Result<Self, GameError> {
        if self.cursor != 0 || self.retired_steps != 0 || self.live.steps() != 0 {
            return Err(GameError::BadConfig("duel_tables"));
        }
        let mut spec = MatchSpec::duel(self.arena, self.world.clone());
        spec.spear = self.spear;
        self.live = spec.live().ok_or(GameError::BadConfig("arena"))?;
        self.arena = spec.config;
        self.rules = spec.rules;
        Ok(self)
    }

    /// Turn the spear grapple on: the throw button throws a spear and tethers fighters (see
    /// `docs/plan/spear-grapple.md`). Only before the first tick; later calls refuse with
    /// `BadConfig("spear")`. The match identity changes, so a LAN peer must ask for it too.
    pub fn into_spear(mut self) -> Result<Self, GameError> {
        if self.cursor != 0 || self.retired_steps != 0 || self.live.steps() != 0 {
            return Err(GameError::BadConfig("spear"));
        }
        self.spear = true;
        self.live = self.spec().live().ok_or(GameError::BadConfig("arena"))?;
        Ok(self)
    }

    /// True when the spear grapple is on for this session.
    #[must_use]
    pub const fn spear(&self) -> bool {
        self.spear
    }

    /// The match this session plays: config, rules and world as one value.
    /// The net server and client build from the same spec
    /// (`Server::from_spec`, `Client::from_spec`), so a peer given this
    /// value plays the match the window shows.
    #[must_use]
    pub fn spec(&self) -> MatchSpec {
        let spec = MatchSpec::new(self.arena, self.rules, self.world.clone());
        if self.spear {
            spec.with_spear()
        } else {
            spec
        }
    }

    /// Start a session with an explicit local human slot (LAN guests play
    /// slot 1). The slot must name a fighter below `config.fighters`;
    /// anything else fails. Single-player callers use [`App::new`], which
    /// pins slot 0 and is behavior-identical to the single-player session.
    pub fn new_for_slot(config: GameConfig, human_slot: u8) -> Result<Self, GameError> {
        Self::new_for_slot_world(config, human_slot, MatchWorld::empty())
    }

    /// [`App::new_for_slot`] over an explicit obstacle world (see
    /// [`App::with_world`]).
    fn new_for_slot_world(
        config: GameConfig,
        human_slot: u8,
        world: MatchWorld,
    ) -> Result<Self, GameError> {
        config.validate()?;
        if human_slot >= config.fighters {
            return Err(GameError::BadConfig("human_slot"));
        }
        // Short sessions hold every shot longer than they run: clamp the
        // floor to the session so the director stays constructible.
        let min_shot = if config.ticks < MIN_SHOT_TICKS {
            config.ticks
        } else {
            MIN_SHOT_TICKS
        };
        let cut_director = CutDirector::new(DirectorConfig {
            seed: config.seed,
            focus: human_slot,
            min_shot_ticks: min_shot,
            max_ticks: config.ticks,
        })
        .map_err(|_| GameError::BadConfig("director"))?;
        let arena = config.arena_config();
        let rules = arena.rules();
        let live = LiveMatch::with_rules_world(&arena, rules, world.clone())
            .ok_or(GameError::BadConfig("arena"))?;
        Ok(Self {
            config,
            arena,
            human_slot,
            tape: FighterTape::default(),
            cursor: 0,
            live,
            rules,
            spear: false,
            retired_steps: 0,
            checkpoints: Vec::new(),
            hp: [MAX_HP_PERMILLE; MAX_PARTICIPANTS as usize],
            hitstop_remaining: 0,
            rounds: crate::rounds::MatchState::new(),
            hud: Hud::new(
                0,
                u32::from(MAX_HP_PERMILLE),
                u32::from(MAX_HP_PERMILLE),
                0,
                0,
            ),
            crowd: Crowd::new(),
            crowd_gpu: CrowdGpuSim::rich(config.seed),
            director: Director::scripted(),
            cut_director,
            laterals: Vec::with_capacity(usize::from(config.fighters)),
            views: Vec::with_capacity(usize::from(config.fighters)),
            tick_cues: Vec::new(),
            tick_started: Vec::new(),
            tick_breaks: Vec::new(),
            director_layers: Layers {
                intensity: 0.0,
                music: 0.0,
                murmur: 1.0,
                cheer: 0.0,
                gasp: 0.0,
                boo: 0.0,
            },
            kills: Vec::new(),
            matches_completed: 0,
            wins: 0,
            losses: 0,
            decided: false,
            present: PresentationData::default(),
            present_ticks: 0,
            present_latest: Vec::new(),
            present_live: true,
            present_sim: None,
            present_retired_steps: 0,
            world,
        })
    }

    /// Session configuration.
    #[must_use]
    pub fn config(&self) -> GameConfig {
        self.config
    }

    /// Local human slot ([`HUMAN_SLOT`] in single player).
    #[must_use]
    pub fn human_slot(&self) -> u8 {
        self.human_slot
    }

    /// Fixed-tick cursor: ticks simulated so far.
    #[must_use]
    pub fn cursor(&self) -> u32 {
        self.cursor
    }

    /// Checkpoints for every simulated tick, in tick order.
    #[must_use]
    pub fn checkpoints(&self) -> &[Checkpoint] {
        &self.checkpoints
    }

    /// Arena sim steps the session has paid so far (EG1 cost proof): exactly
    /// one per accepted tick on the normal path, so a 600-tick match costs
    /// 600 steps (the old prefix replays cost about `3 * n` per tick).
    #[must_use]
    pub fn sim_steps(&self) -> u64 {
        self.retired_steps.saturating_add(self.live.steps())
    }

    /// Director updates served so far: exactly one per accepted tick
    /// (rejected ticks never drive it, mirroring the crowd clock).
    #[must_use]
    pub fn director_updates(&self) -> u64 {
        self.director.tick_count()
    }

    /// Reacting crowd visual state: integer excitement plus the room clock,
    /// driven once per accepted tick. The window stages it through
    /// [`CrowdGpuSim::feed_frame`]; presentation only, never hashed.
    #[must_use]
    pub fn crowd_gpu(&self) -> &CrowdGpuSim {
        &self.crowd_gpu
    }

    /// Copy the reacting room's 64 block levels into a caller buffer for the
    /// offline evidence: returns the entries written. The buffer must hold
    /// 64. Presentation only, never hashed; same script gives the same levels
    /// twice. Caller: `crates/game/src/bin/staged_fight.rs` (staged run
    /// exports after the fight, failing loudly on a short buffer).
    pub fn crowd_levels(&self, out: &mut [u16]) -> Result<usize, crate::crowd_gpu::CrowdGpuError> {
        self.crowd_gpu.export_levels(out)
    }

    /// Last director mix answer (crowd beds plus music scalar).
    #[must_use]
    pub fn director_layers(&self) -> Layers {
        self.director_layers
    }

    /// Room arc state for the mix (`Murmur`/`Rising`/`Climax`/`Release`).
    #[must_use]
    pub fn director_state(&self) -> DirectorState {
        self.director.state()
    }

    /// Active excitation table (copy; the director owns its state).
    #[must_use]
    pub fn director_table(&self) -> TensionTable {
        self.director.table()
    }

    /// Retune the excitation table at a tick boundary (tuning hook); a bad
    /// table fails with the culprit row and keeps the old one.
    pub fn set_director_table(&mut self, table: TensionTable) -> Result<(), DirectorError> {
        self.director.set_table(table)
    }

    /// Spectator-camera cut log in tick order (presentation only, never
    /// hashed; the headless capture files its digest beside the frames).
    #[must_use]
    pub fn director_cuts(&self) -> &[crate::director::Cut] {
        self.cut_director.cuts()
    }

    /// Spectator-camera digest over the cut log plus the held shot: identical
    /// sessions give identical digests.
    #[must_use]
    pub fn director_digest(&self) -> u64 {
        self.cut_director.digest()
    }

    /// Spectator-camera shot served this tick (the opener before tick 0).
    #[must_use]
    pub fn director_shot(&self) -> crate::director::Shot {
        self.cut_director.shot()
    }

    /// Continuity audit over the spectator-camera cut log so far (jump cuts,
    /// line crosses, short holds, slew overs, stale beats), judged against the
    /// director's own hold floor.
    ///
    /// Caller: `crates/game/src/bin/arena_capture.rs` (`cut_log_audit_line`
    /// turns this report into the seal's receipt line). Presentation only,
    /// never hashed. Budget: one pass over the cut log, on the seal path.
    #[must_use]
    pub fn director_audit(&self) -> crate::director::AuditReport {
        self.cut_director.audit()
    }

    /// Live HUD snapshot (kill feed, human hp, ladder score).
    #[must_use]
    pub fn hud(&self) -> &Hud {
        &self.hud
    }

    /// Exact hp per fighter slot in permille, tracked 1:1 from combat events
    /// (combat never heals: damage saturates down, knockdown pins to zero).
    #[must_use]
    pub fn hp(&self) -> &[u16; MAX_PARTICIPANTS as usize] {
        &self.hp
    }

    /// Sim hitstop ticks left, mirrored from the live tick events. Equals 4
    /// right after a Hit or Parried tick, then drops by one per quiet tick.
    /// Read-only visibility for the look (it holds clip clocks while this is
    /// above zero); presentation only, never hashed. Units: fixed ticks.
    ///
    /// Caller: `crates/game/src/fighter_look.rs` (`FighterLook::view_run`
    /// polls the mirrored count each tick; the window twin polls the same).
    #[must_use]
    pub fn hitstop_remaining(&self) -> u32 {
        self.hitstop_remaining
    }

    /// True while hitstop holds clip clocks (`remaining > 0`). Pure over the
    /// count; the look calls it with [`App::hitstop_remaining`].
    ///
    /// Caller: `crates/game/src/fighter_look.rs` (`FighterLook::view_run`
    /// polls the hold each tick; the window twin polls the same).
    #[must_use]
    pub fn hitstop_holds(remaining: u32) -> bool {
        hitstop::holds(remaining)
    }

    /// Current round verdict from live hp, without scoring. Pure.
    #[must_use]
    pub fn round_winner(&self) -> crate::rounds::RoundWinner {
        crate::rounds::decide_round(&self.hp, self.config.fighters)
    }

    /// Close the current round: fold its verdict into the best-of-5.
    /// Ties replay and score nothing. Pure aside from the fold.
    pub fn close_round(&mut self) -> crate::rounds::RoundWinner {
        let winner = self.round_winner();
        self.rounds.apply(winner);
        winner
    }

    /// Rounds played including tie replays. Pure.
    #[must_use]
    pub fn rounds_played(&self) -> u32 {
        self.rounds.rounds_played()
    }

    /// Match winner once a side takes [`crate::rounds::WINS_FOR_MATCH`]. Pure.
    #[must_use]
    pub fn match_winner(&self) -> Option<crate::rounds::RoundWinner> {
        self.rounds.winner()
    }

    /// Reset hp for the next round; the best-of-5 score is kept. Pure.
    pub fn reset_for_next_round(&mut self) {
        self.hp.fill(MAX_HP_PERMILLE);
    }

    /// Live progression counters (persisted by [`App::save_progress`]).
    #[must_use]
    pub fn progression(&self) -> GameProgress {
        GameProgress {
            matches_completed: self.matches_completed,
            wins: self.wins,
            losses: self.losses,
            match_seed: self.config.seed,
            match_ticks: self.cursor,
        }
    }

    /// Advance one fixed tick on a human input. The input's tick is stamped
    /// to the cursor (caller ticks are transport order, not authority); the
    /// fighter must be the local human slot and every field must validate, or
    /// the tick is rejected without advancing. Rejected ticks repeat the last
    /// accepted data so the consumer can keep rendering.
    pub fn tick(&mut self, input: FighterInput) -> TickOutcome {
        let mut out = TickOutcome::default();
        self.tick_into(input, &mut out);
        out
    }

    /// Advance one fixed tick, refilling a caller-held outcome: same bytes
    /// as [`App::tick`] with no per-tick heap growth once the outcome is
    /// sized. Production hot loops (bin/arena.rs) hold one scratch outcome.
    pub fn tick_into(&mut self, input: FighterInput, out: &mut TickOutcome) {
        let mut stamped = input;
        stamped.tick = self.cursor;
        if self.cursor >= self.config.ticks
            || stamped.fighter.get() != self.human_slot
            || stamped.validate().is_err()
        {
            self.rejected_into(out);
            return;
        }
        // Single local input per tick plus at most one staged remote input:
        // ticks grow monotonically, so the tape stays `(tick, fighter)`-sorted
        // by construction as long as every stage goes through `stage_input`.
        self.stage_input(stamped);
        let tick = self.cursor;
        let Some((checkpoint, cues, started)) = self.simulate_tick(tick) else {
            // Unreachable for validated configs/tapes (arena rejects a run
            // instead of skipping); stay put rather than diverge.
            self.unstage_input(stamped);
            self.rejected_into(out);
            return;
        };
        self.checkpoints.push(checkpoint);
        self.cursor += 1;
        self.observe_into(tick, &cues, &started, checkpoint, out);
        self.extend_presentation();
        self.restore_tick_scratch(cues, started);
    }

    /// Stage one validated input on the tape, keeping `(tick, fighter)` order
    /// (the arena rejects unsorted tapes, and a LAN tick stages two inputs).
    /// Callers validate first; this never fails.
    fn stage_input(&mut self, input: FighterInput) {
        let key = (input.tick, input.fighter.0);
        // Monotonic ticks append in O(1); only out-of-order LAN arrivals pay
        // the `partition_point` plus memmove.
        if tape_should_append(&self.tape, key) {
            self.tape.0.push(input);
            return;
        }
        let index = self
            .tape
            .0
            .partition_point(|staged| (staged.tick, staged.fighter.0) <= key);
        self.tape.0.insert(index, input);
    }

    /// Remove one staged input again (the unreachable rollback in
    /// [`App::tick`]); a positional remove is wrong once two inputs share a
    /// tick, so the exact staged value is located instead.
    fn unstage_input(&mut self, input: FighterInput) {
        if let Some(index) = self.tape.0.iter().position(|staged| *staged == input) {
            self.tape.0.remove(index);
        }
    }

    /// Stage a LAN peer's input for the cursor tick without advancing. The
    /// input's tick is stamped to the cursor like [`App::tick`]; it must name
    /// a non-local slot below `config.fighters`, pass validation and not
    /// duplicate an already-staged input for this tick, or it fails loudly
    /// and stages nothing. At most one remote slot is ever staged per tick:
    /// [`NetGame`](super::NetGame) pins it, so the match holds 2 humans max
    /// with AI filling the rest.
    pub fn offer_remote(&mut self, input: FighterInput) -> Result<(), GameError> {
        if self.cursor >= self.config.ticks {
            return Err(GameError::BadRemote("over"));
        }
        if input.fighter.get() == self.human_slot || input.fighter.get() >= self.config.fighters {
            return Err(GameError::BadRemote("slot"));
        }
        if input.validate().is_err() {
            return Err(GameError::BadRemote("range"));
        }
        let mut staged = input;
        staged.tick = self.cursor;
        if self
            .tape
            .0
            .iter()
            .any(|known| known.tick == staged.tick && known.fighter == staged.fighter)
        {
            return Err(GameError::BadRemote("duplicate"));
        }
        self.stage_input(staged);
        Ok(())
    }

    /// Advance one fixed tick with no human input: every slot is AI-filled.
    /// This is the resimulation parity for server-side late-input loss (the
    /// net server authors the tick AI-filled when a human input arrives
    /// late); the cursor still advances and the outcome reports
    /// `input_accepted: true` because the tick ran. Single-player sessions
    /// never call this: [`App::tick`] rejects instead of AI-filling the human.
    pub fn tick_empty(&mut self) -> TickOutcome {
        if self.cursor >= self.config.ticks {
            return self.rejected();
        }
        let tick = self.cursor;
        let Some((checkpoint, cues, started)) = self.simulate_tick(tick) else {
            return self.rejected();
        };
        self.checkpoints.push(checkpoint);
        self.cursor += 1;
        let out = self.observe(tick, &cues, &started, checkpoint);
        self.extend_presentation();
        self.restore_tick_scratch(cues, started);
        out
    }

    /// Outcome repeating the last accepted tick (rejected inputs never move
    /// the match, the crowd clock, hp or the feed).
    fn rejected(&self) -> TickOutcome {
        let mut out = TickOutcome::default();
        self.rejected_into(&mut out);
        out
    }

    /// Refill a caller-held outcome with the last accepted tick: same bytes
    /// as [`App::rejected`] with no per-tick heap growth.
    fn rejected_into(&self, out: &mut TickOutcome) {
        out.tick = self.cursor;
        out.input_accepted = false;
        out.checkpoint = self.checkpoints.last().copied();
        sync_hud(&mut out.hud, &self.hud);
        out.audio.clear();
        out.cues.clear();
        out.reactions.clear();
        out.started.clear();
        out.breaks.clear();
        out.bed_gain = self.crowd.bed_gain();
        out.layers = self.director_layers;
    }

    /// Simulate tick `tick` over the tape grown by this tick's input: the
    /// live match steps exactly once (EG1: constant cost per tick, byte-equal
    /// to the old `run_prefix`/`run_events`/`run_dataset` replays of
    /// `0..=tick`, which stay the test oracle). Refreshes the lateral cache
    /// from the authoritative dataset rows, so the spectator camera below
    /// reads real geometry; a row miss keeps the previous laterals. A stale
    /// live match (poisoned by a failed step, unreachable for validated
    /// tapes) rebuilds from tick 0 first: the only replay path.
    fn simulate_tick(&mut self, tick: u32) -> Option<(Checkpoint, Vec<FightCue>, Vec<u8>)> {
        if self.live.is_poisoned() || self.live.tick() != tick {
            self.rebuild_live(tick)?;
        }
        let fighters = self.config.fighters;
        let current = self.hitstop_remaining;
        let inputs = tick_inputs(&self.tape, tick);
        let stepped = self.live.step(inputs)?;
        let checkpoint = stepped.checkpoint;
        // Steady-state zero alloc: refill the scratch buffers in place
        // instead of collecting fresh vecs every tick.
        self.tick_cues.clear();
        self.tick_cues
            .extend(stepped.events.events.iter().map(map_cue));
        let next_hitstop = hitstop::next(current, &stepped.events.events);
        fill_laterals_for_tick(stepped.rows, tick, fighters, &mut self.laterals);
        fill_views_for_tick(stepped.rows, tick, fighters, &mut self.views);
        // The authority's break events for this tick, kept for `observe_into` to hand out.
        self.tick_breaks.clear();
        self.tick_breaks.extend_from_slice(&stepped.breaks.events);
        // First production read of the press-tick tell (EG19b): one starter
        // slot per fresh v2 attack, in fighter-ID order; empty under v1 rows
        // and on quiet ticks. Both borrows are shared, so the live match
        // lends its cue buffer beside the stepped tick. Presentation only,
        // never hashed.
        self.tick_started.clear();
        self.tick_started
            .extend(self.live.cues().iter().map(|cue| cue.attacker));
        self.hitstop_remaining = next_hitstop;
        // Move the refilled scratch out for this tick only; both callers
        // restore the buffers below, so the next tick reuses the capacity.
        let cues = std::mem::take(&mut self.tick_cues);
        let started = std::mem::take(&mut self.tick_started);
        Some((checkpoint, cues, started))
    }

    /// Restore the [`App::simulate_tick`] scratch buffers after one accepted
    /// tick, keeping their capacity for the next tick. Callers run this on
    /// every path that consumed the tick's owned cue buffers.
    fn restore_tick_scratch(&mut self, mut cues: Vec<FightCue>, mut started: Vec<u8>) {
        cues.clear();
        started.clear();
        self.tick_cues = cues;
        self.tick_started = started;
    }

    /// Rare path: replace the live match with a fresh one stepped through
    /// the tape's first `ticks` ticks. `None` (live match untouched) when
    /// the tape no longer replays.
    fn rebuild_live(&mut self, ticks: u32) -> Option<()> {
        let mut live = self.spec().live()?;
        let mut hitstop = 0u32;
        for tick in 0..ticks {
            let stepped = live.step(tick_inputs(&self.tape, tick))?;
            hitstop = hitstop::next(hitstop, &stepped.events.events);
        }
        // The fresh match counts its own replay steps; bill the old one here.
        self.retired_steps = self.retired_steps.saturating_add(self.live.steps());
        self.live = live;
        self.hitstop_remaining = hitstop;
        Some(())
    }

    /// Fold one tick's cues into hp, kill feed, crowd, audio data and
    /// progression. All integers; the crowd clock advances exactly once per
    /// accepted tick, so the same cue script gives the same reactions twice.
    /// Fresh-buffer wrapper over [`App::observe_into`].
    fn observe(
        &mut self,
        tick: u32,
        cues: &[FightCue],
        started: &[u8],
        checkpoint: Checkpoint,
    ) -> TickOutcome {
        let mut out = TickOutcome::default();
        self.observe_into(tick, cues, started, checkpoint, &mut out);
        out
    }

    /// Fold one fight cue into hp, kill feed, audio and crowd reactions, in
    /// cue emission order. The crowd clock still advances once per accepted
    /// tick in [`App::observe_into`], so the same cue script gives the same
    /// reactions twice. Budget: O(1) time, no heap growth at steady state
    /// (the caller sizes `out` once).
    fn fold_cue_into(&mut self, tick: u32, cue: FightCue, out: &mut TickOutcome) {
        match cue {
            FightCue::Hit { victim, damage, .. } => {
                let slot = usize::from(victim);
                if slot < self.hp.len() {
                    self.hp[slot] = self.hp[slot].saturating_sub(damage);
                }
                out.audio.push(AudioCommand {
                    tick,
                    sound: BankSound::Hit,
                    gain: 1.0,
                });
            }
            FightCue::Blocked { victim, chip, .. } => {
                let slot = usize::from(victim);
                if slot < self.hp.len() {
                    self.hp[slot] = self.hp[slot].saturating_sub(chip);
                }
                out.audio.push(AudioCommand {
                    tick,
                    sound: BankSound::Hit,
                    gain: 0.4,
                });
            }
            FightCue::Parried { .. } => {
                out.audio.push(AudioCommand {
                    tick,
                    sound: BankSound::Hit,
                    gain: 0.7,
                });
            }
            FightCue::Dodged { .. } | FightCue::StaminaOut { .. } => {}
            FightCue::Knockdown { attacker, victim } => {
                let slot = usize::from(victim);
                if slot < self.hp.len() {
                    self.hp[slot] = 0;
                }
                out.audio.push(AudioCommand {
                    tick,
                    sound: BankSound::Hit,
                    gain: 1.0,
                });
                // Arena slots are always valid fighter IDs by
                // construction; a corrupt index skips the feed entry
                // rather than panicking.
                if let (Some(killer), Some(dead)) =
                    (FighterId::new(attacker), FighterId::new(victim))
                {
                    let entry = KillEntry {
                        tick: u64::from(tick),
                        killer: Some(killer),
                        victim: dead,
                        cause: KillCause::Strike,
                    };
                    self.kills.push(entry);
                    let _ = self.hud.push_kill(entry);
                }
                if !self.decided {
                    self.decided = true;
                    self.matches_completed = self.matches_completed.saturating_add(1);
                }
                if attacker == HUMAN_SLOT {
                    self.wins = self.wins.saturating_add(1);
                }
                if victim == HUMAN_SLOT {
                    self.losses = self.losses.saturating_add(1);
                }
            }
        }
        out.reactions.push(self.crowd.on_fight_event(cue));
        self.crowd_gpu.on_cue(cue);
    }

    /// Refill a caller-held outcome for one accepted tick: clears its buffers
    /// and extends them in place, so a loop reusing one [`TickOutcome`]
    /// performs no heap allocation per tick once the buffers are sized.
    /// Same folded state and same outcome bytes as [`App::observe`].
    /// Budget: O(cues) time, zero heap growth at steady state.
    fn observe_into(
        &mut self,
        tick: u32,
        cues: &[FightCue],
        started: &[u8],
        checkpoint: Checkpoint,
        out: &mut TickOutcome,
    ) {
        out.audio.clear();
        out.reactions.clear();
        out.cues.clear();
        out.started.clear();
        // The tick's break events move from the scratch `simulate_tick` filled; a direct
        // call that never simulated finds it empty, so nothing stale is ever replayed.
        out.breaks.clear();
        out.breaks.append(&mut self.tick_breaks);
        out.audio.reserve(cues.len());
        out.reactions.reserve(cues.len());
        out.cues.reserve(cues.len());
        out.started.reserve(started.len());
        out.started.extend_from_slice(started);
        for cue in cues {
            self.fold_cue_into(tick, *cue, out);
        }
        self.crowd.tick();
        self.crowd_gpu.tick();
        self.director_layers = drive(&mut self.director, cues);
        self.drive_cut_director(tick, cues);
        self.hud.tick = u64::from(tick);
        self.hud.hp = u32::from(self.hp[HUMAN_SLOT as usize]);
        self.hud.score = u64::from(self.wins);
        out.cues.extend_from_slice(cues);
        sync_hud(&mut out.hud, &self.hud);
        out.tick = tick;
        out.input_accepted = true;
        out.checkpoint = Some(checkpoint);
        out.bed_gain = self.crowd.bed_gain();
        out.layers = self.director_layers;
    }

    /// Drive the spectator camera from one accepted tick: the tick's real cues,
    /// the cached authoritative laterals, live health and the bed just folded
    /// above. A corrupt lateral keeps the last-good shot; the outcome is
    /// untouched either way. Rejected ticks never reach here.
    fn drive_cut_director(&mut self, tick: u32, cues: &[FightCue]) {
        let bed_gain = self.crowd.bed_gain();
        if self
            .cut_director
            .observe_tick(tick, cues, &self.hp, bed_gain, &self.laterals)
            .is_err()
        {
            // Last-good shot holds.
        }
    }

    /// Presentation tracks over the simulated prefix, served from the
    /// rolling cursor: byte-equal to the movement-path legs, with no
    /// re-simulation while the cursor is current (every accepted tick
    /// extends it once via [`App::extend_presentation`]). A stale cursor
    /// (only reachable through direct `observe_into` callers, never the
    /// tick paths) falls back to the full legs. Empty before tick 0.
    #[must_use]
    pub fn presentation(&self) -> PresentationData {
        if self.cursor == 0 {
            return PresentationData::default();
        }
        if self.present_ticks == self.cursor {
            return self.presentation_ref().clone();
        }
        Self::prefix_presentation(&self.arena, &self.tape, self.cursor)
    }

    /// Ticks covered by the rolling presentation cursor: equals
    /// [`App::cursor`] on every tick path (accepted ticks extend it once,
    /// rejected ticks leave it alone).
    #[must_use]
    pub fn presentation_len(&self) -> u32 {
        self.present_ticks
    }

    /// Rolling cursor tracks by reference: the per-frame read path. No
    /// simulation and no allocation at any cursor, so frame cost stays flat
    /// over a long match. The window frame loop reads through here and
    /// [`App::pose_cursor`] (see [`App::pose`]).
    #[must_use]
    pub fn presentation_ref(&self) -> &PresentationData {
        &self.present
    }

    /// Latest locomotion pose per fighter in slot order: the tail of the
    /// rolling cursor and the per-frame skinning read. Empty before tick 0.
    /// Byte-equal to the full replay's last poses, twice over. Served
    /// through [`App::pose`] on the frame path.
    #[must_use]
    pub fn pose_cursor(&self) -> &[LocalPose] {
        &self.present_latest
    }

    /// Keep (default) or drop the rolling presentation cursor. A window
    /// that poses fighters from [`App::fighter_views`] drops it: each
    /// accepted tick then saves one presentation sim step. Dropping clears
    /// the cursor, so [`App::pose_cursor`] reads empty and
    /// [`App::presentation`] replays the legs on demand; checkpoints, cues
    /// and views are untouched (the tick hash cannot move).
    pub fn set_presentation_cursor(&mut self, live: bool) {
        self.present_live = live;
        if !live {
            self.retire_presentation();
            self.present = PresentationData::default();
            self.present_ticks = 0;
            self.present_latest.clear();
        }
    }

    /// Window frame entry: the window frame loop calls this once per
    /// frame for the skinning read. Same slice as [`App::pose_cursor`],
    /// so the window never touches the cached tracks directly.
    #[must_use]
    pub fn window_frame_poses(&self) -> &[LocalPose] {
        self.pose_cursor()
    }

    /// Fighter slots at their simulated positions for the latest tick: slot,
    /// feet position and yaw read from the authoritative dataset each tick
    /// (VS4). Empty before tick 0; a dataset miss keeps the previous views.
    /// The fighter-draw leg (bind pose, then VS5 skinning) reads through here.
    #[must_use]
    pub fn fighter_views(&self) -> &[FighterView] {
        &self.views
    }

    /// Read-only live match for presentation polling: the window look reads
    /// the per-slot integer verdicts through here and never writes match
    /// state, so the tick hash cannot move.
    #[must_use]
    pub fn live_match(&self) -> &LiveMatch {
        &self.live
    }

    /// Movement-path legs over the first `ticks` of the tape: the full
    /// replay spelling the rolling cursor stays byte-equal to.
    fn prefix_presentation(
        arena: &ArenaConfig,
        tape: &FighterTape,
        ticks: u32,
    ) -> PresentationData {
        if ticks == 0 {
            return PresentationData::default();
        }
        let short = ArenaConfig { ticks, ..*arena };
        PresentationData {
            pose_tracks: run_loco_poses(&short, tape),
            root_tables: engine_arena::run_presentation(&short, tape),
        }
    }

    /// Extend the rolling cursor by the accepted tick just simulated (EG1b):
    /// the live presentation sim steps exactly once and appends the tick to
    /// the cached tracks and root tables, byte-equal to the whole-prefix
    /// legs ([`App::prefix_presentation`], kept as the oracle). A cold or
    /// out-of-step cache (first tick, the cursor turned back on, a failed
    /// step) rebuilds from tick 0 over the tape: the only replay path. An
    /// invalid step (unreachable after an accepted tick) keeps the old cache.
    fn extend_presentation(&mut self) {
        let next = self.cursor;
        if !self.present_live || self.present_ticks == next || next == 0 {
            return;
        }
        let tick = next - 1;
        let in_step = self.present_ticks == tick
            && self
                .present_sim
                .as_ref()
                .is_some_and(|sim| sim.tick() == tick);
        if !in_step && self.rebuild_presentation(tick).is_none() {
            return;
        }
        let inputs = tick_inputs(&self.tape, tick);
        let Some(sim) = self.present_sim.as_mut() else {
            return;
        };
        if sim
            .step_into(
                inputs,
                &mut self.present.pose_tracks,
                &mut self.present.root_tables,
            )
            .is_none()
        {
            // Poisoned or refused: drop it so the next tick rebuilds.
            self.retire_presentation();
            return;
        }
        self.present_latest.clear();
        for track in &self.present.pose_tracks {
            let Some(tail) = track.last() else {
                return;
            };
            self.present_latest.push(tail.clone());
        }
        self.present_ticks = next;
    }

    /// Rare path: a fresh presentation sim stepped through the tape's first
    /// `ticks` ticks into a fresh cache reserved for the whole session.
    /// `None` (cache untouched) when the tape no longer replays.
    fn rebuild_presentation(&mut self, ticks: u32) -> Option<()> {
        let fighters = usize::from(self.config.fighters);
        let session = usize::try_from(self.config.ticks).ok()?;
        let mut sim = LivePresentation::from_spec(&self.spec())?;
        let mut fresh = PresentationData {
            pose_tracks: (0..fighters).map(|_| Vec::with_capacity(session)).collect(),
            root_tables: (0..fighters)
                .map(|_| RootTable {
                    deltas: Vec::with_capacity(session),
                })
                .collect(),
        };
        for tick in 0..ticks {
            sim.step_into(
                tick_inputs(&self.tape, tick),
                &mut fresh.pose_tracks,
                &mut fresh.root_tables,
            )?;
        }
        self.retire_presentation();
        self.present_sim = Some(sim);
        self.present = fresh;
        self.present_ticks = ticks;
        self.present_latest.clear();
        self.present_latest.reserve(fighters);
        for track in &self.present.pose_tracks {
            if let Some(tail) = track.last() {
                self.present_latest.push(tail.clone());
            }
        }
        Some(())
    }

    /// Drop the presentation sim, billing its steps to the retired count.
    fn retire_presentation(&mut self) {
        if let Some(sim) = self.present_sim.take() {
            self.present_retired_steps = self.present_retired_steps.saturating_add(sim.steps());
        }
    }

    /// Pose for a presentation source. The arena movement path lives in
    /// [`App::presentation`]; the imported fighter waits on the follow-up
    /// art hook (authored content is the visual gate) and reports
    /// [`GameError::ArtDeferred`].
    pub fn pose(&self, source: PoseSource) -> Result<Option<LocalPose>, GameError> {
        match source {
            PoseSource::LocoPresentation => {
                if self.present_ticks == self.cursor {
                    return Ok(self.window_frame_poses().first().cloned());
                }
                Ok(self
                    .presentation()
                    .pose_tracks
                    .first()
                    .and_then(|track| track.last().cloned()))
            }
            PoseSource::ImportedFighter => Err(GameError::ArtDeferred),
        }
    }

    /// Seal the live progression through `engine-save`: envelope seed and
    /// tick are the match seed and cursor, so the roundtrip preserves both
    /// the counters and the match coordinates.
    pub fn save_progress(&self, dir: &std::path::Path, slot: u8) -> Result<(), GameError> {
        if slot >= engine_save::SLOTS {
            return Err(GameError::BadSlot(slot));
        }
        let payload = self.progression().encode();
        save_generation(
            dir,
            slot,
            self.config.seed,
            u64::from(self.cursor),
            &payload,
        )
        .map_err(|err| GameError::Io(err.kind()))?;
        // Best-effort Steam cloud mirror: skips silently without a
        // backend, so headless and null-platform runs behave identically.
        let _ = crate::steam::mirror_generation(slot, &payload);
        Ok(())
    }

    /// Open a saved generation and decode its progression payload: envelope
    /// version, CRC and length via `engine-save`, payload version/length
    /// here. A future envelope or payload version fails; truncated or corrupt
    /// bytes fail; the prior valid generation on disk is never touched.
    pub fn load_progress(dir: &std::path::Path, slot: u8) -> Result<LoadedProgress, GameError> {
        let name = slot_name(slot).map_err(|_| GameError::BadSlot(slot))?;
        let bytes = std::fs::read(dir.join(name)).map_err(|err| GameError::Io(err.kind()))?;
        let envelope = open(&bytes).map_err(GameError::Save)?;
        let envelope = migrate(&envelope).map_err(GameError::Save)?;
        let progress = GameProgress::decode(&envelope.payload)?;
        Ok(LoadedProgress {
            progress,
            envelope_seed: envelope.seed,
            envelope_tick: envelope.tick,
        })
    }
}

/// Copy one HUD snapshot into a reused one without reallocating: scalars
/// copy directly while the fighter rows, kill feed and map dots resync in
/// place and keep their buffers. Skipped push/upsert errors are unreachable:
/// entries come from an already-validated snapshot under the same caps.
fn sync_hud(dst: &mut Hud, src: &Hud) {
    dst.tick = src.tick;
    dst.hp = src.hp;
    dst.max_hp = src.max_hp;
    dst.ammo = src.ammo;
    dst.score = src.score;
    dst.kill_feed = src.kill_feed;
    dst.fighters.clear();
    dst.fighters.extend_from_slice(&src.fighters);
    dst.feed.clear();
    for entry in src.feed.iter() {
        let _ = dst.feed.push(*entry);
    }
    dst.map.clear();
    for dot in src.map.iter() {
        let _ = dst.map.upsert(
            dot.id,
            f32::from(dot.x) / CELLS_PER_METER,
            f32::from(dot.z) / CELLS_PER_METER,
        );
    }
}

/// Map one combat event into its audio fight cue, field for field (the
/// mapping `engine-audio` crowd.rs documents for the game). Pure.
fn map_cue(event: &FightEvent) -> FightCue {
    match *event {
        FightEvent::Hit {
            attacker,
            victim,
            damage,
        } => FightCue::Hit {
            attacker,
            victim,
            damage,
        },
        FightEvent::Blocked {
            attacker,
            victim,
            chip,
        } => FightCue::Blocked {
            attacker,
            victim,
            chip,
        },
        FightEvent::Parried { attacker, victim } => FightCue::Parried { attacker, victim },
        FightEvent::Dodged { attacker, victim } => FightCue::Dodged { attacker, victim },
        FightEvent::StaminaOut { fighter } => FightCue::StaminaOut { fighter },
        FightEvent::Knockdown { attacker, victim } => FightCue::Knockdown { attacker, victim },
    }
}

/// One fighter's simulated placement for a tick (VS4): the draw leg binds the
/// imported body at exactly this slot, position and yaw.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FighterView {
    /// Fighter slot, `0..fighters`.
    pub slot: u8,
    /// Feet position `[x, y, z]` in meters, finite.
    pub pos_m: [f32; 3],
    /// Facing yaw in radians, finite.
    pub yaw: f32,
}

/// True when `key` sorts at or after the tape tail, so the input appends in
/// O(1). Pure over the tail and the key; out-of-order LAN arrivals return
/// false and pay the sorted insert. Caller: [`App::stage_input`].
fn tape_should_append(tape: &FighterTape, key: (u32, u8)) -> bool {
    tape.0
        .last()
        .is_none_or(|last| (last.tick, last.fighter.0) <= key)
}

/// The tape's inputs for one tick: the tape is `(tick, fighter)`-sorted, so
/// the tick's slice is found by two binary searches (no scan, no alloc).
fn tick_inputs(tape: &FighterTape, tick: u32) -> &[FighterInput] {
    let start = tape.0.partition_point(|input| input.tick < tick);
    let end = tape.0.partition_point(|input| input.tick <= tick);
    tape.0.get(start..end).unwrap_or_default()
}

/// Views for one dataset tick: same rows as [`laterals_for_tick`], carrying
/// the full position plus yaw. `rows` starts at the tick's first row (the
/// live match serves exactly the tick's rows). `None` on any shape mismatch
/// (wrong tick, wrong count); the caller keeps the previous views then.
/// Pure over the rows. Allocating form for the tests; the live path refills
/// in place through [`fill_views_for_tick`].
#[cfg(test)]
fn views_for_tick(rows: &[Row], tick: u32, fighters: u8) -> Option<Vec<FighterView>> {
    let mut out = Vec::with_capacity(usize::from(fighters));
    fill_views_for_tick(rows, tick, fighters, &mut out).then_some(out)
}

/// Refill `out` with the tick's views in place (clear plus extend, no
/// allocation once sized). True on a shape match; false (buffer untouched)
/// on any mismatch, so the caller keeps the previous views then.
fn fill_views_for_tick(rows: &[Row], tick: u32, fighters: u8, out: &mut Vec<FighterView>) -> bool {
    let stride = usize::from(fighters);
    let row = match rows.first() {
        Some(row) => row,
        None => return false,
    };
    if row.observation.tick != tick || row.observation.fighters.len() != stride {
        return false;
    }
    out.clear();
    out.extend(row.observation.fighters.iter().map(|entry| FighterView {
        slot: entry.id,
        pos_m: entry.pos,
        yaw: entry.yaw,
    }));
    true
}

/// Laterals for one dataset tick: the tick's first row carries the shared
/// pre-step observation, so its fighter entries map straight onto the camera
/// axis (`x`). Rows are tick-major with one row per fighter; `rows` starts
/// at the tick's first row. `None` on any shape mismatch (wrong tick, wrong
/// count); the caller keeps the previous laterals then. Pure over the rows.
/// Allocating form for the tests; the live path refills in place through
/// [`fill_laterals_for_tick`].
#[cfg(test)]
fn laterals_for_tick(rows: &[Row], tick: u32, fighters: u8) -> Option<Vec<FighterLateral>> {
    let mut out = Vec::with_capacity(usize::from(fighters));
    fill_laterals_for_tick(rows, tick, fighters, &mut out).then_some(out)
}

/// Refill `out` with the tick's laterals in place (clear plus extend, no
/// allocation once sized). True on a shape match; false (buffer untouched)
/// on any mismatch, so the caller keeps the previous laterals then.
fn fill_laterals_for_tick(
    rows: &[Row],
    tick: u32,
    fighters: u8,
    out: &mut Vec<FighterLateral>,
) -> bool {
    let stride = usize::from(fighters);
    let row = match rows.first() {
        Some(row) => row,
        None => return false,
    };
    if row.observation.tick != tick || row.observation.fighters.len() != stride {
        return false;
    }
    out.clear();
    out.extend(row.observation.fighters.iter().map(|entry| FighterLateral {
        id: entry.id,
        x_m: entry.pos[0],
    }));
    true
}

// The unit-test binary's one counting allocator; the zero-allocation gates
// in this crate measure through `engine_core::alloc_probe::measure`.
#[cfg(test)]
#[global_allocator]
static TEST_ALLOCATOR: engine_core::alloc_probe::CountingAlloc =
    engine_core::alloc_probe::CountingAlloc;

#[cfg(test)]
mod tests {
    use super::*;
    use engine_arena::{run_dataset, run_events, run_prefix};
    use engine_core::alloc_probe;
    use engine_input::Buttons;

    const TICKS: u32 = 600;

    fn config() -> GameConfig {
        GameConfig {
            seed: 2040,
            ticks: TICKS,
            fighters: 2,
            half_extent_m: 20.0,
        }
    }

    fn small_config() -> GameConfig {
        GameConfig {
            seed: 7,
            ticks: 64,
            fighters: 2,
            half_extent_m: 12.0,
        }
    }

    fn human(tick: u32, mx: i8, mz: i8, buttons: Buttons) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId::new(HUMAN_SLOT).unwrap(),
            mx,
            mz,
            facing: 0,
            buttons,
        }
    }

    fn idle(tick: u32) -> FighterInput {
        human(tick, 0, 0, Buttons::NONE)
    }

    /// Scripted session input: circle-strafe with an attack every 30 ticks.
    fn script(tick: u32) -> FighterInput {
        let phase = (tick / 15) % 4;
        let (mx, mz) = match phase {
            0 => (100, 0),
            1 => (0, 100),
            2 => (-100, 0),
            _ => (0, -100),
        };
        let buttons = if tick.is_multiple_of(30) {
            Buttons(engine_input::fighter::ATTACK)
        } else {
            Buttons::NONE
        };
        human(tick, mx, mz, buttons)
    }

    fn run_session(config: GameConfig) -> (App, Vec<TickOutcome>) {
        let mut app = App::new(config).unwrap();
        let mut outcomes = Vec::with_capacity(config.ticks as usize);
        for tick in 0..config.ticks {
            outcomes.push(app.tick(script(tick)));
        }
        (app, outcomes)
    }

    #[test]
    fn present_every_decouples_ticks_from_frames() {
        // N>1 plays every tick but presents fewer frames with identical tick
        // hashes (GM3 present decoupling, piece 1).
        let ticks = small_config().ticks;
        let run = |every: u32| {
            let mut app = App::new(small_config()).unwrap();
            let mut presented = 0u32;
            let mut hashes = Vec::with_capacity(ticks as usize);
            for tick in 0..ticks {
                let outcome = app.tick(idle(tick));
                presented += u32::from(should_present(tick, ticks, every));
                hashes.push(outcome.checkpoint.unwrap().hash);
            }
            assert_eq!(app.cursor(), ticks);
            (presented, hashes)
        };
        let (every1, hashes1) = run(1);
        let (every10, hashes10) = run(10);
        assert_eq!(every1, ticks);
        assert!(every10 < every1, "{every10} presented of {every1}");
        assert_eq!(hashes1, hashes10);
        assert_eq!(parse_present_every("10"), Ok(10));
        assert!(parse_present_every("0").is_err());
    }

    #[test]
    fn rejects_bad_configs() {
        assert!(App::new(config()).is_ok());
        for bad in [
            GameConfig {
                ticks: 0,
                ..config()
            },
            GameConfig {
                ticks: MAX_TICKS + 1,
                ..config()
            },
            GameConfig {
                fighters: 1,
                ..config()
            },
            GameConfig {
                fighters: 17,
                ..config()
            },
            GameConfig {
                half_extent_m: f32::NAN,
                ..config()
            },
            GameConfig {
                half_extent_m: 4.9,
                ..config()
            },
            GameConfig {
                half_extent_m: 101.0,
                ..config()
            },
        ] {
            assert!(App::new(bad).is_err(), "config accepted: {bad:?}");
        }
        // 5 fighters fits the 16-slot cap.
        assert!(App::new(GameConfig {
            fighters: 5,
            ..config()
        })
        .is_ok());
    }

    #[test]
    fn match_advances_and_emits_hud_and_audio_data() {
        let (app, outcomes) = run_session(small_config());
        assert_eq!(outcomes.len(), 64);
        assert_eq!(app.cursor(), 64);
        assert_eq!(app.checkpoints().len(), 64);
        for (tick, outcome) in outcomes.iter().enumerate() {
            assert!(outcome.input_accepted);
            assert_eq!(outcome.tick, tick as u32);
            let checkpoint = outcome.checkpoint.unwrap();
            assert_eq!(checkpoint.tick, tick as u32);
            assert_eq!(outcome.hud.tick, tick as u64);
            assert_eq!(outcome.audio.len(), outcome.cues.len());
            assert_eq!(outcome.reactions.len(), outcome.cues.len());
            assert!((0.0..=1.0).contains(&outcome.bed_gain));
            for command in &outcome.audio {
                assert_eq!(command.tick, tick as u32);
                assert!((0.0..=1.0).contains(&command.gain));
            }
        }
        // The scripted match moves: checkpoints are tick-indexed and the run
        // is not one repeated hash.
        let mut distinct = outcomes
            .iter()
            .filter_map(|outcome| outcome.checkpoint.map(|checkpoint| checkpoint.hash))
            .collect::<Vec<_>>();
        distinct.sort_unstable();
        distinct.dedup();
        assert!(distinct.len() > 1, "match never moved");
    }

    #[test]
    fn into_duel_moves_a_fresh_session_to_tables_v2_and_refuses_later() {
        use engine_combat::tables::{TABLES_VERSION_V1, TABLES_VERSION_V2};
        // Default sessions stay on the frozen v1 rules (the LAN authority
        // builds from the same v1 config).
        let v1 = App::new(small_config()).unwrap();
        assert_eq!(v1.arena.table_version, TABLES_VERSION_V1);
        assert_eq!(v1.rules, ArenaRules::V1);
        let duel = App::new(small_config()).unwrap().into_duel().unwrap();
        assert_eq!(duel.arena.table_version, TABLES_VERSION_V2);
        assert_eq!(duel.rules, ArenaRules::v2_free_for_all());
        assert_eq!(duel.live.rules(), duel.rules);
        // Same inputs: v2 checkpoints differ from v1 and repeat exactly.
        let hashes = |mut app: App| -> Vec<u64> {
            (0..24)
                .filter_map(|tick| app.tick(script(tick)).checkpoint.map(|cp| cp.hash))
                .collect()
        };
        let first = hashes(App::new(small_config()).unwrap().into_duel().unwrap());
        assert_eq!(first.len(), 24);
        assert_eq!(
            first,
            hashes(App::new(small_config()).unwrap().into_duel().unwrap())
        );
        assert_ne!(first, hashes(App::new(small_config()).unwrap()));
        // After the first tick the tables are fixed.
        let mut ticked = App::new(small_config()).unwrap();
        ticked.tick(script(0));
        assert!(matches!(
            ticked.into_duel(),
            Err(GameError::BadConfig("duel_tables"))
        ));
    }

    /// EG44: the window plays rules v2 through `into_duel`
    /// (`windowed.rs::run_windowed`), never through `GameConfig`, so
    /// `App::new` stays v1 by design; both constructors must agree on that v1
    /// start and on the v2 duel they then select.
    #[test]
    fn app_new_equals_new_for_slot_on_rules_v2() {
        use engine_combat::fight::{FIGHT_RULES_V1, FIGHT_RULES_V2};
        use engine_combat::tables::TABLES_VERSION_V1;
        let plain = App::new(small_config()).unwrap();
        let slotted = App::new_for_slot(small_config(), HUMAN_SLOT).unwrap();
        assert_eq!(
            plain.rules.version, FIGHT_RULES_V1,
            "App::new stays v1 by design"
        );
        assert_eq!(plain.arena.table_version, TABLES_VERSION_V1);
        assert_eq!(plain.rules, slotted.rules);
        assert_eq!(plain.human_slot(), slotted.human_slot());
        assert_eq!(plain.spec(), slotted.spec(), "one v1 identity");
        // The v2 match the window builds: the same `into_duel` step.
        let mut plain = plain.into_duel().unwrap();
        let mut slotted = slotted.into_duel().unwrap();
        assert_eq!(plain.rules.version, FIGHT_RULES_V2);
        assert_eq!(plain.rules, slotted.rules);
        assert_eq!(plain.spec(), slotted.spec(), "one v2 identity");
        let mut plain_hashes = Vec::new();
        let mut slotted_hashes = Vec::new();
        for tick in 0..8 {
            let a = plain.tick(script(tick)).checkpoint.expect("duel tick");
            let b = slotted.tick(script(tick)).checkpoint.expect("duel tick");
            assert_eq!(a, b, "tick {tick}");
            plain_hashes.push(a.hash);
            slotted_hashes.push(b.hash);
        }
        assert_eq!(plain_hashes, slotted_hashes, "the same v2 hashes");
        assert_eq!(
            plain.checkpoints(),
            slotted.checkpoints(),
            "the same v2 checkpoints"
        );
    }

    /// DSN1: the window session and the net server and client, all built from
    /// one spec, play the same match: equal state hash on every tick of a
    /// scripted 300-tick duel on the kit walls.
    #[test]
    fn spec_window_server_and_client_play_one_match_for_300_ticks() {
        use engine_net::session::{Client, Server};
        let config = GameConfig {
            seed: 0x5EED,
            ticks: 300,
            fighters: 4,
            half_extent_m: 18.0,
        };
        let mut window = App::with_world(config, crate::arena_kit::kit_match_world_or_empty())
            .unwrap()
            .into_duel()
            .unwrap();
        let spec = MatchSpec::kit_duel(config.arena_config()).expect("kit duel");
        assert_eq!(
            window.spec().world_digest(),
            spec.world_digest(),
            "the game and the arena load one kit world"
        );
        assert_eq!(window.spec().identity(), spec.identity());
        let mut server = Server::from_spec(window.spec()).expect("server");
        let mut client =
            Client::from_spec(window.spec(), FighterId::new(HUMAN_SLOT).unwrap()).expect("client");
        let mut checkpoints = Vec::new();
        for tick in 0..config.ticks {
            let input = script(tick);
            let seen = window.tick(input).checkpoint.expect("window tick");
            let (authored, _) = server.tick(&[input]).expect("server tick");
            let predicted = client.predict(input).expect("client predict");
            assert_eq!(seen, authored, "window vs server at tick {tick}");
            assert_eq!(seen, predicted, "window vs client at tick {tick}");
            checkpoints.push(seen.hash);
        }
        let default = App::new(config).unwrap().spec();
        assert_eq!(default.rules, ArenaRules::V1, "the default stays v1");
        assert_eq!(default.world_digest(), 0, "the default stays open sand");
        let mut v1 = LiveMatch::new(&config.arena_config()).unwrap();
        let v1_hashes: Vec<u64> = (0..config.ticks)
            .map(|tick| v1.step(&[script(tick)]).unwrap().checkpoint.hash)
            .collect();
        assert_ne!(checkpoints, v1_hashes, "the spec is not the old default");
    }

    #[test]
    fn attack_started_tell_surfaces_on_duel_press_tick() {
        // EG19b remainder: the game surfaces the v2 press tell on the tick
        // outcome (the swing clip starts from these); v1 sessions stay silent.
        use engine_input::fighter::ATTACK;
        let mut v1 = App::new(small_config()).unwrap();
        let pressed = v1.tick(human(0, 0, 0, Buttons(ATTACK)));
        assert!(pressed.input_accepted, "press accepted");
        assert!(
            pressed.started.is_empty(),
            "v1 rows cue nothing: {:?}",
            pressed.started
        );
        let mut duel = App::new(small_config()).unwrap().into_duel().unwrap();
        let first = duel.tick(human(0, 0, 0, Buttons(ATTACK)));
        assert!(first.input_accepted, "duel press accepted");
        assert_eq!(
            first.started,
            vec![HUMAN_SLOT],
            "press tick tells its starter once"
        );
        for slot in &first.started {
            assert!(*slot < small_config().fighters, "tell names a live slot");
        }
        let second = duel.tick(idle(1));
        assert!(second.input_accepted, "idle accepted");
        assert!(
            !second.started.contains(&HUMAN_SLOT),
            "the tell never lingers: {:?}",
            second.started
        );
        // Same duel script tells the same starter twice.
        let mut replay = App::new(small_config()).unwrap().into_duel().unwrap();
        let again = replay.tick(human(0, 0, 0, Buttons(ATTACK)));
        assert_eq!(again.started, first.started, "tell replays");
    }

    #[test]
    fn hitstop_hold_sets_four_on_hit_then_counts_down() {
        // Feel LF10: the sim freezes 4 ticks on impact; the app mirrors the
        // count read-only so the look can hold clip clocks (presentation
        // only, never hashed). The scripted bout lands Hit at ticks 4 and 34.
        let mut app = App::new(small_config()).unwrap();
        assert_eq!(app.hitstop_remaining(), 0);
        assert!(!App::hitstop_holds(app.hitstop_remaining()));
        let ticks = small_config().ticks;
        let mut counts: Vec<u32> = Vec::with_capacity(usize::try_from(ticks).expect("ticks fit"));
        let mut seen: Option<u32> = None;
        for tick in 0..ticks {
            let outcome = app.tick(script(tick));
            assert!(outcome.input_accepted, "tick {tick}");
            counts.push(app.hitstop_remaining());
            if seen.is_none()
                && outcome
                    .cues
                    .iter()
                    .any(|cue| matches!(cue, FightCue::Hit { .. }))
            {
                seen = Some(tick);
            }
        }
        let impact = seen.expect("scripted bout must land a Hit");
        let base = usize::try_from(impact).expect("tick fits");
        assert_eq!(counts[base], 4, "hitstop equals 4 right after impact");
        for (step, want) in [3u32, 2, 1, 0].iter().enumerate() {
            let offset = u32::try_from(step).expect("step fits");
            let at_tick = impact + 1 + offset;
            let at = base + 1 + step;
            assert_eq!(counts[at], *want, "tick {at_tick}");
        }
        assert!(App::hitstop_holds(4));
        assert!(App::hitstop_holds(1));
        assert!(!App::hitstop_holds(0));
    }

    #[test]
    fn hitstop_hold_predicate_true_at_four_and_one_false_at_zero() {
        assert!(App::hitstop_holds(4));
        assert!(App::hitstop_holds(1));
        assert!(!App::hitstop_holds(0));
        let fresh = App::new(small_config()).unwrap();
        assert_eq!(fresh.hitstop_remaining(), 0);
        assert!(!App::hitstop_holds(fresh.hitstop_remaining()));
    }

    #[test]
    fn headless_600_tick_session_identical_twice() {
        let (first_app, first_outcomes) = run_session(config());
        let (second_app, second_outcomes) = run_session(config());
        assert_eq!(first_app.checkpoints(), second_app.checkpoints());
        assert_eq!(first_app.checkpoints().len(), TICKS as usize);
        assert_eq!(first_app.hp(), second_app.hp());
        assert_eq!(first_app.hud().state_hash(), second_app.hud().state_hash());
        assert_eq!(first_app.progression(), second_app.progression());
        for (first, second) in first_outcomes.iter().zip(second_outcomes.iter()) {
            assert_eq!(first, second);
        }
    }

    #[test]
    fn hp_and_progression_accounting_match_emitted_cues() {
        let (app, outcomes) = run_session(config());
        let mut hp = [MAX_HP_PERMILLE; MAX_PARTICIPANTS as usize];
        let (mut wins, mut losses) = (0u32, 0u32);
        let mut knockdowns = 0u32;
        for outcome in &outcomes {
            for cue in &outcome.cues {
                match *cue {
                    FightCue::Hit { victim, damage, .. } => {
                        hp[victim as usize] = hp[victim as usize].saturating_sub(damage);
                    }
                    FightCue::Blocked { victim, chip, .. } => {
                        hp[victim as usize] = hp[victim as usize].saturating_sub(chip);
                    }
                    FightCue::Knockdown { attacker, victim } => {
                        hp[victim as usize] = 0;
                        knockdowns += 1;
                        if attacker == HUMAN_SLOT {
                            wins += 1;
                        }
                        if victim == HUMAN_SLOT {
                            losses += 1;
                        }
                    }
                    FightCue::Parried { .. }
                    | FightCue::Dodged { .. }
                    | FightCue::StaminaOut { .. } => {}
                }
            }
        }
        assert_eq!(app.hp(), &hp);
        let progress = app.progression();
        assert_eq!((progress.wins, progress.losses), (wins, losses));
        assert_eq!(progress.matches_completed, u32::from(knockdowns > 0));
        assert_eq!(progress.match_seed, config().seed);
        assert_eq!(progress.match_ticks, TICKS);
        assert_eq!(app.hud().hp, u32::from(hp[HUMAN_SLOT as usize]));
    }

    #[test]
    fn invalid_inputs_rejected_without_advancing() {
        let mut app = App::new(small_config()).unwrap();
        let accepted = app.tick(idle(999));
        assert!(
            accepted.input_accepted,
            "caller tick is stamped, not trusted"
        );
        assert_eq!(app.cursor(), 1);

        let wrong_slot = FighterInput {
            fighter: FighterId::new(1).unwrap(),
            ..idle(0)
        };
        let rejected = app.tick(wrong_slot);
        assert!(!rejected.input_accepted);
        assert_eq!(app.cursor(), 1);
        assert_eq!(rejected.checkpoint, accepted.checkpoint);

        let bad_wish = human(0, 101, 0, Buttons::NONE);
        let rejected = app.tick(bad_wish);
        assert!(!rejected.input_accepted);
        assert_eq!(app.cursor(), 1);

        // First input rejected before tick 0: no checkpoint yet, empty data.
        let mut fresh = App::new(small_config()).unwrap();
        let rejected = fresh.tick(wrong_slot);
        assert!(!rejected.input_accepted);
        assert_eq!(rejected.checkpoint, None);
        assert!(rejected.audio.is_empty() && rejected.cues.is_empty());
        assert_eq!(fresh.cursor(), 0);

        // Exhausted match rejects without moving.
        let mut app = App::new(GameConfig {
            ticks: 2,
            ..small_config()
        })
        .unwrap();
        assert!(app.tick(idle(0)).input_accepted);
        assert!(app.tick(idle(1)).input_accepted);
        assert!(!app.tick(idle(2)).input_accepted);
        assert_eq!(app.cursor(), 2);
    }

    fn progress_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("e2040-game-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_load_roundtrip_preserves_progression_and_seed() {
        // `save_progress` mirrors through `steam::probe`, which reads the
        // process-global Steam switches: share the Steam tests' env lock so a
        // mirror here can never land inside their cloud dir mid-window.
        let _env = crate::steam::ENV_LOCK.lock().expect("steam env lock");
        let dir = progress_dir("roundtrip");
        let (app, _) = run_session(small_config());
        app.save_progress(&dir, 3).unwrap();
        let loaded = App::load_progress(&dir, 3).unwrap();
        assert_eq!(loaded.progress, app.progression());
        assert_eq!(loaded.envelope_seed, small_config().seed);
        assert_eq!(loaded.envelope_tick, u64::from(app.cursor()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn failed_save_write_preserves_prior_valid_generation() {
        // Same env-lock owner as the Steam tests: this save may mirror, and
        // the Steam switches are process-global.
        let _env = crate::steam::ENV_LOCK.lock().expect("steam env lock");
        let dir = progress_dir("recover");
        let (app, _) = run_session(small_config());
        app.save_progress(&dir, 1).unwrap();
        let before = App::load_progress(&dir, 1).unwrap();
        // Bad slot: rejected before touching the filesystem.
        assert_eq!(app.save_progress(&dir, 99), Err(GameError::BadSlot(99)));
        assert_eq!(App::load_progress(&dir, 1).unwrap(), before);
        // Missing directory: IO error, prior generation intact.
        let missing = dir.join("no-such-dir");
        assert!(app.save_progress(&missing, 1).is_err());
        assert_eq!(App::load_progress(&dir, 1).unwrap(), before);
        // Corrupt bytes in the slot: load fails loudly.
        std::fs::write(dir.join("save01.e2040"), b"not a save").unwrap();
        assert!(App::load_progress(&dir, 1).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn dropping_the_presentation_cursor_keeps_ticks_and_serves_legs_on_demand() {
        let config = small_config();
        let (live, live_out) = run_session(config);
        let mut lean = App::new(config).unwrap();
        lean.set_presentation_cursor(false);
        let mut lean_out = Vec::new();
        for tick in 0..config.ticks {
            lean_out.push(lean.tick(script(tick)));
        }
        // Same authority: checkpoints, cues and views match tick for tick.
        assert_eq!(live.checkpoints(), lean.checkpoints());
        assert_eq!(live_out, lean_out);
        assert_eq!(live.fighter_views(), lean.fighter_views());
        // No rolling cursor, yet the full legs still replay on demand.
        assert!(lean.pose_cursor().is_empty());
        assert_eq!(lean.presentation(), live.presentation());
        assert_eq!(
            lean.pose(PoseSource::LocoPresentation).unwrap(),
            live.pose(PoseSource::LocoPresentation).unwrap()
        );
    }

    #[test]
    fn presentation_calls_run_loco_poses_for_the_movement_path() {
        let (app, _) = run_session(small_config());
        let first = app.presentation();
        assert_eq!(first.pose_tracks.len(), small_config().fighters as usize);
        for track in &first.pose_tracks {
            assert_eq!(track.len(), app.cursor() as usize);
        }
        assert_eq!(first.root_tables.len(), small_config().fighters as usize);
        // Same prefix twice gives the same tracks.
        assert_eq!(first, app.presentation());
        // Poses validate against the locomotion presentation skeleton.
        let skeleton = engine_anim::locomotion::loco_skeleton();
        for track in &first.pose_tracks {
            engine_anim::contract::validate_pose(&skeleton, &track[0]).unwrap();
        }
        // The arena movement path serves the latest pose; the imported fighter is the
        // documented art-hook stub.
        assert_eq!(
            app.pose(PoseSource::LocoPresentation).unwrap(),
            first.pose_tracks[0].last().cloned()
        );
        assert_eq!(
            app.pose(PoseSource::ImportedFighter),
            Err(GameError::ArtDeferred)
        );
        // Empty before tick 0, never a failure.
        let fresh = App::new(small_config()).unwrap();
        assert!(fresh.presentation().pose_tracks.is_empty());
        assert_eq!(fresh.pose(PoseSource::LocoPresentation).unwrap(), None);
    }

    /// Every cue shape the fold handles, ending on a knockdown pair so the
    /// refill path also covers the kill log and a full feed.
    fn burst_cues() -> Vec<FightCue> {
        vec![
            FightCue::Hit {
                attacker: 0,
                victim: 1,
                damage: 120,
            },
            FightCue::Blocked {
                attacker: 1,
                victim: 0,
                chip: 10,
            },
            FightCue::Parried {
                attacker: 0,
                victim: 1,
            },
            FightCue::Dodged {
                attacker: 1,
                victim: 0,
            },
            FightCue::StaminaOut { fighter: 0 },
            FightCue::Knockdown {
                attacker: 0,
                victim: 1,
            },
            FightCue::Hit {
                attacker: 1,
                victim: 0,
                damage: 40,
            },
            FightCue::Knockdown {
                attacker: 0,
                victim: 1,
            },
        ]
    }

    /// Close-range assault: attacks every 5 ticks so the session lands real
    /// cues (hits, blocks, knockdowns) instead of idling.
    fn assault(tick: u32) -> FighterInput {
        let phase = (tick / 15) % 4;
        let (mx, mz) = match phase {
            0 => (100, 0),
            1 => (0, 100),
            2 => (-100, 0),
            _ => (0, -100),
        };
        let buttons = if tick.is_multiple_of(5) {
            Buttons(engine_input::fighter::ATTACK)
        } else {
            Buttons::NONE
        };
        human(tick, mx, mz, buttons)
    }

    #[test]
    fn default_outcome_matches_rejected_before_tick_zero() {
        let out = TickOutcome::default();
        assert!(!out.input_accepted);
        assert_eq!(out.tick, 0);
        assert_eq!(out.checkpoint, None);
        assert!(out.audio.is_empty() && out.cues.is_empty() && out.reactions.is_empty());
        assert!(out.started.is_empty(), "fresh outcome tells nothing");
        assert_eq!(out.bed_gain, 1.0);
        assert_eq!(out.layers.murmur, 1.0);
        // A fresh session's first rejection carries the same shape.
        let mut fresh = App::new(small_config()).unwrap();
        let wrong_slot = FighterInput {
            fighter: FighterId::new(1).unwrap(),
            ..idle(0)
        };
        let rejected = fresh.tick(wrong_slot);
        assert!(!rejected.input_accepted);
        assert_eq!(rejected.tick, out.tick);
        assert_eq!(rejected.checkpoint, out.checkpoint);
        assert_eq!(rejected.hud, out.hud);
        assert_eq!(rejected.bed_gain, out.bed_gain);
        assert_eq!(rejected.layers, out.layers);
    }

    #[test]
    fn refilled_outcome_matches_fresh_tick_for_tick() {
        // A forgotten clear (stale tail) or a missed HUD sync fails here:
        // one app collects owned outcomes via `tick`, a twin refills a
        // single reused outcome with the same cues and must match exactly.
        let config = GameConfig {
            ticks: 200,
            ..small_config()
        };
        let mut fresh_app = App::new(config).unwrap();
        let mut refill_app = App::new(config).unwrap();
        let mut reused = TickOutcome::default();
        let mut total_cues = 0usize;
        let mut total_breaks = 0usize;
        for tick in 0..config.ticks {
            let fresh = fresh_app.tick(assault(tick));
            assert!(fresh.input_accepted, "assault inputs validate");
            let checkpoint = fresh.checkpoint.unwrap();
            let cues = fresh.cues.clone();
            let started = fresh.started.clone();
            total_cues += cues.len();
            total_breaks += fresh.breaks.len();
            // The twin never simulates, so it has no authority break events of its own: hand it
            // the fresh tick's, the way `simulate_tick` fills the scratch for `observe_into`.
            refill_app.tick_breaks.extend_from_slice(&fresh.breaks);
            refill_app.observe_into(fresh.tick, &cues, &started, checkpoint, &mut reused);
            assert_eq!(reused, fresh, "refilled outcome diverged at tick {tick}");
            assert!(
                refill_app.tick_breaks.is_empty(),
                "the refill moves the break scratch out, tick {tick}"
            );
        }
        assert!(
            total_cues > 0,
            "session must land cues or the check is vacuous"
        );
        assert!(
            total_breaks > 0,
            "session must break bonds or the break leg is vacuous"
        );
        // Folded session state matches too, not just the outcomes. The
        // cursor-derived `match_ticks` is sim state, not observe-fold
        // state, so only the folded counters and seed compare.
        assert_eq!(refill_app.hp(), fresh_app.hp());
        assert_eq!(refill_app.hud(), fresh_app.hud());
        let (refill_progress, fresh_progress) = (refill_app.progression(), fresh_app.progression());
        assert_eq!(
            refill_progress.matches_completed,
            fresh_progress.matches_completed
        );
        assert_eq!(refill_progress.wins, fresh_progress.wins);
        assert_eq!(refill_progress.losses, fresh_progress.losses);
        assert_eq!(refill_progress.match_seed, fresh_progress.match_seed);
    }

    #[test]
    fn observe_refill_allocates_nothing_at_steady_state() {
        // Calibration: the shared probe must observe a real allocation,
        // otherwise a zero reading would prove nothing.
        let (_, probe) = alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(probe.calls >= 1, "alloc probe is blind");

        let mut app = App::new(small_config()).unwrap();
        // Seed checkpoint from a real accepted tick (window closed).
        let first = app.tick(idle(0));
        assert!(first.input_accepted);
        let checkpoint = first.checkpoint.unwrap();

        let mut out = TickOutcome::default();
        let burst = burst_cues();
        // Warmup with the window closed: sizes every buffer on the path
        // (outcome vecs, kill log, full kill feed) to steady capacity, and
        // ends on a full burst so the steady-state shape asserts below.
        for tick in 1..=24u32 {
            let cues = if tick % 4 == 0 { &[][..] } else { &*burst };
            app.observe_into(tick, cues, &[], checkpoint, &mut out);
        }
        app.observe_into(25, &burst, &[], checkpoint, &mut out);
        std::hint::black_box(&out);
        assert_eq!(out.cues.len(), burst.len());
        assert_eq!(out.audio.len(), 6);
        assert_eq!(out.reactions.len(), burst.len());
        assert_eq!(app.hud().feed.len(), app.hud().feed.cap());

        // Steady state: empty, single, partial, full and knockdown-tail
        // refills each perform zero heap allocations.
        let measured: [&[FightCue]; 5] = [&[], &burst[..1], &burst[..6], (&*burst), &burst[6..]];
        for (round, cues) in measured.iter().enumerate() {
            let ((), steady) = alloc_probe::measure(|| {
                app.observe_into(1000 + round as u32, cues, &[], checkpoint, &mut out);
                std::hint::black_box(&out);
            });
            assert_eq!(
                steady.calls,
                0,
                "observe_into allocated on {} cues",
                cues.len()
            );
            assert_eq!(
                out.cues.len(),
                cues.len(),
                "refill must carry exactly the tick's cues"
            );
        }
        assert_eq!(out.cues, burst[6..]);
        assert!(!out.audio.is_empty(), "final refill did real work");
    }

    #[test]
    fn tick_into_matches_tick_byte_for_byte() {
        // Production wiring proof: the reused-outcome path returns the same
        // bytes as the owned path on accepted, rejected and empty ticks.
        let mut fresh = App::new(small_config()).unwrap();
        let mut reused = App::new(small_config()).unwrap();
        let mut scratch = TickOutcome::default();
        for tick in 0..32u32 {
            let expected = fresh.tick(idle(tick));
            reused.tick_into(idle(tick), &mut scratch);
            assert_eq!(scratch, expected, "tick {tick} must refill identically");
        }
        // Rejected inputs repeat the last accepted data on both paths.
        let bad = FighterInput {
            fighter: FighterId::new(1).unwrap(),
            ..idle(0)
        };
        let expected = fresh.tick(bad);
        reused.tick_into(bad, &mut scratch);
        assert_eq!(scratch, expected, "rejected ticks must refill identically");
        assert!(!scratch.input_accepted);
    }

    #[test]
    fn populated_hud_sync_reuses_buffers() {
        // sync_hud paths fighters/feed/map rows: pre-fill both sides fully,
        // then prove the sync performs zero heap allocations. Production
        // never populates these rows today (upsert_* uncalled outside ui
        // tests), so this guards the mechanism, not a live path.
        use engine_ui::{FighterHud, Hud};
        let mut src = Hud::new(0, 1000, 1000, 0, 0);
        let mut dst = Hud::new(0, 1000, 1000, 0, 0);
        for slot in 0..16u8 {
            let id = FighterId::new(slot).expect("slot in cap");
            src.upsert_fighter(FighterHud::new(id, 1000, 1000)).unwrap();
            if slot < 8 {
                src.upsert_dot(id, 1.0, 2.0).unwrap();
            }
        }
        sync_hud(&mut dst, &src);
        let ((), steady) = alloc_probe::measure(|| {
            sync_hud(&mut dst, &src);
            std::hint::black_box(&dst);
        });
        assert_eq!(steady.calls, 0, "populated sync must not alloc");
        assert_eq!(dst.fighters.len(), 16);
    }

    /// Full replay spelling for the first `ticks` of a session's tape: the
    /// legs the rolling cursor must stay byte-equal to.
    fn replay_reference(app: &App, ticks: u32) -> PresentationData {
        App::prefix_presentation(&app.arena, &app.tape, ticks)
    }

    #[test]
    fn rolling_cursor_matches_full_replay_tick_for_tick() {
        let config = GameConfig {
            ticks: 48,
            ..small_config()
        };
        let mut app = App::new(config).unwrap();
        assert!(app.pose_cursor().is_empty());
        assert_eq!(app.presentation_len(), 0);
        for tick in 0..config.ticks {
            let out = app.tick(assault(tick));
            assert!(out.input_accepted, "assault inputs validate");
            assert_eq!(
                app.presentation_len(),
                tick + 1,
                "cursor covers the accepted tick"
            );
            assert_eq!(
                app.pose_cursor().len(),
                usize::from(config.fighters),
                "one latest pose per fighter"
            );
            let expected = replay_reference(&app, tick + 1);
            assert_eq!(
                app.presentation(),
                expected,
                "rolling tracks diverge at tick {tick}"
            );
            assert_eq!(
                app.presentation_ref(),
                &expected,
                "reference read diverges at tick {tick}"
            );
            let tails: Vec<LocalPose> = expected
                .pose_tracks
                .iter()
                .map(|track| track.last().cloned().expect("covered tick has a pose"))
                .collect();
            assert_eq!(app.pose_cursor(), &*tails);
        }
        // P2P twice: a twin session serves byte-equal poses.
        let mut twin = App::new(config).unwrap();
        for tick in 0..config.ticks {
            twin.tick(assault(tick));
        }
        assert_eq!(app.presentation(), twin.presentation());
        assert_eq!(app.pose_cursor(), twin.pose_cursor());
    }

    #[test]
    fn pose_cursor_advances_only_on_accepted_ticks() {
        let mut app = App::new(small_config()).unwrap();
        let first = app.tick(idle(0));
        assert!(first.input_accepted);
        let held = app.pose_cursor().to_vec();
        assert_eq!(held.len(), usize::from(small_config().fighters));
        // Rejected input: the cursor and the presentation both hold.
        let bad = FighterInput {
            fighter: FighterId::new(1).unwrap(),
            ..idle(0)
        };
        let rejected = app.tick(bad);
        assert!(!rejected.input_accepted);
        assert_eq!(app.cursor(), 1);
        assert_eq!(app.presentation_len(), 1);
        assert_eq!(app.pose_cursor(), &*held);
        // AI-filled ticks advance too.
        let out = app.tick_empty();
        assert!(out.input_accepted);
        assert_eq!(app.presentation_len(), 2);
        assert_eq!(
            app.pose_cursor().len(),
            usize::from(small_config().fighters)
        );
        assert_eq!(app.presentation(), replay_reference(&app, 2));
    }

    #[test]
    fn frame_reads_allocate_nothing_at_any_cursor() {
        // Calibration: the shared probe must observe a real allocation,
        // otherwise a zero reading would prove nothing.
        let (_, probe) = alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(probe.calls >= 1, "alloc probe is blind");

        let mut app = App::new(small_config()).unwrap();
        for cursor in [16u32, 48u32] {
            while app.cursor() < cursor {
                let tick = app.cursor();
                assert!(app.tick(idle(tick)).input_accepted);
            }
            assert_eq!(app.presentation_len(), cursor);
            // The frame path (latest poses, reference tracks, length)
            // performs zero heap allocations at either cursor: flat cost.
            let ((), steady) = alloc_probe::measure(|| {
                std::hint::black_box(app.pose_cursor());
                std::hint::black_box(app.presentation_ref());
                std::hint::black_box(app.presentation_len());
            });
            assert_eq!(steady.calls, 0, "frame reads allocated at cursor {cursor}");
            assert!(!app.pose_cursor().is_empty());
        }
        // `pose` serves the same latest pose without the full tracks.
        assert_eq!(
            app.pose(PoseSource::LocoPresentation).unwrap(),
            app.pose_cursor().first().cloned()
        );
    }

    #[test]
    fn window_frame_serves_cursor_and_empty_before_tick0() {
        // Failure case first: no frame before tick 0.
        let fresh = App::new(small_config()).unwrap();
        assert!(fresh.window_frame_poses().is_empty());
        let mut app = App::new(small_config()).unwrap();
        assert!(app.tick(idle(0)).input_accepted);
        // The window entry serves the cursor slice after every tick.
        assert_eq!(app.window_frame_poses(), app.pose_cursor());
        assert_eq!(
            app.window_frame_poses().len(),
            usize::from(small_config().fighters)
        );
        assert_eq!(
            app.pose(PoseSource::LocoPresentation).unwrap(),
            app.window_frame_poses().first().cloned()
        );
    }

    #[test]
    fn layered_extract_env_beats_file_and_defaults() {
        let defaults = GameConfig::default();
        let file_rows = [
            ("seed", "7"),
            ("ticks", "64"),
            ("fighters", "4"),
            ("half_extent_m", "12.0"),
        ];
        let env_rows = [
            ("seed", "99"),
            ("ticks", "120"),
            ("fighters", "5"),
            ("half_extent_m", "30.0"),
        ];
        let merged = layered_extract(defaults, &file_rows, &env_rows);
        assert_eq!(merged.seed, 99);
        assert_eq!(merged.ticks, 120);
        assert_eq!(merged.fighters, 5);
        assert_eq!(merged.half_extent_m, 30.0);
        assert!(merged.validate().is_ok());
        assert!(App::new(merged).is_ok());
    }

    #[test]
    fn layered_extract_file_beats_defaults() {
        let defaults = GameConfig::default();
        let file_rows = [
            ("seed", "7"),
            ("ticks", "64"),
            ("fighters", "4"),
            ("half_extent_m", "12.0"),
        ];
        let empty: [(&str, &str); 0] = [];
        let merged = layered_extract(defaults, &file_rows, &empty);
        assert_eq!(merged.seed, 7);
        assert_eq!(merged.ticks, 64);
        assert_eq!(merged.fighters, 4);
        assert_eq!(merged.half_extent_m, 12.0);
        assert!(merged.validate().is_ok());
    }

    #[test]
    fn layered_extract_defaults_when_layers_empty() {
        let defaults = GameConfig::default();
        let empty: [(&str, &str); 0] = [];
        let merged = layered_extract(defaults, &empty, &empty);
        assert_eq!(merged, defaults);
        assert!(merged.validate().is_ok());
    }

    #[test]
    fn layered_extract_bad_env_row_falls_back() {
        let defaults = GameConfig::default();
        let file_rows = [
            ("seed", "7"),
            ("ticks", "64"),
            ("fighters", "4"),
            ("half_extent_m", "12.0"),
        ];
        let env_rows = [
            ("seed", "99"),
            ("ticks", "not-a-number"),
            ("fighters", "99"),
            ("half_extent_m", "NaN"),
        ];
        let merged = layered_extract(defaults, &file_rows, &env_rows);
        assert_eq!(merged.seed, 99);
        assert_eq!(merged.ticks, 64);
        assert_eq!(merged.fighters, 4);
        assert_eq!(merged.half_extent_m, 12.0);
        assert!(merged.validate().is_ok());
        assert!(App::new(merged).is_ok());
    }

    #[test]
    fn layered_extract_report_names_every_refused_row() {
        let defaults = GameConfig::default();
        let file_rows = [
            ("seed", "7"),
            ("ticks", "64"),
            ("fighters", "4"),
            ("half_extent_m", "12.0"),
        ];
        let env_rows = [
            ("seed", "not-a-number"),
            ("ticks", "not-a-number"),
            ("fighters", "99"),
            ("half_extent_m", "NaN"),
            ("mode", "arena"),
        ];
        let (merged, refused) = layered_extract_report(defaults, &file_rows, &env_rows);
        assert_eq!(merged.seed, 7);
        assert_eq!(merged.ticks, 64);
        assert_eq!(merged.fighters, 4);
        assert_eq!(merged.half_extent_m, 12.0);
        assert_eq!(
            refused,
            vec!["seed", "ticks", "fighters", "half_extent_m", "unknown"]
        );
        assert!(merged.validate().is_ok());
        assert!(App::new(merged).is_ok());
    }

    #[test]
    fn layered_extract_report_empty_when_layers_clean() {
        let defaults = GameConfig::default();
        let file_rows = [
            ("seed", "7"),
            ("ticks", "64"),
            ("fighters", "4"),
            ("half_extent_m", "12.0"),
        ];
        let env_rows = [
            ("seed", "99"),
            ("ticks", "120"),
            ("fighters", "5"),
            ("half_extent_m", "30.0"),
        ];
        let (merged, refused) = layered_extract_report(defaults, &file_rows, &env_rows);
        assert_eq!(merged.seed, 99);
        assert_eq!(merged.ticks, 120);
        assert_eq!(merged.fighters, 5);
        assert_eq!(merged.half_extent_m, 30.0);
        assert!(refused.is_empty());
        assert!(merged.validate().is_ok());
        assert!(App::new(merged).is_ok());
    }

    #[test]
    fn fighter_views_match_sim_positions_every_tick() {
        // VS4 F2P half: a 2-fighter 120-tick run serves one view per fighter
        // after every tick, byte-equal to the dataset rows (slot, feet
        // position, yaw). Empty before tick 0.
        let config = GameConfig {
            ticks: 120,
            ..small_config()
        };
        let mut app = App::new(config).unwrap();
        assert!(app.fighter_views().is_empty());
        for tick in 0..config.ticks {
            assert!(app.tick(idle(tick)).input_accepted);
            let views = app.fighter_views();
            assert_eq!(views.len(), 2, "one view per fighter at tick {tick}");
            let short = ArenaConfig {
                ticks: tick + 1,
                ..app.arena
            };
            let dataset = run_dataset(&short, &app.tape).expect("dataset replays");
            let row = &dataset.rows[usize::try_from(tick).unwrap() * 2];
            for (view, entry) in views.iter().zip(row.observation.fighters.as_slice()) {
                assert_eq!(view.slot, entry.id, "slot order at tick {tick}");
                assert_eq!(view.pos_m, entry.pos, "position at tick {tick}");
                assert_eq!(view.yaw, entry.yaw, "yaw at tick {tick}");
            }
        }
    }

    /// One oracle tick: checkpoint, cues, laterals and views.
    type OracleTick = (
        Checkpoint,
        Vec<FightCue>,
        Vec<FighterLateral>,
        Vec<FighterView>,
    );

    /// The pre-EG1 `simulate_tick`, kept as the oracle: three whole-prefix
    /// replays of `0..=tick` over the session's tape.
    fn prefix_oracle(app: &App, tick: u32) -> OracleTick {
        let fighters = app.config.fighters;
        let (_, checkpoints) = run_prefix(&app.arena, &app.tape, tick + 1).expect("prefix");
        let short = ArenaConfig {
            ticks: tick + 1,
            ..app.arena
        };
        let cues = run_events(&short, &app.tape)
            .last()
            .map(|row| row.events.iter().map(map_cue).collect())
            .unwrap_or_default();
        let dataset = run_dataset(&short, &app.tape).expect("dataset replays");
        let base = usize::try_from(tick).unwrap() * usize::from(fighters);
        let rows = &dataset.rows[base..];
        (
            *checkpoints.last().expect("one checkpoint per tick"),
            cues,
            laterals_for_tick(rows, tick, fighters).expect("laterals"),
            views_for_tick(rows, tick, fighters).expect("views"),
        )
    }

    #[test]
    fn live_match_equals_prefix_replay_oracle_600_ticks() {
        // EG1 parity: per-tick checkpoints, cues, laterals and views from the
        // incremental live match equal the old three prefix replays, at 2
        // and 8 fighters, with a LAN peer staging slot 1 every 7th tick.
        for fighters in [2u8, 8] {
            let config = GameConfig {
                fighters,
                ..config()
            };
            let mut app = App::new(config).unwrap();
            let mut fought = false;
            for tick in 0..config.ticks {
                if tick.is_multiple_of(7) {
                    let peer = FighterInput {
                        fighter: FighterId::new(1).unwrap(),
                        mx: -50,
                        mz: 25,
                        ..idle(tick)
                    };
                    app.offer_remote(peer).unwrap();
                }
                let outcome = app.tick(script(tick));
                assert!(outcome.input_accepted, "tick {tick} at {fighters} fighters");
                let (checkpoint, cues, laterals, views) = prefix_oracle(&app, tick);
                assert_eq!(outcome.checkpoint, Some(checkpoint), "checkpoint {tick}");
                assert_eq!(outcome.cues, cues, "cues {tick}");
                assert_eq!(app.laterals, laterals, "laterals {tick}");
                assert_eq!(app.fighter_views(), views.as_slice(), "views {tick}");
                fought |= !cues.is_empty();
            }
            assert!(fought, "the {fighters}-fighter match lands cues");
            assert_eq!(app.sim_steps(), u64::from(config.ticks));
        }
    }

    #[test]
    fn ticking_600_times_costs_exactly_600_sim_steps() {
        // EG1 cost proof without a wall clock: one arena step per accepted
        // tick (the old prefix replays cost 3 * (tick + 1) steps per tick,
        // 540,900 over 600 ticks). Rejected ticks step nothing.
        let mut app = App::new(config()).unwrap();
        assert_eq!(app.sim_steps(), 0);
        for tick in 0..TICKS {
            assert!(app.tick(script(tick)).input_accepted);
            assert_eq!(app.sim_steps(), u64::from(tick) + 1, "tick {tick}");
        }
        assert!(!app.tick(script(TICKS)).input_accepted, "match over");
        assert_eq!(app.sim_steps(), u64::from(TICKS));
    }

    #[test]
    fn stale_live_match_rebuilds_from_tape_byte_identically() {
        // The rare path: a live match out of step with the cursor rebuilds
        // from tick 0 over the tape, then continues identically.
        let config = small_config();
        let (reference, reference_out) = run_session(config);
        let mut app = App::new(config).unwrap();
        let mut outs = Vec::new();
        for tick in 0..config.ticks {
            if tick == 20 {
                app.live = LiveMatch::with_rules(&app.arena, app.rules).unwrap();
            }
            outs.push(app.tick(script(tick)));
        }
        assert_eq!(outs, reference_out);
        assert_eq!(app.checkpoints(), reference.checkpoints());
        assert_eq!(app.fighter_views(), reference.fighter_views());
        // The hand swap above dropped the first 20 steps unbilled; the
        // rebuild then replays 20 ticks and steps the 44 left.
        assert_eq!(app.sim_steps(), u64::from(config.ticks));
    }

    /// Presentation sim steps the session paid (EG1b cost proof).
    fn presentation_steps(app: &App) -> u64 {
        app.present_retired_steps
            .saturating_add(app.present_sim.as_ref().map_or(0, LivePresentation::steps))
    }

    #[test]
    fn live_presentation_equals_prefix_replay_oracle_600_ticks() {
        // EG1b parity: with the cursor on (the default), the cached tracks,
        // root tables and latest poses equal the old two whole-prefix legs
        // after every tick, at 2 and 8 fighters, LAN peer on slot 1.
        for fighters in [2u8, 8] {
            let config = GameConfig {
                fighters,
                ..config()
            };
            let mut app = App::new(config).unwrap();
            for tick in 0..config.ticks {
                if tick.is_multiple_of(7) {
                    let peer = FighterInput {
                        fighter: FighterId::new(1).unwrap(),
                        mx: -50,
                        mz: 25,
                        ..idle(tick)
                    };
                    app.offer_remote(peer).unwrap();
                }
                assert!(app.tick(script(tick)).input_accepted);
                let expected = replay_reference(&app, tick + 1);
                assert_eq!(expected.pose_tracks.len(), usize::from(fighters));
                assert_eq!(app.presentation_ref(), &expected, "tick {tick}");
                let tails: Vec<LocalPose> = expected
                    .pose_tracks
                    .iter()
                    .map(|track| track.last().cloned().expect("covered tick"))
                    .collect();
                assert_eq!(app.pose_cursor(), tails.as_slice(), "tails {tick}");
            }
            assert_eq!(presentation_steps(&app), u64::from(config.ticks));
        }
    }

    #[test]
    fn presentation_costs_one_step_per_tick_and_rebuilds_only_when_turned_back_on() {
        // EG1b cost proof without a wall clock: one presentation step per
        // accepted tick (the old cursor replayed the prefix twice per tick,
        // 2 * (tick + 1) record steps). Turning the cursor off and back on
        // is the one replay path: it rebuilds once, then steps singly again.
        let mut app = App::new(config()).unwrap();
        for tick in 0..TICKS / 2 {
            assert!(app.tick(script(tick)).input_accepted);
            assert_eq!(presentation_steps(&app), u64::from(tick) + 1, "tick {tick}");
        }
        app.set_presentation_cursor(false);
        assert!(app.tick(script(TICKS / 2)).input_accepted);
        assert_eq!(presentation_steps(&app), u64::from(TICKS / 2));
        app.set_presentation_cursor(true);
        let before = presentation_steps(&app);
        assert!(app.tick(script(TICKS / 2 + 1)).input_accepted);
        // Rebuild replays TICKS / 2 + 1 ticks, then steps the new one.
        assert_eq!(presentation_steps(&app), before + u64::from(TICKS / 2 + 2));
        for tick in TICKS / 2 + 2..TICKS {
            let paid = presentation_steps(&app);
            assert!(app.tick(script(tick)).input_accepted);
            assert_eq!(presentation_steps(&app), paid + 1, "tick {tick}");
        }
        assert_eq!(app.presentation_ref(), &replay_reference(&app, TICKS));
        assert_eq!(app.sim_steps(), u64::from(TICKS));
    }

    #[test]
    fn presentation_frame_cost_flat_to_18k_bench() {
        // Rolling-cursor accept bench: frame-read cost at tick 18,000 within
        // 2 times the cost at tick 10. Frame reads are slice borrows, so this seeds
        // a true 18,000-tick cache from one real pose and measures the
        // reads directly. Ticking 18k times through `App::tick` stays out:
        // each extension still replays the prefix (the arena keeps no
        // single-tick pose step), so that loop is O(n^2) by construction.
        use engine_anim::contract::RootDelta;
        const BENCH_TICKS: u32 = 18_000;
        const NEAR_TICKS: u32 = 10;
        const READS: u32 = 100_000;
        let (_, probe) = alloc_probe::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(probe.calls >= 1, "alloc probe is blind");

        let mut near = App::new(small_config()).unwrap();
        while near.cursor() < NEAR_TICKS {
            let tick = near.cursor();
            assert!(near.tick(idle(tick)).input_accepted);
        }
        assert_eq!(near.presentation_len(), NEAR_TICKS);

        let mut far = App::new(small_config()).unwrap();
        while far.cursor() < NEAR_TICKS {
            let tick = far.cursor();
            assert!(far.tick(idle(tick)).input_accepted);
        }
        let seed_pose = far
            .pose_cursor()
            .first()
            .cloned()
            .expect("covered tick has a pose");
        let seed_table = far
            .presentation_ref()
            .root_tables
            .first()
            .cloned()
            .unwrap_or_default();
        let seed_delta = seed_table.deltas.last().copied().unwrap_or(RootDelta {
            tick: 0,
            dp: [0.0, 0.0],
            dyaw: 0.0,
        });
        let fighters = usize::from(small_config().fighters);
        let long = usize::try_from(BENCH_TICKS).unwrap_or(0);
        let mut tracks = Vec::with_capacity(fighters);
        let mut tables = Vec::with_capacity(fighters);
        let mut tails = Vec::with_capacity(fighters);
        for _ in 0..fighters {
            tracks.push(vec![seed_pose.clone(); long]);
            let mut table = seed_table.clone();
            table.deltas.clear();
            table.deltas.extend((0..BENCH_TICKS).map(|tick| RootDelta {
                tick,
                dp: seed_delta.dp,
                dyaw: seed_delta.dyaw,
            }));
            tables.push(table);
            tails.push(seed_pose.clone());
        }
        far.present = PresentationData {
            pose_tracks: tracks,
            root_tables: tables,
        };
        far.present_latest = tails;
        far.present_ticks = BENCH_TICKS;
        assert_eq!(far.presentation_len(), BENCH_TICKS);
        assert_eq!(far.window_frame_poses(), far.pose_cursor());

        let ((), near_probe) = alloc_probe::measure(|| {
            for _ in 0..READS {
                std::hint::black_box(near.pose_cursor());
                std::hint::black_box(near.presentation_ref());
                std::hint::black_box(near.window_frame_poses());
                std::hint::black_box(near.presentation_len());
            }
        });
        let ((), far_probe) = alloc_probe::measure(|| {
            for _ in 0..READS {
                std::hint::black_box(far.pose_cursor());
                std::hint::black_box(far.presentation_ref());
                std::hint::black_box(far.window_frame_poses());
                std::hint::black_box(far.presentation_len());
            }
        });
        assert_eq!(near_probe.calls, 0, "frame reads allocated at tick 10");
        assert_eq!(far_probe.calls, 0, "frame reads allocated at tick 18000");

        let start = std::time::Instant::now();
        for _ in 0..READS {
            std::hint::black_box(near.pose_cursor());
            std::hint::black_box(near.presentation_ref());
            std::hint::black_box(near.window_frame_poses());
            std::hint::black_box(near.presentation_len());
        }
        let near_elapsed = start.elapsed();
        let start = std::time::Instant::now();
        for _ in 0..READS {
            std::hint::black_box(far.pose_cursor());
            std::hint::black_box(far.presentation_ref());
            std::hint::black_box(far.window_frame_poses());
            std::hint::black_box(far.presentation_len());
        }
        let far_elapsed = start.elapsed();
        assert!(
            far_elapsed <= near_elapsed * 2 + std::time::Duration::from_millis(50),
            "18k frame reads took {far_elapsed:?} vs 10-tick {near_elapsed:?}"
        );
    }

    #[test]
    fn crowd_levels_export_64_in_range_and_repeat() {
        // Behavior: the staged helper copies the room's 64 levels through the
        // new `crowd_levels` caller; levels sit in 300..=1000 and repeat.
        let mut app = App::new(small_config()).unwrap();
        for tick in 0..3u32 {
            assert!(app.tick(idle(tick)).input_accepted);
        }
        let mut first = [0u16; crate::crowd_gpu::CROWD_BLOCKS];
        let n = app.crowd_levels(&mut first).expect("64 fit");
        assert_eq!(n, crate::crowd_gpu::CROWD_BLOCKS);
        for level in first.iter() {
            assert!(
                (*level >= crate::crowd_gpu::BASELINE_PERMILLE)
                    && (*level <= crate::crowd_gpu::PEAK_PERMILLE),
                "level {level} outside murmur..peak"
            );
        }
        let mut again = [0u16; crate::crowd_gpu::CROWD_BLOCKS];
        let m = app.crowd_levels(&mut again).expect("64 fit twice");
        assert_eq!(m, n);
        assert_eq!(first, again, "same room exports the same levels");
    }

    #[test]
    fn crowd_levels_short_buffer_refuses_loudly() {
        // Failure: a short caller buffer refuses with the want/got sizes,
        // never a truncation; the staged run maps this into `crowd: ...`.
        let app = App::new(small_config()).unwrap();
        let mut short = [0u16; 7];
        match app.crowd_levels(&mut short) {
            Err(crate::crowd_gpu::CrowdGpuError::BadBuffer { want, got }) => {
                assert_eq!(want, crate::crowd_gpu::CROWD_BLOCKS);
                assert_eq!(got, 7);
            }
            other => panic!("short crowd buffer must refuse, got {other:?}"),
        }
    }

    #[test]
    fn out_of_order_stage_keeps_tape_sorted() {
        // Tape-sort repair: out-of-order LAN arrivals insert sorted, so the
        // tape stays `(tick, fighter)`-sorted and validates.
        let mut app = App::new(small_config()).unwrap();
        let at = |tick: u32, fighter: u8| FighterInput {
            tick,
            fighter: FighterId::new(fighter).unwrap(),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        };
        app.stage_input(at(2, 1));
        app.stage_input(at(0, 0));
        app.stage_input(at(0, 1));
        app.stage_input(at(2, 0));
        app.stage_input(at(1, 0));
        let keys: Vec<(u32, u8)> = app
            .tape
            .0
            .iter()
            .map(|input| (input.tick, input.fighter.0))
            .collect();
        assert_eq!(
            keys,
            vec![(0, 0), (0, 1), (1, 0), (2, 0), (2, 1)],
            "tape must stay sorted"
        );
        assert!(app.tape.validate().is_ok());
        assert!(tape_should_append(&app.tape, (2, 1)));
        assert!(tape_should_append(&app.tape, (3, 0)));
        assert!(!tape_should_append(&app.tape, (1, 0)));
    }
}
