//! Memory leak onset for soak runs: a least-squares slope with a 95 percent
//! interval, then a two-line change point that names the first bad minute.
//! Method: Georges et al. (OOPSLA 2007) and Mytkowicz et al. (ASPLOS 2009),
//! reimplemented std-only. Inputs are per-minute resident sizes in KiB. The
//! verdict is text for the report and is never hashed.

use crate::online_soak::MEM_DRIFT_PCT;
use engine_core::CounterRng;
use std::error::Error;
use std::fmt::{self, Display, Formatter};

/// Fewest per-minute samples a verdict accepts. One hour gives 60.
pub const MIN_SAMPLES: usize = 30;
/// Most samples a verdict accepts. Bounds the split search.
pub const MAX_SAMPLES: usize = 4096;
/// Fewest samples on each side of a change point.
pub const MIN_SEGMENT: usize = 10;
/// Base resident size of the seeded fixture: 512 MiB, in KiB.
pub const BASE_KIB: u32 = 512 * 1024;

/// Two-sided 95 percent Student t values for 1 to 30 degrees of freedom.
const T95: [f64; 30] = [
    12.706, 4.303, 3.182, 2.776, 2.571, 2.447, 2.365, 2.306, 2.262, 2.228, 2.201, 2.179, 2.160,
    2.145, 2.131, 2.120, 2.110, 2.101, 2.093, 2.086, 2.080, 2.074, 2.069, 2.064, 2.060, 2.056,
    2.052, 2.048, 2.045, 2.042,
];

/// t value for `df` degrees of freedom. Above 30 degrees, 2.05 is at least
/// as wide as the true value, so the interval never looks tighter.
fn t95(df: usize) -> f64 {
    match df {
        0 => f64::INFINITY,
        1..=30 => T95[df - 1],
        _ => 2.05,
    }
}

/// Exact count-to-float conversion. Counts here stay below `MAX_SAMPLES`.
fn count_f64(n: usize) -> f64 {
    f64::from(u32::try_from(n).unwrap_or(u32::MAX))
}

/// Least-squares line over minute samples, with the error of its slope.
#[derive(Clone, Copy, Debug)]
struct Line {
    intercept: f64,
    slope: f64,
    sse: f64,
    se_slope: f64,
    len: usize,
}

/// Fit a line to `ys`, where the first sample is minute `start`.
fn fit(ys: &[f64], start: usize) -> Line {
    let len = ys.len();
    let n = count_f64(len);
    let x0 = count_f64(start);
    let mean_x = x0 + (n - 1.0) / 2.0;
    let mean_y = ys.iter().sum::<f64>() / n;
    let mut sxx = 0.0;
    let mut sxy = 0.0;
    for (i, y) in ys.iter().enumerate() {
        let dx = x0 + count_f64(i) - mean_x;
        sxx += dx * dx;
        sxy += dx * (y - mean_y);
    }
    let slope = sxy / sxx;
    let intercept = mean_y - slope * mean_x;
    let sse: f64 = ys
        .iter()
        .enumerate()
        .map(|(i, y)| {
            let r = y - (intercept + slope * (x0 + count_f64(i)));
            r * r
        })
        .sum();
    let se_slope = (sse / (n - 2.0) / sxx).sqrt();
    Line {
        intercept,
        slope,
        sse,
        se_slope,
        len,
    }
}

/// Percent change from `start` to `end`. Zero when there is no positive base.
fn growth_pct(start: f64, end: f64) -> f64 {
    if start > 0.0 {
        100.0 * (end - start) / start
    } else {
        0.0
    }
}

/// Why a series is refused before any fit runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeakError {
    /// Fewer than [`MIN_SAMPLES`] minutes.
    TooFew { got: usize },
    /// More than [`MAX_SAMPLES`] minutes.
    TooMany { got: usize },
}

impl Display for LeakError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooFew { got } => {
                write!(
                    f,
                    "soak needs at least {MIN_SAMPLES} minute samples, got {got}"
                )
            }
            Self::TooMany { got } => {
                write!(
                    f,
                    "soak takes at most {MAX_SAMPLES} minute samples, got {got}"
                )
            }
        }
    }
}

impl Error for LeakError {}

/// One soak verdict. `first_bad_minute` is the minute index of the first
/// sample on the leaking line, set only when the leak is red.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Verdict {
    /// Number of per-minute samples judged.
    pub minutes: usize,
    /// Whole-run slope, in KiB per minute.
    pub slope: f64,
    /// Lower bound of the 95 percent interval of `slope`.
    pub slope_low: f64,
    /// Growth from the baseline to the last minute, in percent.
    pub growth_pct: f64,
    /// Minute index where the over-budget leak starts, if any.
    pub first_bad_minute: Option<u32>,
}

impl Verdict {
    /// True when no leak is over budget.
    #[must_use]
    pub fn is_green(&self) -> bool {
        self.first_bad_minute.is_none()
    }
}

impl Display for Verdict {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let minute = match self.first_bad_minute {
            Some(minute) => minute.to_string(),
            None => "none".to_string(),
        };
        write!(
            f,
            "soak memory minutes={} slope={:.1} KiB/min low95={:.1} growth={:.1}% \
             (bar {MEM_DRIFT_PCT:.1}%) first_bad_minute={minute} verdict={}",
            self.minutes,
            self.slope,
            self.slope_low,
            self.growth_pct,
            if self.is_green() { "green" } else { "RED" },
        )
    }
}

/// Judge one run of per-minute memory samples in KiB. The run is red only
/// when the slope after the change point is growing and the growth is over
/// the bar.
pub fn analyze(kib: &[u32]) -> Result<Verdict, LeakError> {
    let minutes = kib.len();
    if minutes < MIN_SAMPLES {
        return Err(LeakError::TooFew { got: minutes });
    }
    if minutes > MAX_SAMPLES {
        return Err(LeakError::TooMany { got: minutes });
    }
    let ys: Vec<f64> = kib.iter().map(|&size| f64::from(size)).collect();
    let last = count_f64(minutes - 1);
    let whole = fit(&ys, 0);
    let mut verdict = Verdict {
        minutes,
        slope: whole.slope,
        slope_low: whole.slope - t95(whole.len - 2) * whole.se_slope,
        growth_pct: growth_pct(whole.intercept, whole.intercept + whole.slope * last),
        first_bad_minute: None,
    };
    if verdict.slope_low <= 0.0 {
        return Ok(verdict);
    }
    // Two lines meet at split `k`: samples before it, and samples from it on.
    // Keep the split with the least total squared error.
    let mut best: Option<(usize, Line, Line)> = None;
    let mut best_sse = f64::INFINITY;
    for k in MIN_SEGMENT..=minutes - MIN_SEGMENT {
        let pre = fit(&ys[..k], 0);
        let post = fit(&ys[k..], k);
        if pre.sse + post.sse < best_sse {
            best_sse = pre.sse + post.sse;
            best = Some((k, pre, post));
        }
    }
    let Some((k, pre, post)) = best else {
        return Ok(verdict);
    };
    let post_low = post.slope - t95(post.len - 2) * post.se_slope;
    verdict.growth_pct = growth_pct(pre.intercept, post.intercept + post.slope * last);
    if post_low > 0.0 && verdict.growth_pct > MEM_DRIFT_PCT {
        verdict.first_bad_minute = u32::try_from(k).ok();
    }
    Ok(verdict)
}

/// Seeded per-minute samples in KiB: a flat base with a seeded wobble of up
/// to about 0.8 percent, plus an optional linear leak given as `(first
/// leaking minute, per-mille of base added per minute)`.
pub fn seeded_series(seed: u64, minutes: u32, leak: Option<(u32, u32)>) -> Vec<u32> {
    let rng = CounterRng::new(seed);
    let base = u64::from(BASE_KIB);
    (0..minutes)
        .map(|minute| {
            let wobble = rng.sample(u64::from(minute)) % 4001;
            let leaked = match leak {
                Some((from, permille)) if minute >= from => {
                    base * u64::from(permille) / 1000 * u64::from(minute - from + 1)
                }
                _ => 0,
            };
            let total = base + wobble + leaked;
            u32::try_from(total).unwrap_or(u32::MAX)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: u64 = 2040;
    const HOUR: u32 = 60;

    #[test]
    fn clean_seeded_run_is_green() {
        let verdict = analyze(&seeded_series(SEED, HOUR, None)).expect("an hour fits");
        assert!(verdict.is_green(), "{verdict}");
        assert_eq!(verdict.first_bad_minute, None);
        assert!(verdict.growth_pct.abs() < MEM_DRIFT_PCT, "{verdict}");
    }

    #[test]
    fn seeded_leak_fails_and_names_first_bad_minute() {
        let onset = 24;
        let verdict = analyze(&seeded_series(SEED, HOUR, Some((onset, 6)))).expect("fits");
        assert!(!verdict.is_green(), "{verdict}");
        let named = verdict
            .first_bad_minute
            .expect("a red run names its minute");
        assert!(
            named.abs_diff(onset) <= 2,
            "named minute {named}, leak starts at {onset}"
        );
        assert!(verdict.growth_pct > MEM_DRIFT_PCT, "{verdict}");
        assert!(verdict
            .to_string()
            .contains(&format!("first_bad_minute={named}")));
    }

    #[test]
    fn leak_under_budget_stays_green() {
        let verdict = analyze(&seeded_series(SEED, HOUR, Some((24, 1)))).expect("fits");
        assert!(verdict.is_green(), "{verdict}");
        assert!(verdict.growth_pct < MEM_DRIFT_PCT, "{verdict}");
    }

    #[test]
    fn short_series_is_refused_loudly() {
        assert_eq!(analyze(&[BASE_KIB; 29]), Err(LeakError::TooFew { got: 29 }));
        assert!(LeakError::TooFew { got: 29 }.to_string().contains("got 29"));
    }

    #[test]
    fn same_seed_gives_same_verdict_and_other_seed_differs() {
        let a = analyze(&seeded_series(SEED, HOUR, Some((24, 6)))).expect("fits");
        let b = analyze(&seeded_series(SEED, HOUR, Some((24, 6)))).expect("fits");
        assert_eq!(a, b);
        assert_ne!(
            seeded_series(SEED, HOUR, None),
            seeded_series(SEED + 1, HOUR, None)
        );
    }
}
