//! arena_frame: per-frame helpers for the arena window. Holds the boot
//! helpers `ArenaApp::new` calls, the fixed-tick loop `tick_and_present`
//! runs, and the per-pass timing report written at exit. The methods stay on
//! `ArenaApp` (defined in the crate root); this module only moves them out.
use engine_audio::device::{AudioSink, DeviceOutput};
use engine_audio::CueBank;

use super::*;

pub(super) mod pace;
pub(super) mod trace;
pub(crate) use pace::FixedDtPacer;
pub(crate) use trace::{FrameSplit, FrameTrace};

/// Watch content files plus shaders by file; missing roots watch nothing,
/// notes still stage. The designer combat rows file rides explicitly via
/// the practice path const: the `content` scan only sees top-level files
/// and skips the `design` dir as non-file, so without this a designer
/// number edit never raises an event and the practice swap never stages.
pub(super) fn watch_content_roots() -> Watcher {
    let mut watcher = Watcher::new();
    for root in ["content", "crates/render/src/shaders"] {
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten().take(128) {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let Some(text) = path.to_str() else {
                continue;
            };
            watcher.watch(text, WatchSettings::default());
        }
    }
    watcher.watch(
        engine_game::practice::DESIGN_COMBAT_ROWS_PATH,
        WatchSettings::default(),
    );
    watcher
}

/// Parse the test-only panic tick from `TEST_PANIC_ENV`; unset means none.
pub(super) fn test_panic_tick() -> Result<Option<u32>, WinError> {
    match std::env::var(TEST_PANIC_ENV) {
        Ok(text) => Ok(Some(
            text.trim()
                .parse::<u32>()
                .map_err(|_| WinError::BadConfig(TEST_PANIC_ENV))?,
        )),
        Err(_) => Ok(None),
    }
}

/// What opening the cue feed produced: the sender, then either the live
/// device that owns the engine or the offline engine the game thread pumps,
/// plus the one-line state of the audio for the log.
pub(super) struct CueFeedOpen {
    pub(super) tx: Option<FeedTx>,
    /// Offline engine, pumped by `run_sim_ticks`; `None` while a device plays.
    pub(super) engine: Option<AudioEngine>,
    /// Live stream that owns the engine; `None` when muted or refused.
    pub(super) device: Option<DeviceOutput>,
    pub(super) notice: String,
}

/// Stage the arena cue bank and open the feed over it, saying how the audio
/// came up. Real `content/audio` takes only: missing clips fail loudly instead
/// of falling back to silent synth. The device (when one opens) plays the cue
/// engine the feed drives, so what the window hears is placed by the camera
/// (EG21w).
pub(super) fn open_window_audio(muted: bool) -> Result<CueFeedOpen, WinError> {
    let (cue_bank, _) = stage_arena_bank(WINDOWED_SAMPLE_RATE)?;
    let open = open_cue_feed(Some(cue_bank), muted);
    if open.device.is_some() {
        tell!("arena: {}", open.notice);
    } else {
        tell!("arena notice: {}", open.notice);
    }
    Ok(open)
}

/// Feed sender plus the engine behind it for the arena cue bank: the real
/// takes when the bank decoded, nothing when it did not. Unless `muted`, the
/// default device plays the engine itself, so the feed's placed voices are
/// the ones heard; a refused open (or `muted`) keeps an offline engine over
/// the same bank and says why in `notice` (never silent about it).
pub(super) fn open_cue_feed(cue_opt: Option<Arc<CueBank>>, muted: bool) -> CueFeedOpen {
    let Some(cue_bank) = cue_opt else {
        return CueFeedOpen {
            tx: None,
            engine: None,
            device: None,
            notice: "audio degraded: no cue bank".to_string(),
        };
    };
    let (tx, sink, refusal) = AudioSink::open(
        &cue_bank,
        engine_audio::FEED_CAPACITY,
        WINDOWED_SAMPLE_RATE,
        2040,
        muted,
    );
    let (engine, device, notice) = match (sink, refusal) {
        (AudioSink::Device(output), _) => {
            let notice = format!(
                "audio live: {}ch {}Hz",
                output.channels(),
                output.sample_rate()
            );
            (None, Some(output), notice)
        }
        (AudioSink::Offline(engine), Some(err)) => (
            Some(*engine),
            None,
            format!("audio degraded: {err}; cue engine renders offline"),
        ),
        (AudioSink::Offline(engine), None) => (
            Some(*engine),
            None,
            engine_game::windowed::MUTED_AUDIO_NOTICE.to_string(),
        ),
    };
    CueFeedOpen {
        tx: Some(tx),
        engine,
        device,
        notice,
    }
}

/// Title boot greeting: the stranger sees the start key before the bout.
pub(super) fn announce_title_boot(boot: Flow) {
    if boot == Flow::Title {
        let (cells, quads) = engine_game::windowed::title_cells_quads();
        say!(
            "arena {} hud_cells={cells} hud_quads={quads}",
            engine_game::windowed::title_text()
        );
    }
}

/// One presented frame's wall split in us, kept so the frame-split receipt
/// can name the frames that cost (the table's mean rides a minority of them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FrameRow {
    /// 0-based presented-frame index (`frames_presented` minus one).
    pub(super) index: u32,
    /// Fixed ticks plus the present call: the `cpu_ms` sample of this frame.
    pub(super) work_us: u64,
    /// The present call alone (skin, surface frame, capture).
    pub(super) present_us: u64,
    /// Fighter pose plus upload share of `present_us`.
    pub(super) fighters_us: u64,
}

/// A frame is slow when its work wall is strictly over this many times the
/// table's work p50.
const SLOW_FRAME_FACTOR: u64 = 2;
/// Slowest frames named on the receipt.
const SLOWEST_SHOWN: usize = 3;

/// Slow-frame fields of the frame-split receipt: how many frames ran over
/// twice the work p50, the index of the first one (`-` when none) and the
/// slowest frames with their present and fighter share. A zero p50 gives no
/// scale, so nothing counts as slow. Pure over the table: no window needed.
pub(super) fn slow_frame_fields(rows: &[FrameRow]) -> String {
    let mut sorted: Vec<u64> = rows.iter().map(|row| row.work_us).collect();
    sorted.sort_unstable();
    let p50 = percentile_us(&sorted, 50);
    let limit = p50.saturating_mul(SLOW_FRAME_FACTOR);
    let is_slow = |row: &&FrameRow| p50 > 0 && row.work_us > limit;
    let count = rows.iter().filter(is_slow).count();
    let first = rows
        .iter()
        .filter(is_slow)
        .map(|row| row.index)
        .min()
        .map_or_else(|| "-".to_string(), |index| index.to_string());
    let mut worst: Vec<&FrameRow> = rows.iter().collect();
    worst.sort_by(|a, b| b.work_us.cmp(&a.work_us).then(a.index.cmp(&b.index)));
    worst.truncate(SLOWEST_SHOWN);
    let named = worst
        .iter()
        .map(|row| {
            format!(
                "{}:{}ms/present={}ms/fighters={}ms",
                row.index,
                fmt_us_as_ms(row.work_us),
                fmt_us_as_ms(row.present_us),
                fmt_us_as_ms(row.fighters_us),
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let named = if named.is_empty() {
        "-".to_string()
    } else {
        named
    };
    format!("slow_frames={count} first_slow_index={first} slowest={named}")
}

/// Mean us per fixed tick paid: a wall total over the tick count. The window
/// steps one tick per wall-time step, so the same presented frames divide a
/// 21-tick run and a 96-tick run; this is the figure that compares across
/// runs. `-` before the first tick, never a division by zero.
pub(super) fn per_tick_us(total_us: u64, ticks: u32) -> String {
    match u64::from(ticks) {
        0 => "-".to_string(),
        ticks => (total_us / ticks).to_string(),
    }
}

/// Tick-normalized fields of the frame-split receipt: the ticks paid and the
/// present and fighter wall per tick in us. Pure over the totals.
pub(super) fn tick_fields(ticks: u32, present_us: u64, fighters_us: u64) -> String {
    format!(
        "ticks={ticks} us_per_tick_present={} us_per_tick_fighters={}",
        per_tick_us(present_us, ticks),
        per_tick_us(fighters_us, ticks),
    )
}

/// Which frame count divides per-pass column `index`: the probe columns (sky,
/// layers, particles, decals, ibl) divide by the frames that probed, every
/// other column and every run that probed nothing divides by all folded
/// frames. Never 0, so a column cannot silently print a run's sum.
pub(super) fn pass_divisor(index: usize, probes: u32, frames: u32) -> u32 {
    if index < PROBE_PASS_COLUMNS && probes > 0 {
        probes
    } else {
        frames.max(1)
    }
}

/// Leading per-pass columns that only probe frames fill.
const PROBE_PASS_COLUMNS: usize = 5;

/// Whether a wall step skips its present under `--present-skip` (all steps)
/// or `--present-every N`. The cadence is `engine_game::should_present` over
/// the 0-based index of the newest tick run, so the final tick always
/// presents. Before the first tick (title, warmup) there is nothing to
/// throttle and the window still draws. Ticks are never skipped.
pub(super) fn present_skipped(
    ticks_done: u32,
    ticks_total: u32,
    present_every: u32,
    present_skip: bool,
) -> bool {
    if present_skip {
        return true;
    }
    match ticks_done.checked_sub(1) {
        Some(newest) => !engine_game::should_present(newest, ticks_total, present_every),
        None => false,
    }
}

/// Whether the `--exit` run has seen enough. A staged fight leaves once the
/// whole session ran and drew a frame; `--frames N` leaves after N presented
/// frames. `--present-skip` presents none, so it leaves when the ticks are
/// done instead of waiting for frames that can never come.
pub(super) fn exit_reached(
    staged: bool,
    ticks_done: u32,
    ticks_total: u32,
    frames_presented: u32,
    frames: Option<u32>,
    present_skip: bool,
) -> bool {
    if present_skip {
        return ticks_done >= ticks_total;
    }
    if staged {
        return ticks_done >= ticks_total && frames_presented > 0;
    }
    frames.is_some_and(|wanted| frames_presented >= wanted)
}

/// Floor distance in meters past which the rival counts as far and the window
/// title walks the player toward them. A melee starts well inside this.
const RIVAL_FAR_M: f32 = 10.0;

/// The window title: the controls, with the rival hint first when there is one
/// (a narrow title bar clips the tail, and the hint is the part that matters).
pub(super) fn window_title(hint: Option<&str>) -> String {
    match hint {
        Some(hint) => format!("engine2040 arena | {hint} | {CONTROLS_LINE}"),
        None => format!("engine2040 arena | {CONTROLS_LINE}"),
    }
}

/// The opening hint: while the nearest other fighter stands farther than
/// [`RIVAL_FAR_M`] from the human, say how to reach them and how far they are.
/// `None` once they are close, or when there is nobody to walk to. The spawn
/// is 38 m across the pit, so the first seconds are a walk. Pure over the views.
pub(super) fn rival_hint(views: &[engine_game::FighterView], human: u8) -> Option<String> {
    let me = views.iter().find(|view| view.slot == human)?;
    let near = views
        .iter()
        .filter(|view| view.slot != human)
        .map(|view| (view.pos_m[0] - me.pos_m[0]).hypot(view.pos_m[2] - me.pos_m[2]))
        .min_by(f32::total_cmp)?;
    (near.is_finite() && near > RIVAL_FAR_M)
        .then(|| format!("walk toward the rival with W ({near:.0} m away)"))
}

impl ArenaApp {
    /// The human slot's input for `tick`. A bound capture tape owns the slot
    /// first, then the staged fight script and the `--spear-demo` script (all
    /// three speak world axes and are used as they are). Otherwise it is the
    /// keyboard, merged with the pad when one is held, made camera-relative so
    /// W walks where the camera looks and aimed at the nearest living rival so
    /// a standing swing and the spear go at them. The spear key rides last.
    pub(super) fn human_input(&mut self, tick: u32, human: FighterId) -> FighterInput {
        let staged = self
            .capture
            .as_mut()
            .and_then(|slot| slot.tape.input_for_tick(tick))
            // An armed agent seat authors the same slot as a tape; the flags
            // refuse both together, so at most one of them answers.
            .or_else(|| {
                self.seat
                    .as_ref()
                    .and_then(|seat| seat.input_for_tick(tick, human))
            });
        let input = match staged {
            // Tape-bound capture: the tape owns the human slot this tick.
            Some(input) => input,
            None if self.staged.is_some() => staged_fight_input(tick, human),
            None if self.spear.demo => self.spear_demo_input(tick, human),
            // No pad held: the keyboard path exactly as before. A held pad
            // or deflected stick merges into the same single input.
            None if self.pad.is_idle() => self.aim_at_rival(self.camera_relative(
                fighter_input_from_pressed(tick, human, &self.pressed, &self.bindings),
            )),
            None => {
                let held = self
                    .pad
                    .fighter_input(tick, human, &self.pressed, &self.bindings)
                    // The slot is the session's own human, so the table cannot
                    // refuse; the keyboard input is the safe answer if it ever did.
                    .unwrap_or_else(|_| {
                        fighter_input_from_pressed(tick, human, &self.pressed, &self.bindings)
                    });
                self.aim_at_rival(self.camera_relative(held))
            }
        };
        self.with_spear_key(input)
    }

    /// The agent seat reads back this tick's match hash, and the view
    /// receipt so far every VIEW_EVERY_TICKS. One short write per tick; a
    /// dead client is counted, never waited for.
    pub(super) fn publish_seat(&self, tick: u32) {
        let Some(seat) = self.seat.as_ref() else {
            return;
        };
        let hash = self.app.hud().state_hash();
        seat.publish(&format!("tick {tick} {hash:016x}\n"));
        if tick % engine_game::agent_seat::VIEW_EVERY_TICKS == 0 {
            if let Some(line) = self.view.line() {
                seat.publish(&format!("view {line}\n"));
            }
        }
    }

    /// Fold this tick's joint motion into the pop law; presentation only.
    fn note_pop_tick(&mut self, tick: u32) {
        if let Some(look) = self.look.as_mut() {
            if let Ok(bodies) = look.pose_frame_joints(usize::from(self.config.fighters), 0.5) {
                self.view.note_tick(f64::from(tick) + 0.5, &bodies);
            }
        }
    }

    /// Run `steps` fixed ticks for the human slot: input, feed and mixer
    /// pumps, staged and look tallies, then the crash ring. A network seat
    /// (`--host`/`--join`) pumps first and flushes last; its ticks come from
    /// the seat (the host authors once the guest's input is in, the guest
    /// replays authority), and a tick the seat cannot run yet ends the loop.
    /// A seat failure is kept for `tick_and_present` to end the window with.
    pub(super) fn run_sim_ticks(&mut self, steps: u32, human: FighterId) {
        let steps = match self.net_begin(steps, human) {
            Ok(steps) => steps,
            Err(err) => return self.net_fail(err),
        };
        for _ in 0..steps {
            if self.ticks_done >= self.config.ticks {
                break;
            }
            let tick = self.app.cursor();
            let input = if self.net.is_some() {
                match self.net_tick(tick, human) {
                    Ok(Some(input)) => input,
                    Ok(None) => break,
                    Err(err) => return self.net_fail(err),
                }
            } else {
                let input = self.human_input(tick, human);
                self.check_forced_tick(tick);
                self.app.tick_into(input, &mut self.tick_scratch);
                input
            };
            debug_assert!(self.tick_scratch.input_accepted);
            self.publish_seat(tick);
            // Lock-free feed first (tick-stamped, bounded, `dropped_full`
            // counted): a busy mixer never loses commands because the feed
            // already queued them with the camera listener.
            if let Some(tx) = self.feed_tx.as_mut() {
                feed_tick_audio(
                    tx,
                    &mut self.audio_space,
                    tick,
                    &self.tick_scratch,
                    self.app.fighter_views(),
                );
            }
            // Offline mirror into the shared mixer (best effort), only while
            // no device stream is live: with a device the feed's engine is
            // what plays, and a mixer nobody renders would just grow voices.
            // A refused lock loses mirror voices only, never feed commands
            // which are already queued and counted above. Reused buffer keeps
            // the per-tick path heap-free.
            if self._device.is_none() {
                if let Ok(mut mixer) = self.mixer.try_lock() {
                    for command in &self.tick_scratch.audio {
                        mixer.play(self.bank.get(command.sound), command.gain);
                    }
                    mixer.render_into(&mut self.audio_scratch);
                }
                // Offline pump when no device stream is live so sent == applied
                // even headless; a live stream pumps itself on the callback.
                if let Some(engine) = self.feed_engine.as_mut() {
                    engine.render_stereo_into(&mut self.feed_scratch);
                }
            }
            // Presentation reads the tick after it ran; never writes back.
            if let Some(tally) = self.staged.as_mut() {
                tally.observe(&self.tick_scratch.cues);
            }
            self.feel.observe(&self.tick_scratch.cues);
            self.observe_spear_events(tick);
            if let Some(look) = self.look.as_mut() {
                let (views, hp) = (self.app.fighter_views(), self.app.hp());
                observe_tick_with_sim(look, tick, views, hp, &self.tick_scratch.cues, None);
                if let Some(scene) = self.spear.scene.as_mut() {
                    scene.observe_tick(look, self.app.live_match(), views);
                }
                // EG45: poll authoritative balance gaits into the pose (presentation only).
                look.observe_live_balances(
                    self.app.live_match(),
                    usize::from(self.config.fighters),
                );
            }
            self.note_pop_tick(tick);
            // The crate's break picture takes the tick's authority break events (an empty
            // slice on quiet ticks) and steps its debris once; presentation only (RZ4w).
            let break_cue = match self.break_scene.as_mut() {
                Some(scene) => match scene.observe_outcome(&self.tick_scratch) {
                    Ok(()) => scene.take_cue(),
                    Err(err) => {
                        tell!("arena notice: break picture dropped: {err}");
                        self.break_scene = None;
                        None
                    }
                },
                None => None,
            };
            // The break sounds where the blow struck: presentation only (RZ4).
            if let (Some(cue), Some(tx)) = (break_cue, self.feed_tx.as_mut()) {
                self.audio_space
                    .feed_break(tx, tick, cue.name, cue.gain, cue.at_m);
            }
            self.record_crash_tick(input);
            self.ticks_done += 1;
        }
        if let Err(err) = self.net_end() {
            self.net_fail(err);
        }
    }

    /// The exit lines for the audio feed: what the queue sent, applied and
    /// refused (a missing cue is `unknown_cue`, never silent) and what the
    /// window's space layer asked of it.
    pub(super) fn report_audio_feed(&self) {
        let Some(tx) = self.feed_tx.as_ref() else {
            return;
        };
        let stats = tx.stats();
        let space = self.audio_space.stats();
        say!(
            "arena audio feed: sent={} applied={} dropped_full={} unknown_cue={}",
            stats.sent,
            stats.applied,
            stats.dropped_full,
            stats.unknown_cue,
        );
        say!(
            "arena audio space: placed={} flat={} footsteps={} tension_sends={} tension_unheard={} bed_writes={} refused={} breaks={}",
            space.positioned,
            space.flat,
            space.footsteps,
            space.tension_sends,
            space.tension_unheard,
            space.bed_writes,
            space.refused,
            space.breaks,
        );
    }

    /// Decide this wall step's present under the throttle flags and count a
    /// skip in `frames_skipped`. A skipped step still ran its ticks; it is not
    /// a presented frame, so `frames_presented` and the framing tally stay put.
    pub(super) fn take_present_skip(&mut self) -> bool {
        let skip = present_skipped(
            self.ticks_done,
            self.config.ticks,
            self.args.present_every,
            self.args.present_skip,
        );
        self.frames_skipped = self.frames_skipped.saturating_add(u32::from(skip));
        skip
    }

    /// Print the bound tape's receipt and refuse a tape the sim never obeyed:
    /// a manifest that binds a tape must not read green when no input of it
    /// reached the human slot, or when a tick of it was skipped. No tape, no
    /// line. Run once at exit, before the capture slot is sealed.
    pub(super) fn report_tape(&mut self) {
        let Some(receipt) = self.capture.as_ref().and_then(|slot| slot.tape.receipt()) else {
            return;
        };
        say!("{}", receipt.line());
        if let Some(why) = receipt.failure() {
            tell!("arena: {why}");
            self.failure = Some(1);
        }
    }

    /// Fold one presented twin receipt into the per-pass means.
    /// Cold receipt path only; the first frame pins adapter/driver/size.
    pub(super) fn note_receipt(&mut self, receipt: &engine_render::present::FrameReceipt) {
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
        }
        // The means above are taken over every folded frame, so the size they
        // were taken at is the newest frame's: the shell resizes the window to
        // the capture size a few frames into the run, and a run that names its
        // boot size would report the 1080p means under a 720p label.
        self.pass_size = (receipt.width, receipt.height);
        self.pass_frames = self.pass_frames.saturating_add(1);
        if receipt.look_passes > 0 {
            self.pass_probes = self.pass_probes.saturating_add(1);
        }
    }

    /// Mean ms of one per-pass column: the probe columns (sky, layers,
    /// particles, decals, ibl) average over frames that probed, fog, gtao
    /// and shadow over every folded frame. A run that probes nothing at all
    /// (`pass_probes == 0`: live without `--look-probes` or capture) folds no
    /// probe sample, so those columns carry the live-path value of every
    /// folded frame and divide by `pass_frames` like the rest; asking for a
    /// divisor of 0 clamps to 1 and prints the run's sum under an `ms` label.
    /// The divisor is [`pass_divisor`], also printed on the per-pass line.
    pub(super) fn pass_mean(&self, index: usize) -> String {
        let over = pass_divisor(index, self.pass_probes, self.pass_frames);
        Self::mean_ms(self.pass_us[index], u64::from(over))
    }

    /// Format device forward p50 ms with three decimals; pure so tests pin
    /// live above zero without a window. Non-blocking device timing only,
    /// never a wait in the frame [render-pipeline-rules rule 2].
    pub(super) fn format_forward_ms(p50: f32) -> String {
        format!("{:.3}", p50)
    }

    /// Device forward ms from the GPU timestamp ring (p50 over resolved
    /// frames), honest `0.000` until the first sample lands or when the
    /// adapter has no timestamp queries. Evidence only, never hashed; the
    /// live frame runs forward every present while the probe columns stay 0
    /// without `--look-probes`, so the EG8 gate tells LIVE (forward above 0)
    /// from PROBE (probe columns above 0).
    pub(super) fn forward_ms(&self) -> String {
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
    /// total validation errors, then the two divisors behind the means:
    /// `gpu_frames` (fog, gtao, shadow, forward) and `probe_frames` (sky
    /// through ibl). `None` until the first presented frame pins the adapter.
    pub(super) fn per_pass_line(&self) -> Option<String> {
        if self.pass_frames == 0 || self.pass_adapter.is_empty() {
            return None;
        }
        Some(format!(
            "arena per-pass: adapter=\"{}\" driver=\"{}\" resolution={}x{} frames={} sky={}ms layers={}ms particles={}ms decals={}ms ibl={}ms fog={}ms gtao={}ms shadow={}ms forward={}ms validation_errors={} gpu_frames={} probe_frames={}",
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
            self.pass_frames,
            pass_divisor(0, self.pass_probes, self.pass_frames),
        ))
    }

    /// Mean millis with three decimals from summed micros over frames,
    /// using integer math so the smoke path needs no float cast.
    pub(super) fn mean_ms(total_us: u64, frames: u64) -> String {
        let frames = frames.max(1);
        let mean_us = total_us / frames;
        format!("{}.{:03}", mean_us / 1000, mean_us % 1000)
    }

    /// File the per-pass means beside the capture manifest (`per_pass_ms.json`)
    /// so the clip-adjacent evidence rides with the clip without touching the
    /// deterministic `manifest.json` bytes. IO failures print loudly; the
    /// smoke exit code is owned by the caller.
    pub(super) fn write_per_pass_sidecar(&self, manifest: &std::path::Path) {
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

    /// The frame-split receipt: mean sim, present and fighter ms, then the
    /// fighter skin wall split into its CPU pose and GPU upload halves, then
    /// the per-frame work and interval percentiles on the CPU, the GPU pass
    /// percentiles from the device clock (`gpu_ms=unsupported` when the
    /// adapter has no timestamp queries), the mean swapchain acquire wait
    /// and the present mode (a vsync-bound mode parks the CPU in acquire).
    /// `fighters_pose`/`fighters_upload` are `0.000` until a skinned tick
    /// lands, so they never read as a measurement of a frame that did work.
    /// The line ends with the slow-frame fields ([`slow_frame_fields`]) and
    /// the tick-normalized fields ([`tick_fields`]), then `skipped=` (wall
    /// steps whose present the throttle skipped on purpose; 0 without
    /// `--present-every`/`--present-skip`): new fields only append, so older
    /// receipts still parse.
    pub(super) fn frame_split_line(&self) -> String {
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
                    "gpu_ms p50={:.3} p99={:.3} (forward p50={:.3} sky p50={:.3} blit p50={:.3}; {} frames)",
                    report.total.p50,
                    report.total.p99,
                    report.forward.p50,
                    report.sky.p50,
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
            "arena frame-split: sim={}ms present={}ms fighters={}ms fighters_pose={}ms fighters_upload={}ms per frame (mean over {} frames) cpu_ms p50={} p99={} interval_ms p50={} p99={} {} acquire={}ms present_mode={} {} {} skipped={}",
            Self::mean_ms(self.sim_us, frames),
            Self::mean_ms(self.present_us, frames),
            Self::mean_ms(self.fighters_us, frames),
            Self::mean_ms(self.fighters_pose_us, frames),
            Self::mean_ms(self.fighters_upload_us, frames),
            self.frames_presented,
            fmt_us_as_ms(percentile_us(&work, 50)),
            fmt_us_as_ms(percentile_us(&work, 99)),
            fmt_us_as_ms(percentile_us(&interval, 50)),
            fmt_us_as_ms(percentile_us(&interval, 99)),
            gpu,
            fmt_us_as_ms(acquire_us),
            mode,
            slow_frame_fields(&self.frame_rows),
            tick_fields(self.ticks_done, self.present_us, self.fighters_us),
            self.frames_skipped,
        )
    }

    /// Print the presenter blocking-call tally and return the failure
    /// code when any live frame mapped a buffer or waited on the GPU.
    /// Probe frames (`--look-probes`) and captures block by design and are
    /// counted apart; a live frame that blocks is a regression.
    pub(super) fn report_waits(&self) -> Option<i32> {
        let stats = self.presenter.as_ref()?.wait_stats();
        say!("arena waits: {}", stats.summary());
        let message = live_wait_failure(&stats)?;
        tell!("arena: {message}");
        Some(1)
    }

    /// Write the per-frame present-path trace to the path
    /// `E2040_FRAME_TRACE` named, and print the slow frames with the
    /// section that ate each one. A run without the variable writes
    /// nothing and prints nothing.
    pub(super) fn write_frame_trace(&mut self) {
        let Some(path) = self.frame_trace_path.clone() else {
            return;
        };
        let trace = std::mem::take(&mut self.frame_trace);
        if let Err(err) = trace::write_trace_csv(&path, &trace) {
            tell!("arena: frame trace {path:?}: {err}");
            self.failure = Some(2);
            return;
        }
        say!(
            "arena frame-trace: file=\"{}\" frames={} slow={}",
            path.display(),
            trace.len(),
            trace::slow_frames_line(&trace),
        );
        for row in trace::slow_frames(&trace).iter().take(10) {
            say!("arena frame-trace-slow: {}", trace::slow_frame_line(row));
        }
    }

    /// One row of the per-frame trace: the frame's wall split and the
    /// present path's own split. Nothing is written unless
    /// `E2040_FRAME_TRACE` named a path, so the default run pays nothing.
    pub(super) fn note_frame_trace(
        &mut self,
        work_us: u64,
        interval_us: u64,
        sim_us: u64,
        present_us: u64,
        fighters_us: u64,
    ) {
        if self.frame_trace_path.is_none() {
            return;
        }
        self.frame_trace.push(arena_frame::FrameTrace {
            index: self.frames_presented.saturating_sub(1),
            wall_us: duration_us(self.started.elapsed()),
            work_us,
            interval_us,
            sim_us,
            present_us,
            fighters_us,
            split: self.last_frame_split,
            ticks: self.ticks_done,
            focused: self.window_focused,
            occluded: self.window_occluded,
        });
    }

    /// The window's focus and occlusion state, for the frame trace: a
    /// present path that parks on an unfocused window is a different stall
    /// from one that parks on the compositor.
    pub(super) fn note_window_state(&mut self, focused: bool, occluded: bool) {
        self.window_focused = focused;
        self.window_occluded = occluded;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_game::GameConfig;

    #[test]
    fn live_per_pass_means_divide_by_folded_frames_not_one() {
        // Failing-before: a live run folds frames and probes none, so
        // `pass_probes` stays 0 and the probe columns asked `mean_ms` for a
        // divisor of 0, which clamps to 1: `ibl=` printed the 300-frame sum
        // (32.185 ms) where the encode wall costs 0.107 ms per frame, so FC1's
        // frame cost table had to discount its own biggest column.
        let raw: Vec<String> = ["arena", "--frames", "2", "--exit"]
            .iter()
            .map(|arg| (*arg).to_string())
            .collect();
        let args = WinArgs::parse(&raw).expect("valid smoke fixture parses");
        let config = GameConfig {
            seed: 2040,
            ticks: 24,
            fighters: 2,
            half_extent_m: 20.0,
        };
        let mut app = ArenaApp::new(config, args, None).expect("test session builds");
        app.pass_adapter = "RTX".to_string();
        app.pass_frames = 300;
        app.pass_probes = 0;
        app.pass_us = [0; 8];
        app.pass_us[4] = 32_185;
        assert_eq!(
            app.pass_mean(4),
            "0.107",
            "live ibl averages the folded frames"
        );
        assert_eq!(app.pass_mean(7), "0.000", "a zero column stays 0.000");
        let line = app
            .per_pass_line()
            .expect("pinned adapter plus folded frames build the line");
        assert!(
            line.contains("frames=300") && line.contains("ibl=0.107ms"),
            "the printed column carries the per-frame mean, not the sum: {line}"
        );
        // The clamp that printed the sum: `mean_ms` with a zero divisor
        // divides by `frames.max(1)`, so `pass_mean` must never ask for 0.
        assert_eq!(ArenaApp::mean_ms(32_185, 0), "32.185", "zero divisor sums");
        assert_eq!(ArenaApp::mean_ms(32_185, 300), "0.107", "300 frames mean");
        // A run that does probe keeps the probe divisor, so the capture
        // sidecar still files sky_ms over the frames that probed.
        app.pass_probes = 2;
        app.pass_us[4] = 2_000;
        assert_eq!(app.pass_mean(4), "1.000", "probed ibl keeps its divisor");
    }

    /// Headless app over a 2-fighter 24-tick session: no window, no presenter.
    fn headless_app() -> ArenaApp {
        let raw: Vec<String> = ["arena", "--frames", "2", "--exit"]
            .iter()
            .map(|arg| (*arg).to_string())
            .collect();
        let args = WinArgs::parse(&raw).expect("valid smoke fixture parses");
        let config = GameConfig {
            seed: 2040,
            ticks: 24,
            fighters: 2,
            half_extent_m: 20.0,
        };
        ArenaApp::new(config, args, None).expect("test session builds")
    }

    fn frame_row(index: u32, work_us: u64, present_us: u64, fighters_us: u64) -> FrameRow {
        FrameRow {
            index,
            work_us,
            present_us,
            fighters_us,
        }
    }

    /// 300 fast frames (1 ms) with a 15 ms frame at `slow` indexes.
    fn table_with_slow(slow: &[(u32, u64, u64)]) -> Vec<FrameRow> {
        let mut rows: Vec<FrameRow> = (0..300).map(|i| frame_row(i, 1_000, 800, 300)).collect();
        for &(index, present_us, fighters_us) in slow {
            rows[index as usize] = frame_row(index, 15_000, present_us, fighters_us);
        }
        rows
    }

    #[test]
    fn frame_split_slow_frames_are_named() {
        // Failing-before: the frame-split line printed mean, p50 and p99
        // only, so a 15 ms frame in a 1 ms table had no index and the mean
        // (4.7 ms) could not be traced to startup, a stall or a spike.
        let rows = table_with_slow(&[(0, 14_500, 9_000), (200, 14_000, 10_000)]);
        let fields = slow_frame_fields(&rows);
        assert!(
            fields.contains("slow_frames=2 "),
            "two frames over 2x p50: {fields}"
        );
        assert!(
            fields.contains("first_slow_index=0 "),
            "the first slow frame is index 0: {fields}"
        );
        // Slowest first, ties by index; the third entry is the first fast frame.
        assert!(
            fields.ends_with(
                "slowest=0:15.000ms/present=14.500ms/fighters=9.000ms,\
                 200:15.000ms/present=14.000ms/fighters=10.000ms,\
                 1:1.000ms/present=0.800ms/fighters=0.300ms"
            ),
            "{fields}"
        );
        // Frame 0 fast, only frame 200 slow: the first slow index moves.
        let later = slow_frame_fields(&table_with_slow(&[(200, 14_000, 10_000)]));
        assert!(
            later.starts_with("slow_frames=1 first_slow_index=200 slowest=200:15.000ms"),
            "{later}"
        );
    }

    #[test]
    fn frame_split_slow_frames_need_a_real_gap_to_count() {
        // No slow frame: the field is `-`, not 0 and not an empty string.
        let flat = slow_frame_fields(&table_with_slow(&[]));
        assert!(
            flat.starts_with("slow_frames=0 first_slow_index=- slowest=0:1.000ms"),
            "{flat}"
        );
        // Exactly 2x the p50 is not slow: the bar is strictly over.
        let edge: Vec<FrameRow> = (0..10)
            .map(|i| frame_row(i, if i == 4 { 2_000 } else { 1_000 }, 0, 0))
            .collect();
        assert!(slow_frame_fields(&edge).starts_with("slow_frames=0 first_slow_index=- "));
        // One microsecond over is.
        let over: Vec<FrameRow> = (0..10)
            .map(|i| frame_row(i, if i == 4 { 2_001 } else { 1_000 }, 0, 0))
            .collect();
        assert!(slow_frame_fields(&over).starts_with("slow_frames=1 first_slow_index=4 "));
        // An empty table prints `-` everywhere and never divides or indexes.
        assert_eq!(
            slow_frame_fields(&[]),
            "slow_frames=0 first_slow_index=- slowest=-"
        );
        // A zero p50 has no scale: nothing is slow against it.
        let zero: Vec<FrameRow> = (0..5).map(|i| frame_row(i, 0, 0, 0)).collect();
        assert!(slow_frame_fields(&zero).starts_with("slow_frames=0 first_slow_index=- "));
    }

    #[test]
    fn frame_split_line_appends_the_slow_fields_after_the_old_ones() {
        let mut app = headless_app();
        app.frames_presented = 300;
        app.frame_rows = table_with_slow(&[(0, 14_500, 9_000), (200, 14_000, 10_000)]);
        app.work_us = app.frame_rows.iter().map(|row| row.work_us).collect();
        let line = app.frame_split_line();
        let old_tail = line.find("present_mode=").expect("old field kept");
        let new_head = line.find("slow_frames=2").expect("slow fields printed");
        assert!(line.starts_with("arena frame-split: sim="), "{line}");
        assert!(old_tail < new_head, "new fields only append: {line}");
        assert!(
            line.contains("cpu_ms p50=1.000 p99=1.000"),
            "the p50 the slow bar reads is the printed one: {line}"
        );
        assert!(line.contains("first_slow_index=0 "), "{line}");
    }

    #[test]
    fn frame_split_pass_divisor_is_explicit() {
        // Probe columns (0..5) divide by the probed frames when any probed;
        // a run that probed nothing, and every other column, divide by all
        // folded frames. Never 0.
        assert_eq!(pass_divisor(4, 0, 300), 300, "live ibl: folded frames");
        assert_eq!(pass_divisor(4, 2, 300), 2, "probed ibl: probed frames");
        assert_eq!(pass_divisor(5, 2, 300), 300, "fog: folded frames");
        assert_eq!(pass_divisor(0, 0, 0), 1, "no frames clamps to 1");
        let mut app = headless_app();
        app.pass_adapter = "RTX".to_string();
        app.pass_frames = 300;
        app.pass_probes = 0;
        let line = app.per_pass_line().expect("pinned adapter builds the line");
        assert!(line.ends_with("gpu_frames=300 probe_frames=300"), "{line}");
        app.pass_probes = 7;
        let line = app.per_pass_line().expect("pinned adapter builds the line");
        assert!(line.ends_with("gpu_frames=300 probe_frames=7"), "{line}");
    }

    #[test]
    fn frame_split_ticks_make_three_runs_comparable() {
        // The 2026-10-17T0447Z quiet window: same build, same 300 presented
        // frames, 96 / 40 / 21 ticks paid. The printed means spread 5x; the
        // per-tick figures sit together.
        let runs = [(96u32, 5_824u64), (40, 2_090), (21, 1_116)];
        let per_tick: Vec<String> = runs
            .iter()
            .map(|&(ticks, mean_us)| per_tick_us(mean_us * 300, ticks))
            .collect();
        assert_eq!(per_tick, ["18200", "15675", "15942"]);
        assert_eq!(
            tick_fields(96, 1_747_200, 955_200),
            "ticks=96 us_per_tick_present=18200 us_per_tick_fighters=9950"
        );
        // No tick paid: `-`, never `inf`, NaN or a division by zero.
        assert_eq!(per_tick_us(5_000, 0), "-");
        assert_eq!(
            tick_fields(0, 5_000, 2_000),
            "ticks=0 us_per_tick_present=- us_per_tick_fighters=-"
        );
    }

    #[test]
    fn frame_split_line_prints_the_tick_count_and_per_tick_wall() {
        let mut app = headless_app();
        app.frames_presented = 300;
        app.ticks_done = 40;
        app.present_us = 40 * 15_000;
        app.fighters_us = 40 * 7_000;
        let line = app.frame_split_line();
        assert!(
            line.contains("ticks=40 us_per_tick_present=15000 us_per_tick_fighters=7000"),
            "{line}"
        );
        // The old fields keep their name, order and unit: the mean is still
        // per presented frame (600 ms over 300 frames).
        assert!(
            line.contains("sim=0.000ms present=2.000ms fighters=0.933ms"),
            "{line}"
        );
        assert!(
            line.find("present_mode=") < line.find("ticks="),
            "tick fields append after the old ones: {line}"
        );
        app.ticks_done = 0;
        let idle = app.frame_split_line();
        assert!(
            idle.contains("ticks=0 us_per_tick_present=- us_per_tick_fighters=-"),
            "{idle}"
        );
        assert!(!idle.contains("inf") && !idle.contains("NaN"), "{idle}");
    }

    /// Run the throttle over `ticks` ticks with `steps` wall steps each, the
    /// way the window loop calls it. Returns the `ticks_done` of each skipped
    /// step.
    fn skipped_steps(app: &mut ArenaApp, ticks: u32, steps: u32) -> Vec<u32> {
        let mut skipped = Vec::new();
        for ticks_done in 0..=ticks {
            app.ticks_done = ticks_done;
            for _ in 0..steps {
                if app.take_present_skip() {
                    skipped.push(ticks_done);
                }
            }
        }
        skipped
    }

    #[test]
    fn present_skip_throttle_names_its_skipped_frames() {
        // Failing-before: the window parsed `--present-every`/`--present-skip`
        // and ignored both, and the receipt had no `skipped=` field to tell a
        // skipped frame from a lost one.
        let mut app = headless_app();
        // No flag: nothing skipped, the receipt says so.
        let none = skipped_steps(&mut app, 24, 3);
        assert!(none.is_empty(), "default presents every step: {none:?}");
        assert_eq!(app.frames_skipped, 0);
        assert!(app.frame_split_line().ends_with("skipped=0"));
        // `--present-every 2`: the odd tick counts skip, three steps each.
        let mut app = headless_app();
        app.args.present_every = 2;
        let skipped = skipped_steps(&mut app, 24, 3);
        let odd: Vec<u32> = (1..24).step_by(2).flat_map(|t| [t, t, t]).collect();
        assert_eq!(skipped, odd, "exactly the odd ticks skip");
        assert_eq!(app.frames_skipped, 36, "the counter counts those steps");
        assert_eq!(app.frames_presented, 0, "a skip never counts as presented");
        assert!(app.frame_split_line().ends_with("skipped=36"));
        // `--present-skip`: every step skips, the first tick count too.
        let mut app = headless_app();
        app.args.present_skip = true;
        let all = skipped_steps(&mut app, 4, 2);
        assert_eq!(all.len(), 10, "5 tick counts x 2 steps all skip");
        assert_eq!(app.frames_skipped, 10);
    }

    #[test]
    fn should_present_cadence_keeps_the_last_tick_and_the_title_frame() {
        // The final tick presents even when the count is not a multiple.
        let mut app = headless_app();
        app.config.ticks = 25;
        app.args.present_every = 2;
        let skipped = skipped_steps(&mut app, 25, 1);
        assert!(
            !skipped.contains(&25),
            "the last tick presents: {skipped:?}"
        );
        assert!(!skipped.contains(&0), "no tick yet: nothing to throttle");
        assert_eq!(skipped, (1..25).step_by(2).collect::<Vec<u32>>());
        // The pure decision agrees with the predicate it reuses.
        assert!(!present_skipped(0, 24, 2, false), "title/warmup draws");
        assert!(present_skipped(1, 24, 2, false), "tick 0 of 2 skips");
        assert!(!present_skipped(2, 24, 2, false), "tick 1 of 2 presents");
        assert!(!present_skipped(24, 24, 7, false), "last tick presents");
        assert!(!present_skipped(5, 24, 1, false), "every=1 presents all");
        assert!(present_skipped(0, 24, 1, true), "--present-skip wins");
    }

    #[test]
    fn present_skip_exit_waits_for_ticks_not_for_frames_that_cannot_come() {
        // `--frames N` keeps meaning N presented frames.
        assert!(!exit_reached(false, 10, 300, 299, Some(300), false));
        assert!(exit_reached(false, 10, 300, 300, Some(300), false));
        assert!(
            !exit_reached(false, 300, 300, 0, None, false),
            "no --frames"
        );
        // A staged fight needs the whole session and one drawn frame.
        assert!(!exit_reached(true, 300, 300, 0, None, false));
        assert!(exit_reached(true, 300, 300, 1, None, false));
        // `--present-skip` presents none: leave when the ticks are done.
        assert!(!exit_reached(false, 299, 300, 0, Some(300), true));
        assert!(exit_reached(false, 300, 300, 0, Some(300), true));
        assert!(exit_reached(true, 300, 300, 0, None, true));
    }
}
