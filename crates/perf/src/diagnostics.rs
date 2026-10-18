//! Diagnostics store: named perf series with a bounded history ring, an
//! O(1) mean and a time-aware exponential average, all in whole micros.
//!
//! Donor: bevyengine/bevy `crates/bevy_diagnostic/src/diagnostic.rs`
//! (MIT OR Apache-2.0), the idea only: the moving average steps by
//! `alpha = dt / smoothing_time`, clamped to one, and the first sample seeds
//! it. Ported std-only; no donor bytes are vendored.
//!
//! Consumer: [`crate::profiler::capture`] feeds the `frame_time` series once
//! per captured frame, and [`crate::profiler::FrameReport`] carries the row
//! text and the smoothed ms that `FrameReport::summary` prints.
//! Units: milliseconds at the API edge; stored as whole microseconds.
//! Errors: [`DiagnosticsError`] for an unknown or duplicate series name and
//! a zero smoothing time. A sample that is NaN, infinite, negative, longer
//! than 60 s, or carries a bad `dt` is skipped: nothing is stored and the
//! push returns `false`.
//! Determinism: presentation and diagnostics only. The tick never reads a
//! series and no series enters a simulation hash.
//! Allocation: series are built once; pushing a sample never allocates.

use std::fmt::{self, Display, Formatter};
use std::time::Duration;

/// Samples kept per series; the mean covers this window.
pub const HISTORY_CAP: usize = 120;

/// Longest accepted sample in micros (60 s). Longer samples are skipped.
const SAMPLE_MAX_US: u64 = 60_000_000;

/// Typed refusals from the store. Invalid samples are skipped, not refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiagnosticsError {
    /// No series carries this name.
    UnknownSeries {
        /// Rejected series name.
        name: String,
    },
    /// Two specs name the same series.
    DuplicateSeries {
        /// Repeated series name.
        name: &'static str,
    },
    /// A series asked for a zero smoothing time, which the step divides by.
    ZeroSmoothing {
        /// Series that asked for it.
        name: &'static str,
    },
}

impl Display for DiagnosticsError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownSeries { name } => write!(f, "unknown diagnostics series: {name}"),
            Self::DuplicateSeries { name } => write!(f, "duplicate diagnostics series: {name}"),
            Self::ZeroSmoothing { name } => {
                write!(f, "series {name} needs a non-zero smoothing time")
            }
        }
    }
}

impl std::error::Error for DiagnosticsError {}

/// Milliseconds to whole micros, rounded to nearest. `None` for NaN,
/// infinite, negative or longer than [`SAMPLE_MAX_US`]. Goes through
/// `Duration` so no float-to-integer cast is needed.
fn micros_from_ms(ms: f64) -> Option<u64> {
    let span = Duration::try_from_secs_f64(ms / 1000.0).ok()?;
    let micros = u64::try_from((span.as_nanos() + 500) / 1000).ok()?;
    (micros <= SAMPLE_MAX_US).then_some(micros)
}

/// Moves `from` toward `to` by `permille` of the gap, rounding toward `from`.
fn step_toward(from: u64, to: u64, permille: u64) -> u64 {
    if to >= from {
        from + (to - from) * permille / 1000
    } else {
        from - (from - to) * permille / 1000
    }
}

/// Fixed-precision ms text for one optional micro value, `n/a` when empty.
fn fmt_ms(value: Option<Duration>) -> String {
    match value {
        None => "n/a".to_owned(),
        Some(span) => {
            let micros = span.as_micros();
            format!("{}.{:03}ms", micros / 1000, micros % 1000)
        }
    }
}

/// One named series: a bounded history ring, its running sum, a time-aware
/// exponential average and the latest value. Built by [`Series::new`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Series {
    name: &'static str,
    enabled: bool,
    smoothing_us: u64,
    /// Newest [`HISTORY_CAP`] accepted values, micros.
    ring: [u64; HISTORY_CAP],
    /// Next write slot; once the ring is full it is also the oldest value.
    head: usize,
    /// Values held, at most [`HISTORY_CAP`].
    len: usize,
    /// Sum of the held values, micros. Exact: the evicted value leaves it.
    sum_us: u64,
    /// Time-aware exponential average, micros.
    smoothed_us: u64,
    /// Most recent accepted value, micros.
    latest_us: u64,
}

impl Series {
    /// Empty, enabled series smoothed over `smoothing_us` micros. A zero
    /// smoothing time is refused because each step divides by it.
    pub fn new(name: &'static str, smoothing_us: u64) -> Result<Self, DiagnosticsError> {
        if smoothing_us == 0 {
            return Err(DiagnosticsError::ZeroSmoothing { name });
        }
        Ok(Self {
            name,
            enabled: true,
            smoothing_us,
            ring: [0; HISTORY_CAP],
            head: 0,
            len: 0,
            sum_us: 0,
            smoothed_us: 0,
            latest_us: 0,
        })
    }

    /// Turns recording on or off. Values already held are kept either way.
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
    }

    /// Values held in the ring, at most [`HISTORY_CAP`].
    #[must_use]
    pub fn len(&self) -> usize {
        self.len
    }

    /// True before the first accepted sample.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Most recent accepted value, `None` before the first.
    #[must_use]
    pub fn latest(&self) -> Option<Duration> {
        self.held(self.latest_us)
    }

    /// Time-aware exponential average, `None` before the first.
    #[must_use]
    pub fn smoothed(&self) -> Option<Duration> {
        self.held(self.smoothed_us)
    }

    /// Mean of the values in the ring window: the exact running sum over the
    /// held count, so no per-read walk of the ring.
    #[must_use]
    pub fn mean(&self) -> Option<Duration> {
        let count = u64::try_from(self.len).ok()?;
        (count > 0).then(|| Duration::from_micros(self.sum_us / count))
    }

    /// Records one sample: `value_ms` now, `dt_ms` since the previous sample.
    /// The average steps by `dt / smoothing`, clamped to one. Returns `false`
    /// and stores nothing while the series is disabled or when the value or
    /// `dt` is invalid (NaN, infinite, negative, over 60 s). Never allocates.
    pub fn push_ms(&mut self, value_ms: f64, dt_ms: f64) -> bool {
        if !self.enabled {
            return false;
        }
        let (Some(value_us), Some(dt_us)) = (micros_from_ms(value_ms), micros_from_ms(dt_ms))
        else {
            return false;
        };
        self.push_us(value_us, dt_us);
        true
    }

    /// Integer core shared by every accepted sample.
    fn push_us(&mut self, value_us: u64, dt_us: u64) {
        if self.len == 0 {
            self.smoothed_us = value_us;
        } else {
            // alpha = dt / smoothing, in permille; `dt_us` is capped at 60 s, so
            // the product cannot overflow and `saturating_mul` only documents it.
            let permille = dt_us.saturating_mul(1000) / self.smoothing_us;
            self.smoothed_us = step_toward(self.smoothed_us, value_us, permille.min(1000));
        }
        if self.len == HISTORY_CAP {
            // The slot about to be overwritten holds the oldest value.
            self.sum_us -= self.ring[self.head];
        } else {
            self.len += 1;
        }
        self.ring[self.head] = value_us;
        self.sum_us += value_us;
        self.head = (self.head + 1) % HISTORY_CAP;
        self.latest_us = value_us;
    }

    fn held(&self, micros: u64) -> Option<Duration> {
        (self.len > 0).then(|| Duration::from_micros(micros))
    }

    /// One fixed-precision row: `name value=.. smoothed=.. avg=..`, each in ms
    /// with three decimals and `n/a` before the first sample. Built from
    /// integer micros, so the same samples give the same bytes every time.
    #[must_use]
    pub fn row(&self) -> String {
        format!(
            "{} value={} smoothed={} avg={}",
            self.name,
            fmt_ms(self.latest()),
            fmt_ms(self.smoothed()),
            fmt_ms(self.mean())
        )
    }
}

/// Named series in declaration order. Built once with
/// [`DiagnosticsStore::new`]; the set of names never changes afterwards.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticsStore {
    series: Vec<Series>,
}

fn unknown(name: &str) -> DiagnosticsError {
    DiagnosticsError::UnknownSeries {
        name: name.to_owned(),
    }
}

impl DiagnosticsStore {
    /// One series per `(name, smoothing_us)` spec. Duplicate names and zero
    /// smoothing times are refused.
    pub fn new(specs: &[(&'static str, u64)]) -> Result<Self, DiagnosticsError> {
        let mut series: Vec<Series> = Vec::with_capacity(specs.len());
        for &(name, smoothing_us) in specs {
            if series.iter().any(|held| held.name == name) {
                return Err(DiagnosticsError::DuplicateSeries { name });
            }
            series.push(Series::new(name, smoothing_us)?);
        }
        Ok(Self { series })
    }

    /// The named series, read-only.
    pub fn series(&self, name: &str) -> Result<&Series, DiagnosticsError> {
        self.series
            .iter()
            .find(|held| held.name == name)
            .ok_or_else(|| unknown(name))
    }

    /// Records a sample into the named series; see [`Series::push_ms`].
    /// `Ok(false)` means the sample was skipped, not that the name was wrong.
    pub fn push_ms(
        &mut self,
        name: &str,
        value_ms: f64,
        dt_ms: f64,
    ) -> Result<bool, DiagnosticsError> {
        let held = self
            .series
            .iter_mut()
            .find(|held| held.name == name)
            .ok_or_else(|| unknown(name))?;
        Ok(held.push_ms(value_ms, dt_ms))
    }

    /// Turns the named series on or off.
    pub fn set_enabled(&mut self, name: &str, on: bool) -> Result<(), DiagnosticsError> {
        let held = self
            .series
            .iter_mut()
            .find(|held| held.name == name)
            .ok_or_else(|| unknown(name))?;
        held.set_enabled(on);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::alloc_probe;

    const FRAME: &str = "frame_time";
    const FRAME_SMOOTHING_US: u64 = 100_000;

    fn frame_store() -> DiagnosticsStore {
        DiagnosticsStore::new(&[(FRAME, FRAME_SMOOTHING_US)]).expect("one valid series builds")
    }

    #[test]
    fn ring_wraps_at_capacity_and_mean_stays_exact() {
        let mut series = Series::new(FRAME, FRAME_SMOOTHING_US).expect("valid series");
        // 200 whole-ms samples: the ring keeps the newest HISTORY_CAP (81..=200 ms).
        for ms in 1..=200u32 {
            assert!(series.push_ms(f64::from(ms), 1.0), "sample {ms} accepted");
        }
        assert_eq!(series.len(), HISTORY_CAP);
        // Mean of 81..=200 ms is (81 + 200) / 2 = 140.5 ms, exact in micros.
        assert_eq!(series.mean(), Some(Duration::from_micros(140_500)));
        assert_eq!(series.latest(), Some(Duration::from_millis(200)));
    }

    #[test]
    fn ema_converges_toward_a_constant_input() {
        let mut series = Series::new(FRAME, FRAME_SMOOTHING_US).expect("valid series");
        assert!(series.push_ms(0.0, 16.0));
        for _ in 0..300 {
            assert!(series.push_ms(16.0, 16.0));
        }
        let smoothed = series.smoothed().expect("samples present").as_micros();
        // Integer steps stall within a few micros of the input, never drift off it.
        assert!(smoothed.abs_diff(16_000) <= 10, "smoothed {smoothed} us");
    }

    #[test]
    fn ema_weights_a_long_dt_more_than_a_short_dt() {
        let mut short = Series::new(FRAME, FRAME_SMOOTHING_US).expect("valid series");
        let mut long = Series::new(FRAME, FRAME_SMOOTHING_US).expect("valid series");
        assert!(short.push_ms(10.0, 10.0));
        assert!(long.push_ms(10.0, 10.0));
        // alpha = 1/100 for the 1 ms gap, alpha = 50/100 for the 50 ms gap.
        assert!(short.push_ms(20.0, 1.0));
        assert!(long.push_ms(20.0, 50.0));
        assert_eq!(short.smoothed(), Some(Duration::from_micros(10_100)));
        assert_eq!(long.smoothed(), Some(Duration::from_micros(15_000)));
        // A gap at or past the smoothing time takes the new value whole.
        let mut whole = Series::new(FRAME, FRAME_SMOOTHING_US).expect("valid series");
        assert!(whole.push_ms(10.0, 10.0));
        assert!(whole.push_ms(20.0, 100.0));
        assert_eq!(whole.smoothed(), Some(Duration::from_millis(20)));
    }

    #[test]
    fn invalid_samples_are_skipped() {
        let mut series = Series::new(FRAME, FRAME_SMOOTHING_US).expect("valid series");
        for bad in [f64::NAN, f64::INFINITY, -1.0, 60_001.0] {
            assert!(!series.push_ms(bad, 16.0), "value {bad} is skipped");
        }
        assert!(!series.push_ms(16.0, f64::NAN), "a NaN dt is skipped");
        assert!(series.is_empty());
        assert_eq!(series.smoothed(), None);
        assert!(series.push_ms(5.0, 5.0));
        assert_eq!(series.len(), 1);
        assert_eq!(series.latest(), Some(Duration::from_millis(5)));
    }

    #[test]
    fn disabled_series_ignores_samples() {
        let mut store = frame_store();
        store.set_enabled(FRAME, false).expect("known series");
        assert!(!store.push_ms(FRAME, 16.0, 16.0).expect("known series"));
        assert!(store.series(FRAME).expect("known series").is_empty());
        store.set_enabled(FRAME, true).expect("known series");
        assert!(store.push_ms(FRAME, 16.0, 16.0).expect("known series"));
        assert_eq!(store.series(FRAME).expect("known series").len(), 1);
    }

    #[test]
    fn unknown_duplicate_and_zero_smoothing_are_refused() {
        let mut store = frame_store();
        assert_eq!(
            store.push_ms("missing", 1.0, 1.0),
            Err(DiagnosticsError::UnknownSeries {
                name: "missing".to_owned()
            })
        );
        assert_eq!(
            DiagnosticsStore::new(&[(FRAME, 1), (FRAME, 1)]),
            Err(DiagnosticsError::DuplicateSeries { name: FRAME })
        );
        assert_eq!(
            Series::new(FRAME, 0),
            Err(DiagnosticsError::ZeroSmoothing { name: FRAME })
        );
    }

    #[test]
    fn text_row_is_byte_stable_twice() {
        let row_of = || {
            let mut store = frame_store();
            assert!(store.push_ms(FRAME, 16.667, 16.667).expect("known series"));
            store.series(FRAME).expect("known series").row()
        };
        let first = row_of();
        assert_eq!(first, row_of());
        assert_eq!(
            first,
            "frame_time value=16.667ms smoothed=16.667ms avg=16.667ms"
        );
        let empty = Series::new(FRAME, FRAME_SMOOTHING_US)
            .expect("valid series")
            .row();
        assert_eq!(empty, "frame_time value=n/a smoothed=n/a avg=n/a");
    }

    #[test]
    fn push_never_allocates_after_construction() {
        let mut store = frame_store();
        let ((), count) = alloc_probe::measure(|| {
            for i in 0..1_000u32 {
                let _accepted = store.push_ms(FRAME, f64::from(i % 50), 16.0);
            }
        });
        assert_eq!(count.calls, 0, "push allocated {} times", count.calls);
    }
}
