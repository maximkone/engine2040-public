//! The arena frame in the browser (WEB5): the render crate's own presenter, driven from
//! `requestAnimationFrame` on a WebGPU device the page opened asynchronously.
//!
//! Nothing here blocks. The instance, surface, adapter and device come from `await`s (surface
//! before adapter, a fresh instance and adapter after a lost device because Chrome rejects a
//! second device on one adapter); the presenter is then built from those parts without a wait,
//! its error scopes are answered late (see `Presenter::deferred_validation_errors`), and its
//! frame path has no map or poll wait. The scene is the render crate's procedural arena: a
//! lit ring with eight GPU-skinned fighters, the stand-in until the cooked kit can be fetched.
//! Compiled for `wasm32` only; the decisions it prints live in [`crate::arena_view`].

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use engine_game::fighter_look::BodyLook;
use engine_render::pipeline_warmup;
use engine_render::present::{
    last_live_wall_split, DrawList, PresentError, Presenter, MAX_FRAME_DIM,
};
use engine_render::procedural_arena::ProceduralArena;
use engine_render::quality::{self, Quality, WEB_CUT_ORDER};
use wasm_bindgen::prelude::*;
use web_sys::HtmlCanvasElement;

use crate::anim::{FrameStats, StepClock, MAX_STEPS_PER_FRAME};
use crate::arena_view::{
    flag_value, has_flag, scaled_frame_size, CaptureFacts, Counters, CAPTURE_AFTER_FRAMES,
};
use crate::fight::MatchError;
use crate::limits::{adapter_hint, feature_names, limit_rows};
use crate::perf_window;
use crate::recovery::{reason_word, Cause, LostBox};
use crate::report::{AdapterFacts, ArenaFacts, Failure, Ready};
use crate::shell::{
    browser_clock, check_device, cross_origin_isolated, document, fail, log_error, navigator_gpu,
    now_ms, pause, raw_adapter_info, recover_with, request_frame, run_golden, set_body_attr,
    set_state, set_text, user_agent, wasm_pages, watch_visibility, Check, FrameSlot, LOSSES,
    VISIBILITY_CHANGES,
};
use crate::tier_view::{
    cut_line, fallback_for, restore_line, scale_after_cut, scale_after_restore, us_to_ms, whole_ms,
    Cut, Ladder, Pace, ProbeDone, ProbeRun, TierFacts, SCALE_FLOOR_PCT, WIRED,
};
use crate::COMMIT;

/// The frame budget the ladder holds the page to: 30 frames per second.
const BUDGET_MS: f64 = 1000.0 / 30.0;
/// Most fixed match steps one played frame takes (WEB15): a slow frame must not ask for a pile of
/// catch-up steps, or the match drags the frame rate down and the next frame asks for more.
const FIGHT_STEPS_PER_FRAME: u64 = 2;
/// Frames drawn behind the loading screen so every pipeline has been used once before the page
/// says it is ready.
const WARM_FRAMES: u32 = 3;
/// The animation time (seconds) a `?freeze` page holds: the fighters stand mid-pose and every frame
/// is the same picture, so the headless check can compare two captures (QW14).
const FROZEN_T_S: f32 = 1.5;
/// The fixed match ticks a `?gate` page runs once, before it holds the picture: the fighters
/// stand in the arena and the capture is the same frame on every load.
const GATE_TICKS: u64 = 30;
/// Fewest probe frames that make a median worth believing.
const PROBE_MIN_FRAMES: usize = 8;
/// How many 250 ms waits a hidden tab gets before the startup probe gives up on an answer.
const PROBE_HIDDEN_WAITS: u32 = 40;
/// Longest the loader waits for the GPU to finish the warm-up frames, milliseconds.
const WARM_WAIT_MS: f64 = 60_000.0;
/// QW10: the frames a `?bundle=ab` page captures, one inside the first rebuild block and one
/// inside the first replay block. On a frozen page both draw the same picture, so the two
/// hashes the receipt prints must agree.
const PERF_CAPTURE_OFF: u64 = perf_window::WARMUP_FRAMES + 2;
const PERF_CAPTURE_ON: u64 = perf_window::WARMUP_FRAMES + perf_window::BLOCK_FRAMES + 2;

thread_local! {
    /// Set by [`request_arena_capture`]: the next frame that can arm a capture takes it. A frame
    /// that finds a copy already waiting or in flight keeps it set, so an ask is never dropped.
    static CAPTURE_WANTED: Cell<bool> = const { Cell::new(false) };
}

/// Errors the device raised outside any scope, and the first one's text.
#[derive(Default)]
struct Uncaptured {
    count: AtomicU64,
    first: Mutex<Option<String>>,
}

/// One frame loop's state: the presenter and everything the page reports about it.
struct ArenaLoop {
    presenter: Presenter,
    arena: ProceduralArena,
    /// The played match (`?fight`, WEB15); `None` draws the stand-in.
    fight: Option<crate::fight::Match>,
    canvas: HtmlCanvasElement,
    lost: Arc<LostBox>,
    uncaptured: Arc<Uncaptured>,
    max_dim: u32,
    stats: FrameStats,
    /// The 60 Hz simulation clock the animation time comes from (QW11).
    clock: StepClock,
    /// The animation time of a `?freeze` page; `None` on a normal page.
    frozen_s: Option<f32>,
    /// The page's `visibilitychange` count at the last frame (QT7).
    seen_visibility: u32,
    /// `adapter=` as the tier line prints it (QT3).
    adapter_hint: String,
    counters: Counters,
    first_error: Option<String>,
    surface_format: String,
    last_cpu_ms: f64,
    last_draws: u32,
    capture_asked: bool,
    tier: Quality,
    /// Why this tier (`query`, `limits`, `default` or `probe`).
    tier_why: &'static str,
    /// Render scale in percent: the tier's, lowered by the ladder.
    scale_pct: u32,
    start_scale_pct: u32,
    /// The one-second window, the ladder (off for `?ladder=off` or a software adapter that no
    /// cut helps) and the drift check.
    pace: Pace,
    sim_ms: f64,
    late_reported: usize,
    /// PW37: a `?gate` page holds one known picture (frozen time, no match step), so the capture
    /// is the same frame on every load and the frame gate can compare it with the reference.
    gate: bool,
    /// The fixed match ticks a `?gate` page still owes: it runs them once, then holds.
    gate_ticks: u64,
    /// The word for the last fight cue the page saw (`PARRY`, `BLOCK`, ...).
    cue: Option<&'static str>,
    /// The cues the page already named, so a cue is named once and not on every frame it
    /// stays in the match's cue list.
    named_cues: Vec<engine_audio::FightCue>,
    /// What the frame gate measured against the reference capture, once a frame was captured.
    gate_result: Option<crate::arena_view::Gate>,
    /// QW10: the measured frames, the two arms and the captures.
    perf: perf_window::Run,
    /// QW10: the stand-in list recorded once; a replay frame swaps in this frame's camera only.
    static_list: Option<DrawList>,
    /// QW10: how long the frame spent building the draw list.
    last_list_ms: f64,
    /// QW10: the arm that asked for the capture in flight.
    capture_arm: Option<perf_window::Arm>,
}

impl ArenaLoop {
    /// Keep the canvas backing store at the CSS box times the device pixel ratio, within the
    /// presenter's frame bound and the page's pixel budget.
    fn fit_canvas(&mut self) {
        let (w, h) = scaled_frame_size(
            f64::from(self.canvas.client_width()),
            f64::from(self.canvas.client_height()),
            device_pixel_ratio(),
            self.scale_pct,
            self.max_dim,
        );
        if (w, h) != self.presenter.size() {
            self.canvas.set_width(w);
            self.canvas.set_height(h);
            if let Err(e) = self.presenter.resize(w, h) {
                self.note_error(&format!("resize {w}x{h}: {e}"));
            }
        }
    }

    /// Upload the arena to the presenter and note the surface format.
    fn stage(&mut self) -> Result<(), (Failure, String)> {
        self.arena
            .stage(&mut self.presenter)
            .map_err(|e| (Failure::NoDevice, e.to_string()))?;
        self.surface_format = self
            .presenter
            .surface_format()
            .map(|f| format!("{f:?}").to_ascii_lowercase())
            .unwrap_or_default();
        Ok(())
    }

    /// Move the page to another tier: the presenter takes the knobs, the render scale starts
    /// over from the tier's (the ladder's earlier cuts of it are dropped with the old tier).
    fn set_tier(&mut self, tier: Quality) {
        let knobs = quality::knobs(tier);
        self.presenter.apply_quality(&knobs);
        self.tier = tier;
        self.scale_pct = u32::from(knobs.render_scale_pct);
        self.start_scale_pct = self.scale_pct;
    }

    fn note_error(&mut self, text: &str) {
        self.counters.frame_errors += 1;
        log_error(&format!("arena: {text}"));
        self.first_error.get_or_insert_with(|| text.to_string());
    }

    /// Draw one frame that began at `frame_ms` on the animation clock.
    fn draw(&mut self, frame_ms: f64) {
        // A frame that comes after the tab was hidden or shown spans a gap: it is not a slow frame.
        let changes = VISIBILITY_CHANGES.with(Cell::get);
        let resumed = changes != self.seen_visibility;
        self.seen_visibility = changes;
        if resumed {
            self.stats.skip_gap();
            self.pace.resumed();
            self.counters.hidden_skips = self.pace.hidden_skips();
        }
        self.stats.tick(frame_ms);
        self.clock.advance(frame_ms);
        self.counters.steps = self.clock.steps();
        self.counters.max_steps_per_frame = self.clock.max_per_frame();
        self.fit_canvas();
        self.drive_capture();
        let t_s = self.frozen_s.unwrap_or_else(|| self.clock.time_s() as f32);
        let frame = self.counters.frames;
        let cpu_started = now_ms();
        let (w, h) = self.presenter.size();
        if self.fight.is_none() {
            if let Err(e) = self.arena.pose(&mut self.presenter, t_s) {
                self.note_error(&format!("pose: {e}"));
            }
        }
        // QW10: a replay frame swaps the clock's camera into the stand-in list recorded once;
        // the A/B's rebuild arm builds every item again, as the page did before.
        let replay = self.perf.replay(frame);
        let list_started = now_ms();
        let listed = if let Some(mut fight) = self.fight.take() {
            let listed = self.step_fight(&mut fight, t_s);
            self.fight = Some(fight);
            Some(listed)
        } else {
            None
        };
        if listed.is_none() {
            if let Err(e) = self.fill_stand_in_list(w, h, t_s, replay) {
                self.note_error(&format!("list: {e}"));
            }
        }
        self.last_list_ms = now_ms() - list_started;
        self.sim_ms = now_ms() - cpu_started;
        if let Some(listed) = listed {
            match listed {
                Ok(list) => self.draw_list(&list),
                Err(e) => self.note_error(&format!("list: {e}")),
            }
        } else if let Some(list) = self.static_list.take() {
            // Taken for the call and put straight back: the list is the record, not a copy.
            self.draw_list(&list);
            self.static_list = Some(list);
        } else {
            self.note_error("list: the stand-in list is missing");
        }
        self.last_cpu_ms = now_ms() - cpu_started;
        self.perf.note(
            frame,
            perf_window::Sample {
                cpu_ms: self.last_cpu_ms,
                list_ms: self.last_list_ms,
                draws: self.last_draws,
            },
        );
        self.counters.frames += 1;
        self.counters.deferred_validation_errors = self.presenter.deferred_validation_errors();
        self.counters.uncaptured_errors = self.uncaptured.count.load(Ordering::Relaxed);
        if self.first_error.is_none() {
            if let Some(first) = self.presenter.deferred_validation_first() {
                self.first_error = Some(first.to_string());
            } else if let Ok(first) = self.uncaptured.first.lock() {
                self.first_error.clone_from(&first);
            }
        }
        set_text(
            "hud",
            &format!("{} | cpu {:.1} ms", self.stats.line(), self.last_cpu_ms),
        );
        if self.counters.frames % 15 == 1 {
            self.publish_receipt();
        }
        self.tick_split(frame_ms, resumed);
    }

    /// Draw one list through the presenter's own frame path, with the counters and the typed
    /// surface errors the next animation frame retries.
    fn draw_list(&mut self, list: &DrawList) {
        match self.presenter.render_surface(list) {
            Ok(receipt) => {
                self.last_draws = receipt.draws;
                self.counters.validation_errors += u64::from(receipt.total_validation_errors());
                if self.counters.frames.is_multiple_of(15) {
                    set_text("passes", &receipt.per_pass_summary());
                }
            }
            // The presenter reconfigured the surface or the browser has no image for us this
            // frame: skip it, the next animation frame tries again.
            Err(
                PresentError::SurfaceOutdated { .. }
                | PresentError::SurfaceLost { .. }
                | PresentError::SurfaceTimeout
                | PresentError::SurfaceOccluded,
            ) => {}
            Err(e) => self.note_error(&format!("frame: {e}")),
        }
    }

    /// QW10: put the stand-in ring's draw list in `static_list`. `replay` keeps the items recorded
    /// on the first frame and swaps in this frame's camera: the parts, their materials and their
    /// meshes do not change with the clock or the canvas, only the camera does. The rebuild arm
    /// builds every item again, which is what the page did before QW10 and what `?bundle=ab`
    /// measures against.
    fn fill_stand_in_list(
        &mut self,
        w: u32,
        h: u32,
        t_s: f32,
        replay: bool,
    ) -> Result<(), MatchError> {
        let scenery =
            |e: engine_render::procedural_arena::ArenaError| MatchError::Scenery(e.to_string());
        if !replay || self.static_list.is_none() {
            let fresh = self.arena.list_at(w, h, t_s).map_err(scenery)?;
            self.static_list = Some(fresh);
            return Ok(());
        }
        let camera = self.arena.camera(w, h, t_s).map_err(scenery)?;
        if let Some(list) = self.static_list.as_mut() {
            list.camera = camera;
        }
        Ok(())
    }

    /// WEB15: the page plays the match it draws. One step per fixed tick the clock took, the
    /// human seat's input for each, then the match's own bodies over the scenery.
    fn step_fight(
        &mut self,
        fight: &mut crate::fight::Match,
        t_s: f32,
    ) -> Result<DrawList, MatchError> {
        let (w, h) = self.presenter.size();
        // A slow frame asks the clock for every tick it missed; with a heavy tick that is a spiral
        // (missed ticks make the next frame slower). The page plays at most two steps per frame:
        // below 30 fps the match runs in slow motion instead of dragging the frame rate down.
        // PW37: a `?gate` page runs a fixed number of ticks once (the fighters stand in the
        // arena) and then no step at all, so the capture is the same frame on every load and
        // the frame gate can compare it with the reference capture.
        let steps = if self.gate {
            let left = self.gate_ticks;
            self.gate_ticks = 0;
            left
        } else {
            self.clock.steps().min(FIGHT_STEPS_PER_FRAME)
        };
        let frames = self.counters.frames;
        let (yaw, left, right) = crate::browser::seat_look();
        for _ in 0..steps {
            let tick = fight.next_tick();
            let facing = fight.facing_for(yaw, left, right);
            match crate::browser::seat_input(tick, facing) {
                Some(input) => fight.step(input),
                None => break,
            }
        }
        if steps > 0 {
            let cues = fight.cues();
            if cues != self.named_cues.as_slice() {
                self.named_cues = cues.to_vec();
                crate::browser::queue_fight_cues(cues);
                if let Some(word) = crate::arena_view::cue_word(cues) {
                    self.say_cue(word);
                }
            }
        }
        if frames % 15 == 1 {
            set_text("fight", &fight.line());
        }
        if let Some(hud) = fight.hud_change() {
            paint_hud(hud);
        }
        fight.list(&mut self.presenter, w, h, t_s)
    }

    /// PW37: say the cue the tick gave: the word in the console, in the page's body attribute
    /// and in the receipt, so a headless read sees which cue the fight produced (a parry, a
    /// block, a hit). The word stays until the next cue.
    fn say_cue(&mut self, word: &'static str) {
        self.cue = Some(word);
        set_body_attr("data-cue", &word.to_ascii_lowercase());
        web_sys::console::log_1(&JsValue::from_str(&format!("cue: {word}")));
    }

    /// PW37: the scene the last frame drew: the cooked kit's own name once any of it draws
    /// resident, the stand-in ring's name otherwise.
    fn scene(&self) -> &'static str {
        let kit_drawn = self
            .fight
            .as_ref()
            .map_or(0, crate::fight::Match::kit_drawn);
        crate::arena_view::scene_word(kit_drawn, self.fight.is_some())
    }

    /// Feed the frame to the one-second window (unless it spans a hidden gap); once a second print
    /// the split and let the ladder and the drift check decide, and report any pipeline built
    /// after the loader (`late_compile`).
    fn tick_split(&mut self, now: f64, resumed: bool) {
        let wall = last_live_wall_split();
        let render_ms = us_to_ms(wall.stage_us.saturating_add(wall.submit_us));
        let present_ms = us_to_ms(wall.present_call_us);
        if !resumed {
            self.pace.frame(
                now,
                self.stats.last_ms(),
                self.sim_ms,
                render_ms,
                present_ms,
            );
        }
        // A cut resizes the canvas; do not change the size under a capture that is on its way.
        // A hidden tab is not a slow picture either.
        let capture_pending = self.capture_asked && self.presenter.capture_busy();
        let hidden = document().is_some_and(|d| d.hidden());
        let Some(tick) = self.pace.tick(now, !(capture_pending || hidden)) else {
            return;
        };
        let line = tick
            .split
            .line(self.tier.name(), self.scale_pct, device_pixel_ratio());
        set_text("split", &line);
        web_sys::console::log_1(&JsValue::from_str(&line));
        let ledger = pipeline_warmup::snapshot();
        if let Some(late) = pipeline_warmup::late_line(&ledger) {
            if ledger.late().len() != self.late_reported {
                self.late_reported = ledger.late().len();
                self.counters.late_pipelines =
                    u64::try_from(self.late_reported).unwrap_or(u64::MAX);
                set_text(
                    "pipelines",
                    &format!("{} | {late}", pipeline_warmup::ready_line(&ledger, 0.0)),
                );
                set_body_attr("data-late-pipelines", &self.late_reported.to_string());
                log_error(&late);
            }
        }
        if let Some((cut, undo)) = tick.step {
            self.apply_cut(cut, undo);
        }
    }

    /// Take one cut of the ladder, or give it back (`undo`). Only the knobs the page really owns
    /// change the picture (render scale, bloom); the others are printed with `applied=no` so the
    /// order is visible.
    fn apply_cut(&mut self, cut: Cut, undo: bool) {
        let name = WEB_CUT_ORDER.get(cut.step).copied().unwrap_or("unknown");
        let applied = match name {
            "post_and_particles" => {
                self.presenter.set_bloom_enabled(undo);
                true
            }
            "render_scale" => {
                self.scale_pct = if undo {
                    scale_after_restore(self.start_scale_pct, cut.repeat, SCALE_FLOOR_PCT)
                } else {
                    scale_after_cut(self.start_scale_pct, cut.repeat, SCALE_FLOOR_PCT)
                };
                true
            }
            _ => false,
        };
        let line = if undo {
            restore_line(
                self.tier.name(),
                name,
                cut.repeat,
                applied,
                self.scale_pct,
                self.pace.restores(),
            )
        } else {
            cut_line(self.tier.name(), name, cut.repeat, applied, self.scale_pct)
        };
        set_text("tier", &line);
        web_sys::console::log_1(&JsValue::from_str(&line));
    }

    fn publish_receipt(&self) {
        let (waiting, dropped) = self.presenter.deferred_validation_waiting();
        let last_loss = LOSSES.with(|losses| losses.borrow().last().cloned());
        // QW10: the measured frames, as JSON; absent until the window has enough samples.
        let perf = self.perf.json();
        let receipt = crate::arena_view::Receipt {
            tier: self.tier.name(),
            counters: self.counters,
            frame_ms: self.stats.last_ms(),
            cpu_ms: self.last_cpu_ms,
            draws: self.last_draws,
            first_error: self.first_error.as_deref(),
            last_loss: last_loss.as_ref(),
            scene: self.scene(),
            cue: self.cue,
            gate: self.gate_result,
            perf: perf.as_deref(),
        };
        let line = crate::arena_view::receipt_line(&receipt);
        set_text("receipt", &line);
        set_body_attr(
            "data-validation-errors",
            &self.counters.validation_total().to_string(),
        );
        set_body_attr("data-scopes-waiting", &format!("{waiting}/{dropped}"));
        set_body_attr("data-scene", receipt.scene);
    }

    /// Ask for the GPU copy of the canvas once the first frames have settled (or when the page
    /// asks), and publish it once its map has finished. An ask outlives a busy presenter: a
    /// finished copy is read first, and a request that cannot arm yet stays set for the next
    /// frame, so one ask answers with one line. Dropping it would leave a headless driver
    /// waiting for a line the page never asked the GPU for.
    fn drive_capture(&mut self) {
        self.read_capture();
        let wanted = CAPTURE_WANTED.with(Cell::get)
            || (!self.capture_asked && self.counters.frames == CAPTURE_AFTER_FRAMES)
            // QW10: a `?bundle=ab` page captures one frame per arm, inside the first rebuild
            // block and the first replay block, so both hashes come from the same run.
            || (self.perf.mode() == perf_window::Mode::Ab
                && matches!(self.counters.frames, PERF_CAPTURE_OFF | PERF_CAPTURE_ON));
        if wanted {
            self.capture_asked = true;
            match self.presenter.request_capture() {
                Ok(true) => {
                    CAPTURE_WANTED.with(|c| c.set(false));
                    self.capture_arm = self.perf.arm_at(self.counters.frames);
                }
                // A copy is still waiting for its frame or its map: keep the ask for the next frame.
                Ok(false) => {}
                Err(e) => {
                    CAPTURE_WANTED.with(|c| c.set(false));
                    self.note_error(&format!("capture: {e}"));
                }
            }
        }
        publish_capture_state(self.presenter.capture_busy(), self.presenter.suspended());
    }

    /// Publish the copy whose map has finished, if one arrived. The numbers are the render
    /// crate's own; the pixels go to the page as `window.arenaCapture`.
    fn read_capture(&mut self) {
        match self.presenter.take_capture() {
            Ok(Some(frame)) => {
                let stats = frame.stats();
                let facts = CaptureFacts {
                    width: frame.width,
                    height: frame.height,
                    hash: frame.hash(),
                    distinct: u64::try_from(stats.distinct).unwrap_or(0),
                    dominant_share: stats.dominant_share,
                    mean_luma: stats.mean_luma,
                    blank: stats.is_blank(),
                };
                // QW10: the arm that asked for this capture keeps its hash, so the receipt can
                // say whether the two ways of building the list drew the same picture.
                if let Some(arm) = self.capture_arm.take() {
                    self.perf.note_capture(arm, &format!("{:016x}", facts.hash));
                    web_sys::console::log_1(&JsValue::from_str(&format!(
                        "qw10 capture {} {:016x}",
                        arm.word(),
                        facts.hash
                    )));
                }
                let line = facts.line(self.counters.frames, &self.surface_format);
                publish_capture(&frame.rgba, &line);
                set_text("capture", &line);
                set_body_attr("data-capture", if facts.blank { "blank" } else { "ready" });
                // PW37: the frame gate measures this capture against the reference capture of
                // the cooked arena and prints the SSIM it got.
                let grid = crate::arena_view::luma_grid(&frame.rgba, frame.width, frame.height);
                let gate = crate::arena_view::frame_gate(&grid);
                let gate_line = gate.line(self.scene());
                web_sys::console::log_1(&JsValue::from_str(&gate_line));
                set_body_attr("data-gate", &format!("{:.4}", gate.ssim));
                self.gate_result = Some(gate);
            }
            Ok(None) => {}
            Err(e) => {
                self.note_error(&format!("capture read: {e}"));
                set_body_attr("data-capture", "error");
            }
        }
    }
}

/// Paint the played match's HUD: two health bars, two stamina bars, the poise pips, the round
/// pips, the round number, the one-word call and the K.O./WINNER banner. The caller only reaches
/// here when the snapshot changed, so a quiet tick costs one comparison and no DOM work.
fn paint_hud(hud: crate::fight::MatchHud) {
    let percent = |hp: u16| u32::from(hp.min(1000)) / 10;
    set_style("duel-you-bar", &format!("width:{}%", percent(hud.hp[0])));
    set_style("duel-rival-bar", &format!("width:{}%", percent(hud.hp[1])));
    // WEB16: the stamina bar and the poise pips read the sim's own numbers, so the page shows the
    // v2 exchange's other two gauges, not only health.
    set_style(
        "duel-you-stam",
        &format!("width:{}%", hud.stamina_percent(0)),
    );
    set_style(
        "duel-rival-stam",
        &format!("width:{}%", hud.stamina_percent(1)),
    );
    set_text("duel-you-pips", &hud.pips(0));
    set_text("duel-rival-pips", &hud.pips(1));
    set_text("duel-you-poise", &hud.poise_pips(0));
    set_text("duel-rival-poise", &hud.poise_pips(1));
    set_body_attr(
        "data-poise",
        if hud.poise_broken(0) && hud.poise_broken(1) {
            "both"
        } else if hud.poise_broken(0) {
            "you"
        } else if hud.poise_broken(1) {
            "rival"
        } else {
            "full"
        },
    );
    set_text(
        "duel-round",
        &if hud.winner.is_some() {
            "match".to_string()
        } else {
            format!("round {}", hud.round_number())
        },
    );
    set_text("duel-call", hud.call.unwrap_or(""));
    set_body_attr("data-call", hud.call.unwrap_or(""));
    let (banner, state) = match hud.banner {
        Some(banner) => (
            banner.text(),
            if hud.winner.is_some() { "win" } else { "ko" },
        ),
        None => ("", "off"),
    };
    set_text("duel-banner", banner);
    set_body_attr("data-duel", "on");
    set_body_attr("data-banner", state);
}

/// Set one element's `style` attribute (`width:42%`); missing elements are ignored.
fn set_style(id: &str, style: &str) {
    if let Some(el) = document().and_then(|d| d.get_element_by_id(id)) {
        let _ = el.set_attribute("style", style);
    }
}

/// Publish what the capture handshake waits on for the headless driver's failure line: `idle`,
/// `busy` (a copy is armed or in flight) or `suspended` (a zero-area frame never presents, so no
/// copy can start). The page itself never reads it.
fn publish_capture_state(busy: bool, suspended: bool) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let state = if suspended {
        "suspended"
    } else if busy {
        "busy"
    } else {
        "idle"
    };
    let _ = js_sys::Reflect::set(
        &window,
        &"arenaCaptureState".into(),
        &JsValue::from_str(state),
    );
}

/// Hand the captured pixels to the page: `window.arenaCapture = { line, rgba }` where `rgba` is
/// the tight RGBA bytes. The headless check reads it and writes the PNG.
fn publish_capture(rgba: &[u8], line: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let object = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&object, &"line".into(), &JsValue::from_str(line));
    let _ = js_sys::Reflect::set(&object, &"rgba".into(), &js_sys::Uint8Array::from(rgba));
    let _ = js_sys::Reflect::set(&window, &"arenaCapture".into(), &object);
}

/// Run `frame` on every animation frame until its device is lost. A lost device ends the loop
/// and starts [`recover_with`] on a fresh instance and adapter; a requested destroy or too many
/// losses end it for good (see [`check_device`]).
fn run_loop(mut frame: ArenaLoop, canvas_id: String, generation: u32) {
    let slot: Rc<FrameSlot> = Rc::new(RefCell::new(None));
    let again = Rc::clone(&slot);
    let mut ended_by: Option<Cause> = None;
    *slot.borrow_mut() = Some(Closure::new(move |now: f64| {
        match check_device(frame.presenter.device(), &frame.lost, &mut ended_by, now) {
            Check::Draw => frame.draw(now),
            Check::Wait => {}
            Check::Recover => {
                // Dropping the closure ends this loop; wasm-bindgen frees it after this call returns.
                drop(again.borrow_mut().take());
                wasm_bindgen_futures::spawn_local(recover_with(
                    canvas_id.clone(),
                    generation + 1,
                    |id, g| async move { Box::pin(start(&id, g)).await },
                ));
                return;
            }
            Check::Over => {
                drop(again.borrow_mut().take());
                return;
            }
        }
        request_frame(&again);
    }));
    request_frame(&slot);
}

fn present_failure(e: &PresentError) -> (Failure, String) {
    match e {
        PresentError::NoAdapter => (Failure::NoAdapter, e.to_string()),
        PresentError::NoSurface | PresentError::InvalidSize { .. } => {
            (Failure::NoSurface, e.to_string())
        }
        _ => (Failure::NoDevice, e.to_string()),
    }
}

/// What the browser handed over before the presenter exists: the canvas, a fresh instance, the
/// surface, the adapter that can present to it and the device with its error and loss hooks.
struct Gpu {
    raw: JsValue,
    canvas: HtmlCanvasElement,
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
    lost: Arc<LostBox>,
    uncaptured: Arc<Uncaptured>,
}

/// Ask the browser for everything the presenter needs, with `await`s and no blocking call.
async fn open_gpu(canvas_id: &str) -> Result<Gpu, (Failure, String)> {
    let raw = navigator_gpu().ok_or_else(|| {
        (
            Failure::NoWebGpu,
            "navigator.gpu is undefined: an older browser, or a page that is neither https nor localhost."
                .to_string(),
        )
    })?;
    let canvas = document()
        .and_then(|d| d.get_element_by_id(canvas_id))
        .and_then(|e| e.dyn_into::<HtmlCanvasElement>().ok())
        .ok_or_else(|| {
            (
                Failure::NoCanvas,
                format!("expected <canvas id=\"{canvas_id}\">"),
            )
        })?;

    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::BROWSER_WEBGPU;
    let instance = wgpu::Instance::new(desc);
    // The surface first, then the adapter that can present to it.
    let surface = instance
        .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
        .map_err(|e| (Failure::NoSurface, e.to_string()))?;
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: Some(&surface),
            apply_limit_buckets: false,
        })
        .await
        .map_err(|e| (Failure::NoAdapter, e.to_string()))?;
    let (device, queue) = adapter
        .request_device(&Presenter::device_descriptor(&adapter))
        .await
        .map_err(|e| (Failure::NoDevice, e.to_string()))?;

    let uncaptured = Arc::new(Uncaptured::default());
    let sink = Arc::clone(&uncaptured);
    device.on_uncaptured_error(Arc::new(move |e: wgpu::Error| {
        let text = format!("{e}");
        log_error(&format!("wgpu: {text}"));
        sink.count.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut first) = sink.first.lock() {
            first.get_or_insert(text);
        }
    }));
    let lost = Arc::new(LostBox::default());
    let lost_by_callback = Arc::clone(&lost);
    device.set_device_lost_callback(move |reason, message| {
        log_error(&format!("wgpu device lost ({reason:?}): {message}"));
        lost_by_callback.set(reason_word(reason), &message);
    });

    Ok(Gpu {
        raw,
        canvas,
        instance,
        surface,
        adapter,
        device,
        queue,
        lost,
        uncaptured,
    })
}

/// Everything up to the first frame. `Err` is a failure the page can show, not a crash.
async fn start(canvas_id: &str, generation: u32) -> Result<String, (Failure, String)> {
    let Gpu {
        raw: gpu,
        canvas,
        instance,
        surface,
        adapter,
        device,
        queue,
        lost,
        uncaptured,
    } = open_gpu(canvas_id).await?;
    let max_texture_2d = device.limits().max_texture_dimension_2d;
    let max_dim = max_texture_2d.min(MAX_FRAME_DIM);
    let features = feature_names(device.features());
    let limits = limit_rows(&device.limits());
    let raw = raw_adapter_info(&gpu).await;
    let hint = adapter_hint(&raw);
    let TierPick {
        search,
        tier,
        why: tier_why,
        scale_pct,
        size: (w, h),
    } = pick_tier(&canvas, &device, max_dim, &hint);

    let info = adapter.get_info();
    let software = is_software(&info, &raw[1], &raw[3]);

    // Building the presenter bakes the sky and the image-based light on this thread: say so
    // first and let the browser paint the message.
    set_text("hud", "loading the arena: baking sky and light...");
    pause(30).await;
    let built_at = now_ms();
    // WEB13: a new presenter, a new ledger. Every pipeline it builds from here on is counted.
    pipeline_warmup::reset();
    let work_queue = queue.clone();
    let mut presenter =
        Presenter::init_from_device(&instance, &adapter, device, queue, surface, w, h, false)
            .map_err(|e| present_failure(&e))?;
    presenter.apply_quality(&quality::knobs(tier));
    let arena = ProceduralArena::build().map_err(|e| (Failure::NoDevice, e.to_string()))?;
    let kit_source = fetch_kit(&search).await;
    let fighter_source = fetch_fighters(&search).await;
    // WEB15: `?fight` plays a real match over the same scenery; nothing else changes.
    let fight = open_fight(
        &search,
        kit_source.as_ref(),
        fighter_source.as_ref(),
        &mut presenter,
    )?;
    let mut frame = arena_loop(LoopParts {
        presenter,
        arena,
        fight,
        canvas,
        lost,
        uncaptured,
        max_dim,
        search: search.clone(),
        hint,
        tier,
        tier_why,
        scale_pct,
        software,
    });
    frame.stage()?;
    let surface_format = frame.surface_format.clone();
    let ready = ready_line(
        &info,
        raw,
        features,
        limits,
        generation,
        (w, h),
        surface_format,
    );
    let line = Box::pin(publish_and_run(
        frame,
        ready,
        StartupFacts::new(&search, tier_why, software, max_texture_2d),
        &work_queue,
        built_at,
        canvas_id,
    ))
    .await;
    Ok(line)
}

/// The parts the frame loop needs from the device and the page, gathered so the loop's
/// construction is one call and [`start`] stays one screen of code.
struct LoopParts {
    presenter: Presenter,
    arena: ProceduralArena,
    fight: Option<crate::fight::Match>,
    canvas: HtmlCanvasElement,
    lost: Arc<LostBox>,
    uncaptured: Arc<Uncaptured>,
    max_dim: u32,
    search: String,
    hint: String,
    tier: Quality,
    tier_why: &'static str,
    scale_pct: u32,
    software: bool,
}

/// The frame loop over the device and the page: the presenter, the scenery, the match and the
/// counters the receipt reads. PW37: a `?gate` page holds one known picture, so it runs a fixed
/// number of match ticks once and then freezes the animation time.
fn arena_loop(parts: LoopParts) -> ArenaLoop {
    let LoopParts {
        presenter,
        arena,
        fight,
        canvas,
        lost,
        uncaptured,
        max_dim,
        search,
        hint,
        tier,
        tier_why,
        scale_pct,
        software,
    } = parts;
    let gate = has_flag(&search, "gate");
    // QW10: `?bundle=off|on|ab`; a page without the flag rebuilds the list as it always did.
    let bundle = perf_window::Mode::parse(flag_value(&search, "bundle"));
    ArenaLoop {
        presenter,
        arena,
        fight,
        canvas,
        lost,
        uncaptured,
        max_dim,
        stats: FrameStats::default(),
        clock: StepClock::new(MAX_STEPS_PER_FRAME),
        frozen_s: (has_flag(&search, "freeze") || gate).then_some(FROZEN_T_S),
        seen_visibility: VISIBILITY_CHANGES.with(Cell::get),
        adapter_hint: hint,
        counters: Counters {
            losses: LOSSES.with(|losses| losses.borrow().total()),
            ..Counters::default()
        },
        first_error: None,
        surface_format: String::new(),
        last_cpu_ms: 0.0,
        last_draws: 0,
        capture_asked: false,
        tier,
        tier_why,
        scale_pct,
        start_scale_pct: scale_pct,
        pace: Pace::new(ladder_for(&search, software)),
        sim_ms: 0.0,
        late_reported: 0,
        gate,
        gate_ticks: u64::from(gate) * GATE_TICKS,
        cue: None,
        named_cues: Vec::new(),
        gate_result: None,
        perf: perf_window::Run::new(bundle),
        static_list: None,
        last_list_ms: 0.0,
        capture_arm: None,
    }
}

/// The ready line's facts: the adapter's own words, the device's features and limits, the
/// canvas and the surface format, with the page's own facts filled in.
fn ready_line(
    info: &wgpu::AdapterInfo,
    raw: [String; 4],
    features: Vec<String>,
    limits: Vec<(&'static str, u64)>,
    generation: u32,
    canvas: (u32, u32),
    surface_format: String,
) -> Ready {
    Ready {
        adapter: adapter_facts(info, raw),
        features,
        limits,
        generation,
        canvas,
        surface_format,
        ..page_facts()
    }
}

/// Warm behind the loader, publish the ready line and hand the frame to the loop. Split out of
/// [`start`] so the device and world setup stays one screen of code.
async fn publish_and_run(
    mut frame: ArenaLoop,
    ready: Ready,
    facts: StartupFacts<'_>,
    work_queue: &wgpu::Queue,
    built_at: f64,
    canvas_id: &str,
) -> String {
    // A query or a software adapter is not probed: the query is the owner's word and no tier
    // helps SwiftShader.
    let probe = (facts.tier_why == "default" && !facts.software).then_some(Probe {
        query: quality::tier_from_query(facts.search).map(str::to_string),
        max_texture_2d: facts.max_texture_2d,
    });
    let warmed = Box::pin(warm_behind_loader(&mut frame, work_queue, built_at, probe)).await;
    let init_ms = now_ms() - built_at;
    let line = publish_ready(&frame, &ready, &warmed, init_ms);
    set_body_attr("data-generation", &ready.generation.to_string());
    set_state("running");
    web_sys::console::log_1(&JsValue::from_str(&line));
    run_loop(frame, canvas_id.to_string(), ready.generation);
    line
}

/// The startup facts the publish step needs that are not already in [`Ready`].
struct StartupFacts<'a> {
    /// The page's query string.
    search: &'a str,
    /// Why this tier was picked.
    tier_why: &'a str,
    /// True on a software adapter (no probe, no cuts).
    software: bool,
    /// The device's largest 2D texture, for the probe.
    max_texture_2d: u32,
}

impl<'a> StartupFacts<'a> {
    /// Gather the facts [`start`] already holds.
    const fn new(search: &'a str, tier_why: &'a str, software: bool, max_texture_2d: u32) -> Self {
        Self {
            search,
            tier_why,
            software,
            max_texture_2d,
        }
    }
}

/// Fetch the cooked arena kit for `?fight&kit`: `Some` when the bundle downloaded and opened,
/// `None` for every other query or a refused fetch (the HUD says why and the procedural
/// stand-in stays). The fetch is awaited once, before the first frame, so the first picture is
/// the real arena. The kit is opt-in until the body draw reads near the ring centre and the
/// kit's own spawn pair frames it (see `docs/BLOCKERS.md`, `2026-10-20T00:35Z`).
async fn fetch_kit(search: &str) -> Option<engine_assets::asset_source::BundleSource> {
    // WEB17: the cooked kit is the arena by default; `?nokit` keeps the procedural stand-in.
    if !has_flag(search, "fight") || has_flag(search, "nokit") {
        return None;
    }
    set_text("hud", "loading the arena kit...");
    // QW4: the page answers at the tier-0 boundary, so the first frame starts on
    // the coarse level while the HDRI tier streams behind the progress bar.
    match crate::content::fetch_bundle_prefix(crate::content::ARENA_BUNDLE_URL).await {
        Ok(source) => {
            // WEB6: the receipt says the asset really arrived and how big it was.
            let entries = source.entries_in_file_order().count();
            web_sys::console::log_1(&JsValue::from_str(&format!(
                "content: arena kit prefix bytes={} entries={entries}",
                source.byte_len()
            )));
            Some(source)
        }
        Err(err) => {
            set_text("hud", &format!("arena kit: {err} (stand-in ring)"));
            // The refusal is loud and durable: the frame line overwrites the HUD.
            web_sys::console::log_1(&JsValue::from_str(&format!("content: refused: {err}")));
            None
        }
    }
}

/// Fetch the cooked fighter rig for `?fight`: `Some` when the bundle downloaded and opened,
/// `None` for every other query or a refused fetch (the HUD says why and the stand-in bodies
/// stay). The fetch is awaited once, before the first frame, like the arena kit.
async fn fetch_fighters(search: &str) -> Option<engine_assets::asset_source::BundleSource> {
    if !has_flag(search, "fight") || has_flag(search, "nofighters") {
        return None;
    }
    set_text("hud", "loading the fighters...");
    match crate::content::fetch_bundle(crate::content::RIG_BUNDLE_URL).await {
        Ok(source) => {
            let entries = source.entries_in_file_order().count();
            web_sys::console::log_1(&JsValue::from_str(&format!(
                "content: fighters bytes={} entries={entries}",
                source.byte_len()
            )));
            Some(source)
        }
        Err(err) => {
            set_text("hud", &format!("fighters: {err} (stand-in bodies)"));
            web_sys::console::log_1(&JsValue::from_str(&format!(
                "content: fighters refused: {err}"
            )));
            None
        }
    }
}

/// Load, texture and stage the cooked rig once, on the cold path. A refusal is named and
/// leaves the caller on the stand-in bodies; it never blocks the match.
fn stage_fighters(
    source: &dyn engine_assets::AssetSource,
    presenter: &mut Presenter,
) -> Result<BodyLook, engine_game::fighter_look::LookError> {
    let look = BodyLook::load_cooked_from(source)?;
    look.stage_textures(presenter)?;
    look.stage_meshes(presenter)?;
    Ok(look)
}

/// Build the played match when the query asks for it (`?fight`); `None` keeps the stand-in.
fn open_fight(
    search: &str,
    kit: Option<&engine_assets::asset_source::BundleSource>,
    fighters: Option<&engine_assets::asset_source::BundleSource>,
    presenter: &mut Presenter,
) -> Result<Option<crate::fight::Match>, (Failure, String)> {
    if !has_flag(search, "fight") {
        return Ok(None);
    }
    let look = fighters.and_then(|source| match stage_fighters(source, presenter) {
        Ok(look) => {
            web_sys::console::log_1(&JsValue::from_str("fighters: rig staged"));
            Some(look)
        }
        Err(err) => {
            set_text("hud", &format!("fighters: {err} (stand-in bodies)"));
            web_sys::console::log_1(&JsValue::from_str(&format!("fighters: refused: {err}")));
            None
        }
    });
    crate::fight::Match::new(
        kit.map(|source| source as &dyn engine_assets::AssetSource),
        look,
        presenter,
    )
    .map(Some)
    .map_err(|e| (Failure::NoDevice, e.to_string()))
}

/// The ladder of cuts, unless `?ladder=off` or a software adapter that no cut would help.
fn ladder_for(search: &str, software: bool) -> Option<Ladder> {
    (!search.contains("ladder=off") && !software)
        .then(|| Ladder::new(WEB_CUT_ORDER.len(), 2, BUDGET_MS))
}

/// The parts of the ready line that come from the page and the build, not from the device.
fn page_facts() -> Ready {
    Ready {
        commit: COMMIT.to_string(),
        wasm_pages: wasm_pages(),
        user_agent: user_agent(),
        cross_origin_isolated: cross_origin_isolated(),
        ..Ready::default()
    }
}

/// The tier and the canvas size it implies.
struct TierPick {
    /// The page's query string (`?tier=web`).
    search: String,
    tier: Quality,
    /// `query`, `limits`, `denylist` or `default`.
    why: &'static str,
    scale_pct: u32,
    size: (u32, u32),
}

/// WEB8: the tier comes from the adapter's limits and a `?tier=` in the address, with the denylist
/// (QT10) matched against the `adapter=` text; the canvas backing store is set to the tier's render
/// scale of its CSS box.
fn pick_tier(
    canvas: &HtmlCanvasElement,
    device: &wgpu::Device,
    max_dim: u32,
    adapter: &str,
) -> TierPick {
    let search = page_search();
    let (tier, why) = quality::apply_denylist(
        quality::choose_web_tier(
            quality::tier_from_query(&search),
            device.limits().max_texture_dimension_2d,
        ),
        adapter,
        quality::WEB_DENYLIST,
    );
    let scale_pct = u32::from(quality::knobs(tier).render_scale_pct);
    let size = scaled_frame_size(
        f64::from(canvas.client_width()),
        f64::from(canvas.client_height()),
        device_pixel_ratio(),
        scale_pct,
        max_dim,
    );
    canvas.set_width(size.0);
    canvas.set_height(size.1);
    TierPick {
        search,
        tier,
        why,
        scale_pct,
        size,
    }
}

/// The page's device pixel ratio (1 when it cannot be read).
fn device_pixel_ratio() -> f64 {
    web_sys::window().map_or(1.0, |w| w.device_pixel_ratio())
}

/// A software adapter (SwiftShader) is slow by nature; no cut of the ladder would help it.
fn is_software(info: &wgpu::AdapterInfo, architecture: &str, description: &str) -> bool {
    info.device_type == wgpu::DeviceType::Cpu
        || [architecture, description, info.name.as_str()]
            .iter()
            .any(|text| text.to_ascii_lowercase().contains("swiftshader"))
}

/// What the ready line says about the adapter: wgpu's view plus the browser's raw strings
/// (`[vendor, architecture, device, description]`).
fn adapter_facts(info: &wgpu::AdapterInfo, raw: [String; 4]) -> AdapterFacts {
    let [web_vendor, web_architecture, web_device, web_description] = raw;
    AdapterFacts {
        name: info.name.clone(),
        backend: format!("{:?}", info.backend),
        device_type: format!("{:?}", info.device_type),
        web_vendor,
        web_architecture,
        web_device,
        web_description,
    }
}

/// WEB13: behind the loading screen, build what the presenter did not at creation (the HP-bar
/// overlay), draw a few real frames so every pipeline has been used once and wait for the GPU
/// to finish them: the browser compiles on its own threads meanwhile. wgpu 30 has no async
/// pipeline call, so this is the closest non-blocking route. The ledger is sealed here: a build
/// after it is a `late_compile`. The frames are real, so `running` still means a frame reached
/// the canvas. Returns the builds and the milliseconds since `built_at`.
async fn warm_behind_loader(
    frame: &mut ArenaLoop,
    queue: &wgpu::Queue,
    built_at: f64,
    probe: Option<Probe>,
) -> Warmed {
    frame.presenter.warm_pipelines();
    for _ in 0..WARM_FRAMES {
        frame.draw(now_ms());
        pause(0).await;
    }
    wait_for_gpu(queue, 10, 30).await;
    let mut why = frame.tier_why;
    let mut probe_ms = None;
    let mut late_compile = 0;
    if let Some(probe) = probe {
        let done = run_probe(frame, queue).await;
        late_compile = done.late_compile;
        probe_ms = quality::probe_median_ms(&done.samples);
        let (tier, picked) =
            quality::choose_web_tier_probed(probe.query.as_deref(), probe.max_texture_2d, probe_ms);
        why = picked;
        if tier != frame.tier {
            frame.set_tier(tier);
            // One frame at the new size, so the ledger holds whatever it built.
            frame.draw(now_ms());
            wait_for_gpu(queue, 10, 30).await;
        }
    }
    let ledger = pipeline_warmup::snapshot();
    pipeline_warmup::seal();
    // The scope answers of the warm frames are in by now; count them before the ready line.
    frame.counters.deferred_validation_errors = frame.presenter.deferred_validation_errors();
    frame.counters.uncaptured_errors = frame.uncaptured.count.load(Ordering::Relaxed);
    Warmed {
        ledger,
        warm_ms: now_ms() - built_at,
        probe_ms,
        late_compile,
        why,
    }
}

/// What the startup probe needs to know about the pick it may lower.
struct Probe {
    /// The `?tier=` value, if any.
    query: Option<String>,
    max_texture_2d: u32,
}

/// What the loader's warm-up learned.
struct Warmed {
    ledger: pipeline_warmup::Ledger,
    warm_ms: f64,
    /// Median of the startup probe, `None` when it did not run or stalled.
    probe_ms: Option<f64>,
    /// Pipelines built while the probe timed frames (QT2).
    late_compile: u32,
    /// Why the final tier: the pick's reason, or `probe` when the probe lowered it.
    why: &'static str,
}

/// WEB8 startup probe: draw frames in a row, each one timed from its draw to the GPU saying it is
/// done. The frames are not tied to the display, so a card that keeps up with 60 Hz does not read
/// 16.7 ms: the figure is the work itself. [`ProbeRun`] settles for 30 frames first, runs a
/// window a late compile spoiled once more, and starts over when the tab was hidden or shown
/// (QT2, QT7); the samples are empty when too few frames fit in the time (a stalled or throttled
/// tab says nothing about the GPU).
async fn run_probe(frame: &mut ArenaLoop, queue: &wgpu::Queue) -> ProbeDone {
    let mut run = ProbeRun::new(
        quality::PROBE_FRAMES,
        PROBE_MIN_FRAMES,
        quality::PROBE_MAX_MS,
    );
    let mut hidden_waits = 0;
    loop {
        if document().is_some_and(|d| d.hidden()) {
            // A hidden tab runs its timers slowly: nothing it times is the GPU.
            hidden_waits += 1;
            if hidden_waits > PROBE_HIDDEN_WAITS {
                return run.abandon();
            }
            run.restart();
            pause(250).await;
            continue;
        }
        let changes = VISIBILITY_CHANGES.with(Cell::get);
        let t0 = now_ms();
        frame.draw(t0);
        wait_for_gpu(queue, 0, 0).await;
        let ms = now_ms() - t0;
        if VISIBILITY_CHANGES.with(Cell::get) != changes {
            // The frame spans a gap.
            run.restart();
            continue;
        }
        if let Some(done) = run.frame(ms, pipeline_warmup::snapshot().total()) {
            return done;
        }
    }
}

/// The tier's knobs as the page prints them (see [`TierFacts`]).
fn tier_facts(tier: Quality, why: &str, adapter: &str, probe: (Option<f64>, u32)) -> TierFacts {
    let knobs = quality::knobs(tier);
    TierFacts {
        name: tier.name().to_string(),
        why: why.to_string(),
        adapter: adapter.to_string(),
        late_compile: probe.1,
        probe_ms: probe.0,
        dpr: device_pixel_ratio(),
        scale_pct: u32::from(knobs.render_scale_pct),
        shadow_px: u32::from(knobs.shadow_map_px),
        cascades: u32::from(knobs.shadow_cascades),
        gtao: match knobs.gtao {
            quality::Gtao::Off => "off",
            quality::Gtao::Half => "half",
            quality::Gtao::Full => "full",
        }
        .to_string(),
        taa: knobs.taa,
        particles: knobs.particle_cap,
        decals: knobs.decal_cap,
        physfx_bodies: u32::from(knobs.physfx_bodies),
        physfx_ragdolls: u32::from(knobs.physfx_ragdolls),
        physfx_contacts: u32::from(knobs.physfx_contacts),
        texture_px: u32::from(knobs.texture_max_px),
        wired: WIRED.to_string(),
    }
}

/// Print the tier, the pipeline warm-up and the ready line to the page and the console; returns
/// the ready line (JSON with the tier, the pipelines and the validation count).
fn publish_ready(frame: &ArenaLoop, ready: &Ready, warmed: &Warmed, init_ms: f64) -> String {
    let Warmed {
        ledger,
        warm_ms,
        probe_ms,
        late_compile,
        why: tier_why,
    } = warmed;
    let (tier, warm_ms) = (frame.tier, *warm_ms);
    let tier_line = tier_facts(
        tier,
        tier_why,
        &frame.adapter_hint,
        (*probe_ms, *late_compile),
    )
    .line();
    let pipelines_line = pipeline_warmup::ready_line(ledger, warm_ms);
    let facts = ArenaFacts {
        tier: tier.name().to_string(),
        tier_why: tier_why.to_string(),
        pipelines: u64::try_from(ledger.warm()).unwrap_or(u64::MAX),
        pipelines_ms: whole_ms(warm_ms),
        late_pipelines: 0,
        validation: frame.counters.validation_total(),
        scene: frame.scene().to_string(),
    };
    let line = ready.line_with(&facts);
    set_text("report", &line);
    set_text("tier", &tier_line);
    set_text("pipelines", &pipelines_line);
    set_body_attr("data-tier", tier.name());
    set_body_attr("data-fallback", fallback_for(tier_why));
    set_body_attr("data-pipelines", &ledger.warm().to_string());
    set_body_attr("data-scene", &facts.scene);
    web_sys::console::log_1(&JsValue::from_str(&tier_line));
    web_sys::console::log_1(&JsValue::from_str(&pipelines_line));
    set_text("init", &format!("arena ready in {init_ms:.0} ms"));
    line
}

/// The page's query string (`?tier=web`), empty when it cannot be read.
fn page_search() -> String {
    web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w, &"location".into()).ok())
        .and_then(|loc| js_sys::Reflect::get(&loc, &"search".into()).ok())
        .and_then(|s| s.as_string())
        .unwrap_or_default()
}

/// Wait until the GPU has finished everything submitted so far (or [`WARM_WAIT_MS`] passed),
/// polling every `poll_ms` and then waiting `settle_ms` for the error-scope answers. The callback
/// and a short timer poll: no blocking call.
async fn wait_for_gpu(queue: &wgpu::Queue, poll_ms: i32, settle_ms: i32) {
    let done = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&done);
    queue.on_submitted_work_done(move || flag.store(true, Ordering::SeqCst));
    let started = now_ms();
    while !done.load(Ordering::SeqCst) && now_ms() - started < WARM_WAIT_MS {
        pause(poll_ms).await;
    }
    // Error-scope answers ride the same queue; give them a moment to land.
    if settle_ms > 0 {
        pause(settle_ms).await;
    }
}

/// Start the arena on the canvas with id `canvas_id`. Resolves with the JSON line it printed
/// (`"ok":true` once the first frame is on the canvas, `"ok":false` with a reason when the
/// browser has no WebGPU or refuses a device). It does not throw for those cases. After that
/// line, with or without a GPU, it runs the golden check and prints its verdict line.
#[wasm_bindgen]
pub async fn run_arena(canvas_id: String) -> String {
    std::panic::set_hook(Box::new(|info| log_error(&format!("panic: {info}"))));
    set_state("starting");
    // The headless check keys on this: the arena page has a GPU copy of its canvas to read.
    set_body_attr("data-mode", "arena");
    watch_visibility();
    // Input and audio need the DOM only, so they start before (and without) the GPU, as in the shell.
    crate::browser::install(&canvas_id);
    // First, before anything can read a clock: the page's performance.now() becomes the engine's.
    engine_clock::install(browser_clock);
    let line = match Box::pin(start(&canvas_id, 1)).await {
        Ok(line) => line,
        Err((why, detail)) => fail(why, &detail),
    };
    run_golden();
    line
}

/// Ask for a GPU copy of the next canvas frame (the headless check calls this to re-take it).
#[wasm_bindgen]
pub fn request_arena_capture() {
    CAPTURE_WANTED.with(|c| c.set(true));
}
