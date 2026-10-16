//! Input latency meter: event tick to first sim effect and first present.
//! Consumer: the proof `--perf` gate and the latency evidence read
//! `LatencyReport` (`summary` lines drop into the perf report).
//! Units: ticks/frames on the shared 60 Hz clock, milliseconds in reports.
//! Determinism: `measure` is integer tick math only, no wall clock, sorted by
//! id so input order never matters; wall time lives in `summarize_live`
//! inputs only, and never enters a tick hash.
//! Errors: typed `LatencyError`.
//! Budget: the bar is event-to-present p95 <= 50 ms (3 frames at 60 Hz).

use engine_core::{splitmix64, Rng};
use std::fmt::{self, Display, Formatter};

/// Shared clock: simulation ticks and presented frames both run at 60 Hz.
/// Single owner of the rate is `engine_core::TICK_HZ`.
pub const TICK_HZ: f64 = engine_core::TICK_HZ as f64;
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
/// Differs deliberately from `fight_cost::percentile_sorted` (integer
/// micros, `len * pct / 100`, `None` on empty or `pct > 100` because that
/// gate needs integer math only).
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

/// Sample count for the seeded 3-frame fixture shape; `feel` runs this many
/// events through `fixture_3f`, and the `three_frame_delay_detected` proof
/// pins the same count.
pub const HEADLESS_SAMPLES: u32 = 31;

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

/// Frame-times CSV writing and the proof-gate verdict live in the game crate
/// (`frame_times_csv_row` plus the window timed-smoke writer): the windowed
/// smoke path owns its own CSV shape, so this module keeps no copy.
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
        ];
        for error in cases {
            assert!(!error.to_string().is_empty(), "{error:?} explains itself");
            let _: &dyn std::error::Error = error;
        }
    }
}
