//! Packaged smoke: the quick pass-path check for the frame-time gate.
//! Consumer: CI and the perf report print [`SmokeReport::summary`].
//! Units: milliseconds. Determinism: seeded samples, so the same seed gives
//! the same report on every machine (no wall clock, no GPU).
//! Gate: the one home is `latency::check_p2_bar` (p95 at or under 50 ms);
//! this module only feeds it packaged samples, never re-ranks or re-bars.
//! Errors: reuses [`LatencyError`](crate::latency::LatencyError).

use crate::latency::{self, LatencyError, LatencyReport};
use engine_core::{splitmix64, Rng};

/// Seed behind the packaged smoke samples; EG41's own number.
pub const SMOKE_SEED: u64 = 41;
/// Frames in one packaged smoke run: 2 seconds at 60 Hz, quick everywhere.
pub const SMOKE_FRAMES: usize = 120;

/// Deterministic frame-time samples: healthy frames at 16..20 ms, seeded so
/// every machine draws the same stream. Same seed and count give the same
/// samples twice; the packaged run always clears the P2 bar by margin.
#[must_use]
pub fn samples(seed: u64, frames: usize) -> Vec<f64> {
    let mut rng = Rng::new(splitmix64(seed));
    (0..frames)
        .map(|_| 16.0 + f64::from(u32::try_from(rng.next_u64() % 5).unwrap_or(u32::MAX)))
        .collect()
}

/// Packaged smoke report: the shared latency numbers plus the gate verdict.
/// `pass` is true exactly when `latency::check_p2_bar` accepts the report.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SmokeReport {
    /// The shared latency report over the packaged samples.
    pub report: LatencyReport,
    /// True while the run clears the P2 bar.
    pub pass: bool,
}

impl SmokeReport {
    /// One stable line for the perf report: frame count, present p50/p95,
    /// the P2 bar and the verdict. Numbers vary per input; names never do.
    #[must_use]
    pub fn summary(&self) -> String {
        let verdict = if self.pass { "pass" } else { "FAIL" };
        format!(
            "perf_smoke frames={} present_p50={:.3}ms present_p95={:.3}ms \
             p95_budget={:.1}ms verdict={verdict}\n",
            self.report.events,
            self.report.present_p50_ms,
            self.report.present_p95_ms,
            latency::PRESENT_P95_BUDGET_MS,
        )
    }
}

/// Check caller-supplied frame times through the shared gate: ranks with
/// [`summarize_live`](latency::summarize_live) (sim leg zero, so the present
/// leg is the whole story) and reads the verdict from
/// [`check_p2_bar`](latency::check_p2_bar).
///
/// # Errors
///
/// Reuses [`LatencyError`]: [`EmptyInput`](LatencyError::EmptyInput) on no
/// frames, [`BadSample`](LatencyError::BadSample) on a NaN, infinite or
/// negative frame.
pub fn check(frame_ms: &[f64]) -> Result<SmokeReport, LatencyError> {
    let sim_ms = vec![0.0; frame_ms.len()];
    let report = latency::summarize_live(&sim_ms, frame_ms)?;
    let pass = latency::check_p2_bar(&report).is_ok();
    Ok(SmokeReport { report, pass })
}

/// Run the packaged smoke: [`SMOKE_FRAMES`] seeded frames through [`check`].
/// Same build gives the same report twice.
///
/// # Errors
///
/// Only [`LatencyError::TooManySamples`] past the u32 event counter; the
/// packaged count (120) never reaches it.
pub fn run() -> Result<SmokeReport, LatencyError> {
    check(&samples(SMOKE_SEED, SMOKE_FRAMES))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packaged_smoke_passes_with_margin() {
        let report = run().expect("packaged samples clear the gate");
        assert!(report.pass, "packaged smoke is the pass path");
        assert_eq!(
            report.report.events,
            u32::try_from(SMOKE_FRAMES).unwrap_or(u32::MAX)
        );
        assert!(
            report.report.present_p95_ms < latency::PRESENT_P95_BUDGET_MS,
            "smoke clears the bar by margin, got {}",
            report.report.present_p95_ms
        );
        println!("{}", report.summary());
    }

    #[test]
    fn same_seed_same_report_x2() {
        let first = run().expect("first run measures");
        let second = run().expect("second run measures");
        assert_eq!(first, second, "packaged smoke repeats exactly");
        assert_eq!(
            samples(SMOKE_SEED, SMOKE_FRAMES),
            samples(SMOKE_SEED, SMOKE_FRAMES),
            "seeded samples replay exactly"
        );
    }

    #[test]
    fn summary_names_frames_and_verdict() {
        let report = run().expect("packaged samples clear the gate");
        let text = report.summary();
        assert!(text.contains("perf_smoke"), "{text}");
        assert!(text.contains("frames=120"), "{text}");
        assert!(text.contains("verdict=pass"), "{text}");
    }

    #[test]
    fn slow_frames_trip_the_gate() {
        let slow = vec![60.0; SMOKE_FRAMES];
        let report = check(&slow).expect("slow frames still rank");
        assert!(!report.pass, "60 ms frames fail the 50 ms bar");
        assert!(
            report.summary().contains("verdict=FAIL"),
            "{}",
            report.summary()
        );
        let error = latency::check_p2_bar(&report.report).expect_err("gate trips");
        assert!(
            matches!(error, LatencyError::BudgetBlown { .. }),
            "got {error:?}"
        );
    }

    #[test]
    fn empty_frames_fail_loudly() {
        assert_eq!(check(&[]), Err(LatencyError::EmptyInput));
    }
}
