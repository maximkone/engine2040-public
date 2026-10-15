//! Budget checks over measured workload reports.
//! Each variant names the workload plus the blown budget (ticks, hash,
//! allocation bytes/count, or release-mode milliseconds).

use crate::baselines::Baseline;
use crate::workloads::WorkloadReport;
use std::fmt::{self, Display, Formatter};

/// Budget failure: which workload blew which budget, with both sides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BudgetError {
    /// Canonical hash drifted from the recorded baseline.
    HashMismatch {
        /// Workload name.
        workload: &'static str,
        /// Recorded baseline hash.
        expected: u64,
        /// Hash of the run under test.
        actual: u64,
    },
    /// Tick count drifted from the recorded baseline.
    TickMismatch {
        /// Workload name.
        workload: &'static str,
        /// Recorded tick count.
        expected: u32,
        /// Ticks of the run under test.
        actual: u32,
    },
    /// Run allocated more bytes than the recorded budget.
    AllocBytes {
        /// Workload name.
        workload: &'static str,
        /// Recorded byte budget.
        budget: u64,
        /// Bytes of the run under test.
        actual: u64,
    },
    /// Run allocated more times than the recorded budget.
    AllocCount {
        /// Workload name.
        workload: &'static str,
        /// Recorded allocation-count budget.
        budget: u64,
        /// Count of the run under test.
        actual: u64,
    },
    /// Release-mode p50 exceeded the recorded millisecond budget (`--perf`).
    TimeMs {
        /// Workload name.
        workload: &'static str,
        /// Recorded millisecond budget.
        budget_ms: f64,
        /// Measured p50 of the run under test.
        actual_ms: f64,
    },
}

impl Display for BudgetError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::HashMismatch {
                workload,
                expected,
                actual,
            } => write!(
                f,
                "{workload} hash budget blown: expected {expected:016x}, ran {actual:016x}"
            ),
            Self::TickMismatch {
                workload,
                expected,
                actual,
            } => write!(
                f,
                "{workload} tick budget blown: expected {expected}, ran {actual}"
            ),
            Self::AllocBytes {
                workload,
                budget,
                actual,
            } => write!(
                f,
                "{workload} alloc-bytes budget blown: budget {budget}, ran {actual}"
            ),
            Self::AllocCount {
                workload,
                budget,
                actual,
            } => write!(
                f,
                "{workload} alloc-count budget blown: budget {budget}, ran {actual}"
            ),
            Self::TimeMs {
                workload,
                budget_ms,
                actual_ms,
            } => write!(
                f,
                "{workload} time budget blown: budget {budget_ms:.3} ms, ran {actual_ms:.3} ms"
            ),
        }
    }
}

impl std::error::Error for BudgetError {}

/// Check a measured report against its recorded baseline: ticks, then hash,
/// then allocation bytes and count. Fails on the first blown budget.
pub fn check_report(report: &WorkloadReport, baseline: &Baseline) -> Result<(), BudgetError> {
    if report.ticks != baseline.ticks {
        return Err(BudgetError::TickMismatch {
            workload: report.workload,
            expected: baseline.ticks,
            actual: report.ticks,
        });
    }
    if report.hash != baseline.hash {
        return Err(BudgetError::HashMismatch {
            workload: report.workload,
            expected: baseline.hash,
            actual: report.hash,
        });
    }
    check_alloc(report, baseline)
}

/// Check allocation bytes and count only (used after a hash check passed, or
/// to prove an injected allocation trips the gate).
pub fn check_alloc(report: &WorkloadReport, baseline: &Baseline) -> Result<(), BudgetError> {
    if report.alloc_bytes > baseline.alloc_bytes {
        return Err(BudgetError::AllocBytes {
            workload: report.workload,
            budget: baseline.alloc_bytes,
            actual: report.alloc_bytes,
        });
    }
    if report.alloc_count > baseline.alloc_count {
        return Err(BudgetError::AllocCount {
            workload: report.workload,
            budget: baseline.alloc_count,
            actual: report.alloc_count,
        });
    }
    Ok(())
}

/// Check a release-mode p50 against its millisecond budget. Wall-clock gates
/// run under `--perf` only, never in the default suite.
pub fn check_time_ms(
    workload: &'static str,
    budget_ms: f64,
    actual_ms: f64,
) -> Result<(), BudgetError> {
    if actual_ms > budget_ms {
        return Err(BudgetError::TimeMs {
            workload,
            budget_ms,
            actual_ms,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report() -> WorkloadReport {
        WorkloadReport {
            workload: "probe",
            ticks: 600,
            hash: 0x1234,
            alloc_bytes: 10,
            alloc_count: 2,
        }
    }

    fn baseline() -> Baseline {
        Baseline {
            workload: "probe",
            ticks: 600,
            hash: 0x1234,
            alloc_bytes: 10,
            alloc_count: 2,
        }
    }

    #[test]
    fn green_report_passes() {
        check_report(&report(), &baseline()).expect("matching report passes");
    }

    #[test]
    fn each_blown_budget_names_itself() {
        let cases = [
            (
                WorkloadReport {
                    ticks: 599,
                    ..report()
                },
                "tick budget",
            ),
            (
                WorkloadReport {
                    hash: 0x5678,
                    ..report()
                },
                "hash budget",
            ),
            (
                WorkloadReport {
                    alloc_bytes: 11,
                    ..report()
                },
                "alloc-bytes budget",
            ),
            (
                WorkloadReport {
                    alloc_count: 3,
                    ..report()
                },
                "alloc-count budget",
            ),
        ];
        for (bad, label) in cases {
            let error = check_report(&bad, &baseline()).expect_err("budget must trip");
            assert!(error.to_string().contains(label), "{error}");
            assert!(error.to_string().contains("probe"), "{error}");
        }
        let timed = check_time_ms("probe", 1.0, 2.0).expect_err("time must trip");
        assert!(timed.to_string().contains("time budget"), "{timed}");
        check_time_ms("probe", 2.0, 1.0).expect("headroom passes");
    }
}
