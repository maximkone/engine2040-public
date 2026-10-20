//! arena_frame::latency: the arena window loop's input-to-present stamps.
//! The loop stamps four stages per event: the input it accepted, the sim tick
//! that applied it, the present call, and the frame end. Paired samples feed
//! `engine_perf::latency::WindowLatencyLog`; this module only samples, it
//! never ticks or hashes. Consumer: the `PQ18` window receipt and the measured
//! `input_latency_gpu` block printed at `--exit`.
//!
//! Units: milliseconds on the log's own scale (wall time, `Instant` deltas).
//! Determinism: wall time never enters simulation state or a hash.
//! Errors: a window with no staged pair has no measured receipt; the pending
//! path refuses a blank blocker with `LatencyError::EmptyBlocker`.

use std::time::Instant;

use engine_perf::latency::{
    nearest_rank_sorted, GpuConditions, LatencyError, LatencyReport, WindowLatencyLog,
};

use super::super::*;

/// Staged pairs per run window: one line about every two seconds at 60 Hz.
pub(crate) const WINDOW_EVENTS: usize = 120;

/// Presented frames skipped before sampling: shader and surface warm-up must
/// not be priced as input latency. Reported as `warmup=` in the GPU block.
pub(crate) const WARMUP_FRAMES: u32 = 60;

/// One event's stage stamps. `input` is fixed when the loop accepts it; the
/// later stages fill in as the event reaches the sim and the frame.
#[derive(Clone, Copy, Debug)]
struct EventStamps {
    input: Instant,
    sim: Option<Instant>,
    present: Option<Instant>,
}

/// One closed window's numbers: the log's sim/present legs plus the
/// input-to-present-submission leg the log does not carry.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct WindowSummary {
    /// 0-based window index over the run.
    pub(crate) window: u32,
    /// Log summary for the staged pairs: the sim leg and the event-to-frame-end
    /// present leg the P2 bar reads.
    pub(crate) report: LatencyReport,
    /// Median input accepted to present submission, ms.
    pub(crate) input_p50_ms: f64,
    /// 95th percentile input accepted to present submission, ms.
    pub(crate) input_p95_ms: f64,
}

impl WindowSummary {
    /// Per-window summary line: the log's sim and present percentiles in ms.
    pub(crate) fn line(&self, dropped: u32) -> String {
        format!(
            "PQ18 window={} events={} input_p50={:.3}ms input_p95={:.3}ms \
             sim_p50={:.3}ms sim_p95={:.3}ms present_p50={:.3}ms present_p95={:.3}ms dropped={dropped}",
            self.window,
            self.report.events,
            self.input_p50_ms,
            self.input_p95_ms,
            self.report.sim_p50_ms,
            self.report.sim_p95_ms,
            self.report.present_p50_ms,
            self.report.present_p95_ms,
        )
    }

    /// The receipt line for the run: the player-facing input percentile first,
    /// then the log's sim and present medians beside the GPU leg's state.
    pub(crate) fn receipt(&self, gpu: &str) -> String {
        format!(
            "PQ18 OK window={} input_p50={:.3}ms input_p95={:.3}ms sim_p50={:.3}ms \
             present_p50={:.3}ms gpu={gpu}",
            self.window,
            self.input_p50_ms,
            self.input_p95_ms,
            self.report.sim_p50_ms,
            self.report.present_p50_ms,
        )
    }
}

/// Why a window could not print measured numbers: every stage the open event
/// never reached, so a pending row names its blocker instead of zeros.
fn missing_stage(open: Option<EventStamps>) -> &'static str {
    match open {
        Some(stamps) if stamps.sim.is_none() => "sim tick never submitted",
        Some(stamps) if stamps.present.is_none() => "present never submitted",
        Some(_) => "frame end never arrived",
        None => "",
    }
}

/// Staged samples for one run window plus the open event's chain. The arena
/// window loop stamps each stage through this meter; `note_frame_end` stages a
/// complete chain and closes the window at [`WINDOW_EVENTS`].
#[derive(Clone, Debug)]
pub(crate) struct LatencyMeter {
    /// Closed windows so far (also the index of the open one).
    window: u32,
    /// Pairs that close a window.
    window_events: usize,
    /// Presented frames skipped before sampling.
    warmup: u32,
    /// Presented frames seen, warm-up included.
    frames: u32,
    /// The perf crate's staged collector; cleared after each window.
    log: WindowLatencyLog,
    /// Event-to-present-submission ms, parallel to the log's pairs.
    submit_ms: Vec<f64>,
    /// The event whose input was accepted and waits for its later stages.
    open: Option<EventStamps>,
    /// The newest closed window, printed again if the run ends with no pair.
    last: Option<WindowSummary>,
    /// An accepted input replaced by the next one before its chain closed.
    dropped_incomplete: u32,
    /// A sim stamp arrived with no input, or twice for one input.
    dropped_sim: u32,
    /// A present stamp arrived with no input or no sim, or twice.
    dropped_present: u32,
    /// A chain reached frame end without both later stages.
    dropped_stage: u32,
}

impl LatencyMeter {
    /// A meter closing a window every [`WINDOW_EVENTS`] staged pairs, after
    /// [`WARMUP_FRAMES`] presented frames.
    pub(crate) fn new() -> Self {
        let mut meter = Self::with_window_events(WINDOW_EVENTS);
        meter.warmup = WARMUP_FRAMES;
        meter
    }

    /// A meter with a caller-chosen window size and no warm-up; tests pin the
    /// close point without a window.
    pub(crate) fn with_window_events(window_events: usize) -> Self {
        Self {
            window: 0,
            window_events: window_events.max(1),
            warmup: 0,
            frames: 0,
            log: WindowLatencyLog::new(),
            submit_ms: Vec::with_capacity(window_events.max(1)),
            open: None,
            last: None,
            dropped_incomplete: 0,
            dropped_sim: 0,
            dropped_present: 0,
            dropped_stage: 0,
        }
    }

    /// Presented frames skipped before sampling, for the GPU block.
    pub(crate) fn warmup(&self) -> u32 {
        self.warmup
    }

    /// Stamps dropped for a missing or repeated stage. Presentation only.
    pub(crate) fn dropped(&self) -> u32 {
        self.dropped_incomplete
            .saturating_add(self.dropped_sim)
            .saturating_add(self.dropped_present)
            .saturating_add(self.dropped_stage)
    }

    /// True while an accepted event still waits for its present: the loop only
    /// stamps a present while a chain is open, so a frame with no new tick is
    /// never read as a stray event.
    pub(crate) fn is_waiting(&self) -> bool {
        self.open.is_some()
    }

    /// Stage one: the loop accepted this tick's input. A second tick before the
    /// frame's present keeps the first chain: the player is waiting for the
    /// present of the oldest unshown input, and one present covers both. A
    /// chain whose present went out but whose frame never ended (a skipped
    /// present) is dead, so the new event starts fresh.
    pub(crate) fn note_input_accepted(&mut self, at: Instant) {
        match self.open {
            Some(stamps) if stamps.present.is_some() => {
                self.dropped_incomplete = self.dropped_incomplete.saturating_add(1);
                self.open = Some(EventStamps {
                    input: at,
                    sim: None,
                    present: None,
                });
            }
            Some(_) => {}
            None => {
                self.open = Some(EventStamps {
                    input: at,
                    sim: None,
                    present: None,
                });
            }
        }
    }

    /// Stage two: the tick carrying the accepted input reached the sim. A later
    /// tick in the same frame is folded into the first event, not counted.
    pub(crate) fn note_sim_submitted(&mut self, at: Instant) {
        let stamped = match self.open.as_mut() {
            Some(stamps) if stamps.sim.is_none() => {
                stamps.sim = Some(at);
                true
            }
            Some(_) => true,
            None => false,
        };
        if !stamped {
            self.dropped_sim = self.dropped_sim.saturating_add(1);
        }
    }

    /// Stage three: the frame carrying the tick submitted its present call.
    /// A present with no input (or no sim tick) is a stray and stages nothing.
    pub(crate) fn note_present_submitted(&mut self, at: Instant) {
        let stamped = match self.open.as_mut() {
            Some(stamps) if stamps.present.is_none() && stamps.sim.is_some() => {
                stamps.present = Some(at);
                true
            }
            // A repeat present in the same frame keeps the first stamp.
            Some(stamps) if stamps.present.is_some() => true,
            _ => false,
        };
        if !stamped {
            self.dropped_present = self.dropped_present.saturating_add(1);
        }
    }

    /// Stage four: the presented frame ended. A complete chain stages its
    /// input-to-sim and input-to-frame-end legs; a full window closes and
    /// returns its summary.
    pub(crate) fn note_frame_end(&mut self, at: Instant) -> Option<WindowSummary> {
        self.frames = self.frames.saturating_add(1);
        if self.frames <= self.warmup {
            // Warm-up frames (shader and surface start) never sample: drop the
            // chain they carried instead of pricing it as input latency.
            self.open = None;
            return None;
        }
        let Some(stamps) = self.open.take() else {
            // A frame with no event (title, or a tick the loop held) is fine.
            return None;
        };
        let (Some(sim), Some(present)) = (stamps.sim, stamps.present) else {
            self.dropped_stage = self.dropped_stage.saturating_add(1);
            return None;
        };
        let sim_ms = millis(stamps.input, sim);
        let present_ms = millis(stamps.input, at);
        if self.log.stage(sim_ms, present_ms).is_err() {
            // Stages are ordered and finite by construction; the only refusal
            // left is the collector's ceiling, which the window size bounds.
            self.dropped_stage = self.dropped_stage.saturating_add(1);
            return None;
        }
        // The receipt's input leg: accepted input to present submission, the
        // part of the frame the loop itself owns.
        self.submit_ms.push(millis(stamps.input, present));
        if self.log.len() < self.window_events {
            return None;
        }
        self.close_window()
    }

    /// Summarize and clear the open window, numbering it with `window`.
    fn close_window(&mut self) -> Option<WindowSummary> {
        let report = self.log.summarize().ok()?;
        let mut submit = std::mem::take(&mut self.submit_ms);
        submit.sort_by(f64::total_cmp);
        let summary = WindowSummary {
            window: self.window,
            report,
            input_p50_ms: nearest_rank_sorted(&submit, 50.0),
            input_p95_ms: nearest_rank_sorted(&submit, 95.0),
        };
        self.window = self.window.saturating_add(1);
        self.log.clear();
        self.last = Some(summary);
        Some(summary)
    }

    /// Flush the open window at exit: the summary plus whether it closed here
    /// (so the caller prints its line once). A run with no pair returns the
    /// last closed window, or nothing.
    pub(crate) fn finish(&mut self) -> Option<(WindowSummary, bool)> {
        if self.log.is_empty() {
            return self.last.take().map(|summary| (summary, false));
        }
        let summary = self.close_window()?;
        self.last = None;
        Some((summary, true))
    }

    /// Pending row for this window: the blocker names the stage the open
    /// event never reached. A window with no event names nothing, so the
    /// pending path refuses the blank blocker with
    /// [`LatencyError::EmptyBlocker`].
    ///
    /// # Errors
    ///
    /// [`LatencyError::EmptyBlocker`] when the window has no open event.
    pub(crate) fn pending(&self, conditions: &GpuConditions) -> Result<String, LatencyError> {
        conditions.pending(missing_stage(self.open))
    }
}

/// Wall milliseconds between two stamps on the loop's clock.
fn millis(from: Instant, to: Instant) -> f64 {
    to.saturating_duration_since(from).as_secs_f64() * 1000.0
}

/// The GPU leg's state for the receipt: measured, or the exact reason it is
/// not. `Reported` needs a presented adapter with resolved timestamp queries;
/// a CPU-only bank or a headless loop keeps the leg pending.
enum GpuLeg {
    Reported,
    Pending(&'static str),
}

impl ArenaApp {
    /// Conditions the run was measured under, read from the frames it
    /// presented. A value the loop cannot know stays `not sampled`, never
    /// invented.
    fn latency_conditions(&self, samples: u32) -> GpuConditions {
        let presenter = self.presenter.as_ref();
        let resolution = if self.pass_size.0 == 0 || self.pass_size.1 == 0 {
            "unknown".to_string()
        } else {
            format!("{}x{}", self.pass_size.0, self.pass_size.1)
        };
        GpuConditions {
            adapter: text_or_unknown(&self.pass_adapter),
            driver: text_or_unknown(&self.pass_driver),
            backend: "wgpu".to_string(),
            cpu: std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| "unknown".to_string()),
            power_mode: "not sampled".to_string(),
            clock_mode: "not sampled".to_string(),
            resolution,
            vsync: presenter
                .and_then(Presenter::present_mode_name)
                .unwrap_or_else(|| "unknown".to_string()),
            profile: if cfg!(debug_assertions) {
                "debug".to_string()
            } else {
                "release".to_string()
            },
            scene: "arena".to_string(),
            build: concat!("arena window ", env!("CARGO_PKG_VERSION")).to_string(),
            load: "not sampled".to_string(),
            warmup: self.latency.warmup(),
            samples,
        }
    }

    /// Which leg the receipt may claim: measured only when a presented adapter
    /// exists and its timestamp queries resolved at least one frame.
    fn gpu_leg(&self) -> GpuLeg {
        let Some(presenter) = self.presenter.as_ref() else {
            return GpuLeg::Pending("no visible presenter: the window never created an adapter");
        };
        if !presenter.gpu_timing_supported() {
            return GpuLeg::Pending("the graphics adapter has no timestamp query support");
        }
        if self.pass_adapter.is_empty() {
            return GpuLeg::Pending("no presented frame reported an adapter");
        }
        if presenter.gpu_timing_report().is_none() {
            return GpuLeg::Pending("timestamp queries resolved no GPU sample in this run");
        }
        GpuLeg::Reported
    }

    /// Print the run's latency evidence at `--exit`: the `input_latency_gpu`
    /// block over the last window, or the pending row naming why the GPU leg
    /// cannot report here, then the one-line `PQ18` receipt.
    pub(crate) fn report_latency(&mut self) {
        let Some((summary, fresh)) = self.latency.finish() else {
            let conditions = self.latency_conditions(0);
            // A window left mid-chain names the stage it never reached; a
            // window with no event at all is refused, so name that reason.
            match self.latency.pending(&conditions) {
                Ok(line) => say!("{}", line.trim_end()),
                Err(LatencyError::EmptyBlocker) => {
                    let missing = "no input event was staged in this run window";
                    match conditions.pending(missing) {
                        Ok(line) => say!("{}", line.trim_end()),
                        Err(err) => tell!("arena: {err}"),
                    }
                }
                Err(err) => tell!("arena: {err}"),
            }
            return;
        };
        if fresh {
            say!("{}", summary.line(self.latency.dropped()));
        }
        let conditions = self.latency_conditions(summary.report.events);
        match self.gpu_leg() {
            GpuLeg::Reported => {
                say!("{}", conditions.report(&summary.report).trim_end());
                say!("{}", summary.receipt("reported"));
            }
            GpuLeg::Pending(why) => {
                match conditions.pending(why) {
                    Ok(line) => say!("{}", line.trim_end()),
                    Err(err) => tell!("arena: {err}"),
                }
                say!("{}", summary.receipt("pending"));
            }
        }
    }
}

/// An empty condition string reads `unknown` in the report.
fn text_or_unknown(text: &str) -> String {
    if text.is_empty() {
        "unknown".to_string()
    } else {
        text.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn gpu_conditions() -> GpuConditions {
        GpuConditions {
            adapter: "NVIDIA GeForce RTX 3050 6GB Laptop GPU".to_string(),
            driver: "610.62".to_string(),
            backend: "wgpu".to_string(),
            cpu: "test".to_string(),
            power_mode: "not sampled".to_string(),
            clock_mode: "not sampled".to_string(),
            resolution: "1280x720".to_string(),
            vsync: "AutoVsync".to_string(),
            profile: "release".to_string(),
            scene: "arena".to_string(),
            build: "test".to_string(),
            load: "not sampled".to_string(),
            warmup: 0,
            samples: 1,
        }
    }

    /// One complete chain from `base`, in the loop's stage order.
    fn stamp_chain(meter: &mut LatencyMeter, base: Instant) {
        meter.note_input_accepted(base);
        meter.note_sim_submitted(base + Duration::from_millis(1));
        meter.note_present_submitted(base + Duration::from_millis(3));
        meter.note_frame_end(base + Duration::from_millis(5));
    }

    #[test]
    fn stamp_chain_accepts_events_in_order() {
        let base = Instant::now();
        let mut meter = LatencyMeter::with_window_events(1);
        stamp_chain(&mut meter, base);
        let summary = meter
            .last
            .expect("one complete chain closes the one-pair window");
        assert_eq!(summary.window, 0);
        assert_eq!(summary.report.events, 1);
        assert!(
            (summary.report.sim_p50_ms - 1.0).abs() < 1e-6,
            "sim leg is input to tick: {}",
            summary.report.sim_p50_ms
        );
        assert!(
            (summary.report.present_p50_ms - 5.0).abs() < 1e-6,
            "present leg is input to frame end: {}",
            summary.report.present_p50_ms
        );
        assert!(
            (summary.input_p50_ms - 3.0).abs() < 1e-6,
            "input leg is input to present submission: {}",
            summary.input_p50_ms
        );
        assert_eq!(meter.dropped(), 0, "a clean chain drops nothing");
        let line = summary.line(meter.dropped());
        assert!(line.starts_with("PQ18 window=0 events=1"), "{line}");
        assert!(line.contains("sim_p50=1.000ms"), "{line}");
        assert!(line.contains("present_p50=5.000ms"), "{line}");
        let receipt = summary.receipt("reported");
        assert!(
            receipt.starts_with("PQ18 OK window=0 input_p50=3.000ms"),
            "{receipt}"
        );
        assert!(receipt.ends_with("gpu=reported"), "{receipt}");
    }

    #[test]
    fn second_tick_before_present_keeps_the_first_event() {
        let base = Instant::now();
        let mut meter = LatencyMeter::with_window_events(1);
        meter.note_input_accepted(base);
        meter.note_sim_submitted(base + Duration::from_millis(1));
        // The same wall step ran a second tick before presenting: the frame's
        // present covers both, so the first (older) event stays the sample.
        meter.note_input_accepted(base + Duration::from_millis(2));
        meter.note_sim_submitted(base + Duration::from_millis(3));
        meter.note_present_submitted(base + Duration::from_millis(4));
        let summary = meter
            .note_frame_end(base + Duration::from_millis(5))
            .expect("one window closes");
        assert_eq!(summary.report.events, 1, "one sample per presented frame");
        assert!(
            (summary.report.sim_p50_ms - 1.0).abs() < 1e-6,
            "the first tick's sim stamp is the sample: {}",
            summary.report.sim_p50_ms
        );
        assert!(
            (summary.input_p50_ms - 4.0).abs() < 1e-6,
            "the first input's present stamp is the sample: {}",
            summary.input_p50_ms
        );
        assert_eq!(meter.dropped(), 0, "a folded tick is not a stray");
    }

    #[test]
    fn stray_present_before_input_is_dropped() {
        let base = Instant::now();
        let mut meter = LatencyMeter::with_window_events(WINDOW_EVENTS);
        meter.note_present_submitted(base);
        assert_eq!(meter.log.len(), 0, "a present with no event stages nothing");
        assert_eq!(meter.dropped(), 1, "the stray is counted");
        // A sim before its input is a stray too, and cannot open a chain.
        meter.note_sim_submitted(base + Duration::from_millis(1));
        assert_eq!(meter.log.len(), 0);
        assert_eq!(meter.dropped(), 2);
        // A present after the input but before the sim is still early.
        meter.note_input_accepted(base + Duration::from_millis(2));
        meter.note_present_submitted(base + Duration::from_millis(3));
        assert_eq!(meter.dropped(), 3);
        assert!(meter
            .note_frame_end(base + Duration::from_millis(4))
            .is_none());
        assert_eq!(meter.log.len(), 0);
        // The clean chain still stages after the strays.
        stamp_chain(&mut meter, base + Duration::from_millis(10));
        assert_eq!(meter.log.len(), 1);
    }

    #[test]
    fn frame_end_without_present_drops_the_open_chain() {
        let base = Instant::now();
        let mut meter = LatencyMeter::with_window_events(1);
        meter.note_input_accepted(base);
        meter.note_sim_submitted(base + Duration::from_millis(1));
        assert!(meter
            .note_frame_end(base + Duration::from_millis(2))
            .is_none());
        assert_eq!(
            meter.log.len(),
            0,
            "an unfinished chain never stages a pair"
        );
        assert_eq!(meter.dropped(), 1);
    }

    #[test]
    fn window_without_events_reports_empty_blocker() {
        let meter = LatencyMeter::new();
        assert_eq!(
            meter.pending(&gpu_conditions()),
            Err(LatencyError::EmptyBlocker),
            "an empty window names no missing stage, so the blank blocker is refused"
        );
        // An open chain names the stage it never reached instead of printing
        // zeros, so the pending row stays useful mid-run.
        let mut open = LatencyMeter::new();
        open.note_input_accepted(Instant::now());
        let text = open
            .pending(&gpu_conditions())
            .expect("the open chain names its missing stage");
        assert!(text.contains("sim tick never submitted"), "{text}");
        assert!(text.contains("PENDING"), "{text}");
    }

    #[test]
    fn warmup_frames_are_not_samples() {
        let base = Instant::now();
        let mut meter = LatencyMeter::with_window_events(1);
        meter.warmup = 2;
        assert_eq!(meter.warmup(), 2);
        // Two warm-up frames carry events but must not seed the window.
        stamp_chain(&mut meter, base);
        stamp_chain(&mut meter, base + Duration::from_millis(10));
        assert_eq!(meter.log.len(), 0, "warm-up pairs are dropped, not staged");
        assert_eq!(meter.dropped(), 0, "a skipped warm-up is not a stray");
        // The first post-warm-up chain is the sample and closes the window.
        stamp_chain(&mut meter, base + Duration::from_millis(20));
        let summary = meter.last.expect("the post-warm-up pair closes the window");
        assert_eq!(summary.report.events, 1);
        assert!(
            (summary.report.sim_p50_ms - 1.0).abs() < 1e-6,
            "the sampled chain keeps its own stamps: {}",
            summary.report.sim_p50_ms
        );
    }

    #[test]
    fn partial_window_flushes_once_at_finish() {
        let base = Instant::now();
        let mut meter = LatencyMeter::with_window_events(4);
        stamp_chain(&mut meter, base);
        stamp_chain(&mut meter, base + Duration::from_millis(10));
        let (summary, fresh) = meter.finish().expect("the partial window flushes");
        assert!(fresh, "the caller prints this window's line");
        assert_eq!(summary.window, 0);
        assert_eq!(summary.report.events, 2);
        assert!(
            summary.report.present_p50_ms > 0.0,
            "flushed pairs keep their measured legs"
        );
        assert!(meter.finish().is_none(), "a flushed window prints once");
    }
}
