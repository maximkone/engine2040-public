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
/// Agent seat: one loopback client plays the human slot of the live window
/// and reads each fixed tick's hash back. Tool surface only, never hashed.
/// App leg only (the window owns the socket).
#[cfg(feature = "app")]
pub mod agent_seat;
/// Sim state to clip cues with a per-slot crossfade: the act pick plus the
/// fade driver the fighter look samples. Std-only, presentation only.
pub mod anim_bridge;
/// Announcer: bout voice lines as data, throttled per rule; never hashed.
pub mod announcer;
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
/// The boom's pillar filter (SW155): hold a graze, pull in fast, release slow.
/// Presentation only, never hashed. Std-only.
pub mod boom_filter;
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
/// Third-person fight framing: the bodies in a safe frame at a readable size, and its law.
pub mod camera_frame;
/// The director's shelf of shots: highest priority wins, a cut eases between shots,
/// and a dead zone holds the eye while the framed bodies stay inside it. Presentation
/// only, never hashed. Std-only.
pub mod camera_stack;
/// WASD and the stick relative to the camera: turns the bindings' local wish
/// into the world wish the sim takes. Input mapping only, never hashed. Std-only.
pub mod camera_wish;
/// The camp day is a works table: eight works spend man-ticks, the finished
/// work hands its prop to the next frame. Std-only, cold path per command.
pub mod camp_day;
/// Showcase clip capture: tape-bound PNG sequence plus manifest.
/// Headless-safe: flags, tape codec, std-only PNG writer and frame sink.
pub mod capture;
/// Class content loader: one class file in `content/classes/` gives one
/// sheet, with the schema rules the loader refuses instead of patching.
/// Std-only; setup IO, never on the tick.
pub mod class_sheet;
/// The run clip's authored footstep events, mapped onto `engine-anim`'s
/// frame-exact event lane and played by the audio feed. Std-only.
pub mod clip_events;
/// Dev console: read-only door verbs, pause/step/slow, set-knob; never hashed.
pub mod console;
/// Crash reports to a local file: version, adapter, tape tail,
/// checkpoint, loud exit. Std-only.
pub mod crash;
/// Reacting crowd sim: 20k-seat integer state feeding the instanced
/// stands through the render staging. Std-only; consumer is [`App`].
pub mod crowd_gpu;
/// SW134: the 96 impostor spectators answer the fight as a heat gauge with a
/// one-beat lag. Presentation only, never hashed.
pub mod crowd_mood;
/// Debug draw for the live fighters (C13): hurt-capsule wireframe plus a
/// facing line, off at the `debug-draw` knob. Needs the render camera, so it
/// rides `app`; presentation only, never hashed.
#[cfg(feature = "app")]
pub mod debug_lines;
/// Dev-only hot-patch seam for one gameplay point (practice tuning).
/// Std-only; consumer is the window `arena.rs::poll_hot_reload`.
pub mod dev_hotpatch;
/// Auto spectator camera (trailer cuts from fight beats): shot choice over
/// fixed-tick cues with continuity rules, presentation only. Std-only.
pub mod director;
/// Duel weapon arming (FEEL1b), kept out of `lib.rs` to hold its line pin.
mod duel_weapons;
/// The 6v6 faction match loop and its seats (RY5 of `docs/plan/ryl1.md`.
pub mod faction_match;
/// Landing plumbing: the fall table's rows into the tick outcome and HUD.
/// Std-only, presentation mirror, never hashed.
mod feel_land;
/// Feel flow view: hit-stop, slow-mo and camera shake from fight cues,
/// applied by the window. Presentation only. App leg only.
#[cfg(feature = "app")]
pub mod feel_view;
/// Fight ring: sand pit plus stone tiers under the look. App leg only.
#[cfg(feature = "app")]
pub mod fight_ring;
/// Staged fight: binds the content bundles to fighters and runs the
/// scripted bout headless (fighter_bind + staged_fight bin). Std-only.
pub mod fighter_bind;
#[cfg(feature = "app")]
pub mod fighter_bodies;
/// Fighter look: skinned CT7 bodies posed after each tick. App leg only.
#[cfg(feature = "app")]
pub mod fighter_look;
/// Finisher beat: the 4-tick freeze and the full-screen white overlay on a
/// killing blow (row SW22). Presentation only, pure integers.
pub mod finisher_beat;
/// Window match shell: Title, Match, Paused, Result over the arena match.
/// Std-only, pure transitions; pause holds the tick cursor.
pub mod flow;
pub mod foot_dust;
/// First-person eye pose beside camera: look clamp plus interp eye.
pub mod fps_view;
/// Guard pressure: the posture meter behind a block, its quiet heal and the
/// break that opens a riposte (row SW81). HUD only, never hashed.
pub mod guard_pressure;
/// Damage numbers in the world: the last blows float up and fade where they
/// landed (row SW151). Presentation only; the ring is never hashed.
pub mod hit_number;
/// The struck body's flinch picked by hit direction and weight
/// (`content/design/hit-reactions.rows`). Std-only, presentation only.
pub mod hit_reaction;
/// Sim freeze mirror for the look: the hitstop count combat sets on impact,
/// read-only for clip-clock holds. Std-only.
pub mod hitstop;
/// The moment of the hit as one authored sequence per impact class and weapon
/// (`content/design/impact.rows`), and hit-stop as a composable stack.
/// Std-only, presentation only.
pub mod impact_seq;
/// Engine-owned window input codes (game-agnostic key/mouse maps). Std-only.
pub mod input;
/// The arena kit loads through one queue: logical paths, a byte budget,
/// generational handles and unload. Std-only, no file system of its own.
pub mod kit_assets;
/// Ladder, glory ranks, repair-craft hooks LIGHT slice: integer
/// fold, replayable, nothing CUT. Std-only.
pub mod ladder;
/// The human fighter's soft lock-on: the `facing` toward the nearest living
/// rival that the window puts on the live input (and the spear demo script
/// aims with). Input mapping only, never hashed. Std-only.
pub mod lock_on;
/// The live tuning seam (SW32): the dev's rebuilt knob dylib, polled beside the
/// content poll. Presentation only, never hashed; `live` feature.
#[cfg(feature = "live")]
pub mod live_knobs;
/// Loop rows text loader: designers edit numbers, no Rust edit.
/// Std-only, cold path at a progression boundary.
pub mod loop_text;
/// Living ludus voices — trainer, blacksmith and crowd: fight memories
/// with goals over the district yard, scripted fallback offline. Std-only.
pub mod ludus_npcs;
/// The road march: the column on the road line, the pace of the step, the
/// stamina the pack load costs and the time skip to the next set piece.
/// Std-only, cold path at a chapter boundary.
pub mod march;
/// Display mode list plus validated swap keeping last-good: adapter modes
/// in, one staged swap out at a frame boundary. Std-only, presentation only.
pub mod modes;
/// Mode framework: arena, siege and district free-roam over the
/// ladder. Std-only.
pub mod modes_world;
pub mod mouse_raw;
/// One seat of a two-human match over UDP (LAN or relay), driven per frame by
/// the window or the headless proof loop. Std-only.
pub mod net_seat;
pub mod netgame;
/// Gamepad sticks plus buttons through the shared bindings table. Std-only.
pub mod pad;
/// The browser page's duel: its seed, its arena extent and the start it
/// gives. Std-only.
pub mod page_duel;
/// Photo mode (SW141): freeze the fight, fly a bounded damped camera, set the
/// lens and roll, and plan a supersampled still with a JSON sidecar. View
/// only: the sim holds its tick and never reads this module.
pub mod photo_mode;
/// Clip-clock hold: which fighter slots freeze while sim hitstop is above
/// zero. Std-only.
pub mod pose_hold;
/// Practice swap: staged combat tuning applied at a tick boundary. Std-only.
pub mod practice;
/// Practice dummy (SW80): guard, stance, spacing and recovery as data; one
/// `FighterInput` per tick through the existing seam. Std-only.
pub mod practice_plan;
/// The window's pressure watch (OPT27): presented frame times and the OS VRAM
/// reading into the arbiter, placement and GPU layer count out. App leg only:
/// the headless graph never arbitrates a local model.
#[cfg(feature = "app")]
pub mod pressure_watch;
pub mod progress;
/// Ragdoll stump caps: one dark-red cap draw where a go-down cue took an arm.
/// Presentation only, never hashed. App leg only.
#[cfg(feature = "app")]
pub mod ragdoll_stump_draw;
/// Hot-reload gate: staged generations applied at frame boundaries. Std-only.
pub mod reload;
/// The contubernium has names: seven named men with fights, kills, wounds,
/// rank and one diary line per fight, over `content/rome/contubernium.toml`.
/// The camp/roster view (rules v2) reads it; never hashed. Std-only.
pub mod roster;
/// Best-of-5 duel rounds over one-round arena sessions. Pure folds,
/// no hash input. Std-only.
pub mod rounds;
/// Roman architecture placement: the arches on the wall ring and the coliseum
/// backdrop, over the fight's own radius. Std-only table; the glTF import is
/// the `app` leg.
pub mod roman_dressing;
/// The steer half of the scene tools (rows SP62, EG35): the seat's `scene` and `steer` line
/// grammar, the camera pin with its undo stack and the named shots. App leg only.
#[cfg(feature = "app")]
pub mod scene_desk;
/// The live spatial scene report an agent reads (rows SP62, EG35): every body, prop and spear with
/// its place, screen rect, contacts, occluders and light, plus the view laws. App leg only.
#[cfg(feature = "app")]
pub mod scene_report;
/// The fight's numbers at the bell: hits, wounds, blocks, kills and the
/// longest chain per fighter, folded from the event tape. Presentation only.
pub mod scoreboard;
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
/// The showcase tour (`arena.exe --showcase`): a script that walks slot 0 through every system
/// the arena plays, one captioned station at a time, and grades each. Board: `docs/plan/showcase.md`.
pub mod showcase;
/// CPU pose step of the fighter look: clip sample plus joint bounds, no GPU.
pub mod stage_bodies;
/// Steamworks backend: achievements, stats, cloud generations and presence.
/// Std-only until the orchestrator lands the `steam` feature.
pub mod steam;
/// The swing clip's clock from the press to the hit (EG19c): the clip's impact
/// frame meets the blow the fight lands `startup` ticks after the press.
/// Std-only, presentation only.
pub mod swing_time;
/// Turn in place: each fighter's view yaw (the clamped bearing to the rival)
/// and the presentation root turn it drives (row SW40). Std-only, the
/// authority yaw is never touched.
pub mod turn_look;
/// View truth: every drawn body as numbers, never an image. App leg only.
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
/// A cut outlives the fight: four parts at three severities set from
/// `combat::cut` events and healed by a camp day, over
/// `content/rome/wound.toml`. A severed part never heals. The camp/roster
/// view (rules v2) reads it; never hashed. Std-only.
pub mod wounds;

#[cfg(test)]
mod fighter_draw;

use crate::audio_director::{drive, Director, DirectorError, DirectorState, Layers, TensionTable};
use crate::crowd_gpu::CrowdGpuSim;
use crate::director::{Director as CutDirector, DirectorConfig, FighterLateral, MIN_SHOT_TICKS};
use engine_anim::contract::{LocalPose, RootTable};
use engine_arena::recorder::Row;
use engine_arena::world::MatchWorld;
use engine_arena::{
    run_loco_poses, ArenaConfig, ArenaRules, Checkpoint, FallLanding, LiveMatch, LivePresentation,
    MatchSpec,
};
use engine_audio::combat_cues::{strike_gain, HIT_FULL_PERMILLE, HIT_GAIN_FLOOR};
use engine_audio::cue::{impact_cue, ImpactKind, ImpactSurface};
use engine_audio::{BankSound, Crowd, FightCue, Reaction};
use engine_combat::{fight::MAX_HP_PERMILLE, Cut, FightEvent};
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

/// The window kit's weapon row (FEEL1b): the legionary gladius, equal to the
/// shipped v2 table, so only the armed-weapon fold moves.
pub const DUEL_KIT_WEAPON: &str = crate::fps_view::LegionaryKit::LEGIONARY.weapon;

/// Session configuration: one human vs `fighters - 1` AI brains.
/// The schemars derive is the one key list: `deny_unknown_fields` names the
/// four rows and refuses any other in the generated JSON Schema.
#[derive(Clone, Copy, PartialEq, Debug, schemars::JsonSchema)]
#[schemars(deny_unknown_fields)]
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

/// Property names of the generated [`GameConfig`] JSON Schema, in schema
/// order. The schemars derive owns this list, so strict extract keeps no
/// second copy of the keys. Empty when the schema shape is unexpected, which
/// makes every row refuse as unknown instead of passing silently. Pure.
fn strict_schema_keys() -> Vec<String> {
    schemars::schema_for!(GameConfig)
        .as_object()
        .and_then(|root| root.get("properties"))
        .and_then(|properties| properties.as_object())
        .map(|properties| properties.keys().cloned().collect())
        .unwrap_or_default()
}

/// Refuse the first row whose key the [`GameConfig`] schema does not declare,
/// with `BadConfig("unknown")`. Pure.
fn refuse_unknown_layer_rows(rows: &[(&str, &str)]) -> Result<(), GameError> {
    let known = strict_schema_keys();
    for (key, _) in rows {
        if !known.iter().any(|known| known == key) {
            return Err(GameError::BadConfig("unknown"));
        }
    }
    Ok(())
}

/// Resolve one strict row: a present value in either layer must parse inside
/// its field range, then env wins over file over the default. A bad present
/// row refuses with its key instead of falling back a layer. Pure.
fn strict_layer_row<T: Copy>(
    default: T,
    file_rows: &[(&str, &str)],
    env_rows: &[(&str, &str)],
    key: &'static str,
    parse: fn(&str) -> Option<T>,
) -> Result<T, GameError> {
    let file = extract_inner(file_rows, key)
        .map(|raw| parse(&raw).ok_or(GameError::BadConfig(key)))
        .transpose()?;
    let env = extract_inner(env_rows, key)
        .map(|raw| parse(&raw).ok_or(GameError::BadConfig(key)))
        .transpose()?;
    Ok(env.or(file).unwrap_or(default))
}

/// Strict sibling of [`layered_extract`]: every present row must be a
/// [`GameConfig`] schema property and parse inside its field range; the first
/// bad row refuses with [`GameError::BadConfig`] (keys `seed`, `ticks`,
/// `fighters`, `half_extent_m`, then `unknown`) instead of falling back.
/// Precedence stays env over file over `defaults`; no IO, no clock.
pub fn layered_extract_strict(
    defaults: GameConfig,
    file_rows: &[(&str, &str)],
    env_rows: &[(&str, &str)],
) -> Result<GameConfig, GameError> {
    let seed = strict_layer_row(defaults.seed, file_rows, env_rows, "seed", parse_layer_seed)?;
    let ticks = strict_layer_row(
        defaults.ticks,
        file_rows,
        env_rows,
        "ticks",
        parse_layer_ticks,
    )?;
    let fighters = strict_layer_row(
        defaults.fighters,
        file_rows,
        env_rows,
        "fighters",
        parse_layer_fighters,
    )?;
    let half_extent_m = strict_layer_row(
        defaults.half_extent_m,
        file_rows,
        env_rows,
        "half_extent_m",
        parse_layer_extent,
    )?;
    refuse_unknown_layer_rows(file_rows)?;
    refuse_unknown_layer_rows(env_rows)?;
    let merged = GameConfig {
        seed,
        ticks,
        fighters,
        half_extent_m,
    };
    merged.validate()?;
    Ok(merged)
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
    /// height, weapons, world, delay or config): both match identities
    /// carried, nothing was paired.
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
    /// Placeholder mix voice (the headless [`Bank`](engine_audio::Bank) leg).
    pub sound: BankSound,
    /// Arena-bank cue the kind-by-surface table names for this blow
    /// (`engine_audio::cue::impact_cue`): the name a cue player resolves
    /// against the live [`CueBank`](engine_audio::CueBank). Presentation
    /// only: the sim event carries no surface field.
    pub cue: &'static str,
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
    /// Severed-joint cuts the authority resolved this tick, in resolver order; empty on quiet
    /// and rejected ticks. Presentation only: the arena already hashed them.
    pub cuts: Vec<Cut>,
    /// Crowd reaction per cue, in the same order as [`TickOutcome::cues`].
    pub reactions: Vec<Reaction>,
    /// Fall landings of the tick (FEEL7), slot order: the fall height the
    /// height channel carried and the fall table's health cost. The sim
    /// already charged the fight; this mirror lets the HUD show it and the
    /// look, camera and audio share the same landing tick. Presentation
    /// only: nothing here is hashed.
    pub lands: Vec<FallLanding>,
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
            cuts: Vec::new(),
            reactions: Vec::new(),
            lands: Vec::new(),
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
    /// Laquearius noose on (row SG11): set by [`App::into_noose`] before the first tick, so the
    /// noose key throws a loop that snares a man. Every live sim of the session is built with it.
    noose: bool,
    /// Height channel on (rows SG5/SG4): set by [`App::into_height`] before the
    /// first tick. Every live sim of the session is built with it, so the wall
    /// tether swings instead of zipping.
    height: bool,
    /// Per-slot weapon row ids armed in the duel (FEEL1b), in slot order;
    /// empty when bare. They ride [`App::spec`] into rebuilds and peers.
    weapons: Vec<&'static str>,
    /// Sim steps spent by retired live matches (rebuilds), so
    /// [`App::sim_steps`] counts every step the session ever paid.
    retired_steps: u64,
    checkpoints: Vec<Checkpoint>,
    hp: [u16; MAX_PARTICIPANTS as usize],
    /// Sim hitstop ticks left, read from the live fight after each step (the
    /// count the impact set: 3 light, 6 heavy, 8 parry under v2, 4 under v1).
    /// Presentation only, never hashed; the look and the feel hold while it is
    /// above zero.
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
    /// The tick's authority cut events, filled by [`App::simulate_tick`] and moved into
    /// [`TickOutcome::cuts`] by [`App::observe_into`]; empty between ticks.
    tick_cuts: Vec<Cut>,
    /// The tick's fall landings (FEEL7), filled by [`App::simulate_tick`]
    /// and moved into [`TickOutcome::lands`] by [`App::observe_into`]; empty
    /// between ticks.
    tick_lands: Vec<FallLanding>,
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
    /// When false, accepted ticks skip every presentation-only sim a
    /// drawing caller never reads: the rolling presentation cursor above,
    /// the reacting-crowd visual sim, the tension
    /// director and the spectator cut director (with the laterals only that
    /// camera reads). Set by [`App::without_presentation_extras`] before the
    /// first tick; on by default so every existing caller keeps today's
    /// bytes and every existing test its outcomes (cues, hp, views and the
    /// checkpoint hash are untouched either way).
    presentation_extras: bool,
    /// Incremental presentation (EG1b): one recording sim step per accepted
    /// tick appends to [`App::present`]. `None` until the first extension
    /// and while the cursor is dropped; rebuilt from the tape when stale.
    present_sim: Option<Box<LivePresentation>>,
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

    /// Turn the laquearius noose on (row SG11): the noose key throws a loop that snares a man
    /// (`docs/plan/spear-grapple.md`). Only before the first tick; later calls refuse with
    /// `BadConfig("noose")`. The match identity changes, so a LAN peer must ask for it too.
    pub fn into_noose(mut self) -> Result<Self, GameError> {
        if self.cursor != 0 || self.retired_steps != 0 || self.live.steps() != 0 {
            return Err(GameError::BadConfig("noose"));
        }
        self.noose = true;
        self.live = self.spec().live().ok_or(GameError::BadConfig("arena"))?;
        Ok(self)
    }

    /// True when the noose is on for this session.
    #[must_use]
    pub const fn noose(&self) -> bool {
        self.noose
    }

    /// Arm the fighter height channel (rows SG5 and SG4): fighters fall and
    /// land under gravity, and the wall tether swings instead of zipping.
    /// Only before the first tick; later calls refuse with
    /// `BadConfig("height")`. The match identity changes, so a LAN peer must
    /// ask for it too.
    pub fn into_height(mut self) -> Result<Self, GameError> {
        if self.cursor != 0 || self.retired_steps != 0 || self.live.steps() != 0 {
            return Err(GameError::BadConfig("height"));
        }
        self.height = true;
        self.live = self.spec().live().ok_or(GameError::BadConfig("arena"))?;
        Ok(self)
    }

    /// The match this session plays: config, rules and world as one value.
    /// The net server and client build from the same spec
    /// (`Server::from_spec`, `Client::from_spec`), so a peer given this
    /// value plays the match the window shows.
    #[must_use]
    pub fn spec(&self) -> MatchSpec {
        let spec = MatchSpec::new(self.arena, self.rules, self.world.clone());
        let spec = if self.spear { spec.with_spear() } else { spec };
        let spec = if self.noose { spec.with_noose() } else { spec };
        let spec = if self.height {
            spec.with_height()
        } else {
            spec
        };
        if self.weapons.is_empty() {
            spec
        } else {
            spec.with_weapons(&self.weapons)
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
            noose: false,
            height: false,
            weapons: Vec::new(),
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
            tick_cuts: Vec::new(),
            tick_lands: Vec::new(),
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
            presentation_extras: true,
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

    /// Sim hitstop ticks left, read from the live fight after each step: the
    /// count the impact set for the strike that landed (3 light, 6 heavy, 8
    /// parry under v2; 4 under v1), then one less per frozen tick. Read-only
    /// visibility for the look and the feel (both hold while this is above
    /// zero); presentation only, never hashed. Units: fixed ticks.
    ///
    /// Caller: `crates/game/src/bin/arena_frame/mod.rs` feeds it to the look
    /// and the feel each tick; `fighter_look_proofs.rs` polls it the same way.
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
        if self.presentation_extras {
            self.checkpoints.push(checkpoint);
        }
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
        if self.presentation_extras {
            self.checkpoints.push(checkpoint);
        }
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
        out.cuts.clear();
        out.lands.clear();
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
        let inputs = tick_inputs(&self.tape, tick);
        let stepped = self.live.step(inputs)?;
        let checkpoint = stepped.checkpoint;
        // Steady-state zero alloc: refill the scratch buffers in place
        // instead of collecting fresh vecs every tick.
        self.tick_cues.clear();
        self.tick_cues
            .extend(stepped.events.events.iter().map(map_cue));
        if self.presentation_extras {
            // Only the spectator cut camera reads laterals; the lean session
            // skips the scan and keeps the views the page draws from.
            fill_laterals_for_tick(stepped.rows, tick, fighters, &mut self.laterals);
        }
        fill_views_for_tick(stepped.rows, tick, fighters, &mut self.views);
        // The authority's break events for this tick, kept for `observe_into` to hand out.
        self.tick_breaks.clear();
        self.tick_breaks.extend_from_slice(&stepped.breaks.events);
        self.tick_cuts.clear();
        self.tick_cuts.extend_from_slice(&stepped.cuts.cuts);
        // The live fight's freeze after this tick (FEEL3): the sim's own count.
        let next_hitstop = self.live.hitstop();
        // First production read of the press-tick tell (EG19b): one starter
        // slot per fresh v2 attack, in fighter-ID order; empty under v1 rows
        // and on quiet ticks. Both borrows are shared, so the live match
        // lends its cue buffer beside the stepped tick. Presentation only,
        // never hashed.
        self.tick_started.clear();
        self.tick_started
            .extend(self.live.cues().iter().map(|cue| cue.attacker));
        feel_land::fill_tick_lands(self);
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
        for tick in 0..ticks {
            live.step(tick_inputs(&self.tape, tick))?;
        }
        let hitstop = live.hitstop();
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
                    cue: impact_cue(ImpactKind::Hit, ImpactSurface::Flesh),
                    // Same scale as the window (combat_cues::strike_gain): a
                    // light blow plays softer, weapon mass plugs in at FEEL1.
                    gain: strike_gain(HIT_GAIN_FLOOR, damage, HIT_FULL_PERMILLE),
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
                    cue: impact_cue(ImpactKind::Block, ImpactSurface::ShieldWood),
                    gain: 0.4,
                });
            }
            FightCue::Parried { .. } => {
                out.audio.push(AudioCommand {
                    tick,
                    sound: BankSound::Hit,
                    cue: impact_cue(ImpactKind::Parry, ImpactSurface::Plate),
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
                    cue: impact_cue(ImpactKind::Knockdown, ImpactSurface::Flesh),
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
        if self.presentation_extras {
            self.crowd_gpu.on_cue(cue);
        }
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
        out.cuts.clear();
        out.cuts.append(&mut self.tick_cuts);
        feel_land::apply_landings(self, out);
        out.audio.reserve(cues.len());
        out.reactions.reserve(cues.len());
        out.cues.reserve(cues.len());
        out.started.reserve(started.len());
        out.started.extend_from_slice(started);
        for cue in cues {
            self.fold_cue_into(tick, *cue, out);
        }
        self.crowd.tick();
        if self.presentation_extras {
            self.crowd_gpu.tick();
            self.director_layers = drive(&mut self.director, cues);
            self.drive_cut_director(tick, cues);
        }
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

    /// Drop every presentation-only per-tick sim a caller that only draws the
    /// match never reads: the rolling presentation cursor (one whole arena
    /// sim step per tick, the biggest one), the reacting-crowd visual sim,
    /// the tension director and the spectator cut camera (and the laterals
    /// only that camera reads). Call before the first tick; native windows
    /// and every existing caller keep them (the default).
    ///
    /// The match result is untouched: the authority step, its cues, hp,
    /// [`App::fighter_views`] and every checkpoint hash stay byte-identical
    /// (see `without_presentation_extras_keeps_the_match_bytes`). The audio
    /// crowd clock still ticks, so the outcome's reactions and bed gain keep
    /// their values; the outcome's visual/director fields carry their
    /// initial values ([`App::crowd_gpu`] clock frozen,
    /// [`App::director_layers`] the opener mix), and
    /// [`App::checkpoints`] stays empty: a caller that needs those wants the
    /// default instead.
    ///
    /// Caller: the browser page poses bodies from
    /// [`App::fighter_views`](crate::App::fighter_views), so it may ask for
    /// the lean session.
    #[must_use]
    pub fn without_presentation_extras(mut self) -> Self {
        self.set_presentation_cursor(false);
        self.presentation_extras = false;
        self
    }

    /// True (default) while accepted ticks drive the presentation-only sims;
    /// see [`App::without_presentation_extras`].
    #[must_use]
    pub const fn presentation_extras(&self) -> bool {
        self.presentation_extras
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
        self.present_sim = Some(Box::new(sim));
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

// The unit-test binary's one counting allocator (the gates measure through
// `engine_core::alloc_probe::measure`); `app` links engine-perf's same one.
#[cfg(all(test, not(feature = "app")))]
#[global_allocator]
static TEST_ALLOCATOR: engine_core::alloc_probe::CountingAlloc =
    engine_core::alloc_probe::CountingAlloc;

#[cfg(test)]
#[path = "feel7_tests.rs"]
mod feel7_tests;

#[cfg(test)]
#[path = "lib_tests.rs"]
mod tests;
