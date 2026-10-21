//! Bench-harness allocator baseline: system-allocator rows in the exact shape
//! a future allocator comparison row reuses (same columns, same pinned
//! hash/alloc gates, wall ms presentation-only beside them).
//! Consumer: `tests/allocator_baseline.rs` plus the `--perf` timing proof.
//! Units: ticks, hash, bytes, count, ms. Determinism: ticks, hash and both
//! allocation totals pin to `baselines`; wall ms never enters a hash.
//! Errors: typed `AllocatorError` naming the refused comparison input.
//! Budget: a candidate allocator replaces the system row only with identical
//! hash/alloc rows and `SPEEDUP_PCT` (10%) lower wall ms.
//!
//! The counting global in `alloc` delegates to `System`, so every row below
//! is a system-allocator row by construction. The `Mimalloc` candidate row
//! reuses that same backend until the mimalloc manifest plus deny row and
//! lock update land (orchestrator lane): identical hash/alloc by
//! construction, so the swap stays dropped per `SPEEDUP_PCT` until then.

use crate::workloads::{AiDecide, DestructionStep, PhysicsStep, ScaledArenaTick, WorkloadReport};
use engine_arena::MAX_PARTICIPANTS;
use std::fmt::{self, Display, Formatter};

/// Acceptance bar for an allocator swap: the candidate's wall ms must sit at
/// least this far below the system row at identical hash/alloc rows.
pub const SPEEDUP_PCT: f64 = 10.0;

/// CSV columns shared by the system rows and any future candidate row, so
/// the two compare field for field. `ms` is `-` when no wall sample ran.
pub const CSV_HEADER: &str = "allocator,workload,ticks,hash,alloc_bytes,alloc_count,ms";

/// Which allocator produced a row. System is the pinned default; Mimalloc
/// is the PF18 candidate beside it (same backend until its dependency
/// lane lands, so rows match by construction and the swap stays dropped).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AllocatorKind {
    /// The counting global over `System` (`alloc::CountingAlloc`).
    System,
    /// PF18 candidate row name; backend is `System` until the mimalloc
    /// manifest/deny/lock lane lands. Caller: `perf --alloc-compare`.
    Mimalloc,
}

impl AllocatorKind {
    /// Short name heading the row's allocator column.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Mimalloc => "mimalloc",
        }
    }
}

impl Display for AllocatorKind {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// The allocator this build installs as the process global: `System` in the
/// default build, `Mimalloc` under the `mimalloc` feature. A row names the
/// allocator that produced it, so the harness reads this instead of assuming;
/// the A/B is two builds of the same harness, one per backend.
#[must_use]
pub const fn active_kind() -> AllocatorKind {
    if cfg!(feature = "mimalloc") {
        AllocatorKind::Mimalloc
    } else {
        AllocatorKind::System
    }
}

/// Allocator comparison failure: which side refused, with the value carried.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AllocatorError {
    /// A wall sample is NaN or infinite (`side` is `"system"`, `"candidate"`
    /// or `"row"`).
    NonFiniteMs {
        /// Which side carries the sample.
        side: &'static str,
        /// Offending milliseconds.
        value_ms: f64,
    },
    /// The system sample is zero or negative, so no speedup ratio exists.
    NonPositiveSystem {
        /// Offending system milliseconds.
        value_ms: f64,
    },
    /// A candidate sample is negative, so no honest comparison exists.
    NegativeCandidate {
        /// Offending candidate milliseconds.
        value_ms: f64,
    },
}

impl Display for AllocatorError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NonFiniteMs { side, value_ms } => {
                write!(f, "allocator: {side} sample is not finite ({value_ms} ms)")
            }
            Self::NonPositiveSystem { value_ms } => write!(
                f,
                "allocator: system sample is not positive ({value_ms} ms): no speedup ratio exists"
            ),
            Self::NegativeCandidate { value_ms } => {
                write!(f, "allocator: candidate sample is negative ({value_ms} ms)")
            }
        }
    }
}

impl std::error::Error for AllocatorError {}

/// Wall-clock comparison of one workload: system ms against candidate ms.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AllocatorDelta {
    /// System-allocator wall ms for the workload.
    pub system_ms: f64,
    /// Candidate-allocator wall ms for the same workload.
    pub candidate_ms: f64,
    /// Speedup in percent: `(system - candidate) / system * 100`.
    pub delta_pct: f64,
}

impl AllocatorDelta {
    /// Acceptance verdict: the candidate clears the swap bar only when it is
    /// at least `SPEEDUP_PCT` faster. A slower candidate reads negative and
    /// fails; hash/alloc identity is checked separately by the budget gate.
    #[must_use]
    pub fn passes(self) -> bool {
        self.delta_pct >= SPEEDUP_PCT
    }
}

/// Compare one workload's wall samples: system ms against candidate ms.
/// Pure over its inputs; run-to-run movement lives in the sampling, never
/// in hidden state.
///
/// # Errors
///
/// [`AllocatorError::NonFiniteMs`] on a NaN or infinite sample,
/// [`AllocatorError::NonPositiveSystem`] on a zero or negative system
/// sample, [`AllocatorError::NegativeCandidate`] on a negative candidate.
pub fn compare_ms(system_ms: f64, candidate_ms: f64) -> Result<AllocatorDelta, AllocatorError> {
    if !system_ms.is_finite() {
        return Err(AllocatorError::NonFiniteMs {
            side: "system",
            value_ms: system_ms,
        });
    }
    if !candidate_ms.is_finite() {
        return Err(AllocatorError::NonFiniteMs {
            side: "candidate",
            value_ms: candidate_ms,
        });
    }
    if system_ms <= 0.0 {
        return Err(AllocatorError::NonPositiveSystem {
            value_ms: system_ms,
        });
    }
    if candidate_ms < 0.0 {
        return Err(AllocatorError::NegativeCandidate {
            value_ms: candidate_ms,
        });
    }
    Ok(AllocatorDelta {
        system_ms,
        candidate_ms,
        delta_pct: (system_ms - candidate_ms) / system_ms * 100.0,
    })
}

/// System-allocator canonical rows: the cap-sized arena run on this track
/// (same shape as the canonical default: seed 1, 600 ticks, 20 m
/// half-extent) plus physics, AI and destruction. Hash and allocation
/// totals pin to `baselines`; wall ms joins later via [`csv_row`], never
/// here. The arena params repeat the canonical shape field for field, so
/// production code carries no `Option` unwrap; the harness test pins this
/// row equal to the canonical cap-sized run.
#[must_use]
pub fn system_rows() -> [WorkloadReport; 4] {
    let arena = ScaledArenaTick {
        seed: 1,
        ticks: 600,
        half_extent_m: 20.0,
        fighters: MAX_PARTICIPANTS,
    };
    [
        arena.run_report(),
        PhysicsStep::canonical().run_report(),
        AiDecide::canonical().run_report(),
        DestructionStep::canonical().run_report(),
    ]
}

/// Nearest-rank percentile `percent` (0..=100) over an ascending sorted slice;
/// 0.0 when the slice holds no sample. Integer rank math only, so the system
/// row and a candidate row (`--features mimalloc`) report p50, p95 and p99 by
/// the same rule: rank is `round(end * percent / 100)`, half up.
#[must_use]
pub fn percentile(sorted: &[f64], percent: u32) -> f64 {
    let Some(end) = sorted.len().checked_sub(1) else {
        return 0.0;
    };
    let scale = usize::try_from(percent).unwrap_or(usize::MAX);
    let rank = end.saturating_mul(scale).saturating_add(50) / 100;
    sorted.get(rank.min(end)).copied().unwrap_or(0.0)
}

/// Candidate-allocator rows beside [`system_rows`]: same workloads, same
/// shape, so hash/alloc columns match field for field. The backend is the
/// process's global allocator: `System` in the default build, and `mimalloc`
/// in the `mimalloc`-feature test binary that installs it, so a candidate row
/// names the allocator that actually produced it. Caller:
/// `perf --alloc-compare` plus the candidate-vs-system test.
#[must_use]
pub fn candidate_rows() -> [WorkloadReport; 4] {
    system_rows()
}

/// One CSV row in the shared harness shape: allocator plus the pinned
/// hash/alloc columns, with the wall sample beside them (`None` prints `-`).
/// The candidate row uses this same function, so columns cannot drift.
///
/// Wall ms joins the baseline file only from a quiesced box (load under 10%,
/// High-Performance power plan, three consecutive runs within ±10%);
/// every other timed row stays observed-only.
///
/// # Errors
///
/// [`AllocatorError::NonFiniteMs`] (`side: "row"`) on a NaN or infinite
/// sample, [`AllocatorError::NegativeCandidate`] on a negative one.
pub fn csv_row(
    report: &WorkloadReport,
    allocator: AllocatorKind,
    ms: Option<f64>,
) -> Result<String, AllocatorError> {
    let cell = match ms {
        None => "-".to_owned(),
        Some(value) => {
            if !value.is_finite() {
                return Err(AllocatorError::NonFiniteMs {
                    side: "row",
                    value_ms: value,
                });
            }
            if value < 0.0 {
                return Err(AllocatorError::NegativeCandidate { value_ms: value });
            }
            format!("{value:.3}")
        }
    };
    Ok(format!(
        "{},{},{},{:016x},{},{},{}",
        allocator,
        report.workload,
        report.ticks,
        report.hash,
        report.alloc_bytes,
        report.alloc_count,
        cell
    ))
}

/// Quiesced-box conditions line printed with every timed run, so a wall-ms
/// row names the box it came from. Fields the harness cannot measure stay
/// `unknown` (overridable via `PF18_POWER_SCHEME`, `PF18_CPU_MHZ` and
/// `PF18_BOX_LOAD_PCT` env vars); threads stays 1 because the proof runs
/// single-threaded, and profile follows the build profile.
///
/// Wall ms joins the baseline file only from a quiesced box (load under 10%,
/// High-Performance power plan, three consecutive runs within ±10%);
/// every other timed row stays observed-only.
#[must_use]
pub fn box_conditions() -> String {
    fn field(key: &str, fallback: &str) -> String {
        match std::env::var(key) {
            Ok(raw) => {
                let trimmed = raw.trim();
                if trimmed.is_empty() {
                    fallback.to_owned()
                } else {
                    trimmed.replace(' ', "_")
                }
            }
            Err(_) => fallback.to_owned(),
        }
    }
    let scheme = field("PF18_POWER_SCHEME", "unknown");
    let cpu = field("PROCESSOR_IDENTIFIER", std::env::consts::ARCH);
    let mhz = field("PF18_CPU_MHZ", "unknown");
    let load = field("PF18_BOX_LOAD_PCT", "unknown");
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    format!(
        "# box: allocator={} scheme={scheme} cpu={cpu} mhz={mhz} load={load}% threads=1 profile={profile}",
        active_kind()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_kind_names_its_column() {
        assert_eq!(AllocatorKind::System.name(), "system");
        assert_eq!(AllocatorKind::System.to_string(), "system");
    }

    #[test]
    fn box_line_names_its_fields() {
        let line = box_conditions();
        assert!(line.starts_with("# box:"), "{line}");
        for token in [
            "allocator=",
            "scheme=",
            "cpu=",
            "mhz=",
            "load=",
            "threads=1",
            "profile=",
        ] {
            assert!(line.contains(token), "{line} names {token}");
        }
        assert!(
            line.contains(&format!("allocator={}", active_kind())),
            "{line} names the backend this build installs"
        );
    }

    #[test]
    fn percentile_reads_nearest_rank_on_a_sorted_slice() {
        let sorted: Vec<f64> = (1..=100u32).map(f64::from).collect();
        // Index ranks over 0..=99: 50, 94 and 98, i.e. the 51st, 95th, 99th
        // values. Same figures the float formula gave before the integer rank.
        assert_eq!(percentile(&sorted, 50), 51.0);
        assert_eq!(percentile(&sorted, 95), 95.0);
        assert_eq!(percentile(&sorted, 99), 99.0);
        assert_eq!(percentile(&sorted, 0), 1.0, "rank 0 keeps the smallest");
        assert_eq!(
            percentile(&sorted, 100),
            100.0,
            "rank 100 keeps the largest"
        );
        assert_eq!(
            percentile(&[], 50),
            0.0,
            "no sample reads zero, never a panic"
        );
        assert_eq!(percentile(&[7.5], 99), 7.5);
        // The same slice answers the same figure twice.
        assert_eq!(percentile(&sorted, 95), percentile(&sorted, 95));
    }

    #[test]
    fn system_arena_matches_the_canonical_cap_run() {
        let canonical = ScaledArenaTick::canonical_for(MAX_PARTICIPANTS)
            .expect("the cap is a listed matrix count");
        let rows = system_rows();
        assert_eq!(
            rows[0],
            canonical.run_report(),
            "arena row repeats the canonical shape"
        );
        assert_eq!(rows.len(), 4, "one row per canonical workload");
    }

    #[test]
    fn ten_percent_faster_passes_the_bar() {
        let delta = compare_ms(100.0, 90.0).expect("clean samples compare");
        assert_eq!(delta.delta_pct, 10.0);
        assert!(delta.passes(), "exactly 10% clears the bar");
        let faster = compare_ms(100.0, 80.0).expect("clean samples compare");
        assert!(faster.passes(), "20% clears the bar");
    }

    #[test]
    fn slower_or_narrow_gains_fail_the_bar() {
        let narrow = compare_ms(100.0, 90.001).expect("clean samples compare");
        assert!(
            narrow.delta_pct < SPEEDUP_PCT,
            "9.999% really sits under the bar"
        );
        assert!(!narrow.passes(), "a narrow gain keeps the system row");
        let slower = compare_ms(100.0, 110.0).expect("clean samples compare");
        assert_eq!(slower.delta_pct, -10.0);
        assert!(!slower.passes(), "a slower candidate fails");
    }

    #[test]
    fn bad_samples_refuse_loudly() {
        assert!(
            matches!(
                compare_ms(f64::NAN, 1.0),
                Err(AllocatorError::NonFiniteMs { side: "system", .. })
            ),
            "NaN system sample names its side"
        );
        assert_eq!(
            compare_ms(1.0, f64::INFINITY),
            Err(AllocatorError::NonFiniteMs {
                side: "candidate",
                value_ms: f64::INFINITY,
            })
        );
        assert_eq!(
            compare_ms(0.0, 0.0),
            Err(AllocatorError::NonPositiveSystem { value_ms: 0.0 })
        );
        assert_eq!(
            compare_ms(5.0, -1.0),
            Err(AllocatorError::NegativeCandidate { value_ms: -1.0 })
        );
        for error in [
            AllocatorError::NonFiniteMs {
                side: "row",
                value_ms: f64::NAN,
            },
            AllocatorError::NonPositiveSystem { value_ms: 0.0 },
            AllocatorError::NegativeCandidate { value_ms: -1.0 },
        ] {
            assert!(!error.to_string().is_empty(), "{error:?} explains itself");
            let _: &dyn std::error::Error = &error;
        }
    }

    #[test]
    fn csv_row_keeps_seven_columns() {
        let report = WorkloadReport {
            workload: "probe",
            ticks: 600,
            hash: 0x1234,
            alloc_bytes: 10,
            alloc_count: 2,
        };
        let row = csv_row(&report, AllocatorKind::System, None).expect("row prints");
        assert_eq!(row.split(',').count(), 7, "columns never drift: {row}");
        assert!(row.starts_with("system,probe,600,"), "{row}");
        assert!(row.ends_with(",-"), "{row}");
        let timed = csv_row(&report, AllocatorKind::System, Some(12.5)).expect("timed row prints");
        assert!(timed.ends_with(",12.500"), "{timed}");
        assert!(
            matches!(
                csv_row(&report, AllocatorKind::System, Some(f64::NAN)),
                Err(AllocatorError::NonFiniteMs { side: "row", .. })
            ),
            "NaN row sample names its side"
        );
        assert_eq!(
            csv_row(&report, AllocatorKind::System, Some(-2.0)),
            Err(AllocatorError::NegativeCandidate { value_ms: -2.0 })
        );
    }

    #[test]
    fn candidate_matches_system_at_identical_hash_alloc() {
        let system = system_rows();
        let candidate = candidate_rows();
        assert_eq!(system.len(), 4, "four canonical workloads");
        assert_eq!(candidate.len(), 4, "candidate covers all four");
        for (left, right) in system.iter().zip(candidate.iter()) {
            assert_eq!(left.workload, right.workload, "workload names match");
            assert_eq!(left.ticks, right.ticks, "tick counts match");
            assert_eq!(left.hash, right.hash, "candidate keeps the system hash");
            assert_eq!(
                left.alloc_bytes, right.alloc_bytes,
                "candidate keeps the system bytes"
            );
            assert_eq!(
                left.alloc_count, right.alloc_count,
                "candidate keeps the system count"
            );
            assert_eq!(AllocatorKind::Mimalloc.name(), "mimalloc");
            assert_eq!(AllocatorKind::Mimalloc.to_string(), "mimalloc");
        }
        let delta = compare_ms(100.0, 100.0).expect("equal samples compare");
        assert_eq!(delta.delta_pct, 0.0, "identical backends read 0%");
        assert!(!delta.passes(), "0% keeps the system row per SPEEDUP_PCT");
    }
}
