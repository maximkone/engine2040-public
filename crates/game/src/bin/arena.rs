//! arena.exe: window binary (winit loop, fixed-tick accumulator,
//! bindings input, cpal audio, renderer presenter HUD, `--frames N --exit` smoke).
//!
//! Consumer: operator play on the reference GPU; the editor launches this binary as
//! a child process. Units: fixed ticks at 60 Hz, presented frames per redraw,
//! wall elapsed in milliseconds. Determinism: the windowed ticks run the same
//! [`App`](engine_game::App) match as the headless leg, so the same idle tape
//! gives the same checkpoints. Errors: [`WinError`](engine_game::windowed::WinError);
//! no display fails loudly (headless smoke fallback under `--frames`), audio
//! loss degrades with a notice while the mixer keeps rendering.
//!
//! Crash and log: `main` installs the crash hook first (`engine_game::crash`).
//! Every status line goes through `say!`/`tell!`, which print unchanged and
//! mirror into `target/logs/arena-<UTC>.log`; a panic writes `crash-<UTC>.toml`
//! beside it and exits with the crash code.
//!
//! Presentation: each redraw paints the winit window through the
//! swapchain (`render_surface` over `Presenter::init_for_window`); an armed
//! capture takes its PNG bytes from a separate explicit offscreen render of
//! the same draw list, never from the surface receipt, so an offscreen PNG
//! can never stand in for the OS window. A window that cannot present fails
//! loudly. Automated smoke (`--exit`, `--timed-smoke`, `--mute`) never opens an audio
//! device and owns a 5 min wall-clock bound of its own (`--deadline-ms` overrides per run).

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use arena_args::{AGENT_SEAT_FLAG, VIEW_BRIEF_FLAG, VIEW_LINE_FLAG};
use engine_assets::watcher::{WatchSettings, Watcher};
use engine_audio::{AudioEngine, Bank, FeedTx, FightCue, FrameSource, Mixer};
use engine_game::agent_seat::Seat;
use engine_game::arena_kit::{
    kit_match_world_or_empty, kit_prop_rows, live_kit_notice, push_kit_draws, stage_kit_or_default,
    KitReport,
};
use engine_game::break_window::BreakScene;
use engine_game::camera::{free_cam_step, CameraMode, DirectorParams};
use engine_game::camera_wish::{ground_forward, world_wish};
use engine_game::capture;
use engine_game::console::DevConsole;
use engine_game::crash;
use engine_game::feel_view::FeelView;
use engine_game::fight_ring::{ring_prop_row, ring_wall_row};
use engine_game::fighter_look::{look_down_deg, shot_for_frame, CamFollow, FighterLook, LookFrame};
use engine_game::flow::{Flow, FlowEvent};
use engine_game::fps_view::{LegionaryKit, LegionaryView};
use engine_game::input::{map_key, map_mouse};
use engine_game::lock_on::nearest_rival_facing;
use engine_game::modes::{rank_modes, validate_request, AdapterMode, DisplayMode, SwapRequest};
use engine_game::mouse_raw::MouseRaw;
use engine_game::practice::PracticeSwap;
use engine_game::pressure_watch::PressureWatch;
use engine_game::reload::{settle_paths, HotReload};
use engine_game::spear_window::{demo_input, SpearMode, SpearScene};
use engine_game::view_truth::{brief_line_with_props, FrameLight, PropTruth, ViewLaws};
use engine_game::virt::{speed_label, step_speed, VirtualClock, KEY_RAMP_MS};
use engine_game::window_audio_space::AudioSpace;
use engine_game::windowed::{
    apply_camera_orbit, cap_message as capped, contact_shadow_strength, dressing_camera,
    feed_tick_audio, fighter_input_from_pressed, format_timed_smoke, hud_cells_quads,
    hud_draw_list, live_fight_dots, msaa_setting, push_fight_draws, smoke_muted, split_staged_flag,
    stage_arena_bank, stage_dressing, stage_fight_bodies, staged_fight_input,
    staged_session_config, surface_frame_action, StagedTally, SurfaceFrameAction, TickAccumulator,
    WinArgs, WinError, WindowPad, CONTACT_SHADOW_ENV, MSAA_ENV, WINDOWED_SAMPLE_RATE,
    WIN_FIXED_DT_S,
};
use engine_game::{App, GameConfig, TickOutcome};
use engine_input::{Bindings, FighterId, FighterInput, PhysicalButton};
use engine_render::present::{DrawItem, DrawList, MeshKey, PresentError, Presenter, WaitStats};

use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

/// Status line on stdout: `println!` output, also mirrored into the run log.
macro_rules! say {
    ($($arg:tt)*) => {{
        let line = format!($($arg)*);
        println!("{line}");
        engine_game::crash::mirror_line(&line);
    }};
}

/// Status line on stderr: `eprintln!` output, also mirrored into the run log.
macro_rules! tell {
    ($($arg:tt)*) => {{
        let line = format!($($arg)*);
        eprintln!("{line}");
        engine_game::crash::mirror_line(&line);
    }};
}

/// Window console wiring: Backquote, line routing, the live door and effects.
mod console;

mod cursor;

/// GS7: the discrete-GPU hint of NVIDIA Optimus, read by the driver from the exe's export table
/// (PORT(glfw/glfw@master:src/win32_init.c)). `build.rs` keeps the export; `tools/ship.mjs`
/// refuses a run where the Intel adapter still won.
#[cfg(all(target_os = "windows", target_env = "msvc"))]
#[expect(
    unsafe_code,
    reason = "the dGPU flag must be an exported symbol, the GLFW win32_init pattern"
)]
#[used]
#[export_name = "NvOptimusEnablement"]
static NV_OPTIMUS_ENABLEMENT: u32 = 1;
/// GS7: the same hint for AMD switchable graphics.
#[cfg(all(target_os = "windows", target_env = "msvc"))]
#[expect(
    unsafe_code,
    reason = "the dGPU flag must be an exported symbol, the GLFW win32_init pattern"
)]
#[used]
#[export_name = "AmdPowerXpressRequestHighPerformance"]
static AMD_POWER_XPRESS_REQUEST_HIGH_PERFORMANCE: u32 = 1;

mod arena_args;
mod arena_capture;
mod arena_frame;
mod arena_net;
mod arena_winit;
mod frame_timing;
mod headless;

use arena_capture::capture::{
    frame_list, headless_fallback, headless_gate, run_chart_mode, run_reference_mode,
    setup_capture, split_chart_args, split_reference_args, CaptureSlot,
};
use frame_timing::{
    check_exit_wall_deadline, duration_us, frame_pacer, frame_trace_path, write_frame_times_csv,
};
use headless::split_plain_headless;

/// A fresh fight camera follower, snapping to the next shot (a new window or a new bout).
/// AIN6 (owner 2026-10-20 "ARM GOO ALL"): the 0.5 s velocity lead is armed.
fn new_cam_follow() -> Result<CamFollow, WinError> {
    CamFollow::new()
        .map(CamFollow::with_look_ahead)
        .map_err(|err| WinError::Render(format!("fight camera: {err}")))
}

/// QL12: one line of per-tier particle counts: each quality tier's pool
/// budget and how it splits over dust, flame, sparks and blood. Pure data
/// from the shipped tables, so a capture names the caps its particle pass
/// ran under. Cold path only.
fn particle_tier_line() -> String {
    use engine_render::particles_gpu::KindCaps;
    use engine_render::quality::{knobs, particle_pool_slots, Quality};
    let mut line = String::from("arena particles: per-tier");
    for tier in Quality::ALL {
        let total = particle_pool_slots(&knobs(tier));
        let caps = KindCaps::for_total(total);
        line.push_str(&format!(
            " {}={} dust={} flame={} sparks={} blood={}",
            tier.name(),
            total,
            caps.dust,
            caps.flame,
            caps.sparks,
            caps.blood
        ));
    }
    line
}

/// Default session seed for windowed play.
const DEFAULT_SEED: u64 = 2040;
/// Default session length in fixed ticks when `--ticks` is absent.
const DEFAULT_TICKS: u32 = 600;
/// Flag that turns the look probes on in the window (default off): the
/// bind probes, the IBL chart and the fog plus GTAO readback tranche, each a
/// blocking GPU readback that feeds no pixel.
const LOOK_PROBES_FLAG: &str = "--look-probes";
/// `--play`: boot straight into the bout instead of waiting at Title for
/// Enter, so a double-click launcher shows a fight at once. Presentation only.
const PLAY_FLAG: &str = "--play";
/// `--spear`: the spear grapple on (`docs/plan/spear-grapple.md`): Q throws a spear that tethers
/// a fighter or a wall, and the pair flies as ragdolls. Changes the match identity.
const SPEAR_FLAG: &str = "--spear";
/// `--spear-demo`: `--spear` plus a script for the human slot that walks at the nearest
/// fighter and throws whenever the spear is ready (captures, trying the grapple hands-off).
const SPEAR_DEMO_FLAG: &str = "--spear-demo";
/// `--speed-demo`: fly or drive ever faster over a generated terrain and cut towers in half
/// with a giant sword (docs/SPRINT-33-speed-demo.md). Presentation only.
const SPEED_DEMO_FLAG: &str = "--speed-demo";
/// `--height`: the fighter height channel on (rows SG5 and SG4): fighters fall
/// and land under gravity and the wall tether swings instead of zipping.
const HEIGHT_FLAG: &str = "--height";
/// `--legionary`: the legionary mode. The duel starts with slot 0 as the
/// player, seen in first person from the fighter's eye, wearing the galea,
/// gladius and scutum kit on the rules v2 tables. The third person view stays
/// one key away for the replay. View and kit only: the match, its rules and
/// its hash are the ordinary duel.
const LEGIONARY_FLAG: &str = "--legionary";
/// The key that swaps the legionary mode between the eye and the fight
/// camera: V (the view key). Presentation only; the simulation never reads it.
const LEGIONARY_VIEW_KEY: winit::keyboard::KeyCode = winit::keyboard::KeyCode::KeyV;
/// The eye's own-body rule: the near clip of the first person camera, in
/// meters. The wearer's own head and shoulders sit inside this radius, so
/// the clip keeps the body out of the picture instead of showing the inside
/// of the skull. The table-driven rule (hide distance and near clip from one
/// table) belongs to the no-clip view row; this constant is the coarse
/// version that lets the mode be watched today.
const LEGIONARY_EYE_NEAR_M: f32 = 0.30;
/// Prefix of the one line [`VIEW_LINE_FLAG`] prints, mirrored by
/// `engine_endpoint`'s view host.
const VIEW_LINE_PREFIX: &str = "arena view-line: ";
/// Keys the window answers, shown in the window title and printed at boot.
const CONTROLS_LINE: &str = "WASD move (camera-relative) | F attack | Q throw spear (--spear) | Shift block | Space dodge | Esc pause | ` console | [ ] slow-mo speed | R rematch | T title | C free cam | V first/third person (--legionary)";
/// Surface frames between look-probe rounds under `--look-probes`.
const LOOK_PROBE_EVERY: u32 = 120;
/// Surface frames between fog plus contact refreshes under `--look-probes`.
const LIVE_FOG_EVERY: u32 = 4;
/// Presented frames between hot-reload watcher polls: one poll stats every
/// watched file, so every frame would cost up to ~257 `fs::metadata` calls.
const HOT_RELOAD_POLL_EVERY_FRAMES: u16 = 30;

/// Flag that fails the run when any presented frame shows no fighter (the
/// camera once looked at bare sand for half a perf run, 2026-10-11).
const REQUIRE_FRAMING_FLAG: &str = "--require-framing";

/// `--no-break-look`: the center crate stays out of the picture (no whole crate, no debris).
/// The sim breaks it either way; presentation only (RZ4w).
const NO_BREAK_LOOK_FLAG: &str = "--no-break-look";

/// `--no-ragdoll`: a floored fighter keeps the flat death clip, no `physfx` body behind it.
/// The sim floors it either way; presentation only (RZ2), the off half of a capture pair.
const NO_RAGDOLL_FLAG: &str = "--no-ragdoll";

/// The exact stop line a smoke greps: `paused tick=<N>`, no other tokens.
const PAUSE_LINE_PREFIX: &str = "paused tick=";

/// What the fight camera showed over a run: frames tallied, frames with no
/// fighter in the picture (and the first), the fewest fighters seen, the
/// steepest look down and how many drawn bodies wore the hit tint (a red
/// strobe at the start of an 8-fighter run, 2026-10-11). Tallied only while
/// the fighter look draws.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Framing {
    frames: u32,
    empty: u32,
    first_empty: Option<u32>,
    fewest: usize,
    steepest_deg: f32,
    bodies: u32,
    tinted: u32,
    /// Fighters in the match the tally was taken over (`0` until counted).
    fighters: usize,
    /// Fewest fighters the look had folded at any tallied frame.
    fewest_seen: usize,
}

impl Default for Framing {
    fn default() -> Self {
        Self {
            frames: 0,
            empty: 0,
            first_empty: None,
            fewest: usize::MAX,
            steepest_deg: f32::MIN,
            bodies: 0,
            tinted: 0,
            fighters: 0,
            fewest_seen: usize::MAX,
        }
    }
}

impl Framing {
    fn note(&mut self, frame: u32, in_view: usize, down_deg: f32) {
        self.frames += 1;
        if in_view == 0 {
            self.empty += 1;
            self.first_empty.get_or_insert(frame);
        }
        self.fewest = self.fewest.min(in_view);
        self.steepest_deg = self.steepest_deg.max(down_deg);
    }

    /// One frame's `(tinted, drawn)` bodies from the look.
    fn note_tint(&mut self, (tinted, drawn): (usize, usize)) {
        let count = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
        self.tinted = self.tinted.saturating_add(count(tinted));
        self.bodies = self.bodies.saturating_add(count(drawn));
    }

    /// One frame's denominator: the fighters in the match and how many of
    /// them the look had folded, so `fewest_in_view` reads against a count.
    fn note_count(&mut self, fighters: usize, seen: usize) {
        self.fighters = self.fighters.max(fighters);
        self.fewest_seen = self.fewest_seen.min(seen);
    }

    /// The `arena framing:` receipt line; `None` when nothing was tallied.
    /// `fighters=` and `seen=` ride only once counted: `fewest_in_view` of 2
    /// is fine out of 2 and a fault out of 8.
    fn line(&self) -> Option<String> {
        (self.frames > 0).then(|| {
            let first = self.first_empty.map_or_else(|| "-".to_string(), |f| f.to_string());
            let share = f64::from(self.tinted) / f64::from(self.bodies.max(1));
            let counted = if self.fighters > 0 {
                format!(" fighters={} seen={}", self.fighters, self.fewest_seen)
            } else {
                String::new()
            };
            format!(
                "arena framing: frames={}{counted} empty={} first_empty={first} fewest_in_view={} steepest_down_deg={:.1} tinted_share={share:.2}",
                self.frames, self.empty, self.fewest, self.steepest_deg
            )
        })
    }

    /// Why `--require-framing` fails this run; `None` when every tallied
    /// frame showed a fighter.
    fn failure(&self) -> Option<String> {
        let first = self.first_empty?;
        Some(format!(
            "{} of {} frames showed no fighter (first at frame {first})",
            self.empty, self.frames
        ))
    }
}

/// Steps to run this frame: the accumulator count, plus one warmup tick on
/// the first shown frame. The first redraw buffers about 12 ms for 0 steps,
/// so `observe_tick` never runs before the first `push_fighters` tally and
/// every `slot.seen` stays false. Pure over the same inputs.
fn warmup_steps(steps: u32, ticks_done: u32, ticks_total: u32, shown: bool) -> u32 {
    if steps == 0 && ticks_done == 0 && ticks_total > 0 && shown {
        1
    } else {
        steps
    }
}

/// Whether a presented frame enters the framing tally: only after at least
/// one tick was observed. A 0-tick frame has no seen slots by design, so
/// counting it reports an empty picture that was never a fight view. Pure.
fn framing_should_tally(ticks_done: u32) -> bool {
    ticks_done > 0
}

/// What the shell does with the fixed-tick accumulator on one frame under
/// `--pause-at-tick`: run it, hold it, or run exactly one requested step.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PauseDecision {
    /// Run the ordinary accumulator steps.
    Run,
    /// Hold the accumulator; add zero ticks this frame.
    Hold,
    /// Run exactly one tick for one requested Space press.
    Step,
}

/// The stop-or-step decision for the `--pause-at-tick` hold. Below the pause
/// tick (or with no pause tick) the sim runs; at or past it the sim holds, and
/// a `stepped` Space request runs exactly one tick. Without `--pause-at-tick`
/// the sim always runs, so a stray step is a no-op. Pure, no window and no
/// event loop: the test drives it directly.
fn pause_decision(tick: u64, pause_at: Option<u64>, stepped: bool) -> PauseDecision {
    match pause_at {
        Some(at) if tick >= at => {
            if stepped {
                PauseDecision::Step
            } else {
                PauseDecision::Hold
            }
        }
        _ => PauseDecision::Run,
    }
}

/// One zone line per actor in the shipped hand-placed rows, beside the prop
/// rows the `--view-brief` lists ([`frame_brief_props`] reads the same kit
/// data): `arena zone <id> class=<0..4> advantage=<0|1> dominance=<0|1>
/// risk=<0|1> zone=<Gate|Centre|Ring|Wall>`.
///
/// The flags are the layout `tools/level-check.mjs` gates; the trailing
/// `zone` is the ring classifier's reading of that row's own centre under the
/// same gate row, so a layout edit shows up in the twin output too. A refused
/// table prints one loud line instead of failing the run. Pure: embedded rows
/// plus the classifier, no file IO, no window.
fn zone_report_lines() -> Vec<String> {
    let rows = match engine_game::arena_kit::kit_zone_rows() {
        Ok(rows) => rows,
        Err(err) => return vec![format!("arena zones: {err}")],
    };
    rows.iter()
        .map(|row| {
            let zone = engine_game::fight_ring::ring_zone_at(
                f32::from(row.cx_cm) / 100.0,
                f32::from(row.cz_cm) / 100.0,
            );
            format!(
                "arena zone {} class={} advantage={} dominance={} risk={} zone={zone:?}",
                row.id,
                row.class,
                u8::from(row.advantage),
                u8::from(row.dominance),
                u8::from(row.risk),
            )
        })
        .collect()
}

/// Props and light for one `--view-brief` row: every kit row plus the ring and
/// its wall as one row each, under the law-holding noon light the brief is
/// measured with until the sun bind lands. Pure; built only when the flag is
/// on, so a plain run pays nothing per frame.
fn frame_brief_props() -> (Vec<PropTruth>, FrameLight) {
    let mut props: Vec<PropTruth> = kit_prop_rows()
        .iter()
        .map(|(kind, bounds)| PropTruth {
            kind,
            min: bounds[0],
            max: bounds[1],
        })
        .collect();
    for row in [ring_prop_row(), ring_wall_row()] {
        props.push(PropTruth {
            kind: row.0,
            min: row.1[0],
            max: row.1[1],
        });
    }
    let light = FrameLight {
        sun_dir: [0.0, 1.0, 0.0],
        sun_elev: 0.5,
        exposure: 1.0,
        fog: 0.0,
        passes: 1,
    };
    (props, light)
}

/// Whether the surface list needs fresh kit draws: the skinned look clears
/// the list (`list.items.clear` in `push_fighters`), so a bound look drops
/// kit draws before submit. Skipping that re-staging saves one `mesh_keys`
/// allocation plus 11 resident checks per frame; pixels hold because the
/// cleared draws never reached the submit. Pure.
fn should_push_kit(look_bound: bool) -> bool {
    !look_bound
}

/// Whether the frame list can skip the twin import: a bound skinned look
/// clears `list.items` before its ring draws (`push_fighters` calls
/// `list.items.clear` on both the cached and the fresh-skin arms), so the
/// cube, dressing, twin fighters and kit draws that `frame_list` stages
/// every frame never reach the submit. The twin kit import re-reads plus
/// re-parses the 8 glTF kit files (~45 MB of JSON) on every frame, about
/// a second per present, so 300 presents cannot fit the 180 s look gate
/// while the bound look discards all of it. Pure over the bound flag.
fn should_build_cheap_list(look_bound: bool) -> bool {
    look_bound
}

/// Camera-only frame list for a bound look: the HUD clear list plus the
/// establishing dressing eye with the capture orbit applied, no twin items.
/// `push_fighters` clears then draws the ring plus skinned bodies over this
/// base and re-aims the camera, so the surface and its capture PNGs show
/// the same fight as the full twin list. A bad size refuses loudly instead
/// of filing an empty frame. Presentation only, never hashed.
///
/// Caller: `present_frame` below takes this path while the look is bound;
/// the refused-look path keeps the full twin `frame_list`.
fn cheap_frame_list(
    width: u32,
    height: u32,
    camera_path: Option<engine_game::windowed::CameraPath>,
    frame: u32,
) -> Result<DrawList, WinError> {
    let mut list = hud_draw_list(width.max(1), height.max(1))?;
    list.camera = dressing_camera(width.max(1), height.max(1))?;
    apply_camera_orbit(&mut list, frame, camera_path);
    Ok(list)
}

/// Presentation threading for the EG45 balance look: folds one tick's cues
/// into swings through the look, accepting the authoritative `Sim` when the
/// live poll exposes it. `None` until the later row owns that edge (the look
/// mirrors the same integer verdicts from cues today, so the same blows lean
/// the same roots). Presentation only, never hashed.
fn observe_tick_with_sim(
    look: &mut FighterLook,
    tick: u32,
    views: &[engine_game::FighterView],
    hp: &[u16],
    cues: &[FightCue],
    lands: &[engine_arena::FallLanding],
    _sim: Option<&engine_arena::Sim>,
) {
    // FEEL7: the views are the tick's pre-step positions, so the authority's
    // landing row starts the squash on the very tick the fall table charged;
    // the view tracking still catches any landing the row does not name.
    look.observe_landings(tick, views);
    look.observe_landing_rows(tick, lands);
    look.observe_tick(tick, views, hp, cues);
}

/// Deterministic draw order for the submit: opaque draws sort by mesh then
/// material then texture, so the same tick gives the same draw list x2 and
/// the submit reuses pipeline bindings in runs. `sort_unstable` runs in
/// place, so the steady-state frame allocates nothing. Presentation only,
/// never hashed [render-pipeline-rules rule 7].
fn sort_draw_list(list: &mut DrawList) {
    list.items.sort_unstable_by(|a, b| {
        a.mesh
            .cmp(&b.mesh)
            .then_with(|| a.material_id().cmp(&b.material_id()))
            .then_with(|| a.base_color.cmp(&b.base_color))
    });
}

/// Whether the skinned fighter draws can be reused: the pose tick and the
/// fighter count match the last skinned frame, so the CPU skin plus upload
/// would rebuild identical vertices. The camera still re-aims per frame
/// (slow orbit), so only the skin is cached. Pure.
fn should_reuse_fighters(
    cached_tick: Option<u32>,
    cached_fighters: usize,
    ticks_done: u32,
    fighters: usize,
) -> bool {
    cached_tick == Some(ticks_done) && cached_fighters == fighters
}

/// Shell state at boot: automated runs (`--exit`/`--timed-smoke`) stage the
/// bout at once (Match, ticks run) so captures show fighters; interactive
/// play boots Title and waits for Enter/Space. Pure over the parsed flags.
fn initial_flow(args: &WinArgs) -> Flow {
    if args.exit || args.timed_smoke {
        Flow::Match
    } else {
        Flow::Title
    }
}

/// Nearest-rank percentile of an ascending microsecond list, `0` when
/// empty. `numerator` is the percent (50, 99).
fn percentile_us(sorted: &[u64], numerator: usize) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (sorted.len() * numerator).div_ceil(100).max(1) - 1;
    sorted.get(rank.min(sorted.len() - 1)).copied().unwrap_or(0)
}

/// Failure message when any live frame mapped a buffer or waited on the
/// GPU; `None` when every live frame stayed non-blocking. Probe frames
/// (`--look-probes`) and captures block by design and never count here.
fn live_wait_failure(stats: &WaitStats) -> Option<String> {
    (stats.live_blocking() > 0).then(|| {
        format!(
            "live frames blocked on the GPU (maps={} waits={} over {} live frames)",
            stats.live_maps, stats.live_waits, stats.live_frames
        )
    })
}

/// The metric token of an `arena WARN:` line (the first word after the
/// prefix); `None` for any other line. Names the repro bundle directory.
fn warning_metric(warning: &str) -> Option<&str> {
    let rest = warning.strip_prefix("arena WARN: ")?;
    let metric = rest.split_whitespace().next()?;
    (!metric.is_empty()).then_some(metric)
}

/// The metric that must write this run's one warning bundle now: `None` when
/// the run is clean, a bundle already landed, or the first line is malformed.
fn warning_bundle_metric(warnings: &[String], written: bool) -> Option<&str> {
    if written {
        return None;
    }
    warning_metric(warnings.first()?)
}

/// Whole microseconds as milliseconds with three decimals, integer math.
fn fmt_us_as_ms(us: u64) -> String {
    format!("{}.{:03}", us / 1000, us % 1000)
}

/// Build the session config from CLI overrides. `--ticks` (or `--frames`
/// when smaller) bounds the match; `--seed` picks the arena seed;
/// `--fighters` picks the fighter count (8-fighter captures use 8) and `--scene`
/// selects the arena scene (only `arena` exists). `--camera-path arena600`
/// pins the fixed seed-2040 600-frame orbit over those overrides.
fn session_config(args: &WinArgs) -> Result<GameConfig, WinError> {
    let mut ticks = args.ticks.unwrap_or(DEFAULT_TICKS);
    if let Some(frames) = args.frames {
        ticks = ticks.min(frames);
    }
    // Smoke without `--ticks` runs exactly the requested frames.
    if args.ticks.is_none() {
        if let Some(frames) = args.frames {
            ticks = frames;
        }
    }
    if let Some(scene) = args.scene {
        if scene.id() != engine_game::windowed::ARENA_SCENE_ID {
            return Err(WinError::BadConfig("scene"));
        }
    }
    let (seed, ticks) = engine_game::windowed::pin_session_for_camera_path(
        args.seed.unwrap_or(DEFAULT_SEED),
        ticks,
        args.camera_path,
        args.ticks,
    );
    let config = GameConfig {
        seed,
        ticks,
        fighters: args.fighters.unwrap_or(2),
        half_extent_m: engine_game::windowed::ARENA_HALF_EXTENT_M,
    };
    config.validate().map_err(|err| match err {
        engine_game::GameError::BadConfig(row) => WinError::BadConfig(row),
        other => WinError::Game(other),
    })?;
    Ok(config)
}

/// Test-only forced panic: `E2040_TEST_PANIC_AT_TICK=N` panics before fixed
/// tick N runs. Unset in every real run; the crash-hook test sets it.
const TEST_PANIC_ENV: &str = "E2040_TEST_PANIC_AT_TICK";

fn main() -> std::process::ExitCode {
    // First call in the binary: the crash hook and the run log. A panic from
    // here on writes `crash-<UTC>.toml` beside `arena-<UTC>.log` and exits
    // with the crash code (see `engine_game::crash`).
    match crash::install_run_hook(&crash::default_log_dir()) {
        Ok(paths) => tell!("arena log: {}", paths.log.display()),
        Err(err) => tell!("arena: crash log unavailable ({err}); panics still exit loudly"),
    }
    let raw: Vec<String> = std::env::args().collect();
    // A worker panic that its owner re-raises on main skips the hook; this
    // catch writes the report and exits with the crash code instead.
    let code = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&raw))) {
        Ok(code) => code,
        Err(payload) => crash::fatal_unwind(payload),
    };
    // Return the code instead of calling `std::process::exit`: on Windows that
    // call ends the process without running the CRT exit path, so an
    // instrumented build writes a zero-byte `.profraw` and `tools/wiring-cov.mjs`
    // reads every function as unhit. A plain return flushes the profile. Codes
    // are 0..=2; anything else reads as a failure.
    std::process::ExitCode::from(u8::try_from(code).unwrap_or(1))
}

/// Parse, configure and run; returns the process exit code (0 green).
/// Headless fallback: when no display exists and `--frames` was given, the
/// same smoke runs without winit so CI still presents the path. Chart flags
/// (`--chart/--chart-dir`) split off first, then capture flags
/// (`--capture-dir/--capture-every/--tape`), so `WinArgs`
/// still refuses every other unknown flag exactly as before. `--chart`
/// runs the material-chart path synchronously (no window) and exits.
fn run(raw: &[String]) -> i32 {
    // `--staged-fight` drives the human slot with the staged script over
    // the staged session (seed 2040, 600 ticks, 4 fighters).
    let (staged, raw) = split_staged_flag(raw);
    let (look_probes, raw) = arena_args::split_flag(&raw, LOOK_PROBES_FLAG);
    let (require_framing, raw) = arena_args::split_flag(&raw, REQUIRE_FRAMING_FLAG);
    let (no_break_look, raw) = arena_args::split_flag(&raw, NO_BREAK_LOOK_FLAG);
    let (no_ragdoll, raw) = arena_args::split_flag(&raw, NO_RAGDOLL_FLAG);
    let (play, raw) = arena_args::split_flag(&raw, PLAY_FLAG);
    let (spear, raw) = arena_args::split_flag(&raw, SPEAR_FLAG);
    let (spear_demo, raw) = arena_args::split_flag(&raw, SPEAR_DEMO_FLAG);
    let (speed_demo, raw) = arena_args::split_flag(&raw, SPEED_DEMO_FLAG);
    let (nyc, raw) = arena_args::split_flag(&raw, "--nyc");
    let (height, raw) = arena_args::split_flag(&raw, HEIGHT_FLAG);
    let (legionary, raw) = arena_args::split_flag(&raw, LEGIONARY_FLAG);
    let (agent_seat, raw) = match arena_args::split_agent_seat(&raw) {
        Ok(split) => split,
        Err(err) => {
            tell!("arena: {err}");
            return 2;
        }
    };
    // Windowless `view.laws` host: split and run before any GPU or window work,
    // so the MCP door can fold a bout on a build with no display.
    let (view_line, raw) = match arena_args::split_view_line(&raw) {
        Ok(split) => split,
        Err(err) => {
            tell!("arena: {err}");
            return 2;
        }
    };
    if let Some((seed, ticks, fighters)) = view_line {
        if agent_seat.is_some() {
            tell!("arena: {VIEW_LINE_FLAG} folds a bout; {AGENT_SEAT_FLAG} plays a window; they do not mix");
            return 2;
        }
        let fold = engine_game::fighter_look::FighterLook::view_run_detail(seed, ticks, fighters);
        return match fold {
            Ok((line, pop)) => {
                say!("{VIEW_LINE_PREFIX}{line}");
                // The worst pop of the fold, named (AM24): which slot and joint,
                // when, and how far that joint missed its prediction.
                if let Some(pop) = pop {
                    say!(
                        "arena pop: slot={} joint={} tick={:.1} miss_cm={:.1} act={} clip={} clip_s={:.3} prev_clip={} from={:.2},{:.2},{:.2} at={:.2},{:.2},{:.2}",
                        pop.slot,
                        pop.joint,
                        pop.tick,
                        pop.miss_m * 100.0,
                        pop.act,
                        pop.clip,
                        pop.clip_s,
                        pop.prev_clip,
                        pop.from[0],
                        pop.from[1],
                        pop.from[2],
                        pop.at[0],
                        pop.at[1],
                        pop.at[2],
                    );
                }
                0
            }
            Err(err) => {
                tell!("arena: {VIEW_LINE_FLAG}: {err}");
                2
            }
        };
    }
    // A seat has no Enter key: arming one starts the bout, like `--play`.
    // The legionary mode starts the duel too: the player is slot 0 and the
    // picture is the fighter's eye.
    let play = play || agent_seat.is_some() || legionary;
    // `--headless`: a plain flag is the local no-window route (the shipped
    // check); with a net seat it belongs to the net split, which keeps its
    // meaning there.
    let (headless, raw) = split_plain_headless(&raw);
    let (net, raw) = match arena_net::split_net_args(&raw) {
        Ok(split) => split,
        Err(err) => {
            tell!("arena: {err}");
            return 2;
        }
    };
    let (view_brief, raw) = match arena_args::split_view_brief(&raw) {
        Ok(split) => split,
        Err(err) => {
            tell!("arena: {err}");
            return 2;
        }
    };
    let (pause_at_tick, step_mode, raw) = match arena_args::split_pause_flags(&raw) {
        Ok(split) => split,
        Err(err) => {
            tell!("arena: {err}");
            return 2;
        }
    };
    let (held_keys, raw) = match arena_args::split_hold(&raw) {
        Ok(split) => split,
        Err(err) => {
            tell!("arena: {err}");
            return 2;
        }
    };
    let (reference_args, raw) = match split_reference_args(&raw) {
        Ok(split) => split,
        Err(err) => {
            tell!("arena: {err}");
            return 2;
        }
    };
    let (chart_args, rest) = match split_chart_args(&raw) {
        Ok(split) => split,
        Err(err) => {
            tell!("arena: {err}");
            return 2;
        }
    };
    let (capture_flags, rest) = match capture::split_capture_args(&rest) {
        Ok(split) => split,
        Err(err) => {
            tell!("arena: {err}");
            return 2;
        }
    };
    // One owner of slot 0. A seat beside a tape or a script would make two
    // authors for one tick and neither receipt could say which one played.
    if agent_seat.is_some()
        && (staged
            || spear_demo
            || net.is_some()
            || capture_flags
                .as_ref()
                .is_some_and(|flags| flags.tape.is_some()))
    {
        tell!(
            "arena: {AGENT_SEAT_FLAG} owns slot 0; --staged-fight, --spear-demo, --tape and --host/--join do not mix"
        );
        return 2;
    }
    let args = match WinArgs::parse(&rest) {
        Ok(args) => args,
        Err(err) => {
            tell!("arena: {err}");
            return 2;
        }
    };
    // An automated `--exit` run must leave within its bound even when the
    // loop wedges inside a driver call: the thread prints the receipt and exits.
    if args.exit {
        engine_game::windowed::arm_exit_watchdog(args.wall_limit_ms(), |err| {
            tell!("arena: {err}");
            std::process::exit(1);
        });
    }
    let config = match session_config(&args) {
        Ok(_) if staged => staged_session_config(),
        Ok(config) => config,
        Err(err) => {
            tell!("arena: {err}");
            return 2;
        }
    };
    // A network seat plays the match its code names (the guest) and owns the
    // human slot: no staged script, no spear demo, no capture tape.
    let (config, spear) = match net.as_ref().map(|net| net.session(config, spear)) {
        None => (config, spear),
        Some(_) if staged || spear_demo => {
            tell!("arena: --host/--join play live; --staged-fight and --spear-demo do not mix");
            return 2;
        }
        Some(Ok(session)) => session,
        Some(Err(err)) => {
            tell!("arena: {err}");
            return 2;
        }
    };
    if let Some(net) = net.as_ref().filter(|net| net.headless) {
        return arena_net::run_headless(config, spear, net);
    }
    // `--headless`: play the plain session with no window, or refuse a flag
    // whose promise needs the window (the helper owns the message).
    let window_only =
        play || staged || spear_demo || speed_demo || spear || height || view_brief.is_some();
    if let Some(code) = headless_gate(headless, window_only, config, &args, &capture_flags) {
        return code;
    }
    // Backend pick is resolved (and the env override stuck) before any GPU
    // work on every path, so stdout always names the picked backend.
    let pick = engine_render::backend::apply_cli_override(args.backend);
    say!("arena {}", pick.startup_line());
    // OS VRAM budget before the device takes its share (cold, startup only).
    println!("arena {}", engine_render::backend::read_vram().log_line());
    // The hand-placed zone layout beside the twin's prop rows: a layout edit
    // shows the same flags the design gate reads.
    for line in zone_report_lines() {
        say!("{line}");
    }
    if chart_args.enabled {
        return run_chart_mode(&config, &args, &chart_args);
    }
    if reference_args.enabled {
        return run_reference_mode(&config, &reference_args);
    }
    let loop_result = EventLoop::new();
    let event_loop = match loop_result {
        Ok(loop_) => loop_,
        Err(err) => {
            let err = WinError::NoDisplay(capped(&err.to_string()));
            if args.frames.is_some() {
                if agent_seat.is_some() {
                    tell!(
                        "arena: {AGENT_SEAT_FLAG} needs a window; the headless fallback has no seat"
                    );
                    return 3;
                }
                return headless_fallback(config, &args, &err, &capture_flags);
            }
            tell!("arena: {err}");
            return 3;
        }
    };
    let capture = match setup_capture(&capture_flags, &config) {
        Ok(slot) => slot,
        Err(err) => {
            tell!("arena: {err}");
            return 2;
        }
    };
    let mut app = match ArenaApp::new(config, args, capture) {
        Ok(app) => app,
        Err(err) => {
            tell!("arena: {err}");
            return 2;
        }
    };
    app.staged = staged.then(StagedTally::default);
    app.pause_at_tick = pause_at_tick;
    app.step_mode = step_mode;
    for key in held_keys {
        app.set_pressed(key, true);
    }
    if let Some(code) = app.stage_receipt_session() {
        return code;
    }
    app.look_probes = Presenter::look_probes_for_capture(look_probes, app.capture.is_some());
    app.require_framing = require_framing;
    if play && app.flow == Flow::Title {
        app.flow = app.flow.on(FlowEvent::Start);
        app.last = Instant::now();
    }
    if let Err(err) = app.enable_spear(spear, spear_demo, height) {
        tell!("arena: {SPEAR_FLAG}/{HEIGHT_FLAG}: {err}");
        return 2;
    }
    if let Err(err) = speed_demo
        .then(|| app.enable_speed_demo(nyc, raw.iter().any(|a| a == "--exit")))
        .transpose()
    {
        tell!("arena: {SPEED_DEMO_FLAG}: {err}");
        return 2;
    }
    // The legionary mode: the player is slot 0, the picture is the fighter's
    // eye and the kit is the legionary's. The mode line says so at boot.
    if legionary {
        app.enable_legionary();
    }
    // A script that owns slot 0 must say so: the keys are read but ignored, and
    // without the line the bout looks like a broken controller.
    app.scripted = match (spear_demo, staged) {
        (true, _) => Some("script plays you (--spear-demo)"),
        (false, true) => Some("script plays you (--staged-fight)"),
        (false, false) => None,
    };
    if let Some(script) = app.scripted {
        say!("arena: {script}; the keyboard is ignored in this run");
    }
    if let Some(port) = agent_seat {
        match Seat::bind(port) {
            Ok(seat) => {
                say!(
                    "arena seat: listening on 127.0.0.1:{}; the client owns slot 0 and the keyboard is ignored",
                    seat.port()
                );
                app.seat = Some(seat);
            }
            Err(err) => {
                tell!("arena: {AGENT_SEAT_FLAG}: {err}");
                return 2;
            }
        }
    }
    if let Some(net) = net.as_ref() {
        if let Err(err) = arena_net::attach(&mut app, net) {
            tell!("arena: {err}");
            return 2;
        }
    }
    say!("arena controls: {CONTROLS_LINE}");
    // QL12: the per-tier particle counts ride at the head of the run, so a
    // fight capture names the caps its particle pass ran under beside the
    // pass ms on the per-pass line.
    say!("{}", particle_tier_line());
    if no_break_look {
        app.break_scene = None;
    }
    app.ragdoll_off = no_ragdoll;
    if let Some(path) = view_brief {
        match File::create(&path) {
            Ok(file) => app.view_brief = Some(BufWriter::new(file)),
            Err(err) => {
                tell!("arena: {VIEW_BRIEF_FLAG} {}: {err}", path.display());
                return 2;
            }
        }
    }
    if let Err(err) = event_loop.run_app(&mut app) {
        app.finalize_capture();
        tell!("arena: {}", WinError::NoDisplay(capped(&err.to_string())));
        return 3;
    }
    app.finalize_capture();
    app.failure.unwrap_or_default()
}

// Live fight bodies: one dot per simulated fighter from the rolling pose
// cursor (planar position, feet on the floor), live hp, this tick's flash.

/// Live window session: winit events feed the bindings table, the accumulator
/// steps the [`App`] match at 60 Hz, voices play into the shared mixer, and
/// the renderer presenter draws one HUD frame per redraw with interpolation alpha.
struct ArenaApp {
    config: GameConfig,
    args: WinArgs,
    app: App,
    bindings: Bindings,
    bank: Bank,
    mixer: Arc<Mutex<Mixer>>,
    /// Lock-free cue feed (tick-stamped, bounded, `dropped_full` counted).
    feed_tx: Option<FeedTx>,
    /// Offline cue engine pumped on the game thread when no device is live.
    feed_engine: Option<AudioEngine>,
    /// Reused stereo scratch for the offline feed pump (zero per tick).
    feed_scratch: Vec<f32>,
    /// Where the room's ears are (the camera), the footstep strides, the
    /// music level and the crowd bed: what `feed_tick_audio` keeps between ticks.
    audio_space: AudioSpace,
    /// Live device stream when one opened; `None` degrades with a notice.
    _device: Option<engine_audio::device::DeviceOutput>,
    audio_notice: String,
    presenter: Option<Presenter>,
    presenter_notice: String,
    /// Staged arena kit (11 draws over the 8 glTF files); empty until the
    /// visible presenter stages it, so a refused stage keeps the cube frame.
    kit: KitReport,
    /// Fight draws pushed on the last presented frame (smoke receipt).
    fight_draws: usize,
    /// Skinned CT7 fighters; `None` (refused load) draws the staged boxes.
    look: Option<FighterLook>,
    /// The center crate: whole until a sword hit breaks it, then debris (RZ4w).
    /// Presentation only, never hashed; `None` after a refused build or upload.
    break_scene: Option<BreakScene>,
    /// `--no-ragdoll`: the floored-fighter ragdoll layer (RZ2) stays off, so the
    /// slot keeps the flat death clip. Presentation only, never hashed.
    ragdoll_off: bool,
    /// The fight camera's smoothing state (GP43): springs on yaw, distance and look point.
    cam_follow: CamFollow,
    /// Seconds the camera smooths over on this present: the wall time since the last present,
    /// or one fixed tick under `--pause-at-tick` so a paused capture replays exactly.
    cam_dt_s: f32,
    /// Cached fighter draws for the last skinned tick: items plus draws,
    /// reused when the pose tick is unchanged so the 8-fighter skin runs
    /// once per tick instead of once per present. Presentation only.
    fighter_cache_tick: Option<u32>,
    fighter_cache_fighters: usize,
    fighter_cache_draws: usize,
    fighter_cache_items: Vec<DrawItem>,
    /// Staged-fight cue tally while `--staged-fight` drives the human slot.
    staged: Option<StagedTally>,
    /// Armed capture slot (twin PNG bytes plus tape cursor); `None` idles.
    capture: Option<CaptureSlot>,
    window: Option<Arc<Window>>,
    pressed: Vec<PhysicalButton>,
    /// Gamepad (EG27w): polled once per wall step, merged with `pressed` for
    /// the human slot's input; idle (no device, nothing held) changes nothing.
    pad: WindowPad,
    acc: TickAccumulator,
    /// Virtual time (GP45): the slow-mo keys ease its speed, the accumulator sees its output.
    clock: VirtualClock,
    /// EG26 dev console: Backquote toggles it; its pause/step gate runs before
    /// the accumulator, so a stepped tick is the same fixed tick as real time.
    console: DevConsole,
    /// Window shell state; Title, Paused and Result hold the accumulator
    /// at zero steps while Match runs the bout.
    flow: Flow,
    /// `--pause-at-tick N`: hold the accumulator once the tick reaches `N`.
    pause_at_tick: Option<u64>,
    /// `--step`: a Space press advances exactly one held tick.
    step_mode: bool,
    /// A Space press awaiting exactly one held tick (cleared when served).
    step_requested: bool,
    /// The `paused tick=N` line already printed for this hold.
    pause_announced: bool,
    /// Presentation camera mode; free-fly inspects a held tick.
    cam_mode: CameraMode,
    /// The legionary mode: the kit the player wears and the view the window
    /// plays. `None` keeps the ordinary third person fight camera.
    legionary: Option<LegionaryMode>,
    /// The first-person eye's trauma: the player's own blows only, decaying between them.
    eye_trauma: engine_game::fps_view::EyeTrauma,
    /// Free-fly view pose (view code only: never hashed, never sim input).
    free_cam: DirectorParams,
    /// Raw mouse deltas for the free camera: presentation only.
    mouse_raw: MouseRaw,
    /// Whether the Result line already printed for this bout; cleared on
    /// rematch so the next finished bout announces once.
    result_announced: bool,
    last: Instant,
    /// PQ18: input-to-present stage stamps for the run's latency receipt.
    /// Presentation only; never read by the sim or a hash.
    latency: arena_frame::latency::LatencyMeter,
    /// Wall start of the session for the `--timed-smoke` fps line.
    started: Instant,
    ticks_done: u32,
    frames_presented: u32,
    /// Wall steps whose present the `--present-every`/`--present-skip`
    /// throttle skipped on purpose; they never count as presented frames.
    frames_skipped: u32,
    /// Reused offline mix buffer (`FRAMES_PER_TICK` frames; one heap alloc
    /// at construction, zero per tick via `render_into`).
    audio_scratch: Vec<f32>,
    /// Reused tick outcome for the hot loop (one sizing, zero per-tick
    /// heap growth via `tick_into`).
    tick_scratch: TickOutcome,
    /// Last interpolation alpha used for a present (diagnostics).
    last_alpha: f32,
    failure: Option<i32>,
    /// Reporting seam: summed per-pass wall-clock us in report
    /// order (sky, layers, particles, decals, ibl, fog, gtao, shadow) plus the
    /// adapter identity behind them. Cold receipt accumulation only, never
    /// the tick; evidence only, never hashed.
    pass_us: [u64; 8],
    /// Presented frames folded into `pass_us`.
    pass_frames: u32,
    /// Folded frames that ran the look probes (sky through ibl columns).
    pass_probes: u32,
    /// Summed total validation errors across sampled frames.
    pass_validation: u32,
    /// Adapter name behind the sampled frames (empty until the first).
    pass_adapter: String,
    /// Driver string behind the sampled frames (empty until the first).
    pass_driver: String,
    /// Frame size behind the sampled frames (0 until the first).
    pass_size: (u32, u32),
    /// Wall us spent stepping fixed ticks (sim plus presentation folds).
    sim_us: u64,
    /// Wall us spent presenting (skinning, surface frame, capture).
    present_us: u64,
    /// Wall us of `present_us` spent posing and uploading fighters.
    fighters_us: u64,
    /// Wall us of `fighters_us` spent on the CPU pose half (clip sample,
    /// skin, pose fold, down lift), folded from the skinned frame's split.
    /// Zero until a skinned tick lands.
    fighters_pose_us: u64,
    /// Wall us of `fighters_us` spent on the GPU upload half (`v1::upload_mesh`
    /// plus `Presenter::upload_mesh` per part). Zero until a skinned tick lands.
    fighters_upload_us: u64,
    /// `--look-probes`: run the blocking look probes in the window.
    look_probes: bool,
    /// VS7: the night preset is on (the moonlit rig, the torches burn).
    /// Off by default, so the day frame stays byte-equal.
    night: bool,
    /// VS7: the placement's torch actors, when the placement parsed. They
    /// become the frame's lights under the night preset.
    torch_actors: Option<Vec<engine_game::arena_kit::KitActor>>,
    /// `--require-framing`: a frame with no fighter in the picture fails.
    require_framing: bool,
    /// What the fight camera showed (the `arena framing:` line).
    framing: Framing,
    /// The drawn bodies' view laws (the `arena view:` line).
    view: ViewLaws,
    /// `--view-brief`: one JSON line of view truth per presented frame.
    view_brief: Option<BufWriter<File>>,
    /// The one warning bundle this run wrote, when a view floor broke.
    warn_bundle: Option<PathBuf>,
    /// Wall us of each frame body: fixed ticks plus the present call,
    /// which includes the swapchain acquire.
    work_us: Vec<u64>,
    /// Wall us between consecutive frames (the rate the player sees).
    interval_us: Vec<u64>,
    /// Per-frame wall split behind `work_us`, so the receipt names the slow
    /// frames (`--exit` runs only, like `work_us`).
    frame_rows: Vec<arena_frame::FrameRow>,
    /// Per-frame trace of the present path split, written to
    /// `E2040_FRAME_TRACE` (a path) at the end of the run. Empty when the
    /// variable is unset, so the default run pays nothing.
    frame_trace: Vec<arena_frame::FrameTrace>,
    /// Where the per-frame trace is written at the end of the run.
    frame_trace_path: Option<PathBuf>,
    /// Whether the window held the keyboard focus at the last frame.
    window_focused: bool,
    /// Whether the window was occluded at the last frame.
    window_occluded: bool,
    /// The present path's split for the frame just presented: the
    /// swapchain acquire plus the render crate's live wall split.
    last_frame_split: arena_frame::FrameSplit,
    /// OPT27: the local model's pressure watch — this frame's work and the
    /// OS VRAM reading in, placement and GPU layer count out.
    pressure: Option<PressureWatch>,
    /// Tick index that panics on purpose (`TEST_PANIC_ENV`); `None` otherwise.
    test_panic_at: Option<u32>,
    /// Reused scratch for one tick's canonical input bytes (crash tape).
    crash_bytes: Vec<u8>,
    /// Content watcher polled once per presented frame (EG29 slice 1).
    watcher: Watcher,
    /// Gate settling staged generations at frame boundaries.
    reload: HotReload,
    /// Staged combat-tuning swap applied at the tick boundary (EG29 slice 2).
    practice: PracticeSwap,
    /// Bout-tape lines for applied tuning generations (presentation log only).
    bout_tape: Vec<String>,
    /// Feel flow (GF2/GF3): hit-stop, slow-mo and camera shake from the
    /// tick cues, stepped once per presented frame (presentation only).
    feel: FeelView,
    /// The spear grapple: on or off, scripted or not, and its picture.
    spear: SpearMode,
    /// The speed demo (`--speed-demo`): its world, movers, sword and cut pieces.
    speed: Option<arena_frame::speed_demo::scene::SpeedDemo>,
    /// Height channel on (`--height`): the wall tether swings (rows SG5, SG4).
    height: bool,
    /// Set when a script owns the human slot (`--spear-demo`, `--staged-fight`):
    /// the title says so, because the keys then do nothing.
    scripted: Option<&'static str>,
    /// Which way the live camera looks on the floor (unit `[x, z]`), as the last presented frame
    /// left it: W walks along it ([`engine_game::camera_wish`]). `None` before the first frame,
    /// so the keys keep their world axes until a camera exists. View-side only: the sim sees
    /// just the integer wish that comes out.
    view_forward: Option<[f32; 2]>,
    /// The rival hint the window title carries now (`None` is the plain controls title).
    title_hint: Option<String>,
    /// The network seat of `--host`/`--join`; `None` plays alone against AI.
    net: Option<arena_net::NetPlay>,
    /// The agent seat of `--agent-seat <PORT>`: a loopback client authors the
    /// human slot and reads each fixed tick's hash. `None` keeps the keyboard.
    seat: Option<Seat>,
    /// The presentation pacer: the window presents one frame per slot, so
    /// it never outruns the display.
    pacer: arena_frame::FixedDtPacer,
    /// Whether the sim runs exactly one fixed tick per presented frame
    /// (`--fixed-dt`): the deterministic harness. Off by default, so the
    /// default run keeps the wall-clock accumulator.
    fixed_dt_ticks: bool,
}

/// Build the live session's match over the arena kit's physical placement:
/// the kit walls collide instead of being pictures. A bad placement falls
/// back to the empty world (open sand) instead of failing the window.
fn live_session(config: GameConfig, spear: bool, height: bool) -> Result<App, WinError> {
    let mut app = App::with_world(config, kit_match_world_or_empty())?.into_duel()?;
    if spear {
        app = app.into_spear()?;
    }
    Ok(if height { app.into_height()? } else { app })
}

/// The legionary mode: the kit the player wears and the view the window
/// plays. The view is presentation only: a toggle never touches the
/// simulation, the tape or any hash.
struct LegionaryMode {
    /// The kit the player wears: galea, gladius, scutum.
    kit: LegionaryKit,
    /// The view now: the eye (first person) or the fight camera (third).
    view: LegionaryView,
}

impl LegionaryMode {
    /// The mode line: the view the run plays and the kit the player wears.
    fn line(&self) -> String {
        format!(
            "legionary: eye={} kit={}",
            self.view.name(),
            self.kit.line()
        )
    }
}

impl ArenaApp {
    fn new(
        config: GameConfig,
        args: WinArgs,
        capture: Option<CaptureSlot>,
    ) -> Result<Self, WinError> {
        let test_panic_at = arena_frame::test_panic_tick()?;
        let app = live_session(config, false, false)?;
        let break_scene = BreakScene::new(&app.live_match().config());
        let mixer = Arc::new(Mutex::new(Mixer::new(WINDOWED_SAMPLE_RATE)));
        // Automated smoke never opens a device: `--mute` mutes explicitly and
        // `--exit`/`--timed-smoke` runs mute by construction; either way the
        // cue engine keeps rendering offline.
        let muted = args.mute || smoke_muted(&args);
        let arena_frame::CueFeedOpen {
            tx: feed_tx,
            engine: feed_engine,
            device: _device,
            notice: audio_notice,
        } = arena_frame::open_window_audio(muted)?;
        // An armed capture needs measured sky_ms for the per-pass sidecar,
        // so it arms the blocking look probes even without `--look-probes`;
        // live without capture stays non-blocking (Game Engine Gems 1 ch 9.6).
        let capture_armed = capture.is_some();
        // Automated smoke stages the bout at once; interactive boots Title.
        let boot = initial_flow(&args);
        // Hot reload (EG29 slice 1): content files plus shaders, see arena_frame.
        let watcher = arena_frame::watch_content_roots();
        // Voices of the shared offline mixer (the mirror of the feed's plays).
        let bank = Bank::placeholder(WINDOWED_SAMPLE_RATE);
        let feed_scratch = vec![0.0f32; engine_game::windowed::FRAMES_PER_TICK * 2];
        let shell = Self {
            config,
            args,
            app,
            bindings: Bindings::new(),
            bank,
            feed_tx,
            feed_engine,
            feed_scratch,
            audio_space: AudioSpace::new(),
            mixer,
            _device,
            audio_notice,
            presenter: None,
            presenter_notice: "presenter pending: no window yet".to_string(),
            kit: KitReport::default(),
            fight_draws: 0,
            look: None,
            cam_follow: new_cam_follow()?,
            cam_dt_s: 0.0,
            break_scene,
            ragdoll_off: false,
            fighter_cache_tick: None,
            fighter_cache_fighters: 0,
            fighter_cache_draws: 0,
            fighter_cache_items: Vec::new(),
            staged: None,
            capture,
            window: None,
            pressed: Vec::new(),
            pad: WindowPad::open(),
            acc: TickAccumulator::new(),
            clock: VirtualClock::new(),
            console: DevConsole::new(),
            flow: boot,
            pause_at_tick: None,
            step_mode: false,
            step_requested: false,
            pause_announced: false,
            cam_mode: CameraMode::Director,
            legionary: None,
            eye_trauma: engine_game::fps_view::EyeTrauma::default(),
            free_cam: DirectorParams::arena(),
            mouse_raw: MouseRaw::new(),
            result_announced: false,
            last: Instant::now(),
            latency: arena_frame::latency::LatencyMeter::new(),
            started: Instant::now(),
            ticks_done: 0,
            frames_presented: 0,
            frames_skipped: 0,
            audio_scratch: vec![0.0f32; engine_game::windowed::FRAMES_PER_TICK],
            tick_scratch: TickOutcome::default(),
            last_alpha: 0.0,
            failure: None,
            pass_us: [0; 8],
            pass_frames: 0,
            pass_probes: 0,
            pass_validation: 0,
            pass_adapter: String::new(),
            pass_driver: String::new(),
            pass_size: (0, 0),
            sim_us: 0,
            present_us: 0,
            fighters_us: 0,
            fighters_pose_us: 0,
            fighters_upload_us: 0,
            look_probes: Presenter::look_probes_for_capture(false, capture_armed),
            night: engine_game::windowed::night_enabled(
                std::env::var(engine_game::windowed::NIGHT_ENV)
                    .ok()
                    .as_deref(),
            ),
            torch_actors: engine_game::arena_kit::kit_actors().ok(),
            require_framing: false,
            framing: Framing::default(),
            view: ViewLaws::default(),
            view_brief: None,
            warn_bundle: None,
            work_us: Vec::new(),
            interval_us: Vec::new(),
            frame_rows: Vec::new(),
            frame_trace: Vec::new(),
            frame_trace_path: frame_trace_path(),
            window_focused: true,
            window_occluded: false,
            last_frame_split: arena_frame::FrameSplit::default(),
            // OPT27: the 4 GB, 32-layer local model at 60 Hz. A refused
            // config says so once and the run goes on without a watch.
            pressure: match PressureWatch::reference() {
                Ok(watch) => Some(watch),
                Err(why) => {
                    say!("arena pressure: off ({why})");
                    None
                }
            },
            test_panic_at,
            crash_bytes: Vec::with_capacity(16),
            watcher,
            reload: HotReload::new(),
            practice: PracticeSwap::new(),
            bout_tape: Vec::new(),
            feel: FeelView::new(),
            spear: SpearMode::default(),
            speed: None,
            height: false,
            scripted: None,
            view_forward: None,
            title_hint: None,
            net: None,
            seat: None,
            pacer: frame_pacer(args.fixed_dt_ms),
            fixed_dt_ticks: args.fixed_dt_ms.is_some(),
        };
        // Title boot: the stranger sees the start key before the bout, and
        // the accumulator holds at zero ticks until `FlowEvent::Start`.
        // Automated smoke skips the shell and runs the bout at once.
        arena_frame::announce_title_boot(boot);
        // The pacing receipt: the period the window presents at and the
        // tick rule the sim runs under. A perf run reads both from here.
        say!(
            "arena pacing: period_ms={:.3} tick_rule={}",
            shell.pacer.period().as_secs_f64() * 1000.0,
            if shell.fixed_dt_ticks {
                "one-per-frame"
            } else {
                "wall-clock"
            },
        );
        Ok(shell)
    }

    /// The window's human: slot 0 alone or hosting, slot 1 as a network guest.
    fn human(&self) -> Result<FighterId, WinError> {
        FighterId::new(self.app.human_slot()).ok_or(WinError::BadConfig("fighters"))
    }

    /// Turn the spear grapple on for `--spear` or `--spear-demo` (a fresh match with it and the
    /// scene that draws it) and the height channel on for `--height`; nothing when no flag is
    /// given.
    fn enable_spear(&mut self, spear: bool, demo: bool, height: bool) -> Result<(), WinError> {
        self.height = height;
        if !(spear || demo || height) {
            return Ok(());
        }
        self.spear.on = spear || demo;
        self.spear.demo = demo;
        self.app = live_session(self.config, self.spear.on, height)?;
        self.spear.scene = self.spear.on.then(SpearScene::new);
        Ok(())
    }

    /// The `--spear-demo` script for the human slot this tick.
    fn spear_demo_input(&self, tick: u32, human: FighterId) -> FighterInput {
        let live = self.app.live_match();
        demo_input(tick, human, self.app.fighter_views(), live.spear_view())
    }

    /// One tick's spear events reach the feel flow (shake, hit-stop) and the audio feed (throw,
    /// thunk, rope creak). Quiet when the grapple is off: the events are empty.
    fn observe_spear_events(&mut self, tick: u32) {
        let events = self.app.live_match().spear_events();
        self.feel.observe_spear(events);
        if let Some(tx) = self.feed_tx.as_mut() {
            let views = self.app.fighter_views();
            self.audio_space.feed_spear(tx, tick, events, views);
        }
    }

    /// The input with the throw bit set while Q is held and the spear is on. The sim masks the
    /// bit when the match was built without a spear, so this never changes an old match.
    fn with_spear_key(&self, mut input: FighterInput) -> FighterInput {
        let q = PhysicalButton::Key(engine_input::map::Key::Q.0);
        if self.spear.on && self.pressed.contains(&q) {
            input.buttons =
                engine_input::Buttons(input.buttons.0 | engine_input::fighter::THROW_SPEAR);
        }
        input
    }

    /// Swap the staged leg onto its receipt session (empty world under rules
    /// v1, what `staged_fight.exe` builds) so the window replays its pinned
    /// fight. Reports a rebuild failure; `None` when not staged or it landed.
    fn stage_receipt_session(&mut self) -> Option<i32> {
        self.staged?;
        match App::new(self.config) {
            Ok(session) => {
                self.app = session;
                self.break_scene = BreakScene::new(&self.app.live_match().config());
                None
            }
            Err(err) => {
                tell!("arena: {err}");
                Some(2)
            }
        }
    }

    /// Clear the finished bout so rematch starts clean: a fresh match, a
    /// zeroed tick cursor, a drained accumulator and a fresh scratch
    /// outcome. The staged tally restarts too; the look keeps painting
    /// (presentation only, never hashed). Loud on a refused rebuild.
    fn reset_bout(&mut self) -> Result<(), WinError> {
        if self.net_refuses_rematch() {
            return Ok(());
        }
        self.app = live_session(self.config, self.spear.on, self.height)?;
        self.break_scene = BreakScene::new(&self.app.live_match().config());
        self.spear.scene = self.spear.on.then(SpearScene::new);
        self.ticks_done = 0;
        self.acc = TickAccumulator::new();
        self.tick_scratch = TickOutcome::default();
        self.feel = FeelView::new();
        self.cam_follow = new_cam_follow()?;
        self.last_alpha = 0.0;
        self.result_announced = false;
        self.fighter_cache_tick = None;
        self.fighter_cache_fighters = 0;
        self.fighter_cache_draws = 0;
        self.fighter_cache_items.clear();
        self.last = Instant::now();
        if self.staged.is_some() {
            self.staged = Some(StagedTally::default());
        }
        Ok(())
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, err: WinError, code: i32) {
        tell!("arena: {err}");
        self.failure = Some(code);
        event_loop.exit();
    }

    /// Fixed ticks owed for `elapsed_ms` of wall time. A window that cannot
    /// show a frame (minimized, zero-sized) holds the match: the fight never
    /// plays out unseen. The view truth caught all 300 ticks of a smoke run
    /// spent before its first frame (2026-10-11).
    fn ticks_owed(&mut self, elapsed_ms: f64) -> u32 {
        // Pause holds the accumulator: 60 paused frames add 0 ticks and
        // buffer nothing, so resume never bursts.
        if !self.flow.ticks_run() {
            return 0;
        }
        let shown = self.presenter.as_ref().is_some_and(|p| !p.suspended());
        if !shown {
            return 0;
        }
        // Hit-stop and slow-mo scale the wall time the accumulator sees, so
        // the sim owes fewer ticks; what a tick computes never changes. The
        // virtual clock (slow-mo keys) scales the feel flow's output again.
        let steps = self
            .acc
            .push_elapsed_ms(self.clock.advance(self.feel.scale_elapsed_ms(elapsed_ms)));
        warmup_steps(steps, self.ticks_done, self.config.ticks, true)
    }

    /// Fixed steps owed this frame under the `--pause-at-tick` hold. `Run`
    /// advances one frame-driven step per presented frame, clamped so the tick
    /// lands exactly on the pause tick and never past it; `Hold` prints the
    /// one `paused tick=N` stop line and adds zero ticks; a `Step` runs exactly
    /// one requested tick. Without `--pause-at-tick` this is
    /// [`ArenaApp::ticks_owed`] unchanged, so the default run is byte-identical.
    /// Presentation only: the sim's integer cursor stops, the frozen hash
    /// stands.
    fn pause_steps(&mut self, elapsed_ms: f64) -> u32 {
        // EG26: the dev console's hold and one-tick step run before the
        // wall-clock accumulator. A step owes the same fixed tick as real
        // time, never a different dt inside the tick.
        let shown = self.presenter.as_ref().is_some_and(|p| !p.suspended());
        if matches!(self.flow, Flow::Match | Flow::Paused) && self.console.paused() {
            if !shown {
                return 0;
            }
            let steps = self.console.take_step();
            return steps.min(self.config.ticks.saturating_sub(self.ticks_done));
        }
        let stepped = std::mem::take(&mut self.step_requested);
        let decision = pause_decision(u64::from(self.ticks_done), self.pause_at_tick, stepped);
        match decision {
            PauseDecision::Hold => {
                if !self.pause_announced {
                    self.pause_announced = true;
                    say!("{PAUSE_LINE_PREFIX}{}", self.ticks_done);
                }
                0
            }
            PauseDecision::Step => 1,
            PauseDecision::Run => match self.pause_at_tick {
                // Frame-driven pacing under `--pause-at-tick`: one step per
                // presented frame, so the hold lands on the exact tick whatever
                // the box's present mode (an Immediate swapchain runs only a
                // handful of ticks in 200 frames). Debug pacing only: the
                // integer tick, the sim and the hash are unchanged, and a run
                // without the flag keeps the wall-clock accumulator untouched.
                Some(at) => {
                    let shown = self.flow.ticks_run()
                        && self.presenter.as_ref().is_some_and(|p| !p.suspended());
                    let owed = u32::from(shown);
                    let room = at.saturating_sub(u64::from(self.ticks_done));
                    owed.min(u32::try_from(room).unwrap_or(u32::MAX))
                }
                // The fixed-dt harness: one fixed tick per presented frame, so
                // the tick count is the frame count and two runs of the same
                // fixture pay the same ticks and hash the same.
                None if self.fixed_dt_ticks => self.fixed_dt_steps(),
                None => self.ticks_owed(elapsed_ms),
            },
        }
    }

    /// Fixed ticks owed this frame under `--fixed-dt`: exactly one per
    /// presented frame, clamped to the session's remaining ticks. A window
    /// that cannot show a frame holds the match, like the wall-clock path.
    fn fixed_dt_steps(&self) -> u32 {
        let shown =
            self.flow.ticks_run() && self.presenter.as_ref().is_some_and(|p| !p.suspended());
        arena_frame::pace::fixed_dt_steps(shown, self.ticks_done, self.config.ticks)
    }

    /// Step fixed ticks for the elapsed wall time, then present one HUD frame.
    /// The automated `--exit` run's wall-clock bound (`--deadline-ms`
    /// overrides the 5 min default): a hung smoke ends itself loudly instead
    /// of burning a core behind the open window. False ends the frame.
    fn within_exit_deadline(&mut self, event_loop: &ActiveEventLoop) -> bool {
        if !self.args.exit {
            return true;
        }
        let wall_ms = self.started.elapsed().as_secs_f64() * 1000.0;
        match check_exit_wall_deadline(wall_ms, self.args.wall_limit_ms()) {
            Ok(()) => true,
            Err(err) => {
                self.fail(event_loop, err, 1);
                false
            }
        }
    }

    /// OPT27: this presented frame's work (sim plus present) feeds the local
    /// model's pressure watch, whose OS VRAM read is refreshed once a second.
    /// A placement move or a new layer plan prints its receipt here, off the
    /// tick; the request path reads the placement as one atomic byte.
    fn note_pressure(&mut self, work_us: u64, interval: Duration) {
        let frame_us = u32::try_from(work_us).unwrap_or(u32::MAX);
        let dt_ms = duration_us(interval) / 1_000;
        if let Some(line) = self
            .pressure
            .as_mut()
            .and_then(|watch| watch.frame(frame_us, dt_ms))
        {
            say!("{line}");
        }
    }

    fn tick_and_present(&mut self, event_loop: &ActiveEventLoop) {
        if !self.within_exit_deadline(event_loop) {
            return;
        }
        let interval = self.last.elapsed();
        let elapsed_ms = interval.as_secs_f64() * 1000.0;
        self.last = Instant::now();
        let cam_dt = if self.pause_at_tick.is_some() {
            Duration::from_secs_f64(WIN_FIXED_DT_S)
        } else {
            interval
        };
        self.cam_dt_s = cam_dt.as_secs_f32();
        let steps = self.pause_steps(elapsed_ms);
        let sim_started = Instant::now();
        let human = match self.human() {
            Ok(human) => human,
            Err(err) => return self.fail(event_loop, err, 2),
        };
        self.pad.poll();
        self.run_sim_ticks(steps, human);
        if let Some(err) = self.net_fatal() {
            return self.fail(event_loop, err, 2);
        }
        self.feel.step();
        self.refresh_title_hint();
        if self.ticks_done >= self.config.ticks {
            self.flow = self.flow.on(FlowEvent::MatchOver);
            // Minimal Result: winner plus ticks through the HUD text path
            // once per bout; the overlay rows stay parked.
            if self.flow == Flow::Result && !self.result_announced {
                self.result_announced = true;
                let line = engine_game::windowed::result_text(&self.app);
                let (cells, quads) = engine_game::windowed::result_cells_quads(&self.app);
                say!("arena {line} hud_cells={cells} hud_quads={quads}");
            }
        }
        self.last_alpha = self.acc.alpha();
        let present_started = Instant::now();
        let sim_wall_us = duration_us(present_started - sim_started);
        self.sim_us = self.sim_us.saturating_add(sim_wall_us);
        // A throttled step ran its ticks above and draws nothing: no present
        // wall, no frame sample, no presented-frame count.
        if !self.take_present_skip() {
            let fighters_before = self.fighters_us;
            self.present_frame(event_loop);
            let present_wall_us = duration_us(present_started.elapsed());
            self.present_us = self.present_us.saturating_add(present_wall_us);
            let work_wall_us = duration_us(sim_started.elapsed());
            self.note_pressure(work_wall_us, interval);
            if self.args.exit && self.frames_presented > 0 {
                self.work_us.push(work_wall_us);
                self.interval_us.push(duration_us(interval));
                self.frame_rows.push(arena_frame::FrameRow {
                    index: self.frames_presented.saturating_sub(1),
                    work_us: work_wall_us,
                    present_us: present_wall_us,
                    fighters_us: self.fighters_us.saturating_sub(fighters_before),
                });
            }
            // The per-frame trace (only when `E2040_FRAME_TRACE` names a
            // path): one row per presented frame, so a perf run can say
            // which section of the present path ate a slow frame.
            self.note_frame_trace(
                work_wall_us,
                duration_us(interval),
                sim_wall_us,
                present_wall_us,
                self.fighters_us.saturating_sub(fighters_before),
            );
        }
        // Every `--exit` run carries its own wall-clock bound: a hung window
        // loop fails itself loudly instead of burning a core for hours.
        if self.args.exit {
            let wall_ms = self.started.elapsed().as_secs_f64() * 1000.0;
            if let Err(err) = check_exit_wall_deadline(wall_ms, self.args.wall_limit_ms()) {
                return self.fail(event_loop, err, 1);
            }
        }
        // `--frames N --exit` smoke leaves after N presented frames; a staged
        // fight leaves once the whole staged session ran.
        let finished = self.net_finished().unwrap_or_else(|| {
            arena_frame::exit_reached(
                self.staged.is_some(),
                self.ticks_done,
                self.config.ticks,
                self.frames_presented,
                self.args.frames,
                self.args.present_skip,
            )
        });
        if self.args.exit && finished {
            self.finish_exit(event_loop);
        }
    }

    /// The view receipt, then one `arena WARN:` line per broken product
    /// floor: a bad run says so by itself, and a clean run stays silent. The
    /// first warning of the run also writes its one repro bundle (view only,
    /// never the sim or the hash), so the failing frame leaves a tape.
    fn report_view(&mut self) {
        say!("{}", self.fight_fx_line()); // FEEL5: what the fight threw on screen
        self.report_speed_demo();
        if let Some(line) = self.view.line() {
            say!("{line}");
            // The worst pop of the run, named: which slot, which joint, when,
            // and how far the joint missed its prediction (AM24).
            if let Some(pop) = self.view.pop_detail() {
                say!(
                    "arena pop: slot={} joint={} tick={:.1} miss_cm={:.1} from={:.2},{:.2},{:.2} at={:.2},{:.2},{:.2} act={} clip={} clip_s={:.3} prev_clip={}",
                    pop.slot,
                    pop.joint,
                    pop.tick,
                    pop.miss_m * 100.0,
                    pop.from[0],
                    pop.from[1],
                    pop.from[2],
                    pop.at[0],
                    pop.at[1],
                    pop.at[2],
                    pop.act,
                    pop.clip,
                    pop.clip_s,
                    pop.prev_clip,
                );
            }
            let warnings = self.view.warnings();
            if let Some(metric) = warning_bundle_metric(&warnings, self.warn_bundle.is_some()) {
                match crash::write_warning_bundle(
                    std::path::Path::new("target/repro"),
                    metric,
                    self.config.seed,
                    self.config.ticks,
                    self.ticks_done,
                    self.config.fighters,
                    &line,
                    &warnings,
                ) {
                    Ok(path) => {
                        say!("arena repro: bundle=\"{}\"", path.display());
                        self.warn_bundle = Some(path);
                    }
                    Err(err) => tell!("arena WARN: repro bundle not written ({err})"),
                }
            }
            for warning in warnings {
                say!("{warning}");
            }
        }
        if let Some(seat) = self.seat.as_ref() {
            say!("{}", seat.receipt());
        }
    }

    /// Exit-run receipts: smoke line, timed lines, capture manifest plus
    /// per-pass sidecar, staged-fight receipt, then leave the loop.
    fn finish_exit(&mut self, event_loop: &ActiveEventLoop) {
        let (cells, quads) = hud_cells_quads(self.app.hud());
        say!(
            "arena smoke: ticks={} frames={} alpha={:.3} hud_cells={} hud_quads={} fight_draws={} audio=\"{}\" presenter=\"{}\"",
            self.ticks_done,
            self.frames_presented,
            self.last_alpha,
            cells,
            quads,
            self.fight_draws,
            self.audio_notice,
            self.presenter_notice,
        );
        // Frozen sim hash for the run: the same tick count gives the same hash
        // paused or not, so a `--pause-at-tick` run and an unpaused run at the
        // same tick paste equal lines. Evidence only; never a hash input.
        if let Some(point) = self.app.checkpoints().last() {
            say!(
                "arena sim: ticks={} hash={:016x}",
                self.ticks_done,
                point.hash
            );
        }
        self.report_tape();
        self.report_audio_feed();
        if self.args.timed_smoke {
            let elapsed_ms = self.started.elapsed().as_secs_f64() * 1000.0;
            match format_timed_smoke(self.frames_presented, self.ticks_done, elapsed_ms) {
                Ok(line) => say!("{line}"),
                Err(err) => return self.fail(event_loop, err, 2),
            }
            let path = std::path::Path::new("target/perf/frame-times.csv");
            if let Err(err) = write_frame_times_csv(
                path,
                self.frames_presented,
                &self.work_us,
                &self.interval_us,
                elapsed_ms,
            ) {
                return self.fail(event_loop, err, 2);
            }
        }
        say!("{}", self.frame_split_line());
        say!("{}", self.feel.line());
        if let Some(line) = self.framing.line() {
            say!("{line}");
        }
        self.report_view();
        if let Some(scene) = self.spear.scene.as_ref() {
            say!("{}", scene.look().receipt_line());
        }
        if let Some(scene) = self.break_scene.as_ref() {
            say!("{}", scene.receipt_line());
            say!("{}", scene.stats().receipt_line());
        }
        if let Some(mut out) = self.view_brief.take() {
            if let Err(err) = out.flush() {
                tell!("arena: {VIEW_BRIEF_FLAG}: {err}");
                self.failure = Some(2);
            }
        }
        self.write_frame_trace();
        if let Some(why) = self.framing.failure().filter(|_| self.require_framing) {
            tell!("arena: {why}");
            self.failure = Some(1);
        }
        if let Some(line) = self.per_pass_line() {
            say!("{line}");
        }
        if let Some(code) = self.report_waits() {
            self.failure = Some(code);
        }
        // PQ18: the measured input-latency block plus the one-line receipt.
        self.report_latency();
        if let Some(report) = self.finalize_capture() {
            say!(
                "arena capture: frames={} manifest=\"{}\" sha={} view={}",
                report.frames,
                report.manifest.display(),
                report.sequence_fnv,
                self.view.verdict(),
            );
            self.write_per_pass_sidecar(&report.manifest);
        }
        if let Some(tally) = self.staged {
            let bundles = self
                .look
                .as_ref()
                .map(|look| look.bundle_shas().to_vec())
                .unwrap_or_default();
            match tally.receipt(self.ticks_done, self.frames_presented, &bundles) {
                Ok(line) => say!("{line}"),
                Err(err) => {
                    tell!("arena: {err}");
                    self.failure = Some(1);
                }
            }
        }
        event_loop.exit();
    }

    /// Whether a `render_surface` error is a transient acquire blip that skips
    /// this redraw instead of failing the run. Only the exact
    /// `surface acquire failed` validation (wgpu `Validation` on
    /// `get_current_texture`, a one-frame driver blip seen 2026-10-12) skips;
    /// every other validation still fails loudly so real shader/draw bugs never
    /// report green. Pure over the error, so tests pin it without a window.
    fn should_skip_acquire_validation(err: &PresentError) -> bool {
        matches!(err, PresentError::Validation(msg) if msg == "surface acquire failed")
    }

    /// Hot reload (EG29 slice 1): poll the watcher every
    /// [`HOT_RELOAD_POLL_EVERY_FRAMES`] presented frames and settle staged
    /// generations at this boundary with the tick stamped.
    /// Throttled because one poll stats every watched file (up to ~257
    /// `fs::metadata` calls); at 30 frames the edit still shows within 1 s
    /// at 60 fps. A file the cook refuses (empty, malformed JSON or WGSL,
    /// bad combat rows, unreadable) keeps its last-good generation and logs
    /// its reason; the tick never waits.
    fn poll_hot_reload(&mut self) {
        if !self
            .frames_presented
            .is_multiple_of(u32::from(HOT_RELOAD_POLL_EVERY_FRAMES))
        {
            return;
        }
        let events = self.watcher.poll();
        let frame = self.frames_presented;
        let report = settle_paths(
            &mut self.reload,
            events.iter().map(|event| event.path.as_str()),
            frame,
            self.ticks_done,
        );
        for applied in &report.applied {
            say!(
                "arena reload applied {} gen={} frame={} tick={}",
                applied.path,
                applied.generation,
                applied.frame,
                applied.tick
            );
        }
        for (path, reason) in &report.failed {
            say!("arena reload kept last-good {path} reason={reason:?}");
        }
        // Only rows the cook accepted reach the practice swap.
        for path in report.tuning_paths() {
            self.practice.note_tuning(path);
        }
        let tick = self.ticks_done;
        if let Some(applied) =
            engine_game::dev_hotpatch::apply_practice_at_boundary(&mut self.practice, frame, tick)
        {
            say!(
                "arena practice applied gen={} frame={} tick={}",
                applied.generation,
                applied.frame,
                applied.tick
            );
            self.bout_tape.push(applied.tape_line);
        } else if let Some(reason) = self.practice.last_failure() {
            say!("arena practice kept last-good reason={reason:?}");
        }
    }

    /// Present one frame to the visible OS window through the swapchain.
    /// The surface path is the only green path: transient acquire states
    /// and minimization skip the redraw, every hard failure ends loudly,
    /// and a frame that reports validation errors never counts. Capture
    /// bytes ride a separate explicit offscreen render (see
    /// `capture_offscreen`), never the surface receipt, so an offscreen PNG
    /// can never stand in for the OS window.
    fn present_frame(&mut self, event_loop: &ActiveEventLoop) {
        let mut list = {
            let Some(presenter) = self.presenter.as_ref() else {
                self.fail(
                    event_loop,
                    WinError::Render("no visible presenter: window never painted".to_string()),
                    1,
                );
                return;
            };
            if presenter.suspended() {
                return;
            }
            let (width, height) = presenter.size();
            // A bound look clears the twin items before its ring draws, so
            // build the camera-only list and skip the per-frame twin kit
            // import (re-parsed glTFs every present); the refused look keeps
            // the full twin list below, whose items reach the submit.
            if should_build_cheap_list(self.look.is_some()) {
                match cheap_frame_list(width, height, self.args.camera_path, self.frames_presented)
                {
                    Ok(list) => list,
                    Err(err) => return self.fail(event_loop, err, 2),
                }
            } else {
                match frame_list(
                    presenter,
                    width,
                    height,
                    self.args.camera_path,
                    self.frames_presented,
                ) {
                    Ok(list) => list,
                    Err(err) => return self.fail(event_loop, err, 2),
                }
            }
        };
        // Kit draws ride the same list as the surface and the offscreen
        // capture, so the window and its PNGs frame the same arena. Missing
        // keys draw nothing, so an unstaged kit keeps the cube frame. The
        // skinned look clears the list before its ring draws, so skip the
        // re-staging while it is bound (static kit, never moved this frame).
        if should_push_kit(self.look.is_some()) {
            if let Some(presenter) = self.presenter.as_ref() {
                let resident = presenter.mesh_keys();
                push_kit_draws(&mut list, &resident, &self.kit);
            }
        }
        // VS7: the night preset drops the sun and the placement's torch
        // actors burn, so the torches carry the picture. Off by default: the
        // day frame keeps the built-in light pair and stays byte-equal.
        if self.night {
            if let Some(presenter) = self.presenter.as_mut() {
                if let Some(actors) = self.torch_actors.as_ref() {
                    if let Ok(lights) = engine_game::arena_kit::torch_feed::torch_lights_at(
                        actors,
                        self.app.cursor(),
                    ) {
                        presenter.set_torch_lights(lights);
                    }
                }
                engine_game::arena_kit::torch_feed::apply_night_sun(&mut list);
            }
        }
        let fighters_started = Instant::now();
        if let Err(err) = self.push_fighters(&mut list) {
            return self.fail(event_loop, err, 1);
        }
        self.push_break(&mut list);
        self.push_spear(&mut list);
        self.fighters_us = self
            .fighters_us
            .saturating_add(duration_us(fighters_started.elapsed()));
        // Free-fly overrides the fight camera while the mode is on: the look
        // clears and re-aims `list.camera`, so this must run after it. View
        // only; the default director run leaves the camera untouched.
        if let Some(scene) = self.spear.scene.as_ref() {
            scene.frame(&mut list.camera);
        }
        self.feel.shake_camera(&mut list.camera);
        // The legionary mode: the player's eye takes the frame while the first
        // person view is on. It runs before the walking heading is read, so
        // WASD walks where the player looks. Presentation only; the sim never
        // reads the camera.
        self.aim_legionary_eye(&mut list);
        // WASD walk where the camera looks: the heading of the shot the player
        // sees, read before the free-fly override (that one inspects, it does
        // not steer).
        self.note_view_forward(list.camera.eye.to_array(), list.camera.target.to_array());
        if self.cam_mode == CameraMode::Free {
            cursor::take_look(&mut self.mouse_raw, &mut self.free_cam);
            self.aim_free_cam(&mut list);
        }
        self.push_speed_demo(&mut list);
        // The ears sit where the camera does (EG21w): the next tick's
        // listener, so a blow on the left of the picture sounds on the left.
        self.audio_space
            .follow_camera(list.camera.eye.to_array(), list.camera.target.to_array());
        let receipt = {
            let Some(presenter) = self.presenter.as_mut() else {
                self.fail(
                    event_loop,
                    WinError::Render("no visible presenter: window never painted".to_string()),
                    1,
                );
                return;
            };
            // One health bar per fighter (the capture path already stages them).
            let overlay = match engine_game::windowed::duel_overlay_for_app(&self.app) {
                Ok(overlay) => overlay,
                Err(err) => {
                    self.fail(event_loop, WinError::Render(capped(&err.to_string())), 1);
                    return;
                }
            };
            let bars = self.speed.is_none();
            engine_game::console::overlay::stage_hud(presenter, &overlay, &self.console, bars);
            // PQ18 stage three: the frame submits its present call now. Only a
            // frame with an accepted tick stamps, so an idle frame never reads
            // as a stray event.
            if self.latency.is_waiting() {
                self.latency.note_present_submitted(Instant::now());
            }
            match presenter.render_surface(&list) {
                Ok(receipt) => {
                    if receipt.total_validation_errors() > 0 {
                        // Validation errors fail loudly; last-good is kept.
                        self.fail(
                            event_loop,
                            WinError::Render("surface frame failed validation".to_string()),
                            1,
                        );
                        return;
                    }
                    match surface_frame_action(Ok(receipt.presented)) {
                        SurfaceFrameAction::Presented => receipt,
                        SurfaceFrameAction::Skipped => return,
                        SurfaceFrameAction::Failed => {
                            self.fail(
                                event_loop,
                                WinError::Render("surface frame presented nothing".to_string()),
                                1,
                            );
                            return;
                        }
                    }
                }
                Err(err) => {
                    // One-frame acquire validation blip (2026-10-12
                    // `surface acquire failed`): skip this redraw and try the
                    // next frame, so the 300-frame smoke still prints
                    // `arena framing:`. Persistent blips still end loudly
                    // through the `--exit` wall-clock bound. Every other
                    // validation keeps the loud `Failed` path below.
                    if Self::should_skip_acquire_validation(&err) {
                        return;
                    }
                    match surface_frame_action(Err(&err)) {
                        SurfaceFrameAction::Skipped => return,
                        _ => return self.fail(event_loop, WinError::Present(err), 1),
                    }
                }
            }
        };
        // The present path's split of this frame: the acquire the
        // presenter measured plus the live wall split the render crate
        // recorded (stage, submit, present call). Read before the twin
        // path can overwrite the render crate's last live split.
        let acquire_us = self
            .presenter
            .as_ref()
            .map_or(0, |presenter| presenter.last_acquire_us());
        self.last_frame_split = arena_frame::FrameSplit::of_live_frame(
            acquire_us,
            &engine_render::present::last_live_wall_split(),
        );
        self.frames_presented += 1;
        // PQ18 stage four: the presented frame ends the chain here, before
        // the twin capture can add its own offscreen wall. A full window
        // prints its summary line on this frame.
        if let Some(summary) = self.latency.note_frame_end(Instant::now()) {
            say!("{}", summary.line(self.latency.dropped()));
        }
        self.poll_hot_reload();
        self.note_receipt(&receipt);
        self.capture_offscreen(event_loop, &list);
    }

    /// Fighters over the frame list: the skinned CT7 look in its pit when
    /// bound (presentation only), else the staged boxes. Loud on refusal.
    fn push_fighters(&mut self, list: &mut DrawList) -> Result<(), WinError> {
        if let (Some(look), Some(presenter)) = (self.look.as_mut(), self.presenter.as_mut()) {
            let base = list.camera.clone();
            let frame = LookFrame {
                fighters: usize::from(self.config.fighters),
                alpha: self.last_alpha,
                shot: shot_for_frame(
                    self.frames_presented,
                    self.args.frames,
                    self.args.camera_path.is_some(),
                ),
                frame: self.frames_presented,
            };
            // Reuse the last skinned draws when the pose tick is unchanged:
            // vsync parks about two presents per tick, so every second
            // present skips the 8-fighter CPU skin plus upload and only
            // re-aims the camera. Cached items stay sorted, so the same
            // tick submits the same order x2. Presentation only.
            if should_reuse_fighters(
                self.fighter_cache_tick,
                self.fighter_cache_fighters,
                self.ticks_done,
                frame.fighters,
            ) && !self.fighter_cache_items.is_empty()
            {
                list.items.clear();
                list.items.extend(self.fighter_cache_items.iter().cloned());
                self.fight_draws = self.fighter_cache_draws;
            } else {
                // Real bodies stand in their pit: the placeholder cube, the
                // dressing boxes and the parade kit (laid across the fight
                // area) leave the frame; the look draws the ring instead.
                list.items.clear();
                let report = look
                    .present_into(presenter, list, &frame)
                    .map_err(|err| WinError::Render(err.to_string()))?;
                self.fight_draws = report.draws;
                // The skin wall split rides beside the untouched `fighters_us`
                // lump (FC3 and the row accept read that one): no second timing
                // source, just the fold of the halves the look already measured.
                self.fighters_pose_us = self.fighters_pose_us.saturating_add(report.skin.pose_us);
                self.fighters_upload_us = self
                    .fighters_upload_us
                    .saturating_add(report.skin.upload_us);
                // Cached order: the same tick sorts to the same submit order x2,
                // in place with no per-frame allocation.
                sort_draw_list(list);
                self.fighter_cache_tick = Some(self.ticks_done);
                self.fighter_cache_fighters = frame.fighters;
                self.fighter_cache_draws = self.fight_draws;
                self.fighter_cache_items.clear();
                self.fighter_cache_items.extend(list.items.iter().cloned());
            }
            // One stateful fight camera for every present, cached bodies or fresh: the shot
            // goes through the springs, so a cut or a whipping pair glides.
            if self.fight_draws > 0 {
                list.camera = look.aim_camera(&mut self.cam_follow, &base, &frame, self.cam_dt_s);
            }
            let in_view = look.fighters_in_view(&list.camera, frame.fighters);
            if framing_should_tally(self.ticks_done) {
                self.framing
                    .note(self.frames_presented, in_view, look_down_deg(&list.camera));
                self.framing
                    .note_count(frame.fighters, look.fighters_seen(frame.fighters));
                self.framing.note_tint(look.tinted(frame.fighters));
            }
            let t = look.sim_time(frame.alpha);
            // Frame fields (camera, bounds, sand, cover) are per present; joint
            // pops are folded per sim tick in arena_frame so a cached pose or a
            // two-tick present cannot fake one. Door/test callers use note().
            self.view.note_frame(&list.camera, t, look.view_truth());
            if let Some(scene) = self.spear.scene.as_ref() {
                let (spears, ropes) = scene.counts();
                self.view.note_spear(spears, ropes);
            }
            if let Some(out) = self.view_brief.as_mut() {
                // Props and light ride the brief only: kit plus ring draws as
                // one row each, with a law-holding noon light until the sun
                // bind lands. Built only when the flag is on; zero per tick.
                let (props, light) = frame_brief_props();
                let line = brief_line_with_props(
                    frame.frame,
                    t,
                    &list.camera,
                    look.view_truth(),
                    &props,
                    &light,
                );
                writeln!(out, "{line}")
                    .map_err(|err| WinError::Render(format!("{VIEW_BRIEF_FLAG}: {err}")))?;
            }
        } else if let Some(presenter) = self.presenter.as_mut() {
            let dots = live_fight_dots(&self.app, &self.tick_scratch.cues);
            stage_fight_bodies(presenter, &dots)?;
            let resident = presenter.mesh_keys();
            self.fight_draws = push_fight_draws(list, &resident, &dots)?;
        }
        Ok(())
    }

    /// The center crate over the frame list: whole, then its chunks and the debris they became
    /// (RZ4w). Presentation only. A refused upload drops the picture loudly and the window
    /// keeps running without the crate.
    fn push_break(&mut self, list: &mut DrawList) {
        let (Some(scene), Some(presenter)) = (self.break_scene.as_mut(), self.presenter.as_mut())
        else {
            return;
        };
        let eye = list.camera.eye.to_array();
        if let Err(why) = scene.push(presenter, list, self.last_alpha, eye) {
            tell!("arena notice: break picture dropped: {why}");
            self.break_scene = None;
        }
    }

    /// The spear, its trail and the rope over the frame list (spear grapple). Presentation
    /// only. A refused upload drops the picture loudly and the window keeps running.
    fn push_spear(&mut self, list: &mut DrawList) {
        let (Some(scene), Some(presenter)) = (self.spear.scene.as_mut(), self.presenter.as_mut())
        else {
            return;
        };
        if let Err(why) = scene.push(presenter, list, self.last_alpha) {
            tell!("arena notice: spear picture dropped: {why}");
            self.spear.scene = None;
        }
    }

    /// Explicit offscreen capture path: when an armed sink is due, render
    /// the same draw list through the offscreen twin for PNG bytes. Never
    /// counts as presentation; surface failures already failed loudly
    /// above. Suspended frames carry no bytes and are skipped; IO failures
    /// fail loudly instead of dropping frames silently.
    fn capture_offscreen(&mut self, event_loop: &ActiveEventLoop, list: &DrawList) {
        let due = self
            .capture
            .as_ref()
            .is_some_and(|slot| slot.sink.should_capture(self.frames_presented));
        if !due {
            return;
        }
        let tick = self.app.cursor();
        let hash = self
            .app
            .checkpoints()
            .last()
            .map(|point| point.hash)
            .unwrap_or(0);
        let Some(presenter) = self.presenter.as_mut() else {
            return;
        };
        match presenter.render_offscreen(list) {
            Ok(out) => {
                if !out.manifest.is_clean() {
                    self.fail(
                        event_loop,
                        WinError::Render("capture frame failed validation".to_string()),
                        1,
                    );
                    return;
                }
                let Some(png) = out.png.as_ref() else {
                    return;
                };
                if let Some(slot) = self.capture.as_mut() {
                    let (sink, tap) = (&mut slot.sink, &mut slot.look_tap);
                    if let Err(err) = sink.write_frame_with_hook(
                        tick,
                        hash,
                        png,
                        Some(&mut |bytes| tap.note(bytes)),
                    ) {
                        self.fail(event_loop, WinError::Render(capped(&err.to_string())), 1);
                    }
                }
            }
            Err(err) => self.fail(event_loop, WinError::Present(err), 1),
        }
    }

    /// The fighter look did not load, so the window draws staged boxes. An
    /// armed capture of boxes is no evidence (2026-10-22: a refused body
    /// still exited 0 with a clip), so that run fails.
    fn look_refused(&mut self, why: &str) {
        tell!("arena notice: {why}; drawing staged boxes");
        if self.capture.is_some() {
            tell!("arena: an armed capture of staged boxes is no evidence; the run fails");
            self.failure = Some(1);
        }
    }

    /// Write the capture manifest (if armed) and disarm. Runs on smoke exit
    /// and after the loop, so every armed capture run leaves exactly one.
    /// An armed capture that ends with zero frames fails loudly: a manifest
    /// with no clip is a receipt without evidence, never exit 0.
    fn finalize_capture(&mut self) -> Option<capture::CaptureReport> {
        let slot = self.capture.take()?;
        match slot.sink.finish() {
            Ok(report) => {
                tell!(
                    "arena capture: frames={} manifest=\"{}\" sha={} view={}",
                    report.frames,
                    report.manifest.display(),
                    report.sequence_fnv,
                    self.view.verdict(),
                );
                if report.frames == 0 {
                    tell!("arena: armed capture ended with zero frames (no adapter or all suspended?)");
                    if self.failure.is_none() {
                        self.failure = Some(1);
                    }
                }
                Some(report)
            }
            Err(err) => {
                tell!("arena: {err}");
                if self.failure.is_none() {
                    self.failure = Some(1);
                }
                None
            }
        }
    }

    /// Test-only: the tick that `TEST_PANIC_ENV` arms must never run. Unset in
    /// real runs, where this always passes; the crash-hook test arms it to get
    /// a reported crash through the hook.
    fn check_forced_tick(&self, tick: u32) {
        assert!(
            self.test_panic_at != Some(tick),
            "{TEST_PANIC_ENV}: forced panic at tick {tick}"
        );
    }

    /// Crash snapshot for one tick: the canonical input bytes plus the sim's
    /// latest checkpoint. Reads only; the sim never sees it.
    fn record_crash_tick(&mut self, input: FighterInput) {
        self.crash_bytes.clear();
        input.append_bytes(&mut self.crash_bytes);
        let checkpoint = self
            .app
            .checkpoints()
            .last()
            .map(|point| (point.tick, point.hash));
        crash::record_fight_tick(self.app.cursor(), &self.crash_bytes, checkpoint);
    }

    /// Remember which way the live camera looks on the floor, so WASD and the
    /// stick walk camera-relative. A camera with no heading (straight down)
    /// keeps the last one. View side only: the sim never reads it.
    fn note_view_forward(&mut self, eye: [f32; 3], target: [f32; 3]) {
        if let Some(forward) = ground_forward(eye, target) {
            self.view_forward = Some(forward);
        }
    }

    /// The live keyboard or pad input made camera-relative: the bindings'
    /// local wish (forward, right) turned onto the world axes the sim reads,
    /// by the camera's heading. Buttons, facing, tick and slot pass through,
    /// and with no heading yet the whole input does.
    fn camera_relative(&self, input: FighterInput) -> FighterInput {
        let Some(forward) = self.view_forward else {
            return input;
        };
        let (mx, mz) = world_wish(forward, (input.mx, input.mz));
        FighterInput { mx, mz, ..input }
    }

    /// The live keyboard or pad input aimed at the nearest living rival (soft
    /// lock-on, [`engine_game::lock_on`]): the bindings and the pad emit
    /// `facing = 0`, so a standing swing and the spear went along world +z.
    /// Wish, buttons, tick and slot pass through. With no living rival, or
    /// while the free camera is flown, the input keeps the facing it came
    /// with. Input mapping only: the sim reads the integer and nothing else.
    fn aim_at_rival(&self, input: FighterInput) -> FighterInput {
        if self.cam_mode == CameraMode::Free {
            return input;
        }
        let views = self.app.fighter_views();
        match nearest_rival_facing(input.fighter.0, views, self.app.hp()) {
            Some(facing) => FighterInput { facing, ..input },
            None => input,
        }
    }

    /// Keep the window title's rival hint current: the title is set only when
    /// the text changes (about once per walked meter), never per frame. Off
    /// outside the running bout. A script that owns the human slot leads, so
    /// nobody wonders why the keys do nothing.
    fn refresh_title_hint(&mut self) {
        let rival = (self.flow == Flow::Match)
            .then(|| arena_frame::rival_hint(self.app.fighter_views(), self.app.human_slot()))
            .flatten();
        let mut parts = Vec::new();
        if let Some(script) = self.scripted {
            parts.push(script.to_string());
        }
        if let Some(speed) = speed_label(self.clock.speed_permille()) {
            parts.push(speed);
        }
        if self.console.paused() {
            parts.push("console paused".to_string());
        }
        if let Some(rival) = rival {
            parts.push(rival);
        }
        let hint = (!parts.is_empty()).then(|| parts.join(" | "));
        if hint == self.title_hint {
            return;
        }
        if let Some(window) = self.window.as_ref() {
            window.set_title(&arena_frame::window_title(hint.as_deref()));
        }
        self.title_hint = hint;
    }

    fn set_pressed(&mut self, button: PhysicalButton, down: bool) {
        if down {
            if !self.pressed.contains(&button) {
                self.pressed.push(button);
            }
        } else {
            self.pressed.retain(|known| *known != button);
        }
    }

    /// Toggle the presentation camera between the director and free-fly. View
    /// only: the sim, the tape and every hash are untouched.
    fn toggle_free_cam(&mut self) {
        self.cam_mode = match self.cam_mode {
            CameraMode::Director => CameraMode::Free,
            CameraMode::Free => CameraMode::Director,
        };
        cursor::set_grabbed(
            &mut self.mouse_raw,
            self.window.as_deref(),
            self.cam_mode == CameraMode::Free,
        );
    }

    /// Turn the legionary mode on: the player is slot 0, seen in first person
    /// from the fighter's eye, wearing the legionary kit. The mode line says
    /// which view and which kit the run plays, and a second line names the
    /// rules and tables the match really runs under. The mode line is
    /// returned so a test can read the receipt the row names.
    fn enable_legionary(&mut self) -> String {
        let mode = LegionaryMode {
            kit: LegionaryKit::LEGIONARY,
            view: LegionaryView::Eye,
        };
        let line = mode.line();
        say!("{line}");
        if !mode.kit.backed_by_shipped_data() {
            tell!(
                "arena: --legionary: the shipped data lost the {} row",
                mode.kit.weapon
            );
        }
        let live = self.app.live_match();
        say!(
            "legionary rules: v{} tables={} slot={}",
            live.rules().version,
            live.config().table_version,
            self.app.human_slot()
        );
        self.legionary = Some(mode);
        line
    }

    /// Swap the legionary mode between the eye and the fight camera. View
    /// only: the sim, the tape and every hash are untouched, so the replay
    /// can be watched from outside without changing the match.
    fn toggle_legionary_view(&mut self) {
        let Some(mode) = self.legionary.as_mut() else {
            return;
        };
        mode.view = mode.view.toggled();
        let line = mode.line();
        say!("{line}");
    }

    /// Aim the frame at the player's eye while the legionary mode plays in
    /// first person: the eye pose over the human slot's feet, the eye-space
    /// trauma kick folded in and the render camera built from it. The lens
    /// and the far plane stay the ones of the frame it replaces, so the
    /// third person toggle changes the place and nothing else.
    /// Presentation only: the simulation, the tape and every hash are
    /// untouched. Returns `true` when the frame camera moved to the eye.
    fn aim_legionary_eye(&mut self, list: &mut DrawList) -> bool {
        if self.cam_mode == CameraMode::Free {
            return false;
        }
        let Some(mode) = self.legionary.as_ref() else {
            return false;
        };
        if mode.view != LegionaryView::Eye {
            return false;
        }
        let slot = self.app.human_slot();
        let Some(view) = self.app.fighter_views().iter().find(|one| one.slot == slot) else {
            return false;
        };
        // Only the blows the player is in, decaying between ticks: every cue in the arena for
        // one tick snapped the view in a crowded fight (see `EyeTrauma`).
        let trauma = self
            .eye_trauma
            .feed(self.ticks_done, &self.tick_scratch.cues, slot);
        let eye = match engine_game::fps_view::legionary_eye(
            view.pos_m,
            view.yaw,
            trauma,
            self.ticks_done,
        ) {
            Ok(eye) => eye,
            Err(err) => {
                tell!("arena: legionary eye: {err}");
                return false;
            }
        };
        let mut camera = match engine_game::fps_view::camera_for_eye(
            &eye,
            list.camera.fov_y,
            list.camera.aspect,
        ) {
            Ok(camera) => camera,
            Err(err) => {
                tell!("arena: legionary camera: {err}");
                return false;
            }
        };
        // The near clip sits past the wearer's own head, so the body never
        // shows from the inside, and the far plane stays the one of the frame
        // this camera replaces.
        camera.near = LEGIONARY_EYE_NEAR_M;
        camera.far = list.camera.far;
        if let Err(err) = camera.validate() {
            tell!("arena: legionary camera: {err}");
            return false;
        }
        list.camera = camera;
        true
    }

    /// The presentation camera keys: C flies the free camera, V swaps the
    /// legionary mode between the player's eye and the fight camera.
    /// Presentation only: the match, its rules and its hash stay untouched.
    fn presentation_camera_key(&mut self, code: winit::keyboard::KeyCode) {
        match code {
            winit::keyboard::KeyCode::KeyC => self.toggle_free_cam(),
            LEGIONARY_VIEW_KEY => self.toggle_legionary_view(),
            _ => {}
        }
    }

    /// One free-fly key step: arrows look (yaw/pitch), WASD move world meters
    /// on the floor plane and Q/E move down/up. The caller keeps the look
    /// inside [`free_cam_step`]'s bounds, so the function itself never clamps;
    /// a key with no mapping steps nothing. Presentation only.
    fn step_free_cam(&mut self, code: winit::keyboard::KeyCode) {
        use winit::keyboard::KeyCode;
        const LOOK_RAD: f32 = 0.1;
        const MOVE_M: f32 = 0.5;
        let (mut yaw, mut pitch) = (self.free_cam.free_yaw_rad, self.free_cam.free_pitch_rad);
        let mut move_m = [0.0f32; 3];
        match code {
            KeyCode::ArrowLeft => yaw -= LOOK_RAD,
            KeyCode::ArrowRight => yaw += LOOK_RAD,
            KeyCode::ArrowUp => pitch += LOOK_RAD,
            KeyCode::ArrowDown => pitch -= LOOK_RAD,
            KeyCode::KeyW => move_m[2] -= MOVE_M,
            KeyCode::KeyS => move_m[2] += MOVE_M,
            KeyCode::KeyA => move_m[0] -= MOVE_M,
            KeyCode::KeyD => move_m[0] += MOVE_M,
            KeyCode::KeyQ => move_m[1] -= MOVE_M,
            KeyCode::KeyE => move_m[1] += MOVE_M,
            _ => return,
        }
        yaw = yaw.clamp(-core::f32::consts::PI, core::f32::consts::PI);
        pitch = pitch.clamp(-core::f32::consts::FRAC_PI_2, core::f32::consts::FRAC_PI_2);
        // A refused step (should never happen after the clamp) is ignored, so
        // the window keeps presenting; the function itself never clamps.
        let _ = free_cam_step(&mut self.free_cam, (yaw, pitch), move_m);
    }

    /// Aim the frame camera at the free-fly pose while free mode is on: the eye
    /// is the flown position, the target one meter along the flown yaw/pitch.
    /// Presentation only; the default director run never calls this.
    fn aim_free_cam(&self, list: &mut DrawList) {
        let eye = self.free_cam.free_eye_m;
        let (sin_yaw, cos_yaw) = self.free_cam.free_yaw_rad.sin_cos();
        let (sin_pitch, cos_pitch) = self.free_cam.free_pitch_rad.sin_cos();
        list.camera.eye = [eye.x, eye.y, eye.z].into();
        list.camera.target = [
            eye.x + cos_pitch * sin_yaw,
            eye.y + sin_pitch,
            eye.z - cos_pitch * cos_yaw,
        ]
        .into();
    }

    /// Bring up the visible presenter on the winit window:
    /// backend-picked instance plus surface plus swapchain at the window
    /// size, cube plus dressing staged. Never falls back to offscreen here:
    /// the offscreen twin stays the explicit capture path
    /// (`capture_offscreen`, headless smoke), so a window that cannot paint
    /// fails loudly instead of white-screening behind exit 0. Never panics;
    /// every failure is typed.
    fn init_presenter(&mut self, event_loop: &ActiveEventLoop) {
        let Some(window) = self.window.as_ref() else {
            self.fail(
                event_loop,
                WinError::Render("no window for the visible presenter".to_string()),
                1,
            );
            return;
        };
        let size = window.inner_size();
        let (width, height) = (size.width.max(1), size.height.max(1));
        // VS7: the night preset bakes the night rig; the day frame keeps the day rig.
        match Presenter::init_for_window_with_preset(
            Arc::clone(window),
            width,
            height,
            self.night,
            self.args.uncapped,
        ) {
            Ok(mut presenter) => {
                // The live frame never waits on the GPU: fog is a fragment
                // function and the probes are off. `--look-probes` or an
                // armed capture brings the blocking probes back, spaced out
                // so they hitch rarely; the capture sidecar needs measured
                // sky_ms on the reference 300-frame run.
                presenter.set_look_probes(Presenter::look_probes_for_capture(
                    self.look_probes,
                    self.capture.is_some(),
                ));
                presenter.set_probe_every(LOOK_PROBE_EVERY);
                presenter.set_fog_every(LIVE_FOG_EVERY);
                // EG13: 4x MSAA stays off unless this run asks for it
                // (`E2040_MSAA=1`), so the default window frame keeps the
                // 1x path the look captures were taken with.
                let msaa_on = msaa_setting(std::env::var(MSAA_ENV).ok().as_deref());
                presenter.set_msaa_enabled(msaa_on);
                // EG13: the perf run says which sample count it measured, so a
                // p50 pair cannot be mistaken for the same path twice.
                tell!("arena msaa: {}", if msaa_on { "4x" } else { "1x" });
                // GS43: contact shadows ship on; `E2040_CONTACT_SHADOW=0`
                // keeps the identical frame without them for the capture pair.
                presenter.set_contact_shadow_strength(contact_shadow_strength(
                    std::env::var(CONTACT_SHADOW_ENV).ok().as_deref(),
                ));
                let key = MeshKey::new(1, [0xA1; 32]);
                // A refused cube falls back to clear-only frames, never a panic.
                let _ = presenter.upload_mesh(key, &engine_render::v1::cube());
                // Dressing beside the cube; refused uploads skip draws.
                match stage_dressing(&mut presenter) {
                    Ok(staged) => tell!("arena: staged {staged} dressing boxes"),
                    Err(err) => {
                        self.fail(event_loop, err, 1);
                        return;
                    }
                }
                // Arena kit over the placement boxes; a refused stage keeps
                // the cube/dressing frame (the cube is the visible error
                // mesh), never fails the window.
                self.kit = stage_kit_or_default(&mut presenter);
                tell!("arena: {}", live_kit_notice(&self.kit));
                // CT7 fighters (bodies re-hashed against their pins); a
                // refusal is loud and keeps the staged boxes.
                match FighterLook::load_and_stage(&mut presenter, self.config.half_extent_m) {
                    Ok(mut look) => {
                        tell!(
                            "arena: fighter look bound {} CT7 bodies",
                            look.bundle_shas().len()
                        );
                        look.set_hero(self.app.human_slot());
                        // `--no-ragdoll` (RZ2): the floored-fighter layer stays off, so the
                        // off half of a capture pair shows the flat death clip.
                        look.set_ragdoll(!self.ragdoll_off);
                        if self.ragdoll_off {
                            tell!(
                                "arena: {NO_RAGDOLL_FLAG}: a floored fighter keeps the death clip"
                            );
                        }
                        self.look = Some(look);
                        // The look poses from views: the loco pose cursor
                        // (two whole-prefix replays per tick) is unused.
                        self.app.set_presentation_cursor(false);
                    }
                    Err(err) => self.look_refused(&err.to_string()),
                }
                self.presenter_notice = format!(
                    "presenter surface: {} ({}) {width}x{height}",
                    presenter.adapter_name(),
                    presenter.adapter_driver()
                );
                crash::set_adapter(&format!(
                    "{} ({})",
                    presenter.adapter_name(),
                    presenter.adapter_driver()
                ));
                tell!("arena: {}", self.presenter_notice);
                self.presenter = Some(presenter);
            }
            Err(err) => self.fail(event_loop, WinError::Present(err), 1),
        }
    }
}

impl ArenaApp {
    /// Fight and shell keys, routed only when the console is closed.
    fn game_key(&mut self, event_loop: &ActiveEventLoop, event: &winit::event::KeyEvent) {
        if let winit::keyboard::PhysicalKey::Code(code) = event.physical_key {
            // Escape pauses without repeat: holding the key never
            // flickers the shell.
            if code == winit::keyboard::KeyCode::Escape
                && event.state == ElementState::Pressed
                && !event.repeat
            {
                self.flow = self.flow.on(FlowEvent::TogglePause);
            }
            // Shell keys without repeat: Title starts on Enter/Space; Result
            // rematches on R, titles on T (both clear the finished bout).
            let struck = event.state == ElementState::Pressed && !event.repeat;
            self.speed_demo_key(code, event.state == ElementState::Pressed);
            // `--step`: one Space press advances exactly one held tick.
            // A no-op outside the running bout, so Title/Space still
            // only starts the bout (see `pause_decision`).
            if struck
                && self.step_mode
                && self.flow == Flow::Match
                && code == winit::keyboard::KeyCode::Space
            {
                self.step_requested = true;
            }
            // Slow-mo: [ and ] ease the virtual clock one preset slower or faster.
            if struck
                && matches!(
                    code,
                    winit::keyboard::KeyCode::BracketLeft | winit::keyboard::KeyCode::BracketRight
                )
            {
                let faster = code == winit::keyboard::KeyCode::BracketRight;
                let next = step_speed(self.clock.speed_permille(), faster);
                if let Err(err) = self.clock.set_speed(next, KEY_RAMP_MS) {
                    tell!("arena: speed {next}: {err}");
                }
            }
            // Free-fly: C toggles the presentation camera (F is the
            // attack key); while it is on, WASD/arrows fly the eye one
            // step per press. Never touches the sim (presentation only).
            if struck {
                self.presentation_camera_key(code);
            }
            if struck && self.cam_mode == CameraMode::Free {
                self.step_free_cam(code);
            }
            if struck
                && self.flow == Flow::Title
                && (code == winit::keyboard::KeyCode::Enter
                    || code == winit::keyboard::KeyCode::Space)
            {
                self.flow = self.flow.on(FlowEvent::Start);
            }
            if struck && self.flow == Flow::Result && code == winit::keyboard::KeyCode::KeyR {
                if let Err(err) = self.reset_bout() {
                    return self.fail(event_loop, err, 2);
                }
                self.flow = self.flow.on(FlowEvent::Rematch);
            }
            if struck && self.flow == Flow::Result && code == winit::keyboard::KeyCode::KeyT {
                if let Err(err) = self.reset_bout() {
                    return self.fail(event_loop, err, 2);
                }
                self.flow = self.flow.on(FlowEvent::ToTitle);
            }
            if let Some(key) = arena_winit::from_winit_key(code) {
                self.set_pressed(map_key(key), event.state == ElementState::Pressed);
            }
        }
    }
}

impl ApplicationHandler for ArenaApp {
    /// Raw motion feeds the free camera while it holds the cursor.
    fn device_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _device_id: winit::event::DeviceId,
        event: DeviceEvent,
    ) {
        cursor::push_event(&mut self.mouse_raw, &event);
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        // Display gate before the window exists: `modes` owns the startup
        // mode's validation, so an out-of-range size refuses loudly here
        // instead of opening a window no adapter can fill. Adapter
        // enumeration stays operator-side, so the ranked list is seeded with
        // the requested mode and the check is a range check.
        // `--size` names the inner size in physical pixels, so a 1080p run
        // opens a 1920x1080 window on any display scale; without the flag
        // the window opens at `WINDOW_SIZE`, byte-identical to today.
        let (w_px, h_px) = self.args.window_size();
        let startup = SwapRequest {
            mode: DisplayMode::Windowed { w_px, h_px },
        };
        let ranked = rank_modes(&[AdapterMode {
            w_px,
            h_px,
            refresh_hz: engine_core::TICK_HZ_U32,
        }]);
        if let Err(err) = validate_request(&startup, &ranked) {
            return self.fail(event_loop, WinError::NoDisplay(capped(&err.to_string())), 3);
        }
        let attrs = Window::default_attributes()
            .with_title(arena_frame::window_title(None))
            .with_inner_size(winit::dpi::PhysicalSize::new(w_px, h_px));
        let window = match event_loop.create_window(attrs) {
            Ok(window) => Arc::new(window),
            Err(err) => {
                return self.fail(event_loop, WinError::NoDisplay(capped(&err.to_string())), 3);
            }
        };
        self.window = Some(window);
        self.init_presenter(event_loop);
        self.last = Instant::now();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Focused(focused) => self.note_window_state(focused, self.window_occluded),
            WindowEvent::Occluded(occluded) => {
                self.note_window_state(self.window_focused, occluded);
            }
            WindowEvent::Resized(size) => {
                if let Some(presenter) = self.presenter.as_mut() {
                    match presenter.resize(size.width, size.height) {
                        Ok(_) => {}
                        Err(err) => self.fail(event_loop, WinError::Present(err), 1),
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if self.console_owns_key(&event) {
                    return;
                }
                self.game_key(event_loop, &event);
            }
            WindowEvent::MouseInput { button, state, .. } => {
                if self.console.is_open() {
                    return;
                }
                let pressed = state == ElementState::Pressed;
                self.speed_demo_click(pressed);
                self.set_pressed(map_mouse(arena_winit::from_winit_mouse(button)), pressed);
            }
            WindowEvent::RedrawRequested => {
                self.tick_and_present(event_loop);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // The window presents one frame per slot and parks until the next
        // slot is due. A free-running window presents far above the display
        // rate, and the compositor throttles it with ~80 ms swapchain
        // acquire stalls (the 2026-10-20 perf lane).
        let now = Instant::now();
        let due = self.pacer.deadline();
        // `wait_duration` takes a std Duration: winit's WaitUntil wants its own
        // Instant type, which is web_time's on wasm and not std's.
        if now < due {
            event_loop.set_control_flow(ControlFlow::wait_duration(
                due.saturating_duration_since(now),
            ));
            return;
        }
        self.pacer.advance(now);
        let next = self.pacer.deadline();
        event_loop.set_control_flow(ControlFlow::wait_duration(
            next.saturating_duration_since(now),
        ));
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::arena_capture::capture::{
        chart_encode_png_rgba8, format_headless_capture_line, resolve_capture_scene, ChartArgs,
        CHART_VIEWPORT,
    };
    use super::*;
    use engine_game::capture as cap;
    use engine_game::windowed::{
        check_exit_deadline, check_exit_deadline_with_limit, fight_mesh_key, frame_times_csv_row,
        hud_draw_list, SceneSelect, FRAME_TIMES_CSV_HEADER,
    };
    use engine_game::HUMAN_SLOT;
    use engine_input::FighterInput;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_string()).collect()
    }

    #[test]
    fn look_probes_flag_splits_off_and_defaults_off() {
        let (on, rest) = arena_args::split_flag(
            &strings(&["arena", "--frames", "3", "--look-probes", "--exit"]),
            LOOK_PROBES_FLAG,
        );
        assert!(on, "the flag turns the blocking probes on");
        assert_eq!(rest, strings(&["arena", "--frames", "3", "--exit"]));
        let (off, same) = arena_args::split_flag(&strings(&["arena", "--exit"]), LOOK_PROBES_FLAG);
        assert!(!off, "probes ship off");
        assert_eq!(same, strings(&["arena", "--exit"]));
        // The strict parser still refuses the flag if it were not split.
        assert!(WinArgs::parse(&strings(&["arena", "--look-probes"])).is_err());
    }

    #[test]
    fn legionary_flag_splits_off_and_the_strict_parser_still_refuses_it() {
        let (on, rest) = arena_args::split_flag(
            &strings(&["arena", "--legionary", "--exit"]),
            LEGIONARY_FLAG,
        );
        assert!(on, "the flag turns the mode on");
        assert_eq!(rest, strings(&["arena", "--exit"]));
        let (off, same) = arena_args::split_flag(&strings(&["arena", "--exit"]), LEGIONARY_FLAG);
        assert!(!off, "the mode ships off");
        assert_eq!(same, strings(&["arena", "--exit"]));
        // The strict parser still refuses the flag if it were not split.
        assert!(WinArgs::parse(&strings(&["arena", "--legionary"])).is_err());
    }

    #[test]
    fn the_legionary_mode_plays_the_duel_on_rules_v2_with_slot_0_as_the_player() {
        let app = live_session(bin_config(), false, false).expect("session builds");
        let live = app.live_match();
        assert_eq!(
            live.rules(),
            engine_arena::ArenaRules::v2_free_for_all(),
            "the legionary mode plays rules v2"
        );
        assert_eq!(
            live.config().table_version,
            engine_combat::tables::TABLES_VERSION_V2,
            "rules v2 runs on the v2 tables"
        );
        assert_eq!(app.human_slot(), 0, "slot 0 is the player");
    }

    #[test]
    fn the_legionary_view_toggle_leaves_the_match_alone() {
        let args =
            WinArgs::parse(&strings(&["arena", "--frames", "60", "--exit"])).expect("args parse");
        let mut app = ArenaApp::new(bin_config(), args, None).expect("session builds");
        let line = app.enable_legionary();
        assert_eq!(
            line, "legionary: eye=fps kit=galea,gladius,scutum",
            "the mode line the row names"
        );
        app.run_sim_ticks(app.config.ticks, app.human().expect("human slot"));
        let before: Vec<u64> = app.app.checkpoints().iter().map(|one| one.hash).collect();
        assert!(!before.is_empty(), "the match really ran");
        app.toggle_legionary_view();
        app.toggle_legionary_view();
        let after: Vec<u64> = app.app.checkpoints().iter().map(|one| one.hash).collect();
        assert_eq!(before, after, "the toggle must not move the match");
        let mode = app.legionary.as_ref().expect("the mode is on");
        assert_eq!(mode.view, LegionaryView::Eye, "two toggles come back");
        assert_eq!(mode.kit.line(), "galea,gladius,scutum", "the kit line");
    }

    #[test]
    fn the_third_person_view_is_one_key_away_on_a_free_key() {
        // The view key is V: one key from the eye to the fight camera.
        assert_eq!(LEGIONARY_VIEW_KEY, winit::keyboard::KeyCode::KeyV);
        assert!(
            CONTROLS_LINE.contains("V first/third person (--legionary)"),
            "the boot line names the key: {CONTROLS_LINE}"
        );
        // V is no fighter action, so the toggle never eats a move.
        assert_eq!(
            arena_winit::from_winit_key(LEGIONARY_VIEW_KEY),
            None,
            "the view key must stay off the bindings table"
        );
    }

    #[test]
    fn the_legionary_eye_takes_the_frame_and_the_toggle_gives_it_back() {
        let args =
            WinArgs::parse(&strings(&["arena", "--frames", "60", "--exit"])).expect("args parse");
        let mut app = ArenaApp::new(bin_config(), args, None).expect("session builds");
        app.enable_legionary();
        app.run_sim_ticks(app.config.ticks, app.human().expect("human slot"));
        let mut list = hud_draw_list(1280, 720).expect("hud list builds");
        assert!(
            app.aim_legionary_eye(&mut list),
            "the eye must take the frame"
        );
        assert!(list.camera.eye.y > 1.0, "the eye sits over the feet");
        assert_eq!(
            list.camera.near, LEGIONARY_EYE_NEAR_M,
            "the own-body clip rides the eye camera"
        );
        // The third person view gives the fight camera back.
        app.toggle_legionary_view();
        let mut third = hud_draw_list(1280, 720).expect("hud list builds");
        assert!(
            !app.aim_legionary_eye(&mut third),
            "the eye must step aside"
        );
    }

    #[test]
    fn view_line_flag_splits_off_and_refuses_bad_numbers() {
        let (request, rest) = arena_args::split_view_line(&strings(&[
            "arena",
            "--view-line",
            "2040",
            "600",
            "2",
            "--frames",
            "3",
        ]))
        .expect("a clean request parses");
        assert_eq!(request, Some((2040, 600, 2)));
        assert_eq!(rest, strings(&["arena", "--frames", "3"]));
        assert_eq!(
            arena_args::split_view_line(&strings(&["arena", "--frames", "3"]))
                .expect("absent is fine")
                .0,
            None
        );
        for bad in [
            vec!["arena", "--view-line"],
            vec!["arena", "--view-line", "2040", "600"],
            vec!["arena", "--view-line", "x", "600", "2"],
            vec!["arena", "--view-line", "2040", "x", "2"],
            vec!["arena", "--view-line", "2040", "600", "x"],
            vec!["arena", "--view-line", "2040", "600", "1"],
            // The cap is `engine_input::MAX_PARTICIPANTS` (32 since GE12), so
            // one over it is the count that must refuse.
            vec!["arena", "--view-line", "2040", "600", "33"],
            vec!["arena", "--view-line", "2040", "100001", "2"],
            vec!["arena", "--view-line", "2040", "--frames", "2"],
        ] {
            assert!(
                arena_args::split_view_line(&strings(&bad)).is_err(),
                "bad request {bad:?} refuses"
            );
        }
        // The strict parser still refuses the flag if it were not split.
        assert!(WinArgs::parse(&strings(&["arena", "--view-line"])).is_err());
    }

    #[test]
    fn kit_restaging_skips_while_the_look_is_bound() {
        assert!(
            !should_push_kit(true),
            "bound look clears kit draws before submit, so restaging is wasted work"
        );
        assert!(
            should_push_kit(false),
            "refused look keeps the cube frame, so kit draws still push"
        );
    }

    #[test]
    fn bound_look_builds_the_cheap_frame_list() {
        // The skinned look clears the frame list before its ring draws, so
        // the twin kit import that `frame_list` runs every frame is discarded
        // work (~1 s of glTF parses per frame). A bound look must take the
        // cheap camera-only list; a refused look keeps the full twin list.
        assert!(
            should_build_cheap_list(true),
            "bound look discards twin items, so skip the twin import"
        );
        assert!(
            !should_build_cheap_list(false),
            "refused look draws the twin items, so keep the full list"
        );
    }

    #[test]
    fn cheap_frame_list_carries_the_orbit_camera_with_no_twin_items() {
        // Camera parity: the cheap list aims the same dressing-plus-orbit eye
        // as the full twin list and carries the same light, with no items
        // for the look to clear. Failure case: an absurd size refuses loudly
        // instead of filing an empty frame; a zero size masks to 1 exactly
        // like the full twin list (`width.max(1)`), so it still builds.
        let cheap = cheap_frame_list(1280, 720, None, 7).expect("cheap list builds");
        let orbit = engine_game::windowed::dressing_camera(1280, 720).expect("orbit base builds");
        assert_eq!(
            cheap.camera.eye.to_array(),
            orbit.eye.to_array(),
            "cheap list keeps the establishing eye"
        );
        assert_eq!(
            cheap.camera.target.to_array(),
            orbit.target.to_array(),
            "cheap list keeps the establishing target"
        );
        assert!(
            cheap.items.is_empty(),
            "no twin items for the look to clear, got {}",
            cheap.items.len()
        );
        assert!(cheap_frame_list(0, 720, None, 0).is_ok());
        assert!(cheap_frame_list(9999, 64, None, 0).is_err());
    }

    #[test]
    fn fighter_cache_reuses_only_the_same_pose_tick() {
        assert!(
            should_reuse_fighters(Some(7), 8, 7, 8),
            "same tick plus fighters reuses the skin"
        );
        assert!(
            !should_reuse_fighters(None, 8, 7, 8),
            "no cached tick skins fresh"
        );
        assert!(
            !should_reuse_fighters(Some(6), 8, 7, 8),
            "a stepped tick re-skins, never reuses stale vertices"
        );
        assert!(
            !should_reuse_fighters(Some(7), 8, 7, 2),
            "a fighter-count change re-skins"
        );
    }

    #[test]
    fn draw_order_is_stable_x2_and_keeps_every_draw() {
        fn item(id: u64, red: f32) -> engine_render::present::DrawItem {
            engine_render::present::DrawItem {
                mesh: MeshKey::new(id, [0xA1; 32]),
                material: engine_render::v1::PbrMaterial {
                    base_color: [red, 0.5, 0.5],
                    metallic: 0.0,
                    roughness: 0.5,
                    exposure: 1.0,
                },
                base_color: None,
                normal_map: None,
                orm: None,
                emissive: None,
            }
        }
        let mut first = hud_draw_list(1280, 720).expect("HUD list builds for the order probe");
        first.items.push(item(9, 0.2));
        first.items.push(item(3, 0.8));
        first.items.push(item(3, 0.1));
        let mut second = first.clone();
        second.items.reverse();
        sort_draw_list(&mut first);
        sort_draw_list(&mut second);
        assert_eq!(first.items.len(), 3, "sorting never drops a draw");
        assert_eq!(
            first.items, second.items,
            "same tick in another input order submits the same list x2"
        );
        let ids: Vec<u64> = first.items.iter().map(|draw| draw.mesh.id).collect();
        assert_eq!(
            ids,
            vec![3, 3, 9],
            "mesh id is the first tie-break, got {ids:?}"
        );
        assert!(
            first.items[0].material.base_color[0] < first.items[1].material.base_color[0],
            "duplicate mesh ids break ties by material"
        );
    }

    #[test]
    fn draw_order_empty_and_singleton_hold() {
        let mut empty = hud_draw_list(1280, 720).expect("HUD list builds for the empty probe");
        sort_draw_list(&mut empty);
        assert!(
            empty.items.is_empty(),
            "empty list stays empty, never panics"
        );
    }

    #[test]
    fn capture_arms_the_look_probes_for_the_sidecar() {
        // Behavior: the capture per-pass sidecar needs measured sky_ms on
        // the reference 300-frame run, so an armed capture enables the
        // blocking probes even when `--look-probes` is off; live without
        // capture stays non-blocking (Game Engine Gems 1 ch 9.6).
        assert!(
            !Presenter::look_probes_for_capture(false, false),
            "live stays non-blocking"
        );
        assert!(
            Presenter::look_probes_for_capture(true, false),
            "the flag still enables"
        );
        assert!(
            Presenter::look_probes_for_capture(false, true),
            "capture needs measured sky_ms"
        );
        assert!(Presenter::look_probes_for_capture(true, true));
    }

    #[test]
    fn capture_sidecar_means_sky_over_probed_frames() {
        // Failing-before: live-only averages to 0.000 sky, so the sidecar
        // would file 0 ms; one probed microsecond must average above 0.
        let args = WinArgs::parse(&strings(&["arena", "--frames", "2", "--exit"]))
            .expect("valid smoke fixture parses");
        let mut app = ArenaApp::new(bin_config(), args, None).expect("test session builds");
        app.pass_adapter = "RTX".to_string();
        app.pass_frames = 1;
        app.pass_probes = 0;
        app.pass_us = [0; 8];
        assert_eq!(app.pass_mean(0), "0.000", "live-only sky stays 0");
        app.pass_us[0] = 1000;
        app.pass_probes = 1;
        assert_eq!(app.pass_mean(0), "1.000", "probed sky measures above 0");
    }

    #[test]
    fn framing_counts_frames_with_no_fighter_and_fails_only_on_request() {
        let mut framing = Framing::default();
        assert_eq!(framing.line(), None, "no look drawn, no line");
        framing.note(0, 2, 21.0);
        framing.note(1, 1, 30.0);
        assert_eq!(framing.failure(), None);
        framing.note(2, 0, 74.0);
        framing.note(3, 0, 74.5);
        framing.note_tint((1, 8));
        framing.note_tint((0, 8));
        assert_eq!(
            framing.line().as_deref(),
            Some("arena framing: frames=4 empty=2 first_empty=2 fewest_in_view=0 steepest_down_deg=74.5 tinted_share=0.06")
        );
        assert_eq!(
            framing.failure().as_deref(),
            Some("2 of 4 frames showed no fighter (first at frame 2)")
        );
        let (on, rest) = arena_args::split_flag(
            &strings(&["arena", "--require-framing", "--exit"]),
            REQUIRE_FRAMING_FLAG,
        );
        assert!(on);
        assert_eq!(rest, strings(&["arena", "--exit"]));
    }

    #[test]
    fn framing_line_names_its_denominator() {
        // `fewest_in_view=2` alone cannot be read: 2 of 2 is a clean frame,
        // 2 of 8 is a fault. Once counted, the line says out of how many.
        let mut framing = Framing::default();
        framing.note(0, 2, 17.0);
        framing.note_count(8, 8);
        let line = framing.line().expect("a tallied frame prints");
        assert!(line.contains(" fighters=8 seen=8 "), "{line}");
        assert!(line.contains("fewest_in_view=2"), "{line}");
        // The seen count is the worst frame's; the fighter count the match's.
        framing.note(1, 5, 17.0);
        framing.note_count(8, 6);
        let line = framing.line().expect("a tallied frame prints");
        assert!(line.contains(" fighters=8 seen=6 "), "{line}");
    }

    #[test]
    fn zero_tick_frame_is_excluded_from_framing_tally() {
        // The first redraw buffers about 12 ms for 0 steps, so no
        // `observe_tick` runs before the first tally and every slot is
        // unseen. A frame with no observed tick must not enter the tally;
        // the warmed-up first frame counts its 8 seen fighters instead.
        assert!(!framing_should_tally(0), "0 observed ticks tallies nothing");
        assert!(framing_should_tally(1));
        assert_eq!(
            warmup_steps(0, 0, 600, true),
            1,
            "first shown frame warms up"
        );
        assert_eq!(
            warmup_steps(0, 0, 600, false),
            0,
            "a suspended window holds the match"
        );
        assert_eq!(
            warmup_steps(0, 1, 600, true),
            0,
            "later 0-step frames stay 0"
        );
        assert_eq!(warmup_steps(2, 0, 600, true), 2, "owed steps are kept");
        assert_eq!(warmup_steps(0, 0, 0, true), 0, "no session, no warmup");
        // Old code tallied frame 0 with 0 in view (empty 1/1); the guard
        // skips it and the warmed-up frame counts 8 (empty 0/1).
        let mut framing = Framing::default();
        if framing_should_tally(0) {
            framing.note(0, 0, 74.0);
        }
        assert_eq!(framing.frames, 0, "the 0-tick frame leaves no tally");
        if framing_should_tally(1) {
            framing.note(0, 8, 21.0);
            framing.note_tint((0, 8));
        }
        assert_eq!((framing.frames, framing.empty), (1, 0));
        assert_eq!(framing.first_empty, None);
        assert_eq!(framing.fewest, 8);
    }

    #[test]
    fn a_blocking_live_frame_fails_the_smoke_and_probe_frames_do_not() {
        let mut stats = WaitStats::default();
        assert!(live_wait_failure(&stats).is_none(), "no frames, no failure");
        let mut probe = engine_render::present::FrameWaits::default();
        probe.map_and_wait_n(8);
        stats.fold(engine_render::present::FrameKind::Probe, probe);
        stats.fold(
            engine_render::present::FrameKind::Live,
            engine_render::present::FrameWaits::default(),
        );
        assert!(
            live_wait_failure(&stats).is_none(),
            "probe waits are tallied apart: {}",
            stats.summary()
        );
        let mut blocked = engine_render::present::FrameWaits::default();
        blocked.map_and_wait();
        stats.fold(engine_render::present::FrameKind::Live, blocked);
        let message = live_wait_failure(&stats).expect("a blocking live frame fails");
        assert!(message.contains("maps=1 waits=1"), "{message}");
    }

    #[test]
    fn the_window_presents_one_frame_per_slot_and_the_harness_is_opt_in() {
        // The default window paces itself at the fixed tick period: it
        // presents one frame per slot instead of free-running.
        let args = WinArgs::parse(&strings(&["arena", "--frames", "3", "--exit"]))
            .expect("smoke args parse");
        let app = ArenaApp::new(bin_config(), args, None).expect("session builds");
        assert!(
            !app.fixed_dt_ticks,
            "the default run keeps the wall-clock accumulator"
        );
        assert_eq!(
            app.pacer.period(),
            Duration::from_secs_f64(engine_game::windowed::WIN_FIXED_DT_S),
            "the default pace is the game's own tick rate"
        );
        // With the flag the pacer carries the period the CLI named and the
        // sim runs one fixed tick per presented frame.
        let args = WinArgs::parse(&strings(&[
            "arena",
            "--frames",
            "3",
            "--fixed-dt",
            "17",
            "--exit",
        ]))
        .expect("smoke args parse");
        let app = ArenaApp::new(bin_config(), args, None).expect("session builds");
        assert!(app.fixed_dt_ticks, "the harness runs one tick per frame");
        assert_eq!(
            app.pacer.period(),
            Duration::from_millis(17),
            "the pacer carries the named period"
        );
    }

    #[test]
    fn the_size_flag_names_the_window_the_arena_opens() {
        // The flag names the window inner size; the default keeps the
        // shipped 1280x720, so an existing run is byte-identical to today.
        let args = WinArgs::parse(&strings(&["arena", "--size", "1920x1080"]))
            .expect("a 1080p size parses");
        assert_eq!(args.window_size(), (1920, 1080));
        let plain = WinArgs::parse(&strings(&["arena"])).expect("plain args parse");
        assert_eq!(plain.window_size(), (1280, 720));
    }

    #[test]
    fn frame_percentiles_use_nearest_rank_over_microseconds() {
        assert_eq!(percentile_us(&[], 50), 0, "no frames reads zero");
        assert_eq!(percentile_us(&[7_000], 99), 7_000);
        let ascending: Vec<u64> = (1..=100).map(|ms| ms * 1_000).collect();
        assert_eq!(percentile_us(&ascending, 50), 50_000);
        assert_eq!(percentile_us(&ascending, 99), 99_000);
        assert_eq!(fmt_us_as_ms(16_667), "16.667");
        assert_eq!(fmt_us_as_ms(305), "0.305");
    }

    fn bin_temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("e2040-arena-cap-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("test temp dir builds");
        dir
    }

    fn bin_config() -> GameConfig {
        GameConfig {
            seed: 2040,
            ticks: 24,
            fighters: 2,
            half_extent_m: 20.0,
        }
    }

    #[test]
    fn staged_window_leg_replays_the_receipt_fight() {
        // Failing-before: the window built the staged leg over the kit world
        // under rules v2 (12 hits, 1 knockdown) and failed its own pinned
        // receipt; the receipt's session is the empty world under rules v1.
        let args = WinArgs::parse(&strings(&["arena", "--frames", "600", "--exit"]))
            .expect("smoke args parse");
        use engine_game::windowed::{STAGED_FIGHT_HITS, STAGED_FIGHT_KNOCKDOWNS};
        let mut app = ArenaApp::new(staged_session_config(), args, None).expect("session builds");
        app.staged = Some(StagedTally::default());
        assert!(app.stage_receipt_session().is_none(), "the swap lands");
        app.run_sim_ticks(app.config.ticks, app.human().expect("human slot"));
        let tally = app.staged.expect("staged tally");
        assert_eq!(tally.hits, STAGED_FIGHT_HITS);
        assert_eq!(tally.knockdowns, STAGED_FIGHT_KNOCKDOWNS);
    }

    #[test]
    fn staged_fight_cues_reach_the_window_feel_flow() {
        // GF2/GF3: the tick loop hands every tick's cues to the feel view, and
        // one presented-frame step turns the staged fight's hits into a frozen
        // clock and a shaken camera. Red when `run_sim_ticks` stops observing.
        let args = WinArgs::parse(&strings(&["arena", "--frames", "600", "--exit"]))
            .expect("smoke args parse");
        let mut app = ArenaApp::new(staged_session_config(), args, None).expect("session builds");
        app.staged = Some(StagedTally::default());
        assert!(app.stage_receipt_session().is_none(), "the swap lands");
        app.run_sim_ticks(app.config.ticks, app.human().expect("human slot"));
        app.feel.step();
        let stats = app.feel.stats();
        assert!(
            stats.triggers > 0,
            "the fight's cues reached the flow: {stats:?}"
        );
        assert!(stats.shaken > 0 && stats.stopped > 0, "{stats:?}");
        assert!(
            app.feel.scale_elapsed_ms(1000.0) < 1000.0,
            "the clock slowed"
        );
    }

    #[test]
    fn capture_flags_flow_into_winargs() {
        let raw = strings(&[
            "arena",
            "--frames",
            "60",
            "--capture-dir",
            "out",
            "--capture-every",
            "4",
            "--sha",
            "deadbeef1234",
            "--exit",
        ]);
        let (flags, rest) = cap::split_capture_args(&raw).expect("valid capture fixture splits");
        let flags = flags.expect("capture fixture carries flags");
        assert_eq!(flags.dir, std::path::PathBuf::from("out"));
        assert_eq!(flags.every, 4);
        assert_eq!(flags.tape, None);
        assert_eq!(flags.rows, vec!["SHOW1".to_string()]);
        assert_eq!(flags.sha, "deadbeef1234");
        assert_eq!(flags.fps, 30);
        // The remainder still parses as window args, untouched and in order.
        let parsed = WinArgs::parse(&rest).expect("capture remainder parses");
        assert_eq!(parsed.frames, Some(60));
        assert!(parsed.exit);
    }

    #[test]
    fn capture_unknown_flag_refused_in_binary() {
        let raw = strings(&["arena", "--capture-fps", "30"]);
        assert!(cap::split_capture_args(&raw).is_err());
        // And a non-capture unknown still reaches `WinArgs`, which refuses it.
        let raw = strings(&["arena", "--nope"]);
        let (flags, rest) = cap::split_capture_args(&raw).expect("non-capture passthrough splits");
        assert_eq!(flags, None);
        assert!(WinArgs::parse(&rest).is_err());
    }

    #[test]
    fn capture_headless_fallback_writes_pngs_and_manifest() {
        let root = bin_temp_dir("fallback");
        let tape_path = root.join("script.tape");
        let inputs: Vec<FighterInput> = (0..12u32)
            .map(|tick| FighterInput {
                tick,
                fighter: FighterId::new(HUMAN_SLOT).expect("slot 0 is a fighter"),
                mx: 100,
                mz: 0,
                facing: 0,
                buttons: engine_input::Buttons::NONE,
            })
            .collect();
        std::fs::write(&tape_path, cap::encode_tape_bytes(&inputs)).expect("test tape writes");
        let dir = root.join("clip");
        let flags = cap::CaptureFlags {
            dir: dir.clone(),
            every: 3,
            tape: Some(tape_path),
            rows: vec!["SHOW1".to_string()],
            sha: "deadbeef1234".to_string(),
            fps: 30,
        };
        let args = WinArgs {
            frames: Some(12),
            exit: true,
            seed: None,
            ticks: None,
            fighters: None,
            scene: None,
            timed_smoke: false,
            backend: None,
            camera_path: None,
            mute: true,
            deadline_ms: None,
            present_every: 1,
            present_skip: false,
            fixed_dt_ms: None,
            window_size: None,
            uncapped: false,
        };
        let display = WinError::NoDisplay("test headless".to_string());
        assert_eq!(
            headless_fallback(bin_config(), &args, &display, &Some(flags)),
            0
        );
        // Ticks 0, 3, 6, 9 captured plus the manifest beside them.
        for index in 0..4u32 {
            let png = std::fs::read(dir.join(format!("frame_{index:06}.png")))
                .expect("capture writes headed PNGs");
            assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        }
        let manifest =
            std::fs::read_to_string(dir.join("manifest.json")).expect("capture writes manifest");
        assert!(manifest.contains("\"seed\":2040"), "manifest:\n{manifest}");
        assert!(
            manifest.contains("\"tape\":\"script.tape\""),
            "manifest:\n{manifest}"
        );
        assert!(
            manifest.contains("\"rows\":[\"SHOW1\"]"),
            "manifest:\n{manifest}"
        );
        assert!(
            manifest.contains("\"sha\":\"deadbeef1234\""),
            "manifest:\n{manifest}"
        );
        std::fs::remove_dir_all(&root).expect("test temp dir cleans");
    }

    #[test]
    fn capture_headless_fallback_rejects_bad_tape_loudly() {
        let root = bin_temp_dir("badtape");
        let dir = root.join("clip");
        let flags = cap::CaptureFlags {
            dir,
            every: 2,
            tape: Some(root.join("missing.tape")),
            rows: vec!["SHOW1".to_string()],
            sha: "deadbeef1234".to_string(),
            fps: 30,
        };
        let args = WinArgs {
            frames: Some(6),
            exit: true,
            seed: None,
            ticks: None,
            fighters: None,
            scene: None,
            timed_smoke: false,
            backend: None,
            camera_path: None,
            mute: true,
            deadline_ms: None,
            present_every: 1,
            present_skip: false,
            fixed_dt_ms: None,
            window_size: None,
            uncapped: false,
        };
        let display = WinError::NoDisplay("test headless".to_string());
        assert_eq!(
            headless_fallback(bin_config(), &args, &display, &Some(flags)),
            2
        );
        std::fs::remove_dir_all(&root).expect("test temp dir cleans");
    }

    #[test]
    fn headless_fallback_carries_camera_path_without_failing() {
        // `--camera-path arena600` flows through the headless fallback into
        // the capture: the orbit pins the session seed while either renderer
        // (twin with an adapter, synth without) still files its frames.
        let root = bin_temp_dir("orbitfb");
        let dir = root.join("clip");
        let flags = cap::CaptureFlags {
            dir: dir.clone(),
            every: 2,
            tape: None,
            rows: vec!["SHOW1".to_string()],
            sha: "deadbeef1234".to_string(),
            fps: 30,
        };
        let args = WinArgs::parse(&strings(&[
            "arena",
            "--frames",
            "6",
            "--exit",
            "--camera-path",
            "arena600",
        ]))
        .expect("orbit fixture parses");
        assert_eq!(
            args.camera_path,
            Some(engine_game::windowed::CameraPath::Arena600)
        );
        let config = session_config(&args).expect("orbit session builds");
        assert_eq!(config.seed, engine_game::windowed::ARENA600_SEED);
        let display = WinError::NoDisplay("test headless".to_string());
        assert_eq!(headless_fallback(config, &args, &display, &Some(flags)), 0);
        // 6 ticks at every 2: three frames on either renderer.
        for index in 0..3u32 {
            let png = std::fs::read(dir.join(format!("frame_{index:06}.png")))
                .expect("capture writes headed PNGs");
            assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        }
        let manifest =
            std::fs::read_to_string(dir.join("manifest.json")).expect("capture writes manifest");
        assert!(manifest.contains("\"seed\":2040"), "manifest:\n{manifest}");
        std::fs::remove_dir_all(&root).expect("test temp dir cleans");
    }

    #[test]
    fn session_config_selects_arena8_and_timed_smoke() {
        // On the binary: `--scene arena --fighters 8` flows
        // into the session while `--timed-smoke` only adds the fps line.
        let args = WinArgs::parse(&strings(&[
            "arena",
            "--frames",
            "60",
            "--exit",
            "--fighters",
            "8",
            "--scene",
            "arena",
            "--timed-smoke",
        ]))
        .expect("valid arena8 fixture parses");
        assert!(args.timed_smoke);
        let config = session_config(&args).expect("arena8 session builds");
        assert_eq!(
            (config.seed, config.fighters, config.half_extent_m),
            (
                DEFAULT_SEED,
                engine_game::windowed::ARENA8_FIGHTERS,
                engine_game::windowed::ARENA_HALF_EXTENT_M
            )
        );
        // The 8-fighter session still validates for the capture path.
        config
            .validate()
            .expect("known-good arena8 session validates");
        // Defaults stay a duel.
        let plain =
            session_config(&WinArgs::parse(&strings(&["arena"])).expect("default fixture parses"))
                .expect("default duel session builds");
        assert_eq!(plain.fighters, 2);
        // Bad fighter counts never reach a session.
        let bad = WinArgs::parse(&strings(&["arena", "--fighters", "1"]));
        assert!(bad.is_err());
    }

    #[test]
    fn session_config_pins_the_arena600_orbit() {
        // `--camera-path arena600` pins seed 2040 plus the 600-tick orbit
        // even when the operator passes another seed; free play is untouched.
        let args = WinArgs::parse(&strings(&[
            "arena",
            "--seed",
            "9",
            "--camera-path",
            "arena600",
        ]))
        .expect("valid orbit fixture parses");
        let config = session_config(&args).expect("orbit session builds");
        assert_eq!(
            (config.seed, config.ticks),
            (
                engine_game::windowed::ARENA600_SEED,
                engine_game::windowed::ARENA600_FRAMES
            )
        );
        // An explicit `--ticks` still wins over the orbit floor; `--frames`
        // still caps the run.
        let args = WinArgs::parse(&strings(&[
            "arena",
            "--ticks",
            "30",
            "--frames",
            "30",
            "--camera-path",
            "arena600",
        ]))
        .expect("bounded orbit fixture parses");
        let config = session_config(&args).expect("bounded orbit session builds");
        assert_eq!((config.seed, config.ticks), (2040, 30));
        // No camera path: the operator seed flows through.
        let plain =
            WinArgs::parse(&strings(&["arena", "--seed", "9"])).expect("seeded fixture parses");
        let config = session_config(&plain).expect("seeded session builds");
        assert_eq!(config.seed, 9);
    }

    #[test]
    fn timed_smoke_line_reports_fps_and_refuses_bad_time() {
        let line = format_timed_smoke(60, 60, 1000.0).expect("valid smoke line formats");
        assert!(line.contains("frames=60"), "{line}");
        assert!(line.contains("fps=60.0"), "{line}");
        assert!(format_timed_smoke(60, 60, 0.0).is_err());
        assert!(format_timed_smoke(60, 60, f64::NAN).is_err());
    }

    #[test]
    fn camera_pause_decision_holds_at_the_tick_and_steps_once() {
        // `--pause-at-tick 120`: hold at exactly tick 120; one Space press
        // advances exactly one tick; without a pause tick a stray step is a
        // no-op and the sim runs.
        assert_eq!(pause_decision(119, Some(120), false), PauseDecision::Run);
        assert_eq!(pause_decision(120, Some(120), false), PauseDecision::Hold);
        assert_eq!(pause_decision(120, Some(120), true), PauseDecision::Step);
        // After one step the tick is 121 and holds again: exactly one tick.
        assert_eq!(pause_decision(121, Some(120), false), PauseDecision::Hold);
        assert_eq!(pause_decision(120, None, false), PauseDecision::Run);
        assert_eq!(pause_decision(120, None, true), PauseDecision::Run);
        // A tick below the pause tick still runs even with a step armed.
        assert_eq!(pause_decision(119, Some(120), true), PauseDecision::Run);
        // Pause at 0 holds before the first tick.
        assert_eq!(pause_decision(0, Some(0), false), PauseDecision::Hold);
        assert_eq!(pause_decision(0, Some(0), true), PauseDecision::Step);
        // The stop line a smoke greps is a stable single line, tick only.
        assert_eq!(format!("{PAUSE_LINE_PREFIX}{}", 120u32), "paused tick=120");
    }

    #[test]
    fn camera_pause_flags_split_off_before_winargs() {
        // `--pause-at-tick N` and `--step` are split off before the strict
        // WinArgs parser, which refuses them as unknown flags today.
        let raw = strings(&[
            "arena",
            "--frames",
            "200",
            "--pause-at-tick",
            "120",
            "--step",
            "--exit",
        ]);
        let (pause_at, step, rest) = arena_args::split_pause_flags(&raw).expect("flags split");
        assert_eq!(pause_at, Some(120));
        assert!(step, "--step is armed");
        assert_eq!(rest, strings(&["arena", "--frames", "200", "--exit"]));
        // The remainder parses; the raw flags stay unknown to the strict parser.
        assert!(WinArgs::parse(&rest).is_ok());
        assert!(WinArgs::parse(&raw).is_err());
        // A missing or unparsable tick fails loudly, never silently defaulted.
        assert!(arena_args::split_pause_flags(&strings(&["arena", "--pause-at-tick"])).is_err());
        assert!(
            arena_args::split_pause_flags(&strings(&["arena", "--pause-at-tick", "x"])).is_err()
        );
        assert!(
            arena_args::split_pause_flags(&strings(&["arena", "--pause-at-tick", "--step"]))
                .is_err()
        );
        let (none, no_step, same) = arena_args::split_pause_flags(&strings(&["arena", "--exit"]))
            .expect("no pause flags still splits");
        assert!(none.is_none() && !no_step);
        assert_eq!(same, strings(&["arena", "--exit"]));
    }

    #[test]
    fn timed_smoke_writes_frame_times_csv_with_header_and_rows() {
        // The timed-smoke exit files header plus one row per presented frame,
        // so the proof perf mode reads a real run. Bad times refuse loudly.
        let row = frame_times_csv_row(0, 16.667, 16.667).expect("valid row formats");
        assert!(row.starts_with("0,16.667,16.667"), "{row}");
        assert!(frame_times_csv_row(1, f64::NAN, 16.0).is_err());
        assert!(frame_times_csv_row(1, 16.0, -1.0).is_err());
        assert!(frame_times_csv_row(2, f64::INFINITY, 16.0).is_err());
        let dir = bin_temp_dir("frame-times");
        let path = dir.join("frame-times.csv");
        let work = vec![16_667u64, 16_667, 33_333];
        let intervals = vec![16_667u64, 16_667, 16_667];
        write_frame_times_csv(&path, 3, &work, &intervals, 50.0).expect("csv writes");
        let text = std::fs::read_to_string(&path).expect("csv reads");
        let mut lines = text.lines();
        assert_eq!(lines.next(), Some(FRAME_TIMES_CSV_HEADER));
        let rows: Vec<&str> = lines.collect();
        assert_eq!(rows.len(), 3, "one row per presented frame:\n{text}");
        assert!(rows[0].starts_with("0,"), "{}", rows[0]);
        assert!(rows[1].starts_with("1,"), "{}", rows[1]);
        assert!(rows[2].starts_with("2,"), "{}", rows[2]);
        assert!(text.contains("wall_ms"), "{text}");
        assert!(write_frame_times_csv(&path, 3, &work, &intervals, f64::NAN).is_err());
        assert!(write_frame_times_csv(&path, 3, &work, &intervals, -5.0).is_err());
        std::fs::remove_dir_all(&dir).expect("test temp dir cleans");
    }

    #[test]
    fn exit_smoke_session_opens_no_audio_device() {
        // Automated `--exit` runs mute by construction: the window session
        // opens no audio device while the mixer still renders offline, and
        // the run owns its wall-clock bound from the first redraw.
        let args = WinArgs::parse(&strings(&["arena", "--frames", "2", "--exit"]))
            .expect("valid smoke fixture parses");
        let app = ArenaApp::new(bin_config(), args, None).expect("test session builds");
        assert!(
            app.audio_notice.contains("muted"),
            "smoke must stay muted: {}",
            app.audio_notice
        );
        check_exit_deadline(0.0).expect("fresh smoke is in bound");
        assert!(
            check_exit_deadline(engine_game::windowed::EXIT_WALL_LIMIT_MS + 1.0).is_err(),
            "forced deadline must fail"
        );
        // Legacy single-arg smoke stays green beside the `--deadline-ms` path.
        engine_game::windowed::run_headless_smoke(bin_config(), 2, None)
            .expect("legacy smoke stays green");
    }

    #[test]
    fn automated_smoke_is_muted_with_no_device_and_its_own_deadline() {
        // `--mute` plus `--deadline-ms` flow into the session: no audio
        // device is opened and the `--exit` run carries its own wall clock.
        let args = WinArgs::parse(&strings(&[
            "arena",
            "--frames",
            "4",
            "--exit",
            "--mute",
            "--deadline-ms",
            "60000",
        ]))
        .expect("muted smoke fixture parses");
        assert!(args.mute);
        assert_eq!(args.deadline_ms, Some(60_000));
        assert_eq!(args.wall_limit_ms(), 60_000);
        let app = ArenaApp::new(bin_config(), args, None).expect("muted session builds");
        assert!(app._device.is_none(), "muted smoke opens no audio device");
        assert!(app.audio_notice.contains("muted"), "{}", app.audio_notice);
    }

    #[test]
    fn pad_moves_the_human_slot() {
        // EG27w: recorded stick events move slot 0 on the tape exactly as the
        // keyboard wish does; with no device (or an idle one) nothing changes.
        use engine_game::pad::RecordedPadEvent;
        use engine_input::BoundAction;
        let session = || {
            let args = WinArgs::parse(&strings(&["arena", "--frames", "20", "--exit"]))
                .expect("smoke args parse");
            let mut app = ArenaApp::new(bin_config(), args, None).expect("session builds");
            // Tests never read the host's real pad.
            app.pad = WindowPad::absent();
            app
        };
        let ticks = 20;
        let sim_hash = |app: &ArenaApp| app.app.checkpoints().last().expect("ticks ran").hash;

        let mut keys = session();
        let right = keys.bindings.button(BoundAction::MoveRight);
        keys.set_pressed(right, true);
        keys.run_sim_ticks(ticks, keys.human().expect("human slot"));

        let mut pad = session();
        pad.pad
            .apply(&[RecordedPadEvent::Axis { lx: 1.0, ly: 0.0 }]);
        assert!(!pad.pad.is_idle(), "a deflected stick is not idle");
        pad.run_sim_ticks(ticks, pad.human().expect("human slot"));

        let mut idle = session();
        idle.pad.poll();
        assert!(idle.pad.is_idle(), "no device reports nothing");
        idle.run_sim_ticks(ticks, idle.human().expect("human slot"));

        assert_eq!(sim_hash(&pad), sim_hash(&keys), "stick equals the key");
        assert_ne!(sim_hash(&pad), sim_hash(&idle), "the stick moved the slot");
        let at = |app: &ArenaApp| app.app.fighter_views()[usize::from(HUMAN_SLOT)].pos_m;
        let walked = (at(&pad)[0] - at(&idle)[0]).hypot(at(&pad)[2] - at(&idle)[2]);
        assert!(
            walked > 0.1,
            "slot 0 walked {walked} m from where idle stands"
        );
        assert_eq!(at(&pad), at(&keys));

        // A held pad button stays held between polls until it is released.
        let mut held = WindowPad::absent();
        held.apply(&[RecordedPadEvent::ButtonDown(2)]);
        held.apply(&[RecordedPadEvent::Axis { lx: 0.0, ly: 0.0 }]);
        assert!(!held.is_idle(), "the button is still down");
        held.apply(&[RecordedPadEvent::ButtonUp(2)]);
        assert!(held.is_idle(), "released");
    }

    /// The live arena session (seed 2040, two fighters, the kit world) with
    /// the host's pad out of the picture, room for a few seconds of play and
    /// tick 0 already run idle (the fighters' places exist after a tick).
    fn walking_session() -> ArenaApp {
        let args = WinArgs::parse(&strings(&["arena", "--frames", "400", "--exit"]))
            .expect("smoke args parse");
        let config = GameConfig {
            seed: 2040,
            ticks: 400,
            fighters: 2,
            half_extent_m: engine_game::windowed::ARENA_HALF_EXTENT_M,
        };
        let mut app = ArenaApp::new(config, args, None).expect("session builds");
        app.pad = WindowPad::absent();
        app.run_sim_ticks(1, app.human().expect("human slot"));
        app
    }

    fn standing(app: &ArenaApp, slot: u8) -> [f32; 3] {
        app.app.fighter_views()[usize::from(slot)].pos_m
    }

    fn key(code: engine_input::map::Key) -> PhysicalButton {
        PhysicalButton::Key(code.0)
    }

    #[test]
    fn w_walks_where_the_camera_looks_on_the_real_input_path() {
        // Keys held on the window's own road (pressed -> bindings -> camera
        // heading -> App::tick). The old mapping ran W along world +x whatever
        // the picture showed.
        use engine_input::map::Key;
        let walk = |eye: [f32; 3], target: [f32; 3], held: Key| {
            let mut app = walking_session();
            app.note_view_forward(eye, target);
            let start = standing(&app, HUMAN_SLOT);
            app.set_pressed(key(held), true);
            app.run_sim_ticks(60, app.human().expect("human slot"));
            let end = standing(&app, HUMAN_SLOT);
            [end[0] - start[0], end[2] - start[2]]
        };
        // The body turns from its spawn facing before it is up to speed, so a
        // walk arcs a little: the move is judged by its main axis, not a line.
        let mostly = |moved: [f32; 2], axis: usize, sign: f32, least: f32| {
            let (main, other) = (sign * moved[axis], moved[1 - axis].abs());
            main > least && main > 2.0 * other
        };
        // Camera looking down +z: W goes +z, S -z; D (to its right) -x, A +x.
        let (eye, ahead) = ([0.0, 1.8, 0.0], [0.0, 1.0, 10.0]);
        let w = walk(eye, ahead, Key::W);
        assert!(mostly(w, 1, 1.0, 1.5), "W looking +z moved {w:?}");
        let s = walk(eye, ahead, Key::S);
        assert!(mostly(s, 1, -1.0, 1.5), "S looking +z moved {s:?}");
        let d = walk(eye, ahead, Key::D);
        assert!(mostly(d, 0, -1.0, 1.5), "D looking +z moved {d:?}");
        // Camera looking down -z, whose left hand is -x (away from the +x wall
        // the hero starts 1.2 m from): A goes -x, D would go into the wall.
        let a = walk([0.0, 1.8, 0.0], [0.0, 1.0, -10.0], Key::A);
        assert!(mostly(a, 0, -1.0, 1.5), "A looking -z moved {a:?}");
        // Camera looking down -x: W goes -x, off the wall.
        let w = walk([0.0, 1.8, 0.0], [-10.0, 1.0, 0.0], Key::W);
        assert!(mostly(w, 0, -1.0, 1.5), "W looking -x moved {w:?}");
    }

    #[test]
    fn at_the_live_spawn_w_closes_on_the_rival_instead_of_running_into_the_wall() {
        // The owner's report: the hero is pinned to the +x wall, the rival 38 m
        // away across the pit, and W (world +x) walked into the wall. With the
        // camera looking at the rival W closes the distance.
        use engine_input::map::Key;
        let mut app = walking_session();
        let (hero, rival) = (standing(&app, HUMAN_SLOT), standing(&app, 1));
        let gap = |app: &ArenaApp| {
            let (h, r) = (standing(app, HUMAN_SLOT), standing(app, 1));
            (h[0] - r[0]).hypot(h[2] - r[2])
        };
        let before = gap(&app);
        assert!(before > 30.0, "the spawn is across the pit: {before} m");
        // Eye behind the hero on the far side from the rival, looking at them.
        let toward = [rival[0] - hero[0], rival[2] - hero[2]];
        let span = toward[0].hypot(toward[1]);
        let eye = [
            hero[0] - toward[0] / span * 4.0,
            1.9,
            hero[2] - toward[1] / span * 4.0,
        ];
        app.note_view_forward(eye, [rival[0], 1.0, rival[2]]);
        app.set_pressed(key(Key::W), true);
        app.run_sim_ticks(120, app.human().expect("human slot"));
        let after = gap(&app);
        assert!(
            before - after > 6.0,
            "W for 2 s should close in: {before} m -> {after} m"
        );
        assert!(
            standing(&app, HUMAN_SLOT)[0] < hero[0] - 3.0,
            "W left the +x wall behind"
        );
    }

    #[test]
    fn the_stick_walks_camera_relative_exactly_like_the_key() {
        use engine_game::pad::RecordedPadEvent;
        use engine_input::map::Key;
        let (eye, ahead) = ([0.0, 1.8, 0.0], [0.0, 1.0, 10.0]);
        let mut keys = walking_session();
        keys.note_view_forward(eye, ahead);
        let start = standing(&keys, HUMAN_SLOT);
        keys.set_pressed(key(Key::W), true);
        keys.run_sim_ticks(40, keys.human().expect("human slot"));
        let mut stick = walking_session();
        stick.note_view_forward(eye, ahead);
        stick
            .pad
            .apply(&[RecordedPadEvent::Axis { lx: 0.0, ly: 1.0 }]);
        assert!(!stick.pad.is_idle(), "a deflected stick is not idle");
        stick.run_sim_ticks(40, stick.human().expect("human slot"));
        let (k, s) = (standing(&keys, HUMAN_SLOT), standing(&stick, HUMAN_SLOT));
        assert_eq!(k, s, "stick forward is the W key");
        assert!(k[2] > start[2] + 1.0, "both went +z: {start:?} -> {k:?}");
    }

    /// What one bout of [`walk_up_and_hold_q`] saw.
    struct HeldQ {
        /// The velocity the spear left with, mm per tick.
        velocity: Option<[i32; 3]>,
        /// Whom the spear stuck in, first hit first (`None` is a wall).
        stuck: Vec<Option<u8>>,
        /// Whom the thrower's tether held when the spear stuck (`None`: nobody or a wall).
        tethered: Option<u8>,
        /// Where the rival stood from the thrower at the throw, `[dx, dz]` meters.
        offset: [f32; 2],
    }

    /// One bout on the window's own input road (pressed keys -> bindings -> camera heading ->
    /// lock-on -> tick), spear on: hold W with the camera on the rival until they stand inside
    /// spear reach, let go of W, hold Q. `free_cam` flies the free camera, which keeps the old
    /// facing of 0.
    fn walk_up_and_hold_q(free_cam: bool) -> HeldQ {
        use engine_arena::spear::{Phase, SpearEvent};
        use engine_input::map::Key;
        let args = WinArgs::parse(&strings(&["arena", "--frames", "900", "--exit"]))
            .expect("smoke args parse");
        let config = GameConfig {
            seed: 2040,
            ticks: 900,
            fighters: 2,
            half_extent_m: engine_game::windowed::ARENA_HALF_EXTENT_M,
        };
        let mut app = ArenaApp::new(config, args, None).expect("session builds");
        app.pad = WindowPad::absent();
        app.enable_spear(true, false, false)
            .expect("the grapple turns on");
        if free_cam {
            app.cam_mode = CameraMode::Free;
        }
        let human = app.human().expect("human slot");
        app.run_sim_ticks(1, human);
        let offset = |app: &ArenaApp| {
            let (hero, rival) = (standing(app, HUMAN_SLOT), standing(app, 1));
            [rival[0] - hero[0], rival[2] - hero[2]]
        };
        // Walk up with the camera behind the hero, looking at the rival (the shoulder shot
        // re-aims every frame; the key press stays the same).
        app.set_pressed(key(Key::W), true);
        while app.ticks_done < 600 {
            let (hero, rival) = (standing(&app, HUMAN_SLOT), standing(&app, 1));
            app.note_view_forward(hero, [rival[0], 1.0, rival[2]]);
            let [dx, dz] = offset(&app);
            if dx.hypot(dz) < 8.0 {
                break;
            }
            app.run_sim_ticks(1, human);
        }
        app.set_pressed(key(Key::W), false);
        let at_throw = offset(&app);
        assert!(
            at_throw[0].hypot(at_throw[1]) < 8.0,
            "the walk reached spear range: {at_throw:?}"
        );
        app.set_pressed(key(Key::Q), true);
        let mut seen = HeldQ {
            velocity: None,
            stuck: Vec::new(),
            tethered: None,
            offset: at_throw,
        };
        for _ in 0..60 {
            app.run_sim_ticks(1, human);
            let live = app.app.live_match();
            for event in live.spear_events() {
                match *event {
                    SpearEvent::Thrown {
                        thrower: 0, vel_mm, ..
                    } => seen.velocity = seen.velocity.or(Some(vel_mm)),
                    SpearEvent::Hit {
                        thrower: 0, victim, ..
                    } => seen.stuck.push(victim),
                    _ => {}
                }
            }
            if let Some(Phase::Tether(tether)) = live.spear_view().first().map(|slot| slot.phase) {
                seen.tethered = tether.victim;
            }
            if !seen.stuck.is_empty() {
                break;
            }
        }
        seen
    }

    #[test]
    fn a_held_q_throws_at_the_rival_and_the_tether_forms_with_them() {
        // The owner's report: the window pinned the input's facing to 0, so Q threw along world
        // +z whatever the picture showed. Now the throw leaves along the bearing to the rival.
        let aimed = walk_up_and_hold_q(false);
        let velocity = aimed.velocity.expect("holding Q threw the spear");
        let range = aimed.offset[0].hypot(aimed.offset[1]);
        assert!(
            aimed.offset[0].abs() > 0.5 * range,
            "the rival stands off the +z axis, so the aim shows: {:?}",
            aimed.offset
        );
        // The flight direction (x, z) lines up with the rival's bearing.
        let (vx, vz) = (f64::from(velocity[0]), f64::from(velocity[2]));
        let (ox, oz) = (f64::from(aimed.offset[0]), f64::from(aimed.offset[1]));
        let along = (vx * ox + vz * oz) / (vx.hypot(vz).max(1.0) * ox.hypot(oz));
        assert!(along > 0.99, "the spear left toward the rival: {along}");
        assert_eq!(
            aimed.stuck.first(),
            Some(&Some(1)),
            "the spear stuck in the rival"
        );
        assert_eq!(
            aimed.tethered,
            Some(1),
            "and the tether formed with the rival"
        );

        // The same bout with the free camera out keeps facing 0: the spear flies along +z.
        let old = walk_up_and_hold_q(true);
        let velocity = old.velocity.expect("the old road threw too");
        assert_eq!(velocity[0], 0, "facing 0 is world +z: {velocity:?}");
        assert!(velocity[2] > 0, "{velocity:?}");
        assert_ne!(old.stuck.first(), Some(&Some(1)), "and it misses the rival");
        assert_eq!(old.tethered, None, "so no tether forms with them");
    }
    #[test]
    fn the_lock_on_sets_only_the_facing_and_the_free_camera_leaves_it_alone() {
        let mut app = walking_session();
        let human = app.human().expect("human slot");
        let (hero, rival) = (standing(&app, HUMAN_SLOT), standing(&app, 1));
        let input = FighterInput {
            tick: 5,
            fighter: human,
            mx: 40,
            mz: -60,
            facing: 321,
            buttons: engine_input::Buttons(
                engine_input::fighter::ATTACK | engine_input::fighter::THROW_SPEAR,
            ),
        };
        let want = engine_game::lock_on::bearing_cdeg(rival[0] - hero[0], rival[2] - hero[2])
            .expect("the rival stands elsewhere");
        let out = app.aim_at_rival(input);
        assert_eq!(out.facing, want, "facing is the bearing to the rival");
        assert_eq!(
            (out.tick, out.fighter, out.mx, out.mz, out.buttons),
            (input.tick, input.fighter, input.mx, input.mz, input.buttons),
            "everything else passes through"
        );
        // The free camera is view only: the input keeps the facing it came with.
        app.cam_mode = CameraMode::Free;
        assert_eq!(app.aim_at_rival(input), input);
        // Back on the director shot the lock-on is on again.
        app.cam_mode = CameraMode::Director;
        assert_eq!(app.aim_at_rival(input).facing, want);
    }

    #[test]
    fn camera_relative_turns_only_the_wish() {
        // Attack, block, dodge and spear bits, facing, tick and slot all pass
        // through; the sim keeps deciding facing and the swing.
        let mut app = walking_session();
        let human = app.human().expect("human slot");
        let input = FighterInput {
            tick: 5,
            fighter: human,
            mx: 100,
            mz: 0,
            facing: 321,
            buttons: engine_input::Buttons(
                engine_input::fighter::ATTACK | engine_input::fighter::BLOCK,
            ),
        };
        // No camera yet: the input passes untouched (the keys keep world axes).
        assert_eq!(app.camera_relative(input), input);
        app.note_view_forward([0.0, 1.8, 0.0], [0.0, 1.0, 10.0]);
        let out = app.camera_relative(input);
        assert_eq!((out.mx, out.mz), (0, 100), "W looking +z is world +z");
        assert_eq!(
            (out.tick, out.fighter, out.facing, out.buttons),
            (input.tick, input.fighter, input.facing, input.buttons)
        );
        // A camera with no heading (straight down) keeps the last good one.
        app.note_view_forward([1.0, 9.0, 1.0], [1.0, 0.0, 1.0]);
        assert_eq!(app.view_forward, Some([0.0, 1.0]));
    }

    #[test]
    fn scripts_and_the_hand_off_tape_keep_their_world_axes_under_a_camera() {
        // The staged fight is a pinned receipt in world axes; a camera heading
        // must never turn it (only the live keyboard and pad road does).
        use engine_game::windowed::{STAGED_FIGHT_HITS, STAGED_FIGHT_KNOCKDOWNS};
        let args = WinArgs::parse(&strings(&["arena", "--frames", "600", "--exit"]))
            .expect("smoke args parse");
        let mut app = ArenaApp::new(staged_session_config(), args, None).expect("session builds");
        app.pad = WindowPad::absent();
        app.staged = Some(StagedTally::default());
        assert!(app.stage_receipt_session().is_none(), "the swap lands");
        app.note_view_forward([0.0, 1.8, 0.0], [-10.0, 1.0, 3.0]);
        app.run_sim_ticks(app.config.ticks, app.human().expect("human slot"));
        let tally = app.staged.expect("staged tally");
        assert_eq!(tally.hits, STAGED_FIGHT_HITS);
        assert_eq!(tally.knockdowns, STAGED_FIGHT_KNOCKDOWNS);
    }

    #[test]
    fn hold_flag_splits_off_and_names_its_keys() {
        use engine_input::map::Key;
        let (held, rest) =
            arena_args::split_hold(&strings(&["arena", "--hold", "w,Shift", "--exit"]))
                .expect("valid keys");
        assert_eq!(held, vec![key(Key::W), key(Key::SHIFT)]);
        assert_eq!(rest, strings(&["arena", "--exit"]));
        let (none, same) = arena_args::split_hold(&strings(&["arena", "--exit"])).expect("no flag");
        assert!(none.is_empty() && same == strings(&["arena", "--exit"]));
        // A missing list or an unknown key fails loudly, never silently.
        assert!(arena_args::split_hold(&strings(&["arena", "--hold"])).is_err());
        assert!(arena_args::split_hold(&strings(&["arena", "--hold", "--exit"])).is_err());
        assert!(arena_args::split_hold(&strings(&["arena", "--hold", "W,Z"])).is_err());
        // The strict parser still refuses the flag if it were not split.
        assert!(WinArgs::parse(&strings(&["arena", "--hold", "W"])).is_err());
    }

    #[test]
    fn the_title_tells_a_far_player_to_walk_and_goes_quiet_up_close() {
        let view = |slot: u8, x: f32, z: f32| engine_game::FighterView {
            slot,
            pos_m: [x, 0.0, z],
            yaw: 0.0,
        };
        // The live spawn: 38 m across the pit.
        let spawn = [view(0, 19.185, 3.438), view(1, -16.6, -11.156)];
        let hint = arena_frame::rival_hint(&spawn, HUMAN_SLOT).expect("far rival gets a hint");
        assert!(hint.contains("walk toward the rival with W"), "{hint}");
        assert!(hint.contains("39 m"), "the distance rides along: {hint}");
        // The nearest of several counts: a bot at 4 m silences the hint.
        let crowd = [view(0, 0.0, 0.0), view(1, 30.0, 0.0), view(2, 3.0, 3.0)];
        assert_eq!(arena_frame::rival_hint(&crowd, HUMAN_SLOT), None);
        // Nobody else, no human, or a broken position: no hint, no panic.
        assert_eq!(arena_frame::rival_hint(&spawn[..1], HUMAN_SLOT), None);
        assert_eq!(arena_frame::rival_hint(&spawn[1..], HUMAN_SLOT), None);
        let broken = [view(0, 0.0, 0.0), view(1, f32::NAN, 0.0)];
        assert_eq!(arena_frame::rival_hint(&broken, HUMAN_SLOT), None);
        // The title carries the hint first, then the controls.
        let plain = arena_frame::window_title(None);
        assert!(plain.starts_with("engine2040 arena | WASD move (camera-relative)"));
        let hinted = arena_frame::window_title(Some(&hint));
        assert!(
            hinted.starts_with(&format!("engine2040 arena | {hint} | WASD")),
            "{hinted}"
        );
    }

    #[test]
    fn shell_boots_to_title_and_rematch_clears_the_bout() {
        // Behavior: interactive play boots Title (zero ticks owed), the Start
        // event opens the bout, and rematch clears the prior bout state.
        // Automated `--exit` smoke boots Match instead (see the next test).
        let args = WinArgs::parse(&strings(&["arena", "--frames", "2"]))
            .expect("valid interactive fixture parses");
        let mut app = ArenaApp::new(bin_config(), args, None).expect("test session builds");
        assert_eq!(app.flow, Flow::Title);
        assert!(!app.flow.ticks_run(), "title holds the accumulator");
        assert_eq!(app.ticks_done, 0);
        assert!(!app.result_announced);
        // The Enter key path: Title plus Start runs the bout.
        app.flow = app.flow.on(FlowEvent::Start);
        assert_eq!(app.flow, Flow::Match);
        assert!(app.flow.ticks_run());
        // Failure guard: a dirty bout (ticks banked) resets to zero with a
        // fresh match cursor, ready for a clean rematch.
        app.ticks_done = 7;
        app.result_announced = true;
        app.reset_bout().expect("rematch clears the bout");
        assert_eq!(app.ticks_done, 0);
        assert_eq!(app.app.cursor(), 0);
        assert!(!app.result_announced);
        assert!(app.app.checkpoints().is_empty());
    }

    #[test]
    fn automated_exit_smoke_starts_in_match() {
        // Regression: `--exit` smoke must stage the bout at once (Match,
        // ticks run) so `arena.exe --scene arena --fighters 8 --frames 30
        // --exit` shows fighters; Title is for interactive play only. Fails
        // while the smoke boots Title and holds ticks at 0 (empty sand).
        let args = WinArgs::parse(&strings(&["arena", "--frames", "2", "--exit"]))
            .expect("valid smoke fixture parses");
        let app = ArenaApp::new(bin_config(), args, None).expect("test session builds");
        assert_eq!(
            app.flow,
            Flow::Match,
            "exit smoke must boot Match, not Title"
        );
        assert!(app.flow.ticks_run(), "exit smoke must tick without Enter");
        // Pure branches: timed smoke also runs at once, interactive waits.
        let timed = WinArgs::parse(&strings(&["arena", "--frames", "2", "--timed-smoke"]))
            .expect("timed fixture parses");
        assert_eq!(initial_flow(&timed), Flow::Match);
        let live = WinArgs::parse(&strings(&["arena", "--frames", "2"]))
            .expect("interactive fixture parses");
        assert_eq!(initial_flow(&live), Flow::Title);
    }

    #[test]
    fn capture_arms_look_probes_for_per_pass_sky() {
        // Capture needs measured sky ms: an armed capture must arm the
        // blocking look probes so the per-pass sidecar reports sky above zero.
        // Without probes every receipt reports sky zero and the sidecar lies.
        let args = WinArgs::parse(&strings(&["arena", "--frames", "2", "--exit"]))
            .expect("valid smoke fixture parses");
        let root = bin_temp_dir("probe-cap");
        let flags = cap::CaptureFlags {
            dir: root.join("clip"),
            every: 1,
            tape: None,
            rows: vec!["SHOW1".to_string()],
            sha: "deadbeef1234".to_string(),
            fps: 30,
        };
        let slot = super::arena_capture::capture::setup_capture(&Some(flags), &bin_config())
            .expect("capture arms")
            .expect("slot armed");
        let app = ArenaApp::new(bin_config(), args, Some(slot)).expect("test session builds");
        assert!(
            app.look_probes,
            "an armed capture must arm look probes so sky_ms measures above zero"
        );
        assert!(app.capture.is_some(), "capture stays armed for the sidecar");
        std::fs::remove_dir_all(&root).expect("test temp dir cleans");
    }

    #[test]
    fn window_ticks_obey_the_bound_tape_on_the_arena_scene_session() {
        // The window's own session (kit world, duel rules, eight bodies) takes
        // the bound tape on slot 0: an attack tape moves the sim hash off the
        // idle run, and the tape's receipt counts every input as applied.
        let config = GameConfig {
            seed: 2040,
            ticks: 60,
            fighters: 8,
            half_extent_m: 20.0,
        };
        let args = || {
            WinArgs::parse(&strings(&["arena", "--frames", "60", "--exit"]))
                .expect("smoke args parse")
        };
        let mut idle = ArenaApp::new(config, args(), None).expect("idle session builds");
        idle.run_sim_ticks(60, idle.human().expect("human slot"));
        let root = bin_temp_dir("window-tape");
        let inputs: Vec<FighterInput> = (0..60u32)
            .map(|tick| FighterInput {
                tick,
                fighter: FighterId::new(HUMAN_SLOT).expect("slot 0 is a fighter"),
                mx: 0,
                mz: 0,
                facing: 0,
                buttons: engine_input::Buttons(engine_input::fighter::ATTACK),
            })
            .collect();
        let tape_path = root.join("atk.tape");
        std::fs::write(&tape_path, cap::encode_tape_bytes(&inputs)).expect("test tape writes");
        let flags = cap::CaptureFlags {
            dir: root.join("clip"),
            every: 1000,
            tape: Some(tape_path),
            rows: vec!["SHOW1".to_string()],
            sha: "deadbeef1234".to_string(),
            fps: 30,
        };
        let slot = setup_capture(&Some(flags), &config)
            .expect("capture arms")
            .expect("slot armed");
        let mut taped = ArenaApp::new(config, args(), Some(slot)).expect("taped session builds");
        taped.run_sim_ticks(60, taped.human().expect("human slot"));
        let last_hash = |app: &ArenaApp| app.app.checkpoints().last().expect("ticks ran").hash;
        assert_ne!(
            last_hash(&taped),
            last_hash(&idle),
            "the attack tape reached the sim"
        );
        let receipt = taped
            .capture
            .as_ref()
            .and_then(|slot| slot.tape.receipt())
            .expect("tape bound");
        assert_eq!(
            (receipt.inputs, receipt.applied, receipt.missed),
            (60, 60, 0)
        );
        assert_eq!(receipt.failure(), None);
        std::fs::remove_dir_all(&root).expect("test temp dir cleans");
    }

    #[test]
    fn deadline_override_outlasts_the_five_minute_default_in_the_window_loop() {
        // An 18000-frame clean run is slower than the 300 s default, so
        // the window loop's first bound must read `--deadline-ms` too, not the
        // fixed default. Past 300 s the default run fails, the override passes.
        let plain = WinArgs::parse(&strings(&["arena", "--frames", "18000", "--exit"]))
            .expect("plain smoke parses");
        let long = WinArgs::parse(&strings(&[
            "arena",
            "--frames",
            "18000",
            "--exit",
            "--deadline-ms",
            "9000000",
        ]))
        .expect("long smoke parses");
        let wall_ms = 301_000.0;
        assert!(check_exit_deadline_with_limit(wall_ms, plain.wall_limit_ms()).is_err());
        assert!(check_exit_deadline_with_limit(wall_ms, long.wall_limit_ms()).is_ok());
        assert!(check_exit_deadline_with_limit(9_000_000.0, long.wall_limit_ms()).is_err());
    }

    #[test]
    fn forced_stall_drill_with_sixty_second_deadline_exits_as_timeout() {
        // Forced-stall drill: `--deadline-ms 60000` bounds the `--exit` run at 60 s
        // while the default stays 300 s; a frame stalled past the bound
        // leaves the exit path as `Timeout` naming elapsed plus limit.
        let drill = WinArgs::parse(&strings(&[
            "arena",
            "--frames",
            "30",
            "--exit",
            "--deadline-ms",
            "60000",
        ]))
        .expect("drill fixture parses");
        assert_eq!(drill.wall_limit_ms(), 60_000);
        let plain = WinArgs::parse(&strings(&["arena", "--frames", "30", "--exit"]))
            .expect("plain smoke parses");
        assert_eq!(plain.wall_limit_ms(), 300_000);
        // Just inside the drill bound the exit path stays green.
        check_exit_wall_deadline(59_999.0, drill.wall_limit_ms())
            .expect("drill in bound stays green");
        // Forced stall: elapsed past 60 s fails as `Timeout` naming both.
        let err = check_exit_wall_deadline(60_001.0, drill.wall_limit_ms())
            .expect_err("forced stall must fail the drill bound");
        assert!(
            matches!(&err, WinError::Timeout(msg) if msg.contains("60001") && msg.contains("60000")),
            "drill stall must be Timeout naming elapsed+limit, got: {err}"
        );
        // The untouched default still spans 300 s: inside passes, past fails.
        check_exit_wall_deadline(299_999.0, plain.wall_limit_ms())
            .expect("default bound keeps the normal run green");
        assert!(
            check_exit_wall_deadline(300_001.0, plain.wall_limit_ms()).is_err(),
            "default bound still fires past 300 s"
        );
    }

    /// One folded receipt carrying the fields the per-pass line reads, at the
    /// frame size named. Shared by the per-pass line tests.
    fn folded_receipt(
        width: u32,
        height: u32,
        sky_us: u64,
        gtao_us: u64,
        shadow_us: u64,
        validation: u32,
    ) -> engine_render::present::FrameReceipt {
        engine_render::present::FrameReceipt {
            width,
            height,
            draws: 1,
            validation_errors: 0,
            generation: 1,
            presented: true,
            adapter: "NVIDIA GeForce RTX 3050 6GB Laptop GPU".to_string(),
            driver: "610.62".to_string(),
            mesh_count: 1,
            texture_count: 0,
            mesh_high_water: 1,
            texture_high_water: 0,
            mesh_evicted: 0,
            texture_evicted: 0,
            evicted_mesh_id: None,
            evicted_texture_id: None,
            look_passes: 4,
            look_validation_errors: validation,
            ibl_passes: 1,
            ibl_validation_errors: 0,
            fog_passes: 2,
            fog_validation_errors: 0,
            fog_submit_us: 200,
            gtao_passes: 2,
            gtao_validation_errors: 0,
            gtao_submit_us: gtao_us,
            sky_submit_us: sky_us,
            layer_submit_us: 100,
            particle_submit_us: 100,
            decal_submit_us: 100,
            ibl_submit_us: 300,
            shadow_submit_us: shadow_us,
        }
    }

    #[test]
    fn per_pass_line_names_the_newest_folded_frame_size() {
        // QL12: the capture runner forces the window to its capture size a few
        // frames into a run, and the means are taken over the frames that
        // followed, so the line names the newest folded size and not the boot
        // size (which pinned every 1080p run under a 720p label).
        let args = WinArgs::parse(&strings(&["arena", "--frames", "2", "--exit"]))
            .expect("valid smoke fixture parses");
        let mut app = ArenaApp::new(bin_config(), args, None).expect("test session builds");
        app.note_receipt(&folded_receipt(1280, 720, 1000, 800, 1500, 0));
        app.note_receipt(&folded_receipt(1937, 1080, 3000, 1200, 2500, 0));
        let line = app.per_pass_line().expect("two receipts post a line");
        assert!(line.contains("resolution=1937x1080"), "{line}");
    }

    #[test]
    fn per_pass_line_posts_means_with_adapter_and_validation() {
        // No frames yet: no adapter pinned, so no line (never a fake table).
        let args = WinArgs::parse(&strings(&["arena", "--frames", "2", "--exit"]))
            .expect("valid smoke fixture parses");
        let mut app = ArenaApp::new(bin_config(), args, None).expect("test session builds");
        assert!(app.per_pass_line().is_none());
        // Two folded receipts average to the posted means; validation sums.
        let receipt = |sky_us: u64, gtao_us: u64, shadow_us: u64, validation: u32| {
            folded_receipt(1280, 720, sky_us, gtao_us, shadow_us, validation)
        };
        app.note_receipt(&receipt(1000, 800, 1500, 0));
        app.note_receipt(&receipt(3000, 1200, 2500, 1));
        let line = app.per_pass_line().expect("two receipts post a line");
        assert!(line.contains("adapter=\"NVIDIA GeForce RTX 3050"), "{line}");
        assert!(line.contains("driver=\"610.62\""), "{line}");
        assert!(line.contains("resolution=1280x720"), "{line}");
        assert!(line.contains("frames=2"), "{line}");
        // Means: sky (1+3)/2=2.0, gtao (0.8+1.2)/2=1.0, ibl 0.3, fog 0.2,
        // shadow (1.5+2.5)/2=2.0 over every folded frame.
        assert!(line.contains("sky=2.000ms"), "{line}");
        assert!(line.contains("gtao=1.000ms"), "{line}");
        assert!(line.contains("ibl=0.300ms"), "{line}");
        assert!(line.contains("fog=0.200ms"), "{line}");
        assert!(line.contains("shadow=2.000ms"), "{line}");
        assert!(line.contains("validation_errors=1"), "{line}");
        // The render-side single-frame summary carries the same table.
        let single = receipt(2000, 800, 1500, 0);
        assert_eq!(single.total_validation_errors(), 0);
        let summary = single.per_pass_summary();
        assert!(
            summary.contains("adapter=NVIDIA GeForce RTX 3050"),
            "{summary}"
        );
        assert!(summary.contains("sky=2.000ms"), "{summary}");
        assert!(summary.contains("ibl=0.300ms"), "{summary}");
        assert!(summary.contains("shadow=1.500ms"), "{summary}");
        assert!(summary.contains("validation_errors=0"), "{summary}");
    }

    #[test]
    fn per_pass_line_names_forward_so_live_tells_from_probe() {
        // LIVE vs PROBE: the live forward pass runs every presented frame
        // while the probe columns stay 0 without `--look-probes`; the
        // per-pass line must name forward so the EG8 gate tells them apart.
        // Without a presenter there is no device timing yet, so the honest
        // fallback reads 0.000, never a missing column.
        let args = WinArgs::parse(&strings(&["arena", "--frames", "2", "--exit"]))
            .expect("valid smoke fixture parses");
        let mut app = ArenaApp::new(bin_config(), args, None).expect("test session builds");
        let receipt = engine_render::present::FrameReceipt {
            width: 1280,
            height: 720,
            draws: 1,
            validation_errors: 0,
            generation: 1,
            presented: true,
            adapter: "NVIDIA GeForce RTX 3050 6GB Laptop GPU".to_string(),
            driver: "610.62".to_string(),
            mesh_count: 1,
            texture_count: 0,
            mesh_high_water: 1,
            texture_high_water: 0,
            mesh_evicted: 0,
            texture_evicted: 0,
            evicted_mesh_id: None,
            evicted_texture_id: None,
            look_passes: 0,
            look_validation_errors: 0,
            ibl_passes: 0,
            ibl_validation_errors: 0,
            fog_passes: 0,
            fog_validation_errors: 0,
            fog_submit_us: 0,
            gtao_passes: 0,
            gtao_validation_errors: 0,
            gtao_submit_us: 0,
            sky_submit_us: 0,
            layer_submit_us: 0,
            particle_submit_us: 0,
            decal_submit_us: 0,
            ibl_submit_us: 0,
            shadow_submit_us: 17,
        };
        app.note_receipt(&receipt);
        let line = app.per_pass_line().expect("one live receipt posts a line");
        assert!(line.contains("forward="), "{line}");
        assert!(line.contains("forward=0.000ms"), "{line}");
        assert!(line.contains("validation_errors=0"), "{line}");
    }

    #[test]
    fn forward_format_reports_live_above_zero() {
        // Live device timing must surface above zero so the EG8 gate tells
        // LIVE (forward above 0) from PROBE (probe columns above 0); the
        // fallback stays honest 0.000 without samples, never a fake.
        assert_eq!(ArenaApp::format_forward_ms(0.213), "0.213");
        assert_eq!(ArenaApp::format_forward_ms(0.0), "0.000");
        let live = ArenaApp::format_forward_ms(0.213);
        let value: f32 = live.parse().expect("forward formats as a number");
        assert!(value > 0.0, "live forward above zero, got {live}");
    }

    #[test]
    fn capture_scene_defaults_to_arena_and_names_it() {
        // No `--scene` keeps the baked-arena default; explicit arena names it.
        assert_eq!(resolve_capture_scene(None), "arena");
        assert_eq!(
            resolve_capture_scene(Some(SceneSelect::Arena)),
            engine_game::windowed::ARENA_SCENE_ID
        );
        // Wrong scenes never reach the resolver: parse refuses them loudly.
        assert!(SceneSelect::parse("dungeon").is_err());
        assert!(SceneSelect::parse("ARENA").is_err());
    }

    #[test]
    fn headless_capture_line_reports_scene_and_measured_fps() {
        let manifest = std::path::Path::new("clip/manifest.json");
        let line = format_headless_capture_line(120, 120, "arena", 1000.0, manifest, "488ec35")
            .expect("valid capture line formats");
        assert!(line.contains("ticks=120"), "{line}");
        assert!(line.contains("frames=120"), "{line}");
        assert!(line.contains("scene=\"arena\""), "{line}");
        assert!(line.contains("fps=120.0"), "{line}");
        assert!(line.contains("manifest=\"clip/manifest.json\""), "{line}");
        assert!(line.contains("sha=488ec35"), "{line}");
        // Zero frames, zero time and non-finite time fail loudly: no fake fps.
        assert!(
            format_headless_capture_line(120, 0, "arena", 1000.0, manifest, "488ec35").is_err()
        );
        assert!(format_headless_capture_line(120, 120, "arena", 0.0, manifest, "488ec35").is_err());
        assert!(
            format_headless_capture_line(120, 120, "arena", f64::NAN, manifest, "488ec35").is_err()
        );
        assert!(format_headless_capture_line(
            120,
            120,
            "arena",
            f64::INFINITY,
            manifest,
            "488ec35"
        )
        .is_err());
    }

    #[test]
    fn capture_120frame_arena_scene_writes_pngs_and_receipt() {
        // Follow-up shape: the 8-fighter arena scene, 120 ticks at every
        // tick, through the same headless fallback the binary uses with no
        // display. The receipt line carries scene plus measured fps.
        let root = bin_temp_dir("arena120");
        let dir = root.join("clip");
        let flags = cap::CaptureFlags {
            dir: dir.clone(),
            every: 1,
            tape: None,
            rows: vec!["RL5".to_string()],
            sha: "488ec35".to_string(),
            fps: 30,
        };
        let args = WinArgs::parse(&strings(&[
            "arena",
            "--frames",
            "120",
            "--exit",
            "--fighters",
            "8",
            "--scene",
            "arena",
        ]))
        .expect("valid 120-frame arena fixture parses");
        let config = session_config(&args).expect("120-frame session builds");
        assert_eq!(config.fighters, 8);
        assert_eq!(config.ticks, 120);
        let display = WinError::NoDisplay("test headless".to_string());
        assert_eq!(headless_fallback(config, &args, &display, &Some(flags)), 0);
        let mut pngs = 0u32;
        for index in 0..120u32 {
            let png = std::fs::read(dir.join(format!("frame_{index:06}.png")))
                .expect("capture writes headed PNGs");
            assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
            pngs += 1;
        }
        assert_eq!(pngs, 120);
        let manifest =
            std::fs::read_to_string(dir.join("manifest.json")).expect("capture writes manifest");
        assert!(manifest.contains("\"seed\":2040"), "manifest:\n{manifest}");
        assert!(
            manifest.contains("\"rows\":[\"RL5\"]"),
            "manifest:\n{manifest}"
        );
        assert!(manifest.contains("\"fps\":30"), "manifest:\n{manifest}");
        assert_eq!(manifest.matches("\"tick\":").count(), 120);
        // The receipt line for this run shape names the scene and the rate.
        let line = format_headless_capture_line(
            120,
            120,
            resolve_capture_scene(args.scene),
            1000.0,
            &dir.join("manifest.json"),
            "488ec35",
        )
        .expect("valid 120-frame receipt formats");
        assert!(line.contains("scene=\"arena\""), "{line}");
        assert!(line.contains("fps=120.0"), "{line}");
        std::fs::remove_dir_all(&root).expect("test temp dir cleans");
    }

    #[test]
    fn chart_flags_split_off_before_winargs() {
        let raw = strings(&["arena", "--chart", "--frames", "2", "--exit"]);
        let (chart, rest) = split_chart_args(&raw).unwrap();
        assert!(chart.enabled);
        let parsed = WinArgs::parse(&rest).unwrap();
        assert_eq!(parsed.frames, Some(2));
        assert!(parsed.exit);
        // Custom dir flows through; duplicates and unknowns fail loudly.
        let raw = strings(&["arena", "--chart", "--chart-dir", "out/chart"]);
        let (chart, rest) = split_chart_args(&raw).unwrap();
        assert_eq!(chart.dir, std::path::PathBuf::from("out/chart"));
        assert!(WinArgs::parse(&rest).is_ok());
        assert!(split_chart_args(&strings(&["arena", "--chart", "--chart"])).is_err());
        assert!(split_chart_args(&strings(&["arena", "--chart-fps", "30"])).is_err());
        assert!(split_chart_args(&strings(&["arena", "--chart-dir", "out"])).is_err());
    }

    #[test]
    fn chart_target_downscales_1600x900_to_256x144() {
        let sized = engine_render::ibl::chart_target_size(1600, 900).unwrap();
        assert_eq!(sized, (256, 144));
        assert_eq!((CHART_VIEWPORT.0, CHART_VIEWPORT.1), (1600, 900));
    }

    #[test]
    fn chart_rl6_reference_stays_within_tol() {
        let rows = engine_render::material_layer::chart();
        assert_eq!(rows.len(), 5);
        let mut worst = 0.0f32;
        for entry in &rows {
            let got = engine_render::material_layer::check_entry(entry).unwrap();
            for (live_c, ref_c) in got.iter().zip(entry.expected.iter()) {
                worst = worst.max((live_c - ref_c).abs());
            }
        }
        assert!(
            worst <= engine_render::material_layer::CHART_TOLERANCE,
            "worst {worst}"
        );
    }

    #[test]
    fn chart_png_encoder_emits_valid_png() {
        let pixels = vec![128u8; 4 * 4 * 4];
        let png = chart_encode_png_rgba8(&pixels, 4, 4).unwrap();
        assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        assert!(chart_encode_png_rgba8(&pixels, 0, 4).is_err());
        assert!(chart_encode_png_rgba8(&[0u8; 3], 4, 4).is_err());
    }

    #[test]
    fn chart_run_writes_pngs_and_reference_within_tol() {
        let root = bin_temp_dir("chart");
        let dir = root.join("chart");
        let chart = ChartArgs {
            enabled: true,
            dir: dir.clone(),
        };
        let args = WinArgs::parse(&strings(&["arena"])).unwrap();
        assert_eq!(run_chart_mode(&bin_config(), &args, &chart), 0);
        for name in ["chart_256x144.png", "chart_reference_256x144.png"] {
            let png = std::fs::read(dir.join(name)).unwrap();
            assert_eq!(&png[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
        }
        let json = std::fs::read_to_string(dir.join("chart_reference.json")).unwrap();
        assert!(json.contains("\"target\":[256,144]"), "{json}");
        assert!(json.contains("cloth/front"), "{json}");
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn production_path_draws_live_bodies_never_staged_dots() {
        let mut app = App::new(bin_config()).expect("live session builds");
        assert!(live_fight_dots(&app, &[]).is_empty());
        let human = FighterId::new(HUMAN_SLOT).expect("slot 0 is a fighter");
        let mut cues = Vec::new();
        for tick in 0..12u32 {
            let input = FighterInput {
                tick,
                fighter: human,
                mx: 100,
                mz: 0,
                facing: 0,
                buttons: engine_input::Buttons::NONE,
            };
            let outcome = app.tick(input);
            cues = outcome.cues.clone();
        }
        let dots = live_fight_dots(&app, &cues);
        assert_eq!(dots.len(), 2);
        assert!(dots.iter().all(|dot| {
            dot.hp == app.hp()[usize::from(dot.slot)]
                && dot.pos_m[1] == 0.0
                && dot.pos_m.iter().all(|v| v.is_finite())
        }));
        let resident = vec![fight_mesh_key(0), fight_mesh_key(1)];
        let mut list = hud_draw_list(1280, 720).expect("HUD list builds");
        let draws = push_fight_draws(&mut list, &resident, &dots).expect("live draws push");
        assert_eq!(draws, 2);
        assert_eq!(list.items.len(), 2);
    }

    #[test]
    fn transient_acquire_validation_skips_while_other_validation_fails() {
        use engine_render::present::PresentError;
        // The 2026-10-12 blip: `surface acquire failed` must skip, not exit 1,
        // so a one-frame driver validation still prints `arena framing: frames=300`.
        let blip = PresentError::Validation("surface acquire failed".into());
        assert!(
            ArenaApp::should_skip_acquire_validation(&blip),
            "acquire blip must skip, not fail the smoke"
        );
        // Every other validation still fails loudly via the shared policy.
        let hard = PresentError::Validation("sky look: bad".into());
        assert!(
            !ArenaApp::should_skip_acquire_validation(&hard),
            "real validation must still fail"
        );
        assert_eq!(
            surface_frame_action(Err(&hard)),
            SurfaceFrameAction::Failed,
            "shared policy still fails hard validation"
        );
    }

    #[test]
    fn brief_props_carry_kit_rows_plus_ring_and_wall() {
        let (props, light) = frame_brief_props();
        let kit_rows = kit_prop_rows().len();
        assert_eq!(props.len(), kit_rows + 2, "kit plus ring and wall");
        assert_eq!(light.sun_dir, [0.0, 1.0, 0.0], "noon until the bind");
    }

    /// The twin's zone report lists one line per hand-placed row with the
    /// flags, so a layout edit shows up in the twin output beside the props.
    #[test]
    fn zone_report_lists_every_hand_placed_row() {
        let lines = zone_report_lines();
        let rows = engine_game::arena_kit::kit_zone_rows().expect("zone rows parse");
        assert_eq!(lines.len(), rows.len(), "one zone line per row: {lines:?}");
        assert!(
            lines[0].starts_with("arena zone sand_floor class=0 advantage=0 dominance=0 risk=0"),
            "{:?}",
            lines[0]
        );
        let gate = lines
            .iter()
            .find(|line| line.starts_with("arena zone gate_tunnel "))
            .expect("gate line");
        assert!(
            gate.contains("class=2") && gate.contains("risk=1") && gate.contains("zone=Gate"),
            "{gate}"
        );
        let centre = lines
            .iter()
            .find(|line| line.starts_with("arena zone centre "))
            .expect("centre line");
        assert!(
            centre.contains("advantage=1") && centre.contains("dominance=1"),
            "{centre}"
        );
    }

    #[test]
    fn the_agent_seat_flag_takes_a_port_and_refuses_a_bad_one() {
        let raw = |args: &[&str]| {
            args.iter()
                .map(|arg| (*arg).to_string())
                .collect::<Vec<_>>()
        };
        let (port, rest) = arena_args::split_agent_seat(&raw(&["--agent-seat", "0", "--exit"]))
            .expect("a port splits");
        assert_eq!(port, Some(0), "0 asks the OS for a free port");
        assert_eq!(rest, vec!["--exit".to_string()]);
        for bad in [
            vec!["--agent-seat"],
            vec!["--agent-seat", "--exit"],
            vec!["--agent-seat", "nope"],
            vec!["--agent-seat", "70000"],
        ] {
            assert!(
                arena_args::split_agent_seat(&raw(&bad)).is_err(),
                "{bad:?} must fail loudly"
            );
        }
    }

    #[test]
    fn warning_bundle_lands_once_and_only_for_a_warned_run() {
        let warning = "arena WARN: view_sunk_cm_max value=12.0 floor=3.0".to_string();
        assert_eq!(
            warning_bundle_metric(&[], false),
            None,
            "a clean run writes nothing"
        );
        assert_eq!(
            warning_bundle_metric(std::slice::from_ref(&warning), true),
            None,
            "a written bundle never writes again"
        );
        assert_eq!(
            warning_bundle_metric(std::slice::from_ref(&warning), false),
            Some("view_sunk_cm_max")
        );
        assert_eq!(
            warning_bundle_metric(&["arena: not a warning".to_string()], false),
            None,
            "only WARN lines name a bundle"
        );
        assert_eq!(
            warning_metric("arena WARN: view_eye_in_body value=1 floor=0"),
            Some("view_eye_in_body")
        );
    }
}
