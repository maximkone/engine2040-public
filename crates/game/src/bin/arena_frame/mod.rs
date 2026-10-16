//! arena_frame: per-frame helpers for the arena window. Holds the boot
//! helpers `ArenaApp::new` calls, the fixed-tick loop `tick_and_present`
//! runs, and the per-pass timing report written at exit. The methods stay on
//! `ArenaApp` (defined in the crate root); this module only moves them out.
use engine_audio::CueBank;

use super::*;

/// Watch content files plus shaders by file; missing roots watch nothing,
/// notes still stage.
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

/// Feed sender plus offline engine for the arena cue bank: the real takes
/// when the bank decoded, nothing when it did not.
pub(super) fn open_cue_feed(
    cue_opt: Option<Arc<CueBank>>,
) -> (Option<FeedTx>, Option<AudioEngine>) {
    match cue_opt {
        Some(cue_bank) => {
            let (tx, engine) = AudioEngine::with_feed(
                cue_bank,
                engine_audio::FEED_CAPACITY,
                WINDOWED_SAMPLE_RATE,
                2040,
            );
            (Some(tx), Some(engine))
        }
        None => (None, None),
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

impl ArenaApp {
    /// Run `steps` fixed ticks for the human slot: input, feed and mixer
    /// pumps, staged and look tallies, then the crash ring.
    pub(super) fn run_sim_ticks(&mut self, steps: u32, human: FighterId) {
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
            // Lock-free feed first (tick-stamped, bounded, `dropped_full`
            // counted): a busy mixer never loses commands because the feed
            // already queued them with the camera listener.
            if let Some(tx) = self.feed_tx.as_mut() {
                feed_tick_audio(tx, tick, &self.tick_scratch);
            }
            // Audible mirror into the shared mixer (best effort): a refused
            // lock loses audible voices only, never feed commands which are
            // already queued and counted above.
            if let Ok(mut mixer) = self.mixer.try_lock() {
                for command in &self.tick_scratch.audio {
                    mixer.play(self.bank.get(command.sound), command.gain);
                }
                // Offline render only while no device stream is live: the
                // device callback pulls voices itself when connected.
                // Reused buffer keeps the per-tick path heap-free.
                if self._device.is_none() {
                    mixer.render_into(&mut self.audio_scratch);
                }
            }
            // Offline pump when no device stream is live so sent == applied
            // even headless; a live stream pumps itself on the callback.
            if self._device.is_none() {
                if let Some(engine) = self.feed_engine.as_mut() {
                    engine.render_stereo_into(&mut self.feed_scratch);
                }
            }
            // Presentation reads the tick after it ran; never writes back.
            if let Some(tally) = self.staged.as_mut() {
                tally.observe(&self.tick_scratch.cues);
            }
            if let Some(look) = self.look.as_mut() {
                let (views, hp) = (self.app.fighter_views(), self.app.hp());
                observe_tick_with_sim(look, tick, views, hp, &self.tick_scratch.cues, None);
                // EG45: poll authoritative balance gaits into the pose (presentation only).
                look.observe_live_balances(
                    self.app.live_match(),
                    usize::from(self.config.fighters),
                );
            }
            self.record_crash_tick(input);
            self.ticks_done += 1;
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
    pub(super) fn pass_mean(&self, index: usize) -> String {
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
    /// total validation errors. `None` until the first presented frame pins
    /// the adapter.
    pub(super) fn per_pass_line(&self) -> Option<String> {
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
    /// per-frame work and interval percentiles on the CPU, the GPU pass
    /// percentiles from the device clock (`gpu_ms=unsupported` when the
    /// adapter has no timestamp queries), the mean swapchain acquire wait
    /// and the present mode (a vsync-bound mode parks the CPU in acquire).
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
    pub(super) fn report_waits(&self) -> Option<i32> {
        let stats = self.presenter.as_ref()?.wait_stats();
        say!("arena waits: {}", stats.summary());
        let message = live_wait_failure(&stats)?;
        tell!("arena: {message}");
        Some(1)
    }
}
