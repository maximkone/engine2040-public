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
use std::time::Instant;

use engine_audio::{Bank, FightCue, Mixer};
use engine_game::arena_kit::{
    kit_prop_rows, live_kit_notice, push_kit_draws, stage_kit_or_default, KitReport,
};
use engine_game::capture;
use engine_game::crash;
use engine_game::fight_ring::ring_prop_row;
use engine_game::fighter_look::{look_down_deg, shot_for_frame, FighterLook, LookFrame};
use engine_game::flow::{Flow, FlowEvent};
use engine_game::input::{map_key, map_mouse};
use engine_game::view_truth::{brief_line_with_props, FrameLight, PropTruth, ViewLaws};
use engine_game::windowed::{
    cap_message as capped, check_exit_deadline_with_limit, fighter_input_from_pressed,
    format_timed_smoke, hud_cells_quads, open_audio, push_fight_draws, smoke_muted,
    split_staged_flag, stage_dressing, stage_fight_bodies, staged_fight_input,
    staged_session_config, surface_frame_action, StagedFighterDot, StagedTally, SurfaceFrameAction,
    TickAccumulator, WinArgs, WinError, WINDOWED_SAMPLE_RATE, WINDOW_SIZE,
};
use engine_game::{App, GameConfig, TickOutcome, HUMAN_SLOT};
use engine_input::{Bindings, FighterId, FighterInput, PhysicalButton};
use engine_render::present::{DrawList, MeshKey, PresentError, Presenter, WaitStats};

use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
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

mod arena_capture;
mod arena_winit;

use arena_capture::capture::{
    frame_list, headless_fallback, run_chart_mode, setup_capture, split_chart_args, CaptureSlot,
};

/// Default session seed for windowed play.
const DEFAULT_SEED: u64 = 2040;
/// Default session length in fixed ticks when `--ticks` is absent.
const DEFAULT_TICKS: u32 = 600;
/// Flag that turns the look probes on in the window (default off): the
/// bind probes, the IBL chart and the fog plus GTAO readback tranche, each a
/// blocking GPU readback that feeds no pixel.
const LOOK_PROBES_FLAG: &str = "--look-probes";
/// Surface frames between look-probe rounds under `--look-probes`.
const LOOK_PROBE_EVERY: u32 = 120;
/// Surface frames between fog plus contact refreshes under `--look-probes`.
const LIVE_FOG_EVERY: u32 = 4;

/// Flag that fails the run when any presented frame shows no fighter (the
/// camera once looked at bare sand for half a perf run, 2026-10-11).
const REQUIRE_FRAMING_FLAG: &str = "--require-framing";

/// `--view-brief <path>`: write the view truth of every presented frame as
/// one JSON line (camera, then each drawn body's act, clip, tint, bounds and
/// screen rect), so a script or a model reads the scene with no screenshot.
const VIEW_BRIEF_FLAG: &str = "--view-brief";

/// Pull `--view-brief <path>` out of the argument list; a flag with no
/// path is an error.
fn split_view_brief(raw: &[String]) -> Result<(Option<PathBuf>, Vec<String>), String> {
    let Some(at) = raw.iter().position(|arg| arg == VIEW_BRIEF_FLAG) else {
        return Ok((None, raw.to_vec()));
    };
    let path = raw
        .get(at + 1)
        .filter(|value| !value.starts_with("--"))
        .ok_or_else(|| format!("{VIEW_BRIEF_FLAG} needs a file path"))?;
    let mut rest = raw.to_vec();
    rest.drain(at..=at + 1);
    Ok((Some(PathBuf::from(path)), rest))
}

/// Pull one bare `flag` (`--look-probes`, `--require-framing`) out of the
/// argument list (the rest still goes to the strict `WinArgs` parser, which
/// refuses unknown flags).
fn split_flag(raw: &[String], flag: &str) -> (bool, Vec<String>) {
    let on = raw.iter().any(|arg| arg == flag);
    let rest = raw
        .iter()
        .filter(|arg| arg.as_str() != flag)
        .cloned()
        .collect();
    (on, rest)
}

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

    /// The `arena framing:` receipt line; `None` when nothing was tallied.
    fn line(&self) -> Option<String> {
        (self.frames > 0).then(|| {
            let first = self.first_empty.map_or_else(|| "-".to_string(), |f| f.to_string());
            let share = f64::from(self.tinted) / f64::from(self.bodies.max(1));
            format!(
                "arena framing: frames={} empty={} first_empty={first} fewest_in_view={} steepest_down_deg={:.1} tinted_share={share:.2}",
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

fn main() {
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
    std::process::exit(code);
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
    let (look_probes, raw) = split_flag(&raw, LOOK_PROBES_FLAG);
    let (require_framing, raw) = split_flag(&raw, REQUIRE_FRAMING_FLAG);
    let (view_brief, raw) = match split_view_brief(&raw) {
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
    let args = match WinArgs::parse(&rest) {
        Ok(args) => args,
        Err(err) => {
            tell!("arena: {err}");
            return 2;
        }
    };
    let config = match session_config(&args) {
        Ok(_) if staged => staged_session_config(),
        Ok(config) => config,
        Err(err) => {
            tell!("arena: {err}");
            return 2;
        }
    };
    // Backend pick is resolved (and the env override stuck) before any GPU
    // work on every path, so stdout always names the picked backend.
    let pick = engine_render::backend::apply_cli_override(args.backend);
    say!("arena {}", pick.startup_line());
    // OS VRAM budget before the device takes its share (cold, startup only).
    println!("arena {}", engine_render::backend::read_vram().log_line());
    if chart_args.enabled {
        return run_chart_mode(&config, &args, &chart_args);
    }
    let loop_result = EventLoop::new();
    let event_loop = match loop_result {
        Ok(loop_) => loop_,
        Err(err) => {
            let err = WinError::NoDisplay(capped(&err.to_string()));
            if args.frames.is_some() {
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
    app.look_probes = look_probes;
    app.require_framing = require_framing;
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

/// Exit-path deadline check with the `Timeout` receipt: the shared limit
/// check reports a stall as `Render`, but the `--exit` watchdog names a
/// `Timeout` carrying elapsed plus limit, so the binary refiles that message
/// under `Timeout` (every other error passes through untouched). Pure over
/// the same inputs, so the forced-stall drill pins it without waiting.
fn check_exit_wall_deadline(elapsed_ms: f64, limit_ms: u32) -> Result<(), WinError> {
    check_exit_deadline_with_limit(elapsed_ms, limit_ms).map_err(|err| match err {
        WinError::Render(msg) => WinError::Timeout(msg),
        other => other,
    })
}

/// Whole microseconds of a wall duration, saturating (diagnostics only).
fn duration_us(elapsed: std::time::Duration) -> u64 {
    u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX)
}

/// Live fight bodies: one dot per simulated fighter from the rolling pose
/// cursor (planar position, feet on the floor), live hp, this tick's flash.
fn live_fight_dots(app: &App, cues: &[FightCue]) -> Vec<StagedFighterDot> {
    let fighters = usize::from(app.config().fighters);
    let poses = app.window_frame_poses();
    let mut dots = Vec::with_capacity(fighters);
    for slot in 0..fighters {
        let Some(id) = u8::try_from(slot).ok().and_then(FighterId::new) else {
            continue;
        };
        let Some(root) = poses.get(slot).and_then(|pose| pose.positions.first()) else {
            continue;
        };
        if !root.iter().all(|v| v.is_finite()) {
            continue;
        }
        let hit = cues.iter().any(|cue| match cue {
            FightCue::Hit { victim, .. } => *victim == id.get(),
            FightCue::Knockdown { victim, .. } => *victim == id.get(),
            _ => false,
        });
        dots.push(StagedFighterDot {
            slot: id.get(),
            pos_m: [root[0], 0.0, root[2]],
            hp: app.hp().get(slot).copied().unwrap_or(1000),
            hit,
        });
    }
    dots
}

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
    /// Staged-fight cue tally while `--staged-fight` drives the human slot.
    staged: Option<StagedTally>,
    /// Armed capture slot (twin PNG bytes plus tape cursor); `None` idles.
    capture: Option<CaptureSlot>,
    window: Option<Arc<Window>>,
    pressed: Vec<PhysicalButton>,
    acc: TickAccumulator,
    /// Window shell state; Paused holds the accumulator at zero steps.
    flow: Flow,
    last: Instant,
    /// Wall start of the session for the `--timed-smoke` fps line.
    started: Instant,
    ticks_done: u32,
    frames_presented: u32,
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
    /// `--look-probes`: run the blocking look probes in the window.
    look_probes: bool,
    /// `--require-framing`: a frame with no fighter in the picture fails.
    require_framing: bool,
    /// What the fight camera showed (the `arena framing:` line).
    framing: Framing,
    /// The drawn bodies' view laws (the `arena view:` line).
    view: ViewLaws,
    /// `--view-brief`: one JSON line of view truth per presented frame.
    view_brief: Option<BufWriter<File>>,
    /// Wall us of each frame body: fixed ticks plus the present call,
    /// which includes the swapchain acquire.
    work_us: Vec<u64>,
    /// Wall us between consecutive frames (the rate the player sees).
    interval_us: Vec<u64>,
    /// Tick index that panics on purpose (`TEST_PANIC_ENV`); `None` otherwise.
    test_panic_at: Option<u32>,
    /// Reused scratch for one tick's canonical input bytes (crash tape).
    crash_bytes: Vec<u8>,
}

impl ArenaApp {
    fn new(
        config: GameConfig,
        args: WinArgs,
        capture: Option<CaptureSlot>,
    ) -> Result<Self, WinError> {
        let test_panic_at = match std::env::var(TEST_PANIC_ENV) {
            Ok(text) => Some(
                text.trim()
                    .parse::<u32>()
                    .map_err(|_| WinError::BadConfig(TEST_PANIC_ENV))?,
            ),
            Err(_) => None,
        };
        let app = App::new(config)?.into_duel()?;
        let mixer = Arc::new(Mutex::new(Mixer::new(WINDOWED_SAMPLE_RATE)));
        // Automated smoke never opens a device: `--mute` mutes explicitly,
        // `--exit`/`--timed-smoke` runs mute by construction.
        let muted = smoke_muted(&args);
        // `--mute` plus automated `--exit` runs open no audio device at all;
        // the mixer keeps rendering voices offline either way.
        let muted = args.mute || muted;
        let (_device, audio_notice) = open_audio(Arc::clone(&mixer), muted);
        if _device.is_some() {
            tell!("arena: {audio_notice}");
        } else {
            tell!("arena notice: {audio_notice}");
        }
        Ok(Self {
            config,
            args,
            app,
            bindings: Bindings::new(),
            bank: Bank::placeholder(WINDOWED_SAMPLE_RATE),
            mixer,
            _device,
            audio_notice,
            presenter: None,
            presenter_notice: "presenter pending: no window yet".to_string(),
            kit: KitReport::default(),
            fight_draws: 0,
            look: None,
            staged: None,
            capture,
            window: None,
            pressed: Vec::new(),
            acc: TickAccumulator::new(),
            flow: Flow::Match,
            last: Instant::now(),
            started: Instant::now(),
            ticks_done: 0,
            frames_presented: 0,
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
            look_probes: false,
            require_framing: false,
            framing: Framing::default(),
            view: ViewLaws::default(),
            view_brief: None,
            work_us: Vec::new(),
            interval_us: Vec::new(),
            test_panic_at,
            crash_bytes: Vec::with_capacity(16),
        })
    }

    fn human(&self) -> Result<FighterId, WinError> {
        FighterId::new(HUMAN_SLOT).ok_or(WinError::BadConfig("fighters"))
    }

    fn fail(&mut self, event_loop: &ActiveEventLoop, err: WinError, code: i32) {
        tell!("arena: {err}");
        self.failure = Some(code);
        event_loop.exit();
    }

    /// Fold one presented twin receipt into the per-pass means.
    /// Cold receipt path only; the first frame pins adapter/driver/size.
    fn note_receipt(&mut self, receipt: &engine_render::present::FrameReceipt) {
        let table = receipt.per_pass_us();
        for (index, (_, us)) in table.iter().enumerate() {
            self.pass_us[index] = self.pass_us[index].saturating_add(*us);
        }
        self.pass_validation = self
            .pass_validation
            .saturating_add(receipt.total_validation_errors());
        if self.pass_frames == 0 {
            self.pass_adapter.clone_from(&receipt.adapter);
            self.pass_driver.clone_from(&receipt.driver);
            self.pass_size = (receipt.width, receipt.height);
        }
        self.pass_frames = self.pass_frames.saturating_add(1);
        if receipt.look_passes > 0 {
            self.pass_probes = self.pass_probes.saturating_add(1);
        }
    }

    /// Mean ms of one per-pass column: the probe columns (sky, layers,
    /// particles, decals, ibl) average over frames that probed, fog, gtao
    /// and shadow over every folded frame.
    fn pass_mean(&self, index: usize) -> String {
        let over = if index < 5 {
            self.pass_probes
        } else {
            self.pass_frames
        };
        Self::mean_ms(self.pass_us[index], u64::from(over))
    }

    /// Format device forward p50 ms with three decimals; pure so tests pin
    /// live above zero without a window. Non-blocking device timing only,
    /// never a wait in the frame [render-pipeline-rules rule 2].
    fn format_forward_ms(p50: f32) -> String {
        format!("{:.3}", p50)
    }

    /// Device forward ms from the GPU timestamp ring (p50 over resolved
    /// frames), honest `0.000` until the first sample lands or when the
    /// adapter has no timestamp queries. Evidence only, never hashed; the
    /// live frame runs forward every present while the probe columns stay 0
    /// without `--look-probes`, so the EG8 gate tells LIVE (forward above 0)
    /// from PROBE (probe columns above 0).
    fn forward_ms(&self) -> String {
        self.presenter
            .as_ref()
            .and_then(Presenter::gpu_timing_report)
            .map_or_else(
                || "0.000".to_string(),
                |report| Self::format_forward_ms(report.forward.p50),
            )
    }

    /// One-line per-pass means for the smoke log: adapter plus driver
    /// plus resolution plus mean ms per pass plus device forward ms plus
    /// total validation errors. `None` until the first presented frame pins
    /// the adapter.
    fn per_pass_line(&self) -> Option<String> {
        if self.pass_frames == 0 || self.pass_adapter.is_empty() {
            return None;
        }
        Some(format!(
            "arena per-pass: adapter=\"{}\" driver=\"{}\" resolution={}x{} frames={} sky={}ms layers={}ms particles={}ms decals={}ms ibl={}ms fog={}ms gtao={}ms shadow={}ms forward={}ms validation_errors={}",
            self.pass_adapter,
            self.pass_driver,
            self.pass_size.0,
            self.pass_size.1,
            self.pass_frames,
            self.pass_mean(0),
            self.pass_mean(1),
            self.pass_mean(2),
            self.pass_mean(3),
            self.pass_mean(4),
            self.pass_mean(5),
            self.pass_mean(6),
            self.pass_mean(7),
            self.forward_ms(),
            self.pass_validation,
        ))
    }

    /// Mean millis with three decimals from summed micros over frames,
    /// using integer math so the smoke path needs no float cast.
    fn mean_ms(total_us: u64, frames: u64) -> String {
        let frames = frames.max(1);
        let mean_us = total_us / frames;
        format!("{}.{:03}", mean_us / 1000, mean_us % 1000)
    }

    /// File the per-pass means beside the capture manifest (`per_pass_ms.json`)
    /// so the clip-adjacent evidence rides with the clip without touching the
    /// deterministic `manifest.json` bytes. IO failures print loudly; the
    /// smoke exit code is owned by the caller.
    fn write_per_pass_sidecar(&self, manifest: &std::path::Path) {
        let Some(line) = self.per_pass_line() else {
            return;
        };
        let Some(dir) = manifest.parent() else {
            return;
        };
        let json = format!(
            "{{\"adapter\":\"{}\",\"driver\":\"{}\",\"resolution\":[{},{}],\"frames\":{},\"sky_ms\":{},\"layers_ms\":{},\"particles_ms\":{},\"decals_ms\":{},\"ibl_ms\":{},\"fog_ms\":{},\"gtao_ms\":{},\"shadow_ms\":{},\"forward_ms\":{},\"validation_errors\":{}}}",
            self.pass_adapter.replace('"', "'"),
            self.pass_driver.replace('"', "'"),
            self.pass_size.0,
            self.pass_size.1,
            self.pass_frames,
            self.pass_mean(0),
            self.pass_mean(1),
            self.pass_mean(2),
            self.pass_mean(3),
            self.pass_mean(4),
            self.pass_mean(5),
            self.pass_mean(6),
            self.pass_mean(7),
            self.forward_ms(),
            self.pass_validation,
        );
        let path = dir.join("per_pass_ms.json");
        if let Err(err) = std::fs::write(&path, json.as_bytes()) {
            tell!(
                "arena: per-pass sidecar write failed ({}: {err})",
                path.display()
            );
        } else {
            say!("arena per-pass manifest=\"{}\" {line}", path.display());
        }
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
        let steps = self.acc.push_elapsed_ms(elapsed_ms);
        warmup_steps(steps, self.ticks_done, self.config.ticks, true)
    }

    /// Step fixed ticks for the elapsed wall time, then present one HUD frame.
    fn tick_and_present(&mut self, event_loop: &ActiveEventLoop) {
        // The automated `--exit` run owns its wall-clock bound (`--deadline-ms`
        // overrides the 5 min default): a hung smoke ends itself loudly instead
        // of burning a core behind the open window.
        if self.args.exit {
            let wall_ms = self.started.elapsed().as_secs_f64() * 1000.0;
            if let Err(err) = check_exit_wall_deadline(wall_ms, self.args.wall_limit_ms()) {
                return self.fail(event_loop, err, 1);
            }
        }
        let interval = self.last.elapsed();
        let elapsed_ms = interval.as_secs_f64() * 1000.0;
        self.last = Instant::now();
        let steps = self.ticks_owed(elapsed_ms);
        let sim_started = Instant::now();
        let human = match self.human() {
            Ok(human) => human,
            Err(err) => return self.fail(event_loop, err, 2),
        };
        for _ in 0..steps {
            if self.ticks_done >= self.config.ticks {
                break;
            }
            let tick = self.app.cursor();
            let staged = self
                .capture
                .as_mut()
                .and_then(|slot| slot.tape.input_for_tick(tick));
            let input = match staged {
                // Tape-bound capture: the tape owns the human slot this tick.
                Some(input) => input,
                None if self.staged.is_some() => staged_fight_input(tick, human),
                None => fighter_input_from_pressed(tick, human, &self.pressed, &self.bindings),
            };
            self.check_forced_tick(tick);
            let outcome = {
                self.app.tick_into(input, &mut self.tick_scratch);
                &self.tick_scratch
            };
            debug_assert!(outcome.input_accepted);
            if let Ok(mut mixer) = self.mixer.try_lock() {
                for command in &outcome.audio {
                    mixer.play(self.bank.get(command.sound), command.gain);
                }
                // Offline render only while no device stream is live: the
                // device callback pulls voices itself when connected.
                // Reused buffer keeps the per-tick path heap-free.
                if self._device.is_none() {
                    mixer.render_into(&mut self.audio_scratch);
                }
            }
            // Presentation reads the tick after it ran; never writes back.
            if let Some(tally) = self.staged.as_mut() {
                tally.observe(&self.tick_scratch.cues);
            }
            if let Some(look) = self.look.as_mut() {
                let (views, hp) = (self.app.fighter_views(), self.app.hp());
                look.observe_tick(tick, views, hp, &self.tick_scratch.cues);
            }
            self.record_crash_tick(input);
            self.ticks_done += 1;
        }
        if self.ticks_done >= self.config.ticks {
            self.flow = self.flow.on(FlowEvent::MatchOver);
        }
        self.last_alpha = self.acc.alpha();
        let present_started = Instant::now();
        self.sim_us = self
            .sim_us
            .saturating_add(duration_us(present_started - sim_started));
        self.present_frame(event_loop);
        self.present_us = self
            .present_us
            .saturating_add(duration_us(present_started.elapsed()));
        if self.args.exit && self.frames_presented > 0 {
            self.work_us.push(duration_us(sim_started.elapsed()));
            self.interval_us.push(duration_us(interval));
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
        let finished = match self.staged {
            Some(_) => self.ticks_done >= self.config.ticks && self.frames_presented > 0,
            None => self
                .args
                .frames
                .is_some_and(|frames| self.frames_presented >= frames),
        };
        if self.args.exit && finished {
            self.finish_exit(event_loop);
        }
    }

    /// The frame-split receipt: mean sim, present and fighter ms, then the
    /// per-frame work and interval percentiles on the CPU, the GPU pass
    /// percentiles from the device clock (`gpu_ms=unsupported` when the
    /// adapter has no timestamp queries), the mean swapchain acquire wait
    /// and the present mode (a vsync-bound mode parks the CPU in acquire).
    fn frame_split_line(&self) -> String {
        let frames = u64::from(self.frames_presented.max(1));
        let mut work = self.work_us.clone();
        let mut interval = self.interval_us.clone();
        work.sort_unstable();
        interval.sort_unstable();
        let supported = self
            .presenter
            .as_ref()
            .is_some_and(Presenter::gpu_timing_supported);
        // Probe frames (`--look-probes`) are not timestamped: supported
        // but no samples reads `none`, a missing feature reads `unsupported`.
        let missing = if supported {
            "gpu_ms=none".to_string()
        } else {
            "gpu_ms=unsupported".to_string()
        };
        let gpu = self
            .presenter
            .as_ref()
            .and_then(Presenter::gpu_timing_report)
            .map_or(missing, |report| {
                format!(
                    "gpu_ms p50={:.3} p99={:.3} (forward p50={:.3} blit p50={:.3}; {} frames)",
                    report.total.p50,
                    report.total.p99,
                    report.forward.p50,
                    report.blit.p50,
                    report.frames,
                )
            });
        let (acquire_us, mode) = self.presenter.as_ref().map_or((0, String::new()), |p| {
            (
                p.acquire_mean_us(),
                p.present_mode_name().unwrap_or_default(),
            )
        });
        format!(
            "arena frame-split: sim={}ms present={}ms fighters={}ms per frame (mean over {} frames) cpu_ms p50={} p99={} interval_ms p50={} p99={} {} acquire={}ms present_mode={}",
            Self::mean_ms(self.sim_us, frames),
            Self::mean_ms(self.present_us, frames),
            Self::mean_ms(self.fighters_us, frames),
            self.frames_presented,
            fmt_us_as_ms(percentile_us(&work, 50)),
            fmt_us_as_ms(percentile_us(&work, 99)),
            fmt_us_as_ms(percentile_us(&interval, 50)),
            fmt_us_as_ms(percentile_us(&interval, 99)),
            gpu,
            fmt_us_as_ms(acquire_us),
            mode,
        )
    }

    /// Print the presenter blocking-call tally and return the failure
    /// code when any live frame mapped a buffer or waited on the GPU.
    /// Probe frames (`--look-probes`) and captures block by design and are
    /// counted apart; a live frame that blocks is a regression.
    fn report_waits(&self) -> Option<i32> {
        let stats = self.presenter.as_ref()?.wait_stats();
        say!("arena waits: {}", stats.summary());
        let message = live_wait_failure(&stats)?;
        tell!("arena: {message}");
        Some(1)
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
        if self.args.timed_smoke {
            let elapsed_ms = self.started.elapsed().as_secs_f64() * 1000.0;
            match format_timed_smoke(self.frames_presented, self.ticks_done, elapsed_ms) {
                Ok(line) => say!("{line}"),
                Err(err) => return self.fail(event_loop, err, 2),
            }
        }
        say!("{}", self.frame_split_line());
        if let Some(line) = self.framing.line() {
            say!("{line}");
        }
        if let Some(line) = self.view.line() {
            say!("{line}");
        }
        if let Some(mut out) = self.view_brief.take() {
            if let Err(err) = out.flush() {
                tell!("arena: {VIEW_BRIEF_FLAG}: {err}");
                self.failure = Some(2);
            }
        }
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
        if let Some(report) = self.finalize_capture() {
            say!(
                "arena capture: frames={} manifest=\"{}\" sha={}",
                report.frames,
                report.manifest.display(),
                report.sequence_fnv,
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
        };
        // Kit draws ride the same list as the surface and the offscreen
        // capture, so the window and its PNGs frame the same arena. Missing
        // keys draw nothing, so an unstaged kit keeps the cube frame.
        if let Some(presenter) = self.presenter.as_ref() {
            let resident = presenter.mesh_keys();
            push_kit_draws(&mut list, &resident, &self.kit);
        };
        let fighters_started = Instant::now();
        if let Err(err) = self.push_fighters(&mut list) {
            return self.fail(event_loop, err, 1);
        }
        self.fighters_us = self
            .fighters_us
            .saturating_add(duration_us(fighters_started.elapsed()));
        let receipt = {
            let Some(presenter) = self.presenter.as_mut() else {
                self.fail(
                    event_loop,
                    WinError::Render("no visible presenter: window never painted".to_string()),
                    1,
                );
                return;
            };
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
        self.frames_presented += 1;
        self.note_receipt(&receipt);
        self.capture_offscreen(event_loop, &list);
    }

    /// Fighters over the frame list: the skinned CT7 look in its pit when
    /// bound (presentation only), else the staged boxes. Loud on refusal.
    fn push_fighters(&mut self, list: &mut DrawList) -> Result<(), WinError> {
        if let (Some(look), Some(presenter)) = (self.look.as_mut(), self.presenter.as_mut()) {
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
            // Real bodies stand in their pit: the placeholder cube, the
            // dressing boxes and the parade kit (laid across the fight
            // area) leave the frame; the look draws the ring instead.
            list.items.clear();
            self.fight_draws = look
                .present_into(presenter, list, &frame)
                .map_err(|err| WinError::Render(err.to_string()))?;
            let in_view = look.fighters_in_view(&list.camera, frame.fighters);
            if framing_should_tally(self.ticks_done) {
                self.framing
                    .note(self.frames_presented, in_view, look_down_deg(&list.camera));
                self.framing.note_tint(look.tinted(frame.fighters));
            }
            let t = look.sim_time(frame.alpha);
            self.view.note(&list.camera, t, look.view_truth());
            if let Some(out) = self.view_brief.as_mut() {
                // Props and light ride the brief only: kit plus ring draws as
                // one row each, with a law-holding noon light until the sun
                // bind lands. Built only when the flag is on; zero per tick.
                let mut props: Vec<PropTruth> = kit_prop_rows()
                    .iter()
                    .map(|(kind, bounds)| PropTruth {
                        kind,
                        min: bounds[0],
                        max: bounds[1],
                    })
                    .collect();
                let ring = ring_prop_row();
                props.push(PropTruth {
                    kind: ring.0,
                    min: ring.1[0],
                    max: ring.1[1],
                });
                let light = FrameLight {
                    sun_dir: [0.0, 1.0, 0.0],
                    sun_elev: 0.5,
                    exposure: 1.0,
                    fog: 0.0,
                    passes: 1,
                };
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
                    if let Err(err) = slot.sink.write_frame(tick, hash, png) {
                        self.fail(event_loop, WinError::Render(capped(&err.to_string())), 1);
                    }
                }
            }
            Err(err) => self.fail(event_loop, WinError::Present(err), 1),
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
                    "arena capture: frames={} manifest=\"{}\" sha={}",
                    report.frames,
                    report.manifest.display(),
                    report.sequence_fnv,
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

    fn set_pressed(&mut self, button: PhysicalButton, down: bool) {
        if down {
            if !self.pressed.contains(&button) {
                self.pressed.push(button);
            }
        } else {
            self.pressed.retain(|known| *known != button);
        }
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
        match Presenter::init_for_window(Arc::clone(window), width, height) {
            Ok(mut presenter) => {
                // The live frame never waits on the GPU: fog is a fragment
                // function and the probes are off. `--look-probes` brings
                // the blocking probes back, spaced out so they hitch rarely.
                presenter.set_look_probes(self.look_probes);
                presenter.set_probe_every(LOOK_PROBE_EVERY);
                presenter.set_fog_every(LIVE_FOG_EVERY);
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
                    Ok(look) => {
                        tell!(
                            "arena: fighter look bound {} CT7 bodies",
                            look.bundle_shas().len()
                        );
                        self.look = Some(look);
                        // The look poses from views: the loco pose cursor
                        // (two whole-prefix replays per tick) is unused.
                        self.app.set_presentation_cursor(false);
                    }
                    Err(err) => tell!("arena notice: {err}; drawing staged boxes"),
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

impl ApplicationHandler for ArenaApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("engine2040 arena")
            .with_inner_size(winit::dpi::LogicalSize::new(
                f64::from(WINDOW_SIZE.0),
                f64::from(WINDOW_SIZE.1),
            ));
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
            WindowEvent::Resized(size) => {
                if let Some(presenter) = self.presenter.as_mut() {
                    match presenter.resize(size.width, size.height) {
                        Ok(_) => {}
                        Err(err) => self.fail(event_loop, WinError::Present(err), 1),
                    }
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let winit::keyboard::PhysicalKey::Code(code) = event.physical_key {
                    // Escape pauses without repeat: holding the key never
                    // flickers the shell.
                    if code == winit::keyboard::KeyCode::Escape
                        && event.state == ElementState::Pressed
                        && !event.repeat
                    {
                        self.flow = self.flow.on(FlowEvent::TogglePause);
                    }
                    if let Some(key) = arena_winit::from_winit_key(code) {
                        self.set_pressed(map_key(key), event.state == ElementState::Pressed);
                    }
                }
            }
            WindowEvent::MouseInput { button, state, .. } => {
                let pressed = state == ElementState::Pressed;
                self.set_pressed(map_mouse(arena_winit::from_winit_mouse(button)), pressed);
            }
            WindowEvent::RedrawRequested => {
                self.tick_and_present(event_loop);
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Poll);
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
    use engine_game::windowed::{check_exit_deadline, fight_mesh_key, hud_draw_list, SceneSelect};
    use engine_input::FighterInput;

    fn strings(args: &[&str]) -> Vec<String> {
        args.iter().map(|arg| (*arg).to_string()).collect()
    }

    #[test]
    fn look_probes_flag_splits_off_and_defaults_off() {
        let (on, rest) = split_flag(
            &strings(&["arena", "--frames", "3", "--look-probes", "--exit"]),
            LOOK_PROBES_FLAG,
        );
        assert!(on, "the flag turns the blocking probes on");
        assert_eq!(rest, strings(&["arena", "--frames", "3", "--exit"]));
        let (off, same) = split_flag(&strings(&["arena", "--exit"]), LOOK_PROBES_FLAG);
        assert!(!off, "probes ship off");
        assert_eq!(same, strings(&["arena", "--exit"]));
        // The strict parser still refuses the flag if it were not split.
        assert!(WinArgs::parse(&strings(&["arena", "--look-probes"])).is_err());
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
        let (on, rest) = split_flag(
            &strings(&["arena", "--require-framing", "--exit"]),
            REQUIRE_FRAMING_FLAG,
        );
        assert!(on);
        assert_eq!(rest, strings(&["arena", "--exit"]));
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

    #[test]
    fn per_pass_line_posts_means_with_adapter_and_validation() {
        // No frames yet: no adapter pinned, so no line (never a fake table).
        let args = WinArgs::parse(&strings(&["arena", "--frames", "2", "--exit"]))
            .expect("valid smoke fixture parses");
        let mut app = ArenaApp::new(bin_config(), args, None).expect("test session builds");
        assert!(app.per_pass_line().is_none());
        // Two folded receipts average to the posted means; validation sums.
        let receipt = |sky_us: u64,
                       gtao_us: u64,
                       shadow_us: u64,
                       validation: u32|
         -> engine_render::present::FrameReceipt {
            engine_render::present::FrameReceipt {
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
}
