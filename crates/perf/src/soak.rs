//! Hour soak: the long-run stability check for the frame-time gate.
//! Consumer: nightly CI runs [`SoakConfig::hour`] and prints
//! [`SoakReport::summary`] into the perf report.
//! Units: milliseconds, frames at 60 Hz. Determinism: seeded samples, so the
//! same config gives the same report on every machine (no wall clock).
//! Gate: ranking and the P2 bar stay in `latency` (`summarize_live`,
//! `check_p2_bar`); this module adds only the drift read between halves.
//! Errors: typed [`SoakError`], wrapping [`LatencyError`](crate::latency::LatencyError).

use crate::latency::{self, LatencyError, LatencyReport};
use engine_core::{splitmix64, Rng};
use std::fmt::{self, Display, Formatter};

/// Seed behind every soak sample stream; EG41's own number.
pub const SOAK_SEED: u64 = 41;
/// Frames in the full hour soak: 60 Hz times 3,600 seconds.
pub const SOAK_HOUR_FRAMES: u64 = 216_000;
/// Drift allowance: the second half p95 may sit above the first half p95 by
/// this much before the soak calls the run unstable.
pub const SOAK_DRIFT_TOLERANCE_MS: f64 = 5.0;

/// How long a soak runs and on which seed. [`SoakConfig::hour`] is the
/// nightly shape; [`SoakConfig::short`] is the same logic on fewer frames
/// for fast tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoakConfig {
    /// Frames sampled in the run.
    pub frames: u64,
    /// Seed behind the sample stream.
    pub seed: u64,
}

impl SoakConfig {
    /// The nightly hour soak: 216,000 frames on the packaged seed.
    #[must_use]
    pub fn hour() -> Self {
        Self {
            frames: SOAK_HOUR_FRAMES,
            seed: SOAK_SEED,
        }
    }

    /// A short soak with the same logic for fast tests.
    #[must_use]
    pub fn short(frames: u64) -> Self {
        Self {
            frames,
            seed: SOAK_SEED,
        }
    }
}

/// Soak failure: too short to split, or the shared latency leg refused.
#[derive(Clone, Debug, PartialEq)]
pub enum SoakError {
    /// Fewer than 2 frames: halves need one sample each.
    TooShort {
        /// Frames offered.
        frames: usize,
    },
    /// The shared latency leg refused the samples.
    Latency(LatencyError),
}

impl Display for SoakError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { frames } => {
                write!(f, "soak: {frames} frames cannot split into halves")
            }
            Self::Latency(error) => write!(f, "soak: {error}"),
        }
    }
}

impl std::error::Error for SoakError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::TooShort { .. } => None,
            Self::Latency(error) => Some(error),
        }
    }
}

impl From<LatencyError> for SoakError {
    fn from(error: LatencyError) -> Self {
        Self::Latency(error)
    }
}

/// Deterministic frame-time stream for a config: healthy frames at 16..20 ms
/// on the config seed. Same config gives the same stream twice.
#[must_use]
pub fn samples(config: &SoakConfig) -> Vec<f64> {
    let mut rng = Rng::new(splitmix64(config.seed));
    (0..config.frames)
        .map(|_| 16.0 + f64::from(u32::try_from(rng.next_u64() % 5).unwrap_or(u32::MAX)))
        .collect()
}

/// Soak report: whole-run numbers plus each half p95, so drift reads off the
/// page. `pass` needs both the P2 bar and a drift inside tolerance.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoakReport {
    /// The shared latency report over the whole run.
    pub report: LatencyReport,
    /// First-half present p95 in ms.
    pub first_half_p95_ms: f64,
    /// Second-half present p95 in ms.
    pub second_half_p95_ms: f64,
    /// True while the run clears the P2 bar without drifting.
    pub pass: bool,
}

impl SoakReport {
    /// Second-half p95 minus first-half p95; positive means the run slowed.
    #[must_use]
    pub fn drift_ms(&self) -> f64 {
        self.second_half_p95_ms - self.first_half_p95_ms
    }

    /// One stable block for the perf report: frame count, whole-run p50/p95,
    /// both halves, drift and verdict. Numbers vary; names never do.
    #[must_use]
    pub fn summary(&self) -> String {
        let verdict = if self.pass { "pass" } else { "FAIL" };
        format!(
            "perf_soak frames={} present_p50={:.3}ms present_p95={:.3}ms \
             first_half_p95={:.3}ms second_half_p95={:.3}ms drift={:+.3}ms \
             p95_budget={:.1}ms verdict={verdict}\n",
            self.report.events,
            self.report.present_p50_ms,
            self.report.present_p95_ms,
            self.first_half_p95_ms,
            self.second_half_p95_ms,
            self.drift_ms(),
            latency::PRESENT_P95_BUDGET_MS,
        )
    }
}

/// Summarize one leg through the shared ranker with a zero sim leg, so the
/// present leg is the whole story (same convention as the smoke check).
fn summarize_leg(present_ms: &[f64]) -> Result<LatencyReport, LatencyError> {
    let sim_ms = vec![0.0; present_ms.len()];
    latency::summarize_live(&sim_ms, present_ms)
}

/// Check caller-supplied frame times: whole-run P2 bar plus a drift read
/// between halves. Pure over its inputs: same samples give the same report.
///
/// # Errors
///
/// [`SoakError::TooShort`] on fewer than 2 frames,
/// [`SoakError::Latency`] when the shared leg refuses the samples.
pub fn check(frame_ms: &[f64]) -> Result<SoakReport, SoakError> {
    if frame_ms.len() < 2 {
        return Err(SoakError::TooShort {
            frames: frame_ms.len(),
        });
    }
    let report = summarize_leg(frame_ms)?;
    let half = frame_ms.len() / 2;
    let first = summarize_leg(&frame_ms[..half])?;
    let second = summarize_leg(&frame_ms[half..])?;
    let drift = second.present_p95_ms - first.present_p95_ms;
    let pass = latency::check_p2_bar(&report).is_ok() && drift <= SOAK_DRIFT_TOLERANCE_MS;
    Ok(SoakReport {
        report,
        first_half_p95_ms: first.present_p95_ms,
        second_half_p95_ms: second.present_p95_ms,
        pass,
    })
}

/// Run a soak config through [`check`]. Same config gives the same report.
///
/// # Errors
///
/// [`SoakError::TooShort`] when the config holds fewer than 2 frames,
/// [`SoakError::Latency`] when the shared leg refuses the stream.
pub fn run(config: &SoakConfig) -> Result<SoakReport, SoakError> {
    check(&samples(config))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_soak_passes_with_margin() {
        let report = run(&SoakConfig::short(600)).expect("seeded soak measures");
        assert!(report.pass, "packaged stream is healthy");
        assert_eq!(report.report.events, 600);
        assert!(
            report.report.present_p95_ms < latency::PRESENT_P95_BUDGET_MS,
            "soak clears the bar by margin, got {}",
            report.report.present_p95_ms
        );
        assert_eq!(report.drift_ms(), 0.0, "steady stream never drifts");
        println!("{}", report.summary());
    }

    #[test]
    fn same_config_same_report_x2() {
        let config = SoakConfig::short(600);
        let first = run(&config).expect("first run measures");
        let second = run(&config).expect("second run measures");
        assert_eq!(first, second, "same config repeats exactly");
    }

    #[test]
    fn hour_config_names_216000_frames() {
        assert_eq!(SoakConfig::hour().frames, 216_000);
        assert_eq!(SoakConfig::hour().frames, SOAK_HOUR_FRAMES);
    }

    #[test]
    fn summary_names_halves_and_verdict() {
        let report = run(&SoakConfig::short(600)).expect("seeded soak measures");
        let text = report.summary();
        for token in [
            "perf_soak",
            "frames=600",
            "first_half_p95=",
            "second_half_p95=",
            "drift=",
            "verdict=pass",
        ] {
            assert!(text.contains(token), "{text}");
        }
    }

    #[test]
    fn drift_trips_the_gate_inside_budget() {
        // Both halves clear the 50 ms bar, but the second half runs 24 ms
        // slower: stable gate, unstable soak.
        let mut frames = vec![16.0; 300];
        frames.extend(vec![40.0; 300]);
        let report = check(&frames).expect("crafted halves still rank");
        assert!(!report.pass, "24 ms drift fails the soak");
        assert_eq!(report.drift_ms(), 24.0);
        assert!(
            report.summary().contains("verdict=FAIL"),
            "{}",
            report.summary()
        );
        // The shared bar alone would pass: drift is the soak's own read.
        assert!(
            latency::check_p2_bar(&report.report).is_ok(),
            "both halves sit inside the P2 bar"
        );
    }

    #[test]
    fn over_budget_trips_the_gate() {
        let slow = vec![60.0; 120];
        let report = check(&slow).expect("slow frames still rank");
        assert!(!report.pass, "60 ms frames fail the 50 ms bar");
        assert!(
            report.summary().contains("verdict=FAIL"),
            "{}",
            report.summary()
        );
    }

    #[test]
    fn short_streams_fail_loudly() {
        assert_eq!(check(&[]), Err(SoakError::TooShort { frames: 0 }));
        assert_eq!(check(&[16.0]), Err(SoakError::TooShort { frames: 1 }));
        assert_eq!(
            SoakError::from(LatencyError::EmptyInput).to_string(),
            "soak: latency: no input events to measure"
        );
        let _: &dyn std::error::Error = &SoakError::TooShort { frames: 0 };
    }
}
