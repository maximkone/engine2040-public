//! Perf overlay (E9): frame, tick and GPU numbers from a host-filled source.
//!
//! The editor has no perf-crate dependency: the window host owns the frame
//! loop, so it measures and answers [`PerfSource::samples`] once per frame.
//! This module turns those numbers into rows and bands each one against a
//! budget the caller gives. Units are whatever the source states (microseconds
//! or counts) and the budget is in the same unit as the row it bands.
//! Determinism: rows follow source order and stop at [`PERF_ROWS`].
//! Errors: none; an over-long source is cut, not refused.

/// Most rows one overlay draws.
pub const PERF_ROWS: usize = 12;

/// Budget band of one row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    /// Under the warn line.
    Pass,
    /// At or over the warn line, under the fail line.
    Warn,
    /// At or over the fail line.
    Fail,
}

impl Band {
    /// One word the row shows.
    pub fn label(self) -> &'static str {
        match self {
            Self::Pass => "pass",
            Self::Warn => "warn",
            Self::Fail => "fail",
        }
    }
}

/// Caller budget for one row, in that row's unit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    /// Value at which the row turns warn.
    pub warn_at: u64,
    /// Value at which the row turns fail.
    pub fail_at: u64,
}

impl Budget {
    /// A budget whose warn line sits under its fail line.
    #[must_use]
    pub fn new(warn_at: u64, fail_at: u64) -> Self {
        Self { warn_at, fail_at }
    }

    /// Band one measured value.
    #[must_use]
    pub fn band(&self, value: u64) -> Band {
        if value >= self.fail_at {
            Band::Fail
        } else if value >= self.warn_at {
            Band::Warn
        } else {
            Band::Pass
        }
    }
}

/// One number the host measured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PerfSample {
    /// Row name, e.g. `"frame p99"`.
    pub label: &'static str,
    /// Measured value in [`PerfSample::unit`].
    pub value: u64,
    /// Unit the value is in: `"us"`, `"calls"`, `"tris"`, `"bytes"`.
    pub unit: &'static str,
}

/// The host's measurement surface.
pub trait PerfSource {
    /// Every number this frame, in draw order.
    fn samples(&self) -> Vec<PerfSample>;
}

/// One drawn row: the sample, its band and the budget that banded it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PerfRow {
    /// Row name.
    pub label: &'static str,
    /// Measured value.
    pub value: u64,
    /// Unit of the value.
    pub unit: &'static str,
    /// Band the value fell in.
    pub band: Band,
    /// Budget that banded it.
    pub budget: Budget,
}

impl PerfRow {
    /// One line the overlay draws: `frame p99 1830 us fail budget 1666`.
    ///
    /// A passing row shows no budget; a warn or fail row names the budget line
    /// it crossed, so the reader sees what to beat.
    #[must_use]
    pub fn text(&self) -> String {
        match self.band {
            Band::Pass => format!(
                "{} {} {} {}",
                self.label,
                self.value,
                self.unit,
                self.band.label()
            ),
            Band::Fail => format!(
                "{} {} {} fail budget {}",
                self.label, self.value, self.unit, self.budget.fail_at
            ),
            Band::Warn => format!(
                "{} {} {} warn budget {}",
                self.label, self.value, self.unit, self.budget.warn_at
            ),
        }
    }
}

/// Overlay state: shown or not, plus the last source it drew.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PerfPanel {
    shown: bool,
}

impl PerfPanel {
    /// Hidden overlay (the `P` key shows it).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// True when the overlay draws.
    #[must_use]
    pub fn shown(&self) -> bool {
        self.shown
    }

    /// Flip visibility and answer the new state.
    pub fn toggle(&mut self) -> bool {
        self.shown = !self.shown;
        self.shown
    }

    /// Rows for one frame, banded by `budget(label)`.
    pub fn rows(
        &self,
        source: &dyn PerfSource,
        budget: impl Fn(&'static str) -> Budget,
    ) -> Vec<PerfRow> {
        let mut samples = source.samples();
        samples.truncate(PERF_ROWS);
        samples
            .into_iter()
            .map(|sample| {
                let budget = budget(sample.label);
                PerfRow {
                    label: sample.label,
                    value: sample.value,
                    unit: sample.unit,
                    band: budget.band(sample.value),
                    budget,
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fake host: the same three numbers every frame.
    struct Fake;

    impl PerfSource for Fake {
        fn samples(&self) -> Vec<PerfSample> {
            vec![
                PerfSample {
                    label: "frame p50",
                    value: 900,
                    unit: "us",
                },
                PerfSample {
                    label: "frame p99",
                    value: 2_400,
                    unit: "us",
                },
                PerfSample {
                    label: "draw calls",
                    value: 180,
                    unit: "calls",
                },
            ]
        }
    }

    fn budget(label: &'static str) -> Budget {
        match label {
            "frame p50" => Budget::new(1_200, 1_666),
            "frame p99" => Budget::new(1_666, 2_000),
            _ => Budget::new(400, 600),
        }
    }

    #[test]
    fn rows_band_against_the_caller_budget() {
        let panel = PerfPanel::new();
        let rows = panel.rows(&Fake, budget);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].band, Band::Pass);
        assert_eq!(rows[0].text(), "frame p50 900 us pass");
        assert_eq!(rows[1].band, Band::Fail);
        assert_eq!(rows[1].budget.fail_at, 2_000);
        assert_eq!(rows[1].text(), "frame p99 2400 us fail budget 2000");
        assert_eq!(rows[2].band, Band::Pass);
        // The warn line is inclusive, the fail line wins at its own value.
        assert_eq!(Budget::new(10, 20).band(9), Band::Pass);
        assert_eq!(Budget::new(10, 20).band(10), Band::Warn);
        assert_eq!(Budget::new(10, 20).band(20), Band::Fail);
        assert!(rows[1].text().contains("fail budget 2000"));
    }

    #[test]
    fn the_toggle_flips_once_per_call_and_a_cut_source_is_capped() {
        let mut panel = PerfPanel::new();
        assert!(!panel.shown());
        assert!(panel.toggle());
        assert!(panel.shown());
        assert!(!panel.toggle());
        assert!(!panel.shown());
        struct Long;
        impl PerfSource for Long {
            fn samples(&self) -> Vec<PerfSample> {
                (0..PERF_ROWS + 4)
                    .map(|_| PerfSample {
                        label: "tick",
                        value: 1,
                        unit: "us",
                    })
                    .collect()
            }
        }
        assert_eq!(panel.rows(&Long, |_| Budget::new(5, 9)).len(), PERF_ROWS);
    }
}
