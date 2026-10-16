//! Input latency meter: event tick to first sim effect and first present.
//! Consumer: the proof `--perf` gate and the latency evidence read
//! `LatencyReport` (`summary` lines drop into the perf report).
//! Units: ticks/frames on the shared 60 Hz clock, milliseconds in reports.
//! Determinism: `measure` is integer tick math only, no wall clock, sorted by
//! id so input order never matters; wall time lives in `WallMeter` for live
//! presentation and in `summarize_live` inputs, and never enters a tick hash.
//! Reference-GPU rows print through `GpuConditions::report`, or `pending`
//! with the blocker named when the window run has not landed yet.
//! Errors: typed `LatencyError`.
//! Budget: the bar is event-to-present p95 <= 50 ms (3 frames at 60 Hz).

use engine_core::{splitmix64, Rng};
use std::fmt::{self, Display, Formatter};
use std::time::Instant;

/// Shared clock: simulation ticks and presented frames both run at 60 Hz.
pub const TICK_HZ: f64 = 60.0;
/// Milliseconds per tick/frame; presentation scale only, never sim state.
pub const MS_PER_TICK: f64 = 1000.0 / TICK_HZ;
/// P2 feel bar: event-to-present p95 must stay at or under 50 ms.
pub const PRESENT_P95_BUDGET_MS: f64 = 50.0;

/// One input event: its id plus the tick it arrived on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InputEvent {
    /// Stable id joining the event to its effect and present rows.
    pub id: u64,
    /// Arrival tick on the 60 Hz clock.
    pub tick: u64,
}

/// First tick whose simulation state reflects the input event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SimEffect {
    /// Matches an [`InputEvent::id`].
    pub id: u64,
    /// First affected tick on the 60 Hz clock.
    pub tick: u64,
}

/// First presented frame reflecting the input event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FramePresent {
    /// Matches an [`InputEvent::id`].
    pub id: u64,
    /// First affected frame on the 60 Hz clock.
    pub frame: u64,
}

/// Latency failure: which input or budget refused, with both sides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LatencyError {
    /// No events to measure.
    EmptyInput,
    /// Two events share an id.
    DuplicateEvent {
        /// Offending id.
        id: u64,
    },
    /// Two effect rows share an id.
    DuplicateEffect {
        /// Offending id.
        id: u64,
    },
    /// Two present rows share an id.
    DuplicatePresent {
        /// Offending id.
        id: u64,
    },
    /// An event has no effect row.
    MissingEffect {
        /// Event id without an effect.
        id: u64,
    },
    /// An event has no present row.
    MissingPresent {
        /// Event id without a present.
        id: u64,
    },
    /// An effect predates its event; time never runs backwards.
    EffectBeforeEvent {
        /// Event id.
        id: u64,
        /// Event arrival tick.
        event_tick: u64,
        /// Effect tick, before the event.
        effect_tick: u64,
    },
    /// A present predates its event; time never runs backwards.
    PresentBeforeEvent {
        /// Event id.
        id: u64,
        /// Event arrival tick.
        event_tick: u64,
        /// Present frame, before the event.
        frame: u64,
    },
    /// Event-to-present p95 blew the P2 bar.
    BudgetBlown {
        /// Measured p95 in ms.
        actual_ms: f64,
        /// P2 ceiling in ms.
        budget_ms: f64,
    },
    /// Live legs disagree in length; every sampled event needs both legs.
    MismatchedSamples {
        /// Length of the sim-effect leg in samples.
        sim: usize,
        /// Length of the present leg in samples.
        present: usize,
    },
    /// One live sample is unusable: NaN, infinite or negative wall time.
    BadSample {
        /// Which leg the sample came from.
        leg: SampleLeg,
        /// Position in the caller's sample order.
        index: usize,
        /// Offending milliseconds.
        value_ms: f64,
    },
    /// A present sample predates its sim effect; a frame never precedes the
    /// effect it presents.
    PresentBeforeSim {
        /// Position in the caller's sample order.
        index: usize,
        /// Sim-effect milliseconds for this event.
        sim_ms: f64,
        /// Present milliseconds for this event, before the effect.
        present_ms: f64,
    },
    /// More samples than the report's event counter holds.
    TooManySamples {
        /// Rejected sample count.
        count: usize,
    },
    /// A pending marker without its reason named; pending rows must say
    /// exactly what is still missing so nobody mistakes them for data.
    EmptyBlocker,
}

impl Display for LatencyError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::EmptyInput => write!(f, "latency: no input events to measure"),
            Self::DuplicateEvent { id } => write!(f, "latency: duplicate event id {id}"),
            Self::DuplicateEffect { id } => write!(f, "latency: duplicate effect id {id}"),
            Self::DuplicatePresent { id } => write!(f, "latency: duplicate present id {id}"),
            Self::MissingEffect { id } => write!(f, "latency: event {id} has no sim effect row"),
            Self::MissingPresent { id } => {
                write!(f, "latency: event {id} has no present row")
            }
            Self::EffectBeforeEvent {
                id,
                event_tick,
                effect_tick,
            } => write!(
                f,
                "latency: event {id} effect tick {effect_tick} predates arrival {event_tick}"
            ),
            Self::PresentBeforeEvent {
                id,
                event_tick,
                frame,
            } => write!(
                f,
                "latency: event {id} present frame {frame} predates arrival {event_tick}"
            ),
            Self::BudgetBlown {
                actual_ms,
                budget_ms,
            } => write!(
                f,
                "latency: present p95 {actual_ms:.3} ms blows the {budget_ms:.1} ms P2 bar"
            ),
            Self::MismatchedSamples { sim, present } => write!(
                f,
                "latency: live legs disagree: {sim} sim samples but {present} present samples"
            ),
            Self::BadSample {
                leg,
                index,
                value_ms,
            } => write!(f, "latency: {leg} sample {index} is unusable ({value_ms} ms)"),
            Self::PresentBeforeSim {
                index,
                sim_ms,
                present_ms,
            } => write!(
                f,
                "latency: sample {index} present {present_ms} ms predates its sim effect {sim_ms} ms"
            ),
            Self::TooManySamples { count } => {
                write!(f, "latency: {count} samples overflow the u32 event counter")
            }
            Self::EmptyBlocker => write!(f, "latency: pending marker needs its blocker named"),
        }
    }
}

impl std::error::Error for LatencyError {}

/// Which live leg a wall-clock sample belongs to; names the offending leg in
/// [`LatencyError::BadSample`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleLeg {
    /// Event to first simulated effect.
    SimEffect,
    /// Event to first presented frame.
    Present,
}

impl Display for SampleLeg {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::SimEffect => write!(f, "sim"),
            Self::Present => write!(f, "present"),
        }
    }
}

/// Input-latency report: p50/p95 in ms for both legs, over matched events.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LatencyReport {
    /// Events matched to an effect row and a present row.
    pub events: u32,
    /// Median event-to-first-sim-effect ms.
    pub sim_p50_ms: f64,
    /// 95th percentile event-to-first-sim-effect ms.
    pub sim_p95_ms: f64,
    /// Median event-to-first-present ms.
    pub present_p50_ms: f64,
    /// 95th percentile event-to-first-present ms (gated by the P2 bar).
    pub present_p95_ms: f64,
}

impl LatencyReport {
    /// Human-readable lines for the perf report: both legs plus the bar
    /// verdict. Numbers vary per run; names and units are stable.
    #[must_use]
    pub fn summary(&self) -> String {
        let verdict = if self.present_p95_ms <= PRESENT_P95_BUDGET_MS {
            "pass"
        } else {
            "FAIL"
        };
        format!(
            "input_latency events={} sim_p50={:.3}ms sim_p95={:.3}ms \
             present_p50={:.3}ms present_p95={:.3}ms \
             p95_budget={:.1}ms verdict={verdict}\n",
            self.events,
            self.sim_p50_ms,
            self.sim_p95_ms,
            self.present_p50_ms,
            self.present_p95_ms,
            PRESENT_P95_BUDGET_MS,
        )
    }
}

/// Nearest-rank percentile over a sorted sample slice, owner for the float
/// reports (`latency`, `profiler` and the `perf` harness stats): rank
/// `round(p / 100 * (n - 1))`, clamped to the last sample. Empty gives `0.0`
/// so callers without a sample fail soft.
/// Caller: `measure`, `summarize_live` plus the delegated report sites.
/// Differs deliberately from [`frame_rank`] (ceil rank `ceil(p * n) - 1` so
/// Rust matches the `tools/proof.mjs` JS gate) and from
/// `fight_cost::percentile_sorted` (integer micros, `len * pct / 100`, `None`
/// on empty or `pct > 100` because that gate needs integer math only).
pub fn nearest_rank_sorted(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let end = sorted.len() - 1;
    let index = ((p / 100.0) * end as f64).round() as usize;
    sorted[index.min(end)]
}

/// Sort a copy of `ids` by id, refusing duplicates. Vecs stay small (one row
/// per input event), so the offline sort costs nothing per tick.
fn sorted_ids<T: Copy>(
    rows: &[T],
    id_of: impl Fn(T) -> u64,
    dup: impl Fn(u64) -> LatencyError,
) -> Result<Vec<T>, LatencyError> {
    let mut sorted = rows.to_vec();
    sorted.sort_by_key(|row| id_of(*row));
    if let Some(pair) = sorted
        .windows(2)
        .find(|pair| id_of(pair[0]) == id_of(pair[1]))
    {
        return Err(dup(id_of(pair[0])));
    }
    Ok(sorted)
}

/// Measure input latency: join each event to its first sim effect and first
/// present, then report p50/p95 per leg in ms. Pure tick math, deterministic
/// per input: rows sort by id, quantiles are nearest-rank over exact
/// `frames * 1000 / 60` values (integer-first, so 3 frames is exactly 50 ms).
pub fn measure(
    events: &[InputEvent],
    effects: &[SimEffect],
    presents: &[FramePresent],
) -> Result<LatencyReport, LatencyError> {
    if events.is_empty() {
        return Err(LatencyError::EmptyInput);
    }
    let events = sorted_ids(
        events,
        |row| row.id,
        |id| LatencyError::DuplicateEvent { id },
    )?;
    let effects = sorted_ids(
        effects,
        |row| row.id,
        |id| LatencyError::DuplicateEffect { id },
    )?;
    let presents = sorted_ids(
        presents,
        |row| row.id,
        |id| LatencyError::DuplicatePresent { id },
    )?;
    let mut sim_ms = Vec::with_capacity(events.len());
    let mut present_ms = Vec::with_capacity(events.len());
    for event in &events {
        let effect = effects
            .binary_search_by_key(&event.id, |row| row.id)
            .map(|index| effects[index])
            .map_err(|_| LatencyError::MissingEffect { id: event.id })?;
        let present = presents
            .binary_search_by_key(&event.id, |row| row.id)
            .map(|index| presents[index])
            .map_err(|_| LatencyError::MissingPresent { id: event.id })?;
        if effect.tick < event.tick {
            return Err(LatencyError::EffectBeforeEvent {
                id: event.id,
                event_tick: event.tick,
                effect_tick: effect.tick,
            });
        }
        if present.frame < event.tick {
            return Err(LatencyError::PresentBeforeEvent {
                id: event.id,
                event_tick: event.tick,
                frame: present.frame,
            });
        }
        // Integer-first: exact for whole frames, finite for every u64 input.
        sim_ms.push((effect.tick - event.tick) as f64 * 1000.0 / TICK_HZ);
        present_ms.push((present.frame - event.tick) as f64 * 1000.0 / TICK_HZ);
    }
    sim_ms.sort_by(|a, b| a.total_cmp(b));
    present_ms.sort_by(|a, b| a.total_cmp(b));
    Ok(LatencyReport {
        events: events.len() as u32,
        sim_p50_ms: nearest_rank_sorted(&sim_ms, 50.0),
        sim_p95_ms: nearest_rank_sorted(&sim_ms, 95.0),
        present_p50_ms: nearest_rank_sorted(&present_ms, 50.0),
        present_p95_ms: nearest_rank_sorted(&present_ms, 95.0),
    })
}

/// Enforce the P2 bar: event-to-present p95 at or under 50 ms.
pub fn check_p2_bar(report: &LatencyReport) -> Result<(), LatencyError> {
    if report.present_p95_ms > PRESENT_P95_BUDGET_MS {
        return Err(LatencyError::BudgetBlown {
            actual_ms: report.present_p95_ms,
            budget_ms: PRESENT_P95_BUDGET_MS,
        });
    }
    Ok(())
}

/// Seeded 3-frame fixture: `count` events on a seeded spread, first sim
/// effect 1 frame later, first present 3 frames later (50 ms at 60 Hz).
/// Same seed gives the same rows on every machine. The seed spreads through
/// SplitMix64 first: the frozen legacy seeder forces the low bit, so raw
/// neighboring seeds would share one stream.
#[must_use]
pub fn fixture_3f(seed: u64, count: u32) -> (Vec<InputEvent>, Vec<SimEffect>, Vec<FramePresent>) {
    let mut rng = Rng::new(splitmix64(seed));
    let capacity = count as usize;
    let mut events = Vec::with_capacity(capacity);
    let mut effects = Vec::with_capacity(capacity);
    let mut presents = Vec::with_capacity(capacity);
    for index in 0..count {
        let tick = u64::from(index) * 11 + rng.next_u64() % 7;
        let id = u64::from(index);
        events.push(InputEvent { id, tick });
        effects.push(SimEffect { id, tick: tick + 1 });
        presents.push(FramePresent {
            id,
            frame: tick + 3,
        });
    }
    (events, effects, presents)
}

/// Headless reference run: seed behind every reported number.
pub const HEADLESS_SEED: u64 = 7;
/// Events per headless run; matches the `three_frame_delay_detected` proof.
pub const HEADLESS_SAMPLES: u32 = 31;

/// Conditions a headless latency run was measured under. Tick math is
/// machine-independent; these names only say which build produced the run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadlessConditions {
    /// `os-arch` pair from `std::env::consts` (evidence only, never hashed).
    pub machine: String,
    /// `debug` or `release`, by `cfg!(debug_assertions)` (p50/p95 per build).
    pub profile: &'static str,
    /// Warmup frames before sampling; tick math needs none, so 0.
    pub warmup: u32,
    /// Events sampled; doubles as the fixture count.
    pub samples: u32,
}

impl HeadlessConditions {
    /// Name this build's conditions; `samples` doubles as the fixture count.
    #[must_use]
    pub fn current(warmup: u32, samples: u32) -> Self {
        Self {
            machine: format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH),
            profile: if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            },
            warmup,
            samples,
        }
    }
}

/// Headless perf-report block: conditions, the seeded 3-frame numbers, then
/// the reference-GPU legs marked PENDING (no GPU on a headless runner).
/// The proof `--perf` gate prints this text into the perf report.
pub fn headless_report(conditions: &HeadlessConditions) -> Result<String, LatencyError> {
    let (events, effects, presents) = fixture_3f(HEADLESS_SEED, conditions.samples);
    let report = measure(&events, &effects, &presents)?;
    Ok(format!(
        "input_latency_headless seed={HEADLESS_SEED} samples={} warmup={} machine={} profile={}\n\
         {}reference_gpu sim_p50=PENDING sim_p95=PENDING \
         present_p50=PENDING present_p95=PENDING (no GPU on headless runner)\n",
        conditions.samples,
        conditions.warmup,
        conditions.machine,
        conditions.profile,
        report.summary(),
    ))
}

/// Checked event count for a live sample set: the report counter is 32 bits,
/// so an absurd harness output fails loudly instead of truncating silently.
fn event_count(len: usize) -> Result<u32, LatencyError> {
    u32::try_from(len).map_err(|_| LatencyError::TooManySamples { count: len })
}

/// Summarize live wall-clock samples: per-event event-to-sim-effect and
/// event-to-present milliseconds, in caller order. Every sample must be
/// finite and non-negative, and each present must sit at or after its sim
/// effect; ranks use the same nearest-rank p50/p95 as [`measure`] so tick
/// math and live runs read identically. Pure over its inputs: run-to-run
/// movement lives in the sampling, never in hidden state. Same slices give
/// the same report twice; the timed-smoke wiring stages wall samples here.
/// Slice in, report out; run [`check_p2_bar`] on the result for the P2 verdict.
///
/// # Errors
///
/// [`LatencyError::EmptyInput`] when both legs are empty,
/// [`LatencyError::MismatchedSamples`] when the legs disagree in length,
/// [`LatencyError::BadSample`] on a NaN, infinite or negative sample,
/// [`LatencyError::PresentBeforeSim`] when a present predates its effect,
/// [`LatencyError::TooManySamples`] past the u32 event counter.
pub fn summarize_live(sim_ms: &[f64], present_ms: &[f64]) -> Result<LatencyReport, LatencyError> {
    if sim_ms.is_empty() && present_ms.is_empty() {
        return Err(LatencyError::EmptyInput);
    }
    if sim_ms.len() != present_ms.len() {
        return Err(LatencyError::MismatchedSamples {
            sim: sim_ms.len(),
            present: present_ms.len(),
        });
    }
    let mut sim = Vec::with_capacity(sim_ms.len());
    let mut present = Vec::with_capacity(present_ms.len());
    for (index, (&sim_sample, &present_sample)) in sim_ms.iter().zip(present_ms.iter()).enumerate()
    {
        for (leg, value) in [
            (SampleLeg::SimEffect, sim_sample),
            (SampleLeg::Present, present_sample),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(LatencyError::BadSample {
                    leg,
                    index,
                    value_ms: value,
                });
            }
        }
        if present_sample < sim_sample {
            return Err(LatencyError::PresentBeforeSim {
                index,
                sim_ms: sim_sample,
                present_ms: present_sample,
            });
        }
        sim.push(sim_sample);
        present.push(present_sample);
    }
    sim.sort_by(|a, b| a.total_cmp(b));
    present.sort_by(|a, b| a.total_cmp(b));
    Ok(LatencyReport {
        events: event_count(sim.len())?,
        sim_p50_ms: nearest_rank_sorted(&sim, 50.0),
        sim_p95_ms: nearest_rank_sorted(&sim, 95.0),
        present_p50_ms: nearest_rank_sorted(&present, 50.0),
        present_p95_ms: nearest_rank_sorted(&present, 95.0),
    })
}

/// Ceiling for staged window events: bounds the window loop's scratch so one
/// runaway session cannot grow the report without limit.
pub const MAX_STAGED_EVENTS: usize = 4_096;

/// Staged window-loop collector: holds per-event event-to-sim-effect and
/// event-to-present milliseconds until the frame batch summarizes. The arena
/// window loop stages the input, sim-effect and present legs per event, then
/// `summarize` feeds [`summarize_live`] into [`GpuConditions::report`] for the
/// measured `input_latency_gpu` block. Presentation only: never hashed, never
/// ticked. Empty legs summarize exactly like [`summarize_live`] over empty
/// slices, so an unstaged run fails loudly instead of printing zeros.
///
/// # Errors
///
/// [`LatencyError::TooManySamples`] when already holding [`MAX_STAGED_EVENTS`]
/// pairs, [`LatencyError::BadSample`] on a NaN, infinite or negative leg,
/// [`LatencyError::PresentBeforeSim`] when a present predates its effect.
#[derive(Clone, Debug, Default)]
pub struct WindowLatencyLog {
    /// Staged event-to-sim-effect milliseconds, in stage order.
    sim_ms: Vec<f64>,
    /// Staged event-to-present milliseconds, in stage order.
    present_ms: Vec<f64>,
}

impl WindowLatencyLog {
    /// Empty collector; legs grow to [`MAX_STAGED_EVENTS`] pairs at most.
    #[must_use]
    pub fn new() -> Self {
        Self {
            sim_ms: Vec::new(),
            present_ms: Vec::new(),
        }
    }

    /// Staged pairs so far.
    #[must_use]
    pub fn len(&self) -> usize {
        self.sim_ms.len()
    }

    /// True while no pair is staged.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sim_ms.is_empty()
    }

    /// Stage one input-to-present sample pair in caller order. Rejects a bad
    /// pair at stage time with the same error [`summarize_live`] would raise,
    /// so the window loop learns the leg and index on the spot.
    ///
    /// # Errors
    ///
    /// [`LatencyError::TooManySamples`] past [`MAX_STAGED_EVENTS`] pairs,
    /// [`LatencyError::BadSample`] on a NaN, infinite or negative leg,
    /// [`LatencyError::PresentBeforeSim`] when the present predates its sim.
    pub fn stage(&mut self, sim_ms: f64, present_ms: f64) -> Result<(), LatencyError> {
        if self.sim_ms.len() >= MAX_STAGED_EVENTS {
            return Err(LatencyError::TooManySamples {
                count: self.sim_ms.len() + 1,
            });
        }
        let index = self.sim_ms.len();
        for (leg, value) in [
            (SampleLeg::SimEffect, sim_ms),
            (SampleLeg::Present, present_ms),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(LatencyError::BadSample {
                    leg,
                    index,
                    value_ms: value,
                });
            }
        }
        if present_ms < sim_ms {
            return Err(LatencyError::PresentBeforeSim {
                index,
                sim_ms,
                present_ms,
            });
        }
        self.sim_ms.push(sim_ms);
        self.present_ms.push(present_ms);
        Ok(())
    }

    /// Summarize the staged legs with the shared nearest-rank p50/p95, so the
    /// staged report ranks exactly like [`summarize_live`] over the same pairs.
    /// Run [`check_p2_bar`] on the result for the P2 verdict.
    ///
    /// # Errors
    ///
    /// [`LatencyError::EmptyInput`] while no pair is staged.
    #[must_use]
    pub fn summarize(&self) -> Result<LatencyReport, LatencyError> {
        summarize_live(&self.sim_ms, &self.present_ms)
    }

    /// Print the staged legs as the measured GPU block. One call for the
    /// pilot run: summarize the staged pairs, then print with conditions.
    /// Empty logs fail with [`LatencyError::EmptyInput`], never zeros.
    /// Caller: the pilot PQ18b run hands its conditions here.
    #[must_use]
    pub fn staged_report(&self, conditions: &GpuConditions) -> Result<String, LatencyError> {
        self.summarize().map(|report| conditions.report(&report))
    }

    /// Drop every staged pair; the log reads as fresh for the next batch.
    pub fn clear(&mut self) {
        self.sim_ms.clear();
        self.present_ms.clear();
    }
}

/// Conditions a reference-GPU latency run was measured under. Every field
/// prints into the report beside the numbers, so a later reader knows the
/// adapter, driver, power and clock state, build, scene and machine load
/// behind each p50/p95. Offline report data only: never hashed, never ticked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuConditions {
    /// Graphics adapter name as reported by the OS or driver.
    pub adapter: String,
    /// Graphics driver version behind the run.
    pub driver: String,
    /// Graphics backend used for the run.
    pub backend: String,
    /// Processor the window run executed on.
    pub cpu: String,
    /// OS power scheme during the run.
    pub power_mode: String,
    /// Graphics clock policy observed during the run.
    pub clock_mode: String,
    /// Internal render resolution (output may upscale from it).
    pub resolution: String,
    /// Presentation sync state during the run.
    pub vsync: String,
    /// Build profile: debug or release; numbers never compare across profiles.
    pub profile: String,
    /// Scene and content version behind the run.
    pub scene: String,
    /// Window build identity (binary plus content hash).
    pub build: String,
    /// Machine load during the run: concurrent builds or captures beside it.
    pub load: String,
    /// Warmup frames before sampling.
    pub warmup: u32,
    /// Events sampled.
    pub samples: u32,
}

impl GpuConditions {
    /// Measured reference-GPU block: conditions beside the numbers, then the
    /// shared summary with its P2 verdict. Same report shape as the headless
    /// leg, so one perf report reads both.
    #[must_use]
    pub fn report(&self, report: &LatencyReport) -> String {
        format!(
            "input_latency_gpu samples={} warmup={} profile={} scene=\"{}\" build=\"{}\" \
             adapter=\"{}\" driver=\"{}\" backend=\"{}\" resolution=\"{}\" vsync=\"{}\" \
             power=\"{}\" clocks=\"{}\" cpu=\"{}\" load=\"{}\"\n{}",
            self.samples,
            self.warmup,
            self.profile,
            self.scene,
            self.build,
            self.adapter,
            self.driver,
            self.backend,
            self.resolution,
            self.vsync,
            self.power_mode,
            self.clock_mode,
            self.cpu,
            self.load,
            report.summary(),
        )
    }

    /// Pending reference-GPU block: every leg stays PENDING with the exact
    /// blocker named, beside the known conditions. A run posts this instead
    /// of numbers until the window measurement lands.
    ///
    /// # Errors
    ///
    /// [`LatencyError::EmptyBlocker`] when the blocker is empty or blank:
    /// a pending row must say what is still missing.
    pub fn pending(&self, blocker: &str) -> Result<String, LatencyError> {
        if blocker.trim().is_empty() {
            return Err(LatencyError::EmptyBlocker);
        }
        Ok(format!(
            "reference_gpu sim_p50=PENDING sim_p95=PENDING \
             present_p50=PENDING present_p95=PENDING samples={} warmup={} profile={} \
             scene=\"{}\" build=\"{}\" adapter=\"{}\" driver=\"{}\" backend=\"{}\" \
             resolution=\"{}\" vsync=\"{}\" power=\"{}\" clocks=\"{}\" cpu=\"{}\" \
             load=\"{}\" blocker=\"{blocker}\"\n",
            self.samples,
            self.warmup,
            self.profile,
            self.scene,
            self.build,
            self.adapter,
            self.driver,
            self.backend,
            self.resolution,
            self.vsync,
            self.power_mode,
            self.clock_mode,
            self.cpu,
            self.load,
        ))
    }
}

/// Live wall-clock meter for presentation only (on-screen ms, operator
/// overlays). Never enters `measure`, `summarize_live` or any report hash;
/// timed-smoke timestamps stage via `summarize_live`/`WindowLatencyLog`,
/// never via this meter.
pub struct WallMeter {
    /// Moment the meter started.
    started: Instant,
}

impl WallMeter {
    /// Start a presentation meter.
    #[must_use]
    pub fn start() -> Self {
        Self {
            started: Instant::now(),
        }
    }

    /// Milliseconds since the meter started; evidence only, never hashed.
    #[must_use]
    pub fn elapsed_ms(&self) -> f64 {
        self.started.elapsed().as_secs_f64() * 1000.0
    }
}

/// Frame-times gate outcome mirroring `tools/proof.mjs` `frameCsvVerdict`.
/// Units: ms in `p50_ms`/`p95_ms`; `line` matches the JS gate.
/// Determinism: pure, sorts a copy, ceil-ranks like JS.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameCsvOutcome {
    /// True while frame p95 sits at or under the bar.
    pub ok: bool,
    /// Data rows ranked.
    pub frames: u32,
    /// Ranked median wall ms, `None` while the CSV refuses.
    pub p50_ms: Option<f64>,
    /// Ranked p95 wall ms, `None` while the CSV refuses.
    pub p95_ms: Option<f64>,
    /// Single PASS/FAIL line, shaped like the JS gate.
    pub line: String,
}

/// Write per-frame wall times as the proof-gate CSV.
/// Header `frame,wall_ms`; one row per sample with thousandths.
/// Caller: window timed-smoke; the CSV round-trip is pinned by the tests below.
#[must_use]
pub fn frame_times_csv(samples: &[f64]) -> String {
    let mut out = String::from("frame,wall_ms\n");
    for (index, sample) in samples.iter().enumerate() {
        out.push_str(&format!("{index},{sample:.3}\n"));
    }
    out
}

/// FAIL outcome with a named line; keeps every refusal one line.
fn frame_fail(frames: u32, line: &str) -> FrameCsvOutcome {
    FrameCsvOutcome {
        ok: false,
        frames,
        p50_ms: None,
        p95_ms: None,
        line: String::from(line),
    }
}

/// Read a frame-times CSV and rank it like the JS proof gate.
/// Skips blank lines, needs a `wall_ms` column, rejects empty, missing
/// column, bad cell, and zero rows. Ranks with ceil so Rust matches JS.
#[must_use]
pub fn frame_csv_verdict(csv_text: &str) -> FrameCsvOutcome {
    let bar_ms = PRESENT_P95_BUDGET_MS;
    let lines: Vec<&str> = csv_text.split(['\n', '\r']).collect();
    let Some(header_at) = frame_header_at(&lines) else {
        return frame_fail(0, "[FAIL] frame times: empty csv");
    };
    let Some(wall_index) = frame_wall_index(lines[header_at]) else {
        return frame_fail(0, "[FAIL] frame times: missing wall_ms column");
    };
    let mut values = match frame_wall_values(&lines, header_at, wall_index) {
        Ok(values) => values,
        Err(outcome) => return outcome,
    };
    if values.is_empty() {
        return frame_fail(0, "[FAIL] frame times: zero data rows");
    }
    values.sort_by(|a, b| a.total_cmp(b));
    let (p50, p95) = frame_rank(&values);
    let ok = p95 <= bar_ms;
    let tag = if ok { "PASS" } else { "FAIL" };
    let n = values.len();
    let frames = u32::try_from(n).unwrap_or(u32::MAX);
    FrameCsvOutcome {
        ok,
        frames,
        p50_ms: Some(p50),
        p95_ms: Some(p95),
        line: format!(
            "[{tag}] frame times: frames={n} p50={p50:.3} ms p95={p95:.3} ms (bar {bar_ms} ms)"
        ),
    }
}

/// First non-blank line holds the header; blanks never count as rows.
fn frame_header_at(lines: &[&str]) -> Option<usize> {
    lines.iter().position(|line| !line.trim().is_empty())
}

/// Column holding `wall_ms` in a trimmed header line.
fn frame_wall_index(header_line: &str) -> Option<usize> {
    header_line
        .split(',')
        .map(str::trim)
        .position(|cell| cell == "wall_ms")
}

/// Wall samples after the header; blank lines skip, bad cells refuse.
/// Returns the FAIL outcome for the first bad row, mirroring the JS row count.
fn frame_wall_values(
    lines: &[&str],
    header_at: usize,
    wall_index: usize,
) -> Result<Vec<f64>, FrameCsvOutcome> {
    let mut values: Vec<f64> = Vec::new();
    for line in lines.iter().skip(header_at + 1) {
        if line.trim().is_empty() {
            continue;
        }
        let cell = line.split(',').nth(wall_index).map_or("", str::trim);
        let parsed = if cell.is_empty() {
            None
        } else {
            cell.parse::<f64>().ok()
        };
        match parsed {
            Some(value) if value.is_finite() && value >= 0.0 => values.push(value),
            _ => {
                let row = values.len() + 1;
                let frames = u32::try_from(values.len()).unwrap_or(u32::MAX);
                return Err(frame_fail(
                    frames,
                    &format!("[FAIL] frame times: bad wall_ms value on row {row}"),
                ));
            }
        }
    }
    Ok(values)
}

/// Ceil-ranked p50/p95 over ascending values, integer math only.
/// p50 is `ceil(n/2)-1`, p95 is `ceil(19n/20)-1`, so Rust matches the JS gate
/// without float casts. Caller holds at least one value.
fn frame_rank(sorted: &[f64]) -> (f64, f64) {
    let n = sorted.len();
    let p50_at = (n.saturating_sub(1)) / 2;
    let p95_at = (n.saturating_mul(19).saturating_add(19) / 20)
        .saturating_sub(1)
        .min(n - 1);
    (sorted[p50_at], sorted[p95_at])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delayed_rows(
        count: u32,
        sim_frames: u64,
        present_frames: u64,
    ) -> (Vec<InputEvent>, Vec<SimEffect>, Vec<FramePresent>) {
        let (events, _, _) = fixture_3f(11, count);
        let effects = events
            .iter()
            .map(|event| SimEffect {
                id: event.id,
                tick: event.tick + sim_frames,
            })
            .collect();
        let presents = events
            .iter()
            .map(|event| FramePresent {
                id: event.id,
                frame: event.tick + present_frames,
            })
            .collect();
        (events, effects, presents)
    }

    #[test]
    fn three_frame_delay_detected() {
        let (events, effects, presents) = fixture_3f(7, 31);
        let report = measure(&events, &effects, &presents).expect("fixture rows join");
        assert_eq!(report.events, 31);
        // 1 frame to sim effect, 3 frames to present, exactly at 60 Hz.
        assert_eq!(report.sim_p50_ms, 1.0 * 1000.0 / 60.0);
        assert_eq!(report.sim_p95_ms, 1.0 * 1000.0 / 60.0);
        assert_eq!(report.present_p50_ms, 50.0);
        assert_eq!(report.present_p95_ms, 50.0);
        check_p2_bar(&report).expect("3 frames land exactly on the P2 bar");
        let summary = report.summary();
        assert!(summary.contains("input_latency events=31"), "{summary}");
        assert!(summary.contains("present_p95=50.000ms"), "{summary}");
        assert!(summary.contains("verdict=pass"), "{summary}");
    }

    #[test]
    fn headless_report_catches_seeded_3f_and_marks_gpu_pending() {
        let conditions = HeadlessConditions::current(0, HEADLESS_SAMPLES);
        let text = headless_report(&conditions).expect("seeded headless run measures");
        // Seeded +3-frame delay detected exactly: 1 frame to sim, 3 to present.
        assert!(text.contains("present_p95=50.000ms"), "{text}");
        assert!(text.contains("sim_p95=16.667ms"), "{text}");
        assert!(text.contains("verdict=pass"), "{text}");
        // Conditions travel with the numbers; reference GPU stays PENDING.
        assert!(text.contains("machine="), "{text}");
        assert!(text.contains("warmup=0"), "{text}");
        assert!(
            text.contains(&format!("samples={HEADLESS_SAMPLES}")),
            "{text}"
        );
        assert!(text.contains("reference_gpu"), "{text}");
        assert!(text.contains("PENDING"), "{text}");
        #[cfg(debug_assertions)]
        assert!(text.contains("profile=debug"), "{text}");
        #[cfg(not(debug_assertions))]
        assert!(text.contains("profile=release"), "{text}");
        // Numbers land in the test log for the perf report (`-- --nocapture`).
        println!("{text}");
    }

    #[test]
    fn headless_report_refuses_zero_samples() {
        let conditions = HeadlessConditions::current(0, 0);
        assert_eq!(headless_report(&conditions), Err(LatencyError::EmptyInput));
    }

    #[test]
    fn same_seed_same_report() {
        let first = fixture_3f(99, 25);
        let second = fixture_3f(99, 25);
        assert_eq!(first, second, "seeded fixture replays exactly");
        let left = measure(&first.0, &first.1, &first.2).expect("first measures");
        let right = measure(&second.0, &second.1, &second.2).expect("second measures");
        assert_eq!(left, right, "same rows give the same report");
    }

    #[test]
    fn input_order_never_matters() {
        let (mut events, effects, presents) = fixture_3f(5, 20);
        events.reverse();
        let report = measure(&events, &effects, &presents).expect("shuffled rows join");
        assert_eq!(report.present_p95_ms, 50.0);
    }

    #[test]
    fn four_frame_delay_blows_the_p2_bar() {
        let (events, effects, presents) = delayed_rows(25, 1, 4);
        let report = measure(&events, &effects, &presents).expect("rows join");
        assert_eq!(report.present_p95_ms, 4.0 * 1000.0 / 60.0);
        let error = check_p2_bar(&report).expect_err("4 frames blow the 50 ms bar");
        assert!(
            matches!(error, LatencyError::BudgetBlown { .. }),
            "got {error:?}"
        );
        assert!(error.to_string().contains("p95"), "{error}");
        assert!(error.to_string().contains("50"), "{error}");
        assert!(
            report.summary().contains("verdict=FAIL"),
            "{}",
            report.summary()
        );
    }

    #[test]
    fn neighboring_seeds_do_not_alias() {
        // The legacy seeder forces the low bit, so raw 2k/2k+1 seeds share
        // one stream; the fixture spreads its seed first, keeping them apart.
        let first = fixture_3f(2, 25);
        let second = fixture_3f(3, 25);
        assert_ne!(first.0, second.0, "neighboring seeds spread apart");
        assert_ne!(first.1, second.1, "effect rows spread apart too");
    }

    #[test]
    fn live_summary_detects_constant_delays() {
        let sim_ms = vec![1.0 * 1000.0 / TICK_HZ; 31];
        let present_ms = vec![3.0 * 1000.0 / TICK_HZ; 31];
        let report = summarize_live(&sim_ms, &present_ms).expect("clean samples summarize");
        assert_eq!(report.events, 31);
        assert_eq!(report.sim_p50_ms, 1.0 * 1000.0 / TICK_HZ);
        assert_eq!(report.sim_p95_ms, 1.0 * 1000.0 / TICK_HZ);
        assert_eq!(report.present_p50_ms, 50.0);
        assert_eq!(report.present_p95_ms, 50.0);
        check_p2_bar(&report).expect("3 frames land exactly on the P2 bar");
        assert_eq!(SampleLeg::SimEffect.to_string(), "sim");
        assert_eq!(SampleLeg::Present.to_string(), "present");
    }

    #[test]
    fn live_summary_matches_tick_math_on_varied_delays() {
        // Oracle: identical delays through the tick join and the live summary
        // rank identically, so both report shapes share one convention.
        let frames: Vec<u32> = (0..100u32).map(|i| 10 + (i % 5)).collect();
        let events: Vec<InputEvent> = (0..100u64)
            .map(|id| InputEvent { id, tick: id * 11 })
            .collect();
        let effects: Vec<SimEffect> = events
            .iter()
            .map(|event| SimEffect {
                id: event.id,
                tick: event.tick + 1,
            })
            .collect();
        let presents: Vec<FramePresent> = events
            .iter()
            .zip(frames.iter())
            .map(|(event, &frame)| FramePresent {
                id: event.id,
                frame: event.tick + u64::from(frame),
            })
            .collect();
        let ticked = measure(&events, &effects, &presents).expect("rows join");
        let sim_ms = vec![1.0 * 1000.0 / TICK_HZ; 100];
        let present_ms: Vec<f64> = frames
            .iter()
            .map(|&frame| f64::from(frame) * 1000.0 / TICK_HZ)
            .collect();
        let live = summarize_live(&sim_ms, &present_ms).expect("same delays summarize");
        assert_eq!(live, ticked, "tick join and live summary rank alike");
        assert_eq!(live.present_p50_ms, 12.0 * 1000.0 / TICK_HZ);
        assert_eq!(live.present_p95_ms, 14.0 * 1000.0 / TICK_HZ);
    }

    #[test]
    fn live_summary_rejects_bad_rows() {
        let good_sim = vec![5.0; 4];
        let good_present = vec![20.0; 4];
        assert_eq!(summarize_live(&[], &[]), Err(LatencyError::EmptyInput));
        assert_eq!(
            summarize_live(&good_sim, &good_present[..3]),
            Err(LatencyError::MismatchedSamples { sim: 4, present: 3 })
        );
        assert_eq!(
            summarize_live(&good_sim[..0], &good_present[..2]),
            Err(LatencyError::MismatchedSamples { sim: 0, present: 2 })
        );
        let mut nan = good_sim.clone();
        nan[1] = f64::NAN;
        assert!(
            matches!(
                summarize_live(&nan, &good_present),
                Err(LatencyError::BadSample {
                    leg: SampleLeg::SimEffect,
                    index: 1,
                    ..
                })
            ),
            "NaN sim sample names its leg and index"
        );
        let mut negative = good_present.clone();
        negative[2] = -1.0;
        assert_eq!(
            summarize_live(&good_sim, &negative),
            Err(LatencyError::BadSample {
                leg: SampleLeg::Present,
                index: 2,
                value_ms: -1.0
            })
        );
        let mut infinite = good_sim.clone();
        infinite[0] = f64::INFINITY;
        assert!(
            matches!(
                summarize_live(&infinite, &good_present),
                Err(LatencyError::BadSample { .. })
            ),
            "infinite samples never rank"
        );
        let mut early = good_present.clone();
        early[3] = 2.0;
        assert_eq!(
            summarize_live(&good_sim, &early),
            Err(LatencyError::PresentBeforeSim {
                index: 3,
                sim_ms: 5.0,
                present_ms: 2.0
            })
        );
    }

    #[test]
    fn live_event_count_holds_u32() {
        assert_eq!(super::event_count(31), Ok(31));
        // 64-bit harness: one past the counter holds nothing.
        assert_eq!(
            super::event_count(usize::MAX),
            Err(LatencyError::TooManySamples { count: usize::MAX })
        );
    }

    fn gpu_conditions() -> GpuConditions {
        GpuConditions {
            adapter: "NVIDIA GeForce RTX 3050 6GB Laptop GPU".to_owned(),
            driver: "610.62".to_owned(),
            backend: "vulkan".to_owned(),
            cpu: "i5-13420H".to_owned(),
            power_mode: "Balanced".to_owned(),
            clock_mode: "dynamic".to_owned(),
            resolution: "1920x1080".to_owned(),
            vsync: "on".to_owned(),
            profile: "release".to_owned(),
            scene: "arena".to_owned(),
            build: "local window build".to_owned(),
            load: "idle, single run".to_owned(),
            warmup: 120,
            samples: 31,
        }
    }

    #[test]
    fn gpu_report_carries_conditions_and_verdict() {
        let conditions = gpu_conditions();
        let report = summarize_live(&[1.0 * 1000.0 / TICK_HZ; 31], &[50.0; 31])
            .expect("clean samples summarize");
        let text = conditions.report(&report);
        for token in [
            "input_latency_gpu",
            "adapter=",
            "610.62",
            "power=",
            "clocks=",
            "load=",
            "present_p95=50.000ms",
            "verdict=pass",
        ] {
            assert!(text.contains(token), "{text}");
        }
        // A 4-frame present fails the feel bar in the same report shape.
        let slow = summarize_live(&[1.0 * 1000.0 / TICK_HZ; 25], &[4.0 * 1000.0 / TICK_HZ; 25])
            .expect("slow samples summarize");
        let slow_text = conditions.report(&slow);
        assert!(slow_text.contains("verdict=FAIL"), "{slow_text}");
        assert!(check_p2_bar(&slow).is_err(), "4 frames blow the bar");
        // Measured rows land in the test log for the perf report (`-- --nocapture`).
        println!("{text}");
    }

    #[test]
    fn gpu_pending_names_its_blocker() {
        let conditions = gpu_conditions();
        let text = conditions
            .pending(
                "staged input-to-present timestamps missing in the arena window loop; \
                      no idle release run on the reference graphics card this wave",
            )
            .expect("named blocker prints");
        assert!(text.contains("reference_gpu"), "{text}");
        assert!(text.contains("PENDING"), "{text}");
        assert!(
            text.contains("staged input-to-present timestamps"),
            "{text}"
        );
        assert!(text.contains("adapter="), "{text}");
        assert!(text.contains("load="), "{text}");
        assert_eq!(conditions.pending(""), Err(LatencyError::EmptyBlocker));
        assert_eq!(conditions.pending("   "), Err(LatencyError::EmptyBlocker));
        // The pending row lands in the test log beside the measured rows.
        println!("{text}");
    }

    #[test]
    fn bad_rows_fail_loudly() {
        let (events, effects, presents) = fixture_3f(3, 5);
        assert_eq!(
            measure(&[], &effects, &presents),
            Err(LatencyError::EmptyInput)
        );
        assert_eq!(
            measure(&events, &effects[..4], &presents),
            Err(LatencyError::MissingEffect { id: 4 })
        );
        assert_eq!(
            measure(&events, &effects, &presents[..4]),
            Err(LatencyError::MissingPresent { id: 4 })
        );
        let mut dup_events = events.clone();
        dup_events.push(events[0]);
        assert_eq!(
            measure(&dup_events, &effects, &presents),
            Err(LatencyError::DuplicateEvent { id: 0 })
        );
        let mut dup_effects = effects.clone();
        dup_effects.push(effects[1]);
        assert_eq!(
            measure(&events, &dup_effects, &presents),
            Err(LatencyError::DuplicateEffect { id: 1 })
        );
        let mut dup_presents = presents.clone();
        dup_presents.push(presents[2]);
        assert_eq!(
            measure(&events, &effects, &dup_presents),
            Err(LatencyError::DuplicatePresent { id: 2 })
        );
        let mut early_effects = effects.clone();
        early_effects[3].tick = events[3].tick - 1;
        assert_eq!(
            measure(&events, &early_effects, &presents),
            Err(LatencyError::EffectBeforeEvent {
                id: 3,
                event_tick: events[3].tick,
                effect_tick: events[3].tick - 1,
            })
        );
        let mut early_presents = presents.clone();
        early_presents[3].frame = events[3].tick - 1;
        assert_eq!(
            measure(&events, &effects, &early_presents),
            Err(LatencyError::PresentBeforeEvent {
                id: 3,
                event_tick: events[3].tick,
                frame: events[3].tick - 1,
            })
        );
    }

    #[test]
    fn errors_name_themselves() {
        let cases: &[LatencyError] = &[
            LatencyError::EmptyInput,
            LatencyError::DuplicateEvent { id: 1 },
            LatencyError::DuplicateEffect { id: 1 },
            LatencyError::DuplicatePresent { id: 1 },
            LatencyError::MissingEffect { id: 1 },
            LatencyError::MissingPresent { id: 1 },
            LatencyError::EffectBeforeEvent {
                id: 1,
                event_tick: 9,
                effect_tick: 8,
            },
            LatencyError::PresentBeforeEvent {
                id: 1,
                event_tick: 9,
                frame: 8,
            },
            LatencyError::BudgetBlown {
                actual_ms: 66.667,
                budget_ms: 50.0,
            },
            LatencyError::MismatchedSamples { sim: 4, present: 3 },
            LatencyError::BadSample {
                leg: SampleLeg::SimEffect,
                index: 1,
                value_ms: f64::NAN,
            },
            LatencyError::PresentBeforeSim {
                index: 1,
                sim_ms: 5.0,
                present_ms: 2.0,
            },
            LatencyError::TooManySamples { count: usize::MAX },
            LatencyError::EmptyBlocker,
        ];
        for error in cases {
            assert!(!error.to_string().is_empty(), "{error:?} explains itself");
            let _: &dyn std::error::Error = error;
        }
    }

    #[test]
    fn wall_meter_stays_presentation_only() {
        let meter = WallMeter::start();
        let first = meter.elapsed_ms();
        let second = meter.elapsed_ms();
        assert!(first.is_finite() && first >= 0.0);
        assert!(second >= first, "wall clock never runs backwards");
        // The sim path never reads a clock: reports carry only tick math.
        let (events, effects, presents) = fixture_3f(13, 5);
        let report = measure(&events, &effects, &presents).expect("fixture rows join");
        assert_eq!(report.present_p50_ms, 50.0);
    }

    #[test]
    fn same_seed_live_report_x2_wall_meter_isolated() {
        // Behavior: same seed gives the same tick and live reports twice,
        // and the WallMeter never touches either path; the timed-smoke
        // wiring stages wall samples via `summarize_live`.
        // Failure: live ranking diverges per run or wall time leaks into sim.
        let run_once = || {
            let (events, effects, presents) = fixture_3f(99, 25);
            let ticked = measure(&events, &effects, &presents).expect("seeded rows join");
            let sim_ms: Vec<f64> = events
                .iter()
                .zip(effects.iter())
                .map(|(event, effect)| {
                    let frames = u32::try_from(effect.tick - event.tick).unwrap_or(u32::MAX);
                    f64::from(frames) * 1000.0 / TICK_HZ
                })
                .collect();
            let present_ms: Vec<f64> = events
                .iter()
                .zip(presents.iter())
                .map(|(event, present)| {
                    let frames = u32::try_from(present.frame - event.tick).unwrap_or(u32::MAX);
                    f64::from(frames) * 1000.0 / TICK_HZ
                })
                .collect();
            let live = summarize_live(&sim_ms, &present_ms).expect("seeded samples summarize");
            (ticked, live)
        };
        let (ticked_first, live_first) = run_once();
        let (ticked_second, live_second) = run_once();
        assert_eq!(
            ticked_first, ticked_second,
            "same seed repeats the tick report"
        );
        assert_eq!(live_first, live_second, "same seed repeats the live report");
        assert_eq!(
            live_first, ticked_first,
            "live ranks like tick math on the same seed"
        );
        assert_eq!(
            ticked_first.summary(),
            ticked_second.summary(),
            "tick summaries repeat byte-identically"
        );
        assert_eq!(
            live_first.summary(),
            live_second.summary(),
            "live summaries repeat byte-identically"
        );
        // WallMeter stays presentation-only: reading it never moves either report.
        let meter = WallMeter::start();
        let elapsed = meter.elapsed_ms();
        assert!(
            elapsed.is_finite() && elapsed >= 0.0,
            "wall read stays evidence-only"
        );
        let (ticked_after, live_after) = run_once();
        assert_eq!(
            ticked_after, ticked_first,
            "wall reads never touch tick math"
        );
        assert_eq!(live_after, live_first, "wall reads never touch live math");
        assert_eq!(
            live_after.summary(),
            live_first.summary(),
            "summaries ignore wall reads"
        );
        // Staged p50/p95 math holds on the seeded 3-frame shape.
        assert_eq!(live_first.sim_p95_ms, 1.0 * 1000.0 / TICK_HZ);
        assert_eq!(live_first.present_p95_ms, 50.0);
    }

    #[test]
    fn window_log_stages_measured_legs_for_the_gpu_report() {
        let mut log = WindowLatencyLog::new();
        assert!(log.is_empty());
        assert_eq!(log.len(), 0);
        let sim_ms = vec![1.0 * 1000.0 / TICK_HZ; 31];
        let present_ms = vec![50.0; 31];
        for (&sim, &present) in sim_ms.iter().zip(present_ms.iter()) {
            log.stage(sim, present).expect("clean pair stages");
        }
        assert_eq!(log.len(), 31);
        // Oracle: staged legs rank exactly like the direct live summary.
        let direct = summarize_live(&sim_ms, &present_ms).expect("same pairs summarize");
        let staged = log.summarize().expect("staged legs summarize");
        assert_eq!(staged, direct);
        assert_eq!(staged.present_p95_ms, 50.0);
        // The staged report prints the measured GPU block, PENDING-free.
        let text = gpu_conditions().report(&staged);
        assert!(text.contains("input_latency_gpu"), "{text}");
        assert!(text.contains("adapter="), "{text}");
        assert!(text.contains("present_p95=50.000ms"), "{text}");
        assert!(text.contains("verdict=pass"), "{text}");
        assert!(!text.contains("PENDING"), "{text}");
        println!("{text}");
        // Clear returns the log to fresh: stage again and re-summarize.
        log.clear();
        assert!(log.is_empty());
        log.stage(1.0 * 1000.0 / TICK_HZ, 50.0)
            .expect("cleared log stages again");
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn window_log_staged_report_prints_measured_gpu_block() {
        let mut log = WindowLatencyLog::new();
        // Same legs as the staged summary test: one frame to sim, 50 ms to present.
        for _ in 0..31 {
            log.stage(1.0 * 1000.0 / TICK_HZ, 50.0)
                .expect("clean pair stages");
        }
        let text = log
            .staged_report(&gpu_conditions())
            .expect("staged legs print");
        assert!(text.contains("input_latency_gpu"), "{text}");
        assert!(text.contains("adapter="), "{text}");
        assert!(text.contains("present_p95=50.000ms"), "{text}");
        assert!(text.contains("verdict=pass"), "{text}");
        assert!(!text.contains("PENDING"), "{text}");
        println!("{text}");
        // Empty never prints zeros: the pilot sees the missing batch loudly.
        assert_eq!(
            WindowLatencyLog::new().staged_report(&gpu_conditions()),
            Err(LatencyError::EmptyInput)
        );
    }

    #[test]
    fn window_log_rejects_bad_stage_time() {
        let mut log = WindowLatencyLog::new();
        log.stage(5.0, 20.0).expect("clean pair stages");
        assert!(
            matches!(
                log.stage(f64::NAN, 20.0),
                Err(LatencyError::BadSample {
                    leg: SampleLeg::SimEffect,
                    index: 1,
                    ..
                })
            ),
            "NaN sim names its leg and index at stage time"
        );
        assert_eq!(
            log.stage(5.0, -1.0),
            Err(LatencyError::BadSample {
                leg: SampleLeg::Present,
                index: 1,
                value_ms: -1.0
            })
        );
        assert_eq!(
            log.stage(5.0, 2.0),
            Err(LatencyError::PresentBeforeSim {
                index: 1,
                sim_ms: 5.0,
                present_ms: 2.0
            })
        );
        // Rejections stage nothing: the one clean pair still summarizes alone.
        assert_eq!(log.len(), 1);
        let staged = log.summarize().expect("clean pair survives rejections");
        assert_eq!(staged.events, 1);
        assert_eq!(
            WindowLatencyLog::new().summarize(),
            Err(LatencyError::EmptyInput)
        );
    }

    #[test]
    fn window_log_holds_its_ceiling() {
        let mut log = WindowLatencyLog::new();
        for _ in 0..MAX_STAGED_EVENTS {
            log.stage(1.0 * 1000.0 / TICK_HZ, 50.0)
                .expect("pairs stage to the ceiling");
        }
        assert_eq!(log.len(), MAX_STAGED_EVENTS);
        assert!(
            matches!(
                log.stage(1.0 * 1000.0 / TICK_HZ, 50.0),
                Err(LatencyError::TooManySamples { .. })
            ),
            "one past the ceiling fails loudly"
        );
        let staged = log.summarize().expect("full log still summarizes");
        assert_eq!(staged.present_p95_ms, 50.0);
        log.clear();
        assert!(log.is_empty());
        log.stage(1.0 * 1000.0 / TICK_HZ, 50.0)
            .expect("cleared ceiling stages again");
    }

    #[test]
    fn frame_csv_round_trip_ranks_like_js_gate() {
        // Behavior: the writer emits the proof-gate CSV and the verdict ranks
        // it with the JS ceil convention. Failure: a p95 past 50 ms passes.
        let samples = vec![10.0, 16.667, 16.667, 20.0];
        let csv = frame_times_csv(&samples);
        assert!(csv.starts_with("frame,wall_ms\n"), "{csv}");
        assert!(csv.contains("0,10.000"), "{csv}");
        let verdict = frame_csv_verdict(&csv);
        assert!(verdict.ok, "{}", verdict.line);
        assert_eq!(verdict.frames, 4);
        // ceil(0.5*4)-1 = 1 -> 16.667; ceil(0.95*4)-1 = 3 -> 20.0.
        assert_eq!(verdict.p50_ms, Some(16.667));
        assert_eq!(verdict.p95_ms, Some(20.0));
        assert!(verdict.line.starts_with("[PASS]"), "{}", verdict.line);
        assert!(verdict.line.contains("p50=16.667"), "{}", verdict.line);
        assert!(verdict.line.contains("p95=20.000"), "{}", verdict.line);
        // Same ranking twice: the gate repeats byte-identically.
        assert_eq!(verdict, frame_csv_verdict(&csv));
        // A 60 ms tail fails the 50 ms bar exactly like the JS gate.
        // n=10: ceil(0.95*10)-1 = 9, so one slow tail decides p95.
        let mut slow_tail = vec![16.7; 9];
        slow_tail.push(60.0);
        let slow_csv = frame_times_csv(&slow_tail);
        let slow_verdict = frame_csv_verdict(&slow_csv);
        assert_eq!(slow_verdict.frames, 10);
        assert_eq!(slow_verdict.p95_ms, Some(60.0));
        assert!(!slow_verdict.ok, "{}", slow_verdict.line);
        assert!(
            slow_verdict.line.starts_with("[FAIL]"),
            "{}",
            slow_verdict.line
        );
        assert!(
            slow_verdict.line.contains("p95=60.000"),
            "{}",
            slow_verdict.line
        );
    }

    #[test]
    fn frame_csv_bad_shapes_fail_with_named_lines() {
        // Each bad shape refuses with the JS gate's named FAIL line.
        for (csv, line) in [
            ("", "[FAIL] frame times: empty csv"),
            ("  \n \r\n", "[FAIL] frame times: empty csv"),
            (
                "frame,other\n0,1.0\n",
                "[FAIL] frame times: missing wall_ms column",
            ),
            (
                "frame,wall_ms\n0,16.7\n1,abc\n",
                "[FAIL] frame times: bad wall_ms value on row 2",
            ),
            (
                "frame,wall_ms\n0,-1.0\n",
                "[FAIL] frame times: bad wall_ms value on row 1",
            ),
            (
                "frame,wall_ms\n0,NaN\n",
                "[FAIL] frame times: bad wall_ms value on row 1",
            ),
            ("frame,wall_ms\n", "[FAIL] frame times: zero data rows"),
        ] {
            let verdict = frame_csv_verdict(csv);
            assert!(!verdict.ok, "{}", verdict.line);
            assert_eq!(verdict.line, line);
        }
        let bad = frame_csv_verdict("frame,wall_ms\n0,16.7\n1,abc\n");
        assert_eq!(bad.frames, 1);
    }
}
