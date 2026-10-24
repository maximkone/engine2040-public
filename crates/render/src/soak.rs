//! soak: the mobile soak harness (BG8).
//!
//! A phone runs for a long time: the frame time must hold, the memory must
//! not grow and the device must not fall off a thermal cliff half way
//! through the run. The harness is pure over its inputs: the caller feeds the
//! frame times and the RSS samples, the report prints the p50, the RSS and the
//! cliff flag.
//!
//! Owner: engine-render. Units: milliseconds, KiB, frames at 60 Hz.
//! Determinism: the same samples give the same report on every machine, and
//! the packaged stream (`samples`) is seeded, so a test reads the same
//! numbers everywhere. No wall clock, no tick, no state.
//! Errors: typed `SoakError`.

use engine_core::{splitmix64, Rng};
use std::fmt::{self, Display, Formatter};

/// Seed behind the packaged stream; BG8's own number.
pub const SOAK_SEED: u64 = 8;

/// Frames in a 30-minute soak at 60 Hz.
pub const SOAK_HALF_HOUR_FRAMES: u64 = 108_000;

/// The cliff allowance (ms): the second half's p95 may sit this far above the
/// first half's p95 before the run is called a cliff.
pub const CLIFF_TOLERANCE_MS: f64 = 5.0;

/// How long a soak runs and on which seed. [`SoakConfig::half_hour`] is the
/// packaged shape; [`SoakConfig::short`] is the same logic on fewer frames
/// for fast tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SoakConfig {
    /// Frames sampled in the run.
    pub frames: u64,
    /// Seed behind the sample stream.
    pub seed: u64,
}

impl SoakConfig {
    /// The 30-minute soak: 108,000 frames on the packaged seed.
    #[must_use]
    pub fn half_hour() -> Self {
        Self {
            frames: SOAK_HALF_HOUR_FRAMES,
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

/// Soak failure: too short to split into halves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SoakError {
    /// Fewer than 2 frames: the halves need one sample each.
    TooShort {
        /// Frames offered.
        frames: usize,
    },
}

impl Display for SoakError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort { frames } => {
                write!(f, "soak: {frames} frames cannot split into halves")
            }
        }
    }
}

impl std::error::Error for SoakError {}

/// The memory read of one run: the peak resident set and how far it grew.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rss {
    /// The largest resident set in the run (KiB).
    pub peak_kib: u64,
    /// The last sample less the first, in KiB (negative means it fell).
    pub growth_kib: i64,
}

/// Soak report: the numbers a long run has to hold.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SoakReport {
    /// Frames in the run.
    pub frames: u64,
    /// Median frame time (ms).
    pub p50_ms: f64,
    /// 95th percentile frame time (ms).
    pub p95_ms: f64,
    /// The first half's p95 (ms).
    pub first_half_p95_ms: f64,
    /// The second half's p95 (ms).
    pub second_half_p95_ms: f64,
    /// The memory read, `None` when the run took no RSS sample.
    pub rss: Option<Rss>,
    /// True when the run slowed past the cliff allowance.
    pub cliff: bool,
}

impl SoakReport {
    /// Second-half p95 minus first-half p95; positive means the run slowed.
    #[must_use]
    pub fn drift_ms(&self) -> f64 {
        self.second_half_p95_ms - self.first_half_p95_ms
    }

    /// One block for the run log: the frame count, the p50 and p95 and the
    /// RSS peak and growth. A healthy run prints no cliff line; a run that
    /// fell off the cliff prints one under it.
    #[must_use]
    pub fn summary(&self) -> String {
        let mut text = format!(
            "render_soak frames={} p50={:.3}ms p95={:.3}ms ",
            self.frames, self.p50_ms, self.p95_ms
        );
        match self.rss {
            Some(rss) => text.push_str(&format!(
                "rss_peak={}KiB rss_growth={}KiB\n",
                rss.peak_kib, rss.growth_kib
            )),
            None => text.push_str("rss=none\n"),
        }
        if self.cliff {
            text.push_str(&format!(
                "render_soak_cliff first_half_p95={:.3}ms second_half_p95={:.3}ms drift={:+.3}ms\n",
                self.first_half_p95_ms,
                self.second_half_p95_ms,
                self.drift_ms()
            ));
        }
        text
    }
}

/// The quantile of the sorted samples, in percent (0..=100), nearest rank: the
/// sample at that share of the list.
fn quantile(sorted: &[f64], pct: u32) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let last = sorted.len() - 1;
    let rank = last * usize::try_from(pct).unwrap_or(100) / 100;
    sorted[rank]
}

/// The samples in order, the input left untouched.
fn ordered(samples: &[f64]) -> Vec<f64> {
    let mut copy = samples.to_vec();
    copy.sort_by(f64::total_cmp);
    copy
}

/// The memory read of a run: the largest sample and the growth from the first
/// to the last. `None` when no sample was taken.
fn memory(rss_kib: &[u64]) -> Option<Rss> {
    let first = *rss_kib.first()?;
    let last = *rss_kib.last()?;
    let peak = rss_kib.iter().copied().max().unwrap_or(first);
    let growth = i64::try_from(last).unwrap_or(i64::MAX) - i64::try_from(first).unwrap_or(0);
    Some(Rss {
        peak_kib: peak,
        growth_kib: growth,
    })
}

/// Check caller-supplied frame times and RSS samples: the p50 and p95 of the
/// frames, the RSS peak and growth, and the cliff flag (the second half's p95
/// over the first half's, past [`CLIFF_TOLERANCE_MS`]). Pure over its inputs:
/// the same samples give the same report.
///
/// # Errors
///
/// [`SoakError::TooShort`] on fewer than 2 frames.
pub fn check(frame_ms: &[f64], rss_kib: &[u64]) -> Result<SoakReport, SoakError> {
    if frame_ms.len() < 2 {
        return Err(SoakError::TooShort {
            frames: frame_ms.len(),
        });
    }
    let half = frame_ms.len() / 2;
    let first_p95 = quantile(&ordered(&frame_ms[..half]), 95);
    let second_p95 = quantile(&ordered(&frame_ms[half..]), 95);
    let all = ordered(frame_ms);
    Ok(SoakReport {
        frames: u64::try_from(frame_ms.len()).unwrap_or(u64::MAX),
        p50_ms: quantile(&all, 50),
        p95_ms: quantile(&all, 95),
        first_half_p95_ms: first_p95,
        second_half_p95_ms: second_p95,
        rss: memory(rss_kib),
        cliff: second_p95 - first_p95 > CLIFF_TOLERANCE_MS,
    })
}

/// The packaged stream: healthy frames at 16..20 ms on the config seed and a
/// resident set that starts at 32 MiB and grows 4 KiB a second. Same config,
/// same stream, twice.
#[must_use]
pub fn samples(config: &SoakConfig) -> (Vec<f64>, Vec<u64>) {
    let mut rng = Rng::new(splitmix64(config.seed));
    let frames = usize::try_from(config.frames).unwrap_or(usize::MAX);
    let mut frame_ms = Vec::with_capacity(frames);
    let mut rss_kib = Vec::with_capacity(frames);
    for index in 0..frames {
        let step = u32::try_from(rng.next_u64() % 5).unwrap_or(u32::MAX);
        frame_ms.push(16.0 + f64::from(step));
        let seconds = u64::try_from(index / 60).unwrap_or(u64::MAX);
        rss_kib.push(32_768 + seconds * 4);
    }
    (frame_ms, rss_kib)
}

/// Run a config through [`check`]. Same config gives the same report.
///
/// # Errors
///
/// [`SoakError::TooShort`] when the config holds fewer than 2 frames.
pub fn run(config: &SoakConfig) -> Result<SoakReport, SoakError> {
    let (frame_ms, rss_kib) = samples(config);
    check(&frame_ms, &rss_kib)
}

/// The process's resident set size (KiB), when the host can say it: Linux and
/// Android read `/proc/self/statm`. `None` when the target has no leaf (a
/// Windows build, for one): the caller feeds the samples then.
#[must_use]
pub fn read_rss_kib() -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        linux_rss_kib()
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

/// The resident pages of `/proc/self/statm`, in KiB.
#[cfg(target_os = "linux")]
fn linux_rss_kib() -> Option<u64> {
    /// The page size the resident page count is multiplied by (KiB): Linux and
    /// Android use 4 KiB pages on every 64-bit target the engine builds.
    const PAGE_KIB: u64 = 4;
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let resident = statm.split_whitespace().nth(1)?;
    let pages = resident.parse::<u64>().ok()?;
    Some(pages * PAGE_KIB)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mobile_tier_soak_report_prints_p50_rss_and_no_cliff_line() {
        let report = run(&SoakConfig::half_hour()).expect("the packaged stream measures");
        let text = report.summary();
        assert!(text.contains("render_soak"), "{text}");
        assert!(text.contains("p50="), "{text}");
        assert!(text.contains("rss_peak="), "{text}");
        assert!(
            !text.contains("render_soak_cliff"),
            "a healthy run prints no cliff line: {text}"
        );
        assert!(
            !report.cliff,
            "the packaged stream never falls off the cliff"
        );
        assert_eq!(report.frames, SOAK_HALF_HOUR_FRAMES);
        assert_eq!(report.frames, 108_000);
        assert!(
            report.p50_ms >= 16.0 && report.p50_ms <= 20.0,
            "{}",
            report.p50_ms
        );
        assert!(report.p95_ms >= report.p50_ms, "{}", report.p95_ms);
        let rss = report.rss.expect("the packaged stream samples the memory");
        assert!(rss.peak_kib >= 32_768, "{}", rss.peak_kib);
        assert!(rss.growth_kib > 0, "{}", rss.growth_kib);
        println!("{}", text);
    }

    #[test]
    fn mobile_tier_soak_report_is_the_same_twice() {
        let config = SoakConfig::short(600);
        let first = run(&config).expect("first run measures");
        let second = run(&config).expect("second run measures");
        assert_eq!(first, second, "same config repeats exactly");
    }

    #[test]
    fn mobile_tier_soak_reports_a_cliff_when_the_second_half_slows() {
        // Both halves are inside the 60 fps budget, but the second half runs
        // 24 ms slower: a stable budget, a throttled device.
        let mut frames = vec![16.0; 300];
        frames.extend(vec![40.0; 300]);
        let rss = vec![32_768_u64; 600];
        let report = check(&frames, &rss).expect("crafted halves still rank");
        assert!(report.cliff, "24 ms of drift is a cliff");
        assert_eq!(report.drift_ms(), 24.0);
        let text = report.summary();
        assert!(text.contains("render_soak_cliff"), "{text}");
        assert!(text.contains("drift=+24.000ms"), "{text}");
    }

    #[test]
    fn mobile_tier_soak_without_a_memory_sample_says_so() {
        let report = check(&[16.0, 16.0], &[]).expect("two frames are enough");
        assert_eq!(report.rss, None);
        assert!(report.summary().contains("rss=none"));
    }

    #[test]
    fn mobile_tier_soak_refuses_a_stream_it_cannot_split() {
        assert_eq!(check(&[], &[]), Err(SoakError::TooShort { frames: 0 }));
        assert_eq!(check(&[16.0], &[1]), Err(SoakError::TooShort { frames: 1 }));
        assert_eq!(
            SoakError::TooShort { frames: 0 }.to_string(),
            "soak: 0 frames cannot split into halves"
        );
        let _: &dyn std::error::Error = &SoakError::TooShort { frames: 0 };
    }

    #[test]
    fn mobile_tier_soak_reads_the_memory_where_the_host_can() {
        // The leaf may say nothing on this target; only the shape is asserted.
        if let Some(kib) = read_rss_kib() {
            assert!(kib > 0, "{kib}");
        }
    }
}
