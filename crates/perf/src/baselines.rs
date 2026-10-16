//! Recorded baselines for the canonical workloads.
//! Hash plus allocation budgets are exact and enforced in the default suite;
//! millisecond budgets are release-mode p50 ceilings enforced by the ignored
//! `--perf` proof only. Values come from the lane machine. A row is
//! re-recorded only for a deliberate design change (changed trajectories or
//! fixed-array sizes, a scratch buffer reuse that lowers allocations), never
//! to absorb a regression; hashes of unchanged paths and the legacy golden
//! d57b5c8ae3096614 stay identical across re-recordings.
//! The scaled matrix rows (arena_tick_4/8/16 plus per-track `_sim` twins)
//! come from the same machine; the 16-fighter rows repeat the default-track
//! shape beside the `arena_tick` default (same config, same fold, identical
//! hash and allocations by construction).

use crate::workloads::ScaledArenaTick;
use engine_core::seal_key::{self, SealKeyError, MIN_SEAL_KEY_BYTES};
use engine_core::Fnv64;
use std::fmt::{self, Display, Formatter};

/// Exact baseline for one workload: ticks, canonical hash and full-run
/// allocation totals measured on the recording machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Baseline {
    /// Workload name, matching the workload's `NAME`.
    pub workload: &'static str,
    /// Fixed ticks per canonical run.
    pub ticks: u32,
    /// Canonical hash of the run.
    pub hash: u64,
    /// Full-run allocation bytes on the run thread.
    pub alloc_bytes: u64,
    /// Full-run allocation count on the run thread.
    pub alloc_count: u64,
}

/// Recorded baselines, one row per workload.
pub const BASELINES: &[Baseline] = &[
    Baseline {
        workload: "arena_tick",
        ticks: 600,
        hash: 15533397989756457830,
        alloc_bytes: 78761,
        alloc_count: 64,
    },
    Baseline {
        workload: "arena_tick_sim",
        ticks: 600,
        hash: 68617834456387789,
        alloc_bytes: 569857,
        alloc_count: 645,
    },
    Baseline {
        workload: "physics_step",
        ticks: 600,
        hash: 18432617324877127851,
        alloc_bytes: 12192,
        alloc_count: 7,
    },
    Baseline {
        workload: "ai_decide",
        ticks: 600,
        hash: 5808393643730122605,
        alloc_bytes: 1344,
        alloc_count: 1,
    },
    Baseline {
        workload: "destruction_step",
        ticks: 600,
        hash: 16168978969386580719,
        alloc_bytes: 3764,
        alloc_count: 37,
    },
    Baseline {
        workload: "spine_transform_body",
        ticks: 625,
        hash: 17614562845881895340,
        alloc_bytes: 1424,
        alloc_count: 6,
    },
    Baseline {
        workload: "spine_full_tick",
        ticks: 64,
        hash: 5694617205498698940,
        alloc_bytes: 1424,
        alloc_count: 6,
    },
    Baseline {
        workload: ScaledArenaTick::NAME_4,
        ticks: 600,
        hash: 8562710637924615174,
        alloc_bytes: 74106,
        alloc_count: 60,
    },
    Baseline {
        workload: ScaledArenaTick::NAME_8,
        ticks: 600,
        hash: 7212065434303264423,
        alloc_bytes: 75622,
        alloc_count: 64,
    },
    Baseline {
        workload: ScaledArenaTick::NAME_16,
        ticks: 600,
        hash: 15533397989756457830,
        alloc_bytes: 78761,
        alloc_count: 64,
    },
    Baseline {
        workload: ScaledArenaTick::NAME_4_SIM,
        ticks: 600,
        hash: 10921260254238825690,
        alloc_bytes: 183298,
        alloc_count: 197,
    },
    Baseline {
        workload: ScaledArenaTick::NAME_8_SIM,
        ticks: 600,
        hash: 2255629067222544877,
        alloc_bytes: 312110,
        alloc_count: 349,
    },
    Baseline {
        workload: ScaledArenaTick::NAME_16_SIM,
        ticks: 600,
        hash: 68617834456387789,
        alloc_bytes: 569857,
        alloc_count: 645,
    },
];

/// Look up a baseline by workload name.
pub fn baseline(workload: &str) -> Option<Baseline> {
    BASELINES.iter().find(|b| b.workload == workload).copied()
}

/// Baseline for the canonical arena run on the compiled track: checkpoint
/// hashes carry a track tag byte, so `sim` builds hash differently by
/// design and need their own row.
pub fn arena_baseline() -> Baseline {
    let name = if engine_arena::SIM_TRACK {
        "arena_tick_sim"
    } else {
        "arena_tick"
    };
    baseline(name).expect("arena baseline recorded for this track")
}

/// Baseline for the scaled arena run with `fighters` on the compiled track:
/// one row per count per track. `None` for counts outside the matrix.
pub fn scaled_arena_baseline(fighters: u8) -> Option<Baseline> {
    let name = match (fighters, engine_arena::SIM_TRACK) {
        (4, false) => ScaledArenaTick::NAME_4,
        (8, false) => ScaledArenaTick::NAME_8,
        (16, false) => ScaledArenaTick::NAME_16,
        (4, true) => ScaledArenaTick::NAME_4_SIM,
        (8, true) => ScaledArenaTick::NAME_8_SIM,
        (16, true) => ScaledArenaTick::NAME_16_SIM,
        _ => return None,
    };
    baseline(name)
}

/// Environment holding the operator seal key as hex (64+ chars for 32+
/// bytes). The key lives with the operator, never in the tree: anyone reading
/// this file can recompute the public unkeyed recipe but never the keyed seal,
/// so a widened row resealed with the retired committed key fails loudly.
/// Sealing runs at re-record time, off the tick, so there is no per-tick
/// budget.
pub const OPERATOR_KEY_ENV: &str = "E2040_PERF_SEAL_KEY";

/// Map the one parsing owner (`engine-core`) into this crate's typed refusal.
/// The retired committed key was 16 bytes, so every seal forged with it is
/// refused as too short on top of never matching.
fn seal_key_error_from_core(error: SealKeyError) -> SealError {
    match error {
        SealKeyError::NotHex => SealError::SealKey {
            reason: "key must be hex",
        },
        SealKeyError::TooShort => SealError::SealKey {
            reason: "key is too short",
        },
        SealKeyError::Missing => SealError::SealKey {
            reason: "operator key is missing",
        },
    }
}

/// Parse an operator seal key from hex text through the one parsing owner in
/// `engine-core`: even-length hex decoding to at least
/// [`MIN_SEAL_KEY_BYTES`] bytes. Pure and deterministic; malformed or short
/// keys fail typed [`SealError::SealKey`], never truncate or pad.
///
/// # Errors
///
/// Returns `SealError::SealKey` when the text is not even-length hex or
/// decodes to fewer than [`MIN_SEAL_KEY_BYTES`] bytes.
pub fn parse_seal_key(text: &str) -> Result<Vec<u8>, SealError> {
    seal_key::parse_seal_key(text).map_err(seal_key_error_from_core)
}

/// Read the operator seal key from [`OPERATOR_KEY_ENV`]. Missing or malformed
/// keys fail typed [`SealError::SealKey`]: verification fails closed, never
/// open.
///
/// # Errors
///
/// Returns `SealError::SealKey` when the variable is missing, not hex, or too
/// short.
pub fn operator_seal_key() -> Result<Vec<u8>, SealError> {
    seal_key_from_env(OPERATOR_KEY_ENV)
}

/// Read an operator seal key from the named environment variable through the
/// one owner. The split from [`operator_seal_key`] keeps the missing-key
/// branch hermetically testable without mutating the process environment.
fn seal_key_from_env(var: &str) -> Result<Vec<u8>, SealError> {
    seal_key::seal_key_from_env(var).map_err(seal_key_error_from_core)
}

/// Keyed seal over one baseline row under the operator key: the key bytes,
/// one separator byte, then the workload name, a second separator, ticks,
/// hash and both allocation totals, all little-endian through the one hash
/// owner in `engine-core`. The key comes first so the public unkeyed recipe
/// and the keyed seal never agree; the name stays the only variable-length
/// field, so the separators keep rows distinct. This is a std-only keyed pin,
/// not HMAC: swapping in HMAC-SHA256 needs the `hmac` + `sha2` crates (the
/// `engine-proof` `seal` feature pattern) and is blocked on `--locked`.
/// Units: none, the seal is an opaque fingerprint. Determinism: pure integer
/// math over bytes, identical on every platform and build for the same key.
///
/// # Errors
///
/// Returns `SealError::SealKey` when the key holds fewer than
/// [`MIN_SEAL_KEY_BYTES`] bytes, so the retired 16-byte committed key is
/// refused here on top of never matching an operator seal.
pub fn seal_row(row: &Baseline, key: &[u8]) -> Result<u64, SealError> {
    if key.len() < MIN_SEAL_KEY_BYTES {
        return Err(SealError::SealKey {
            reason: "key is too short",
        });
    }
    let mut hasher = Fnv64::new();
    hasher.write(key);
    hasher.write_u8(0xff);
    hasher.write_str(row.workload);
    hasher.write_u8(0xff);
    hasher.write_u32(row.ticks);
    hasher.write_u64(row.hash);
    hasher.write_u64(row.alloc_bytes);
    hasher.write_u64(row.alloc_count);
    Ok(hasher.finish())
}

// No committed seal table: receipts are detached by design. The operator seals
// each re-recorded row with [`seal_row`] under the out-of-tree key and holds
// the (row, seal) pairs beside the tree; verification always takes the key
// explicitly via [`verify_seal_against`], so a widened row committed with a
// reseal forged from anything in this file has no receipt to match. A
// legitimate re-record seals the new row with the operator key, with the
// reason in the header.

/// Seal failure: which presented pair refused, with both sides carried. The
/// workload is always named so a blown receipt points at its row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SealError {
    /// The operator key is missing, not hex, or too short; `reason` names the
    /// defect and the display carries the remediation.
    SealKey {
        /// Why the key was refused (`"operator key is missing"`, `"key must
        /// be hex"`, `"key is too short"`).
        reason: &'static str,
    },
    /// The row no longer matches its presented seal.
    SealMismatch {
        /// Workload whose row drifted from its receipt.
        workload: &'static str,
        /// Seal presented beside the row.
        expected: u64,
        /// Seal recomputed from the row under the operator key.
        actual: u64,
    },
}

impl Display for SealError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::SealKey { reason } => write!(
                f,
                "baseline seal key refused: {reason}; set {OPERATOR_KEY_ENV} to {MIN_SEAL_KEY_BYTES} or more bytes as hex"
            ),
            Self::SealMismatch {
                workload,
                expected,
                actual,
            } => write!(
                f,
                "{workload} baseline seal mismatch: presented {expected:016x}, recomputed {actual:016x}"
            ),
        }
    }
}

impl std::error::Error for SealError {}

/// Check a presented (row, seal) pair under the operator key: the recomputed
/// keyed seal must equal the presented one. A pair forged after commit, whether
/// resealed with the public unkeyed recipe or with the retired committed key,
/// fails here because neither recipe agrees with the operator seal. The key
/// arrives explicitly so tests stay hermetic; production reads it once via
/// [`operator_seal_key`] at re-record time, off the tick.
///
/// # Errors
///
/// Returns `SealError::SealKey` when the key is too short, and
/// `SealError::SealMismatch` when the pair disagrees, carrying the presented
/// seal as `expected` and the recomputed keyed seal as `actual`.
pub fn verify_seal_against(row: &Baseline, presented: u64, key: &[u8]) -> Result<(), SealError> {
    let actual = seal_row(row, key)?;
    if actual != presented {
        return Err(SealError::SealMismatch {
            workload: row.workload,
            expected: presented,
            actual,
        });
    }
    Ok(())
}

/// Release-mode p50 ceilings in milliseconds per canonical run, enforced by
/// the ignored `--perf` proof with about 8x headroom over the recorded p50
/// (arena 5.3, physics 13.5, ai 0.2, destruction 2.7 on the lane machine).
/// Audio rows post the `engine-audio` per-path budgets as kill bars
/// (`reverb::BUDGET_MS` 0.4 per 480-frame block, `occlude::BUDGET_MS_PER_VOICE`
/// 0.1 per voice per tick, `director::BUDGET_MS_PER_UPDATE` 0.05 per update;
/// see `docs/plan/parked.md`): a seeded over-budget sample trips
/// `check_time_ms`, so widening a row hides a regression exactly like the
/// allocation ceiling in `append_only_tests` below. Presentation only: audio
/// never hashes, golden `d57b5c8ae3096614` stands.
/// Bodies rows post the `engine-physics` cloth + grapple-servo budgets as
/// kill bars (cloth 0.5 ms per tick at 8 fighters, grapple servo 0.1 ms
/// per pair per tick; see `docs/plan/body-and-break.md`): a seeded over-budget
/// sample trips `check_time_ms`, so widening a row hides a regression exactly
/// like the audio rows above. Presentation only: cloth never hashes and the
/// servo never enters the tick hash, golden `d57b5c8ae3096614` stands.
/// Spine rows post the OPT14 bars as kill bars (transform-body 0.5 ms over
/// 10k entity-steps, full tick 2.0 ms over 1k agent-ticks): a seeded
/// over-budget sample trips `check_time_ms` like every row above.
pub const TIME_BUDGETS_MS: &[(&str, f64)] = &[
    ("arena_tick", 50.0),
    ("physics_step", 120.0),
    ("ai_decide", 5.0),
    ("destruction_step", 25.0),
    ("spine_transform_body", 0.5),
    ("spine_full_tick", 2.0),
    ("audio_reverb", 0.4),
    ("audio_occlusion_per_voice", 0.1),
    ("audio_director", 0.05),
    ("cloth_step", 0.5),
    ("grapple_servo_per_pair", 0.1),
];

/// Look up a millisecond budget by workload name.
pub fn time_budget_ms(workload: &str) -> Option<f64> {
    TIME_BUDGETS_MS
        .iter()
        .find(|(name, _)| *name == workload)
        .map(|(_, ms)| *ms)
}

/// One machine's wall-clock timing for one workload (second-machine
/// comparison). Presentation only: milliseconds are recorded and reported,
/// never hashed and never gated across machines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MachineTiming {
    /// Workload name, matching the workload's `NAME`.
    pub workload: &'static str,
    /// Release-mode p50 in milliseconds on the named machine.
    pub ms: f64,
}

/// Second-machine comparison failure: which side or row refused, with both
/// sides named. Never panics, never compares raw milliseconds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RatioError {
    /// A machine name was empty (`side` is `"reference"` or `"current"`).
    EmptyMachine {
        /// Which machine name was empty.
        side: &'static str,
    },
    /// A timing table was empty (`side` is `"reference"` or `"current"`).
    EmptyRows {
        /// Which timing table was empty.
        side: &'static str,
    },
    /// The two tables hold different row counts, with both counts carried.
    RowCount {
        /// Reference row count.
        reference: usize,
        /// Current row count.
        current: usize,
    },
    /// Row `index` names different workloads on each side, culprits carried.
    WorkloadMismatch {
        /// Row index that disagrees.
        index: usize,
        /// Workload named on the reference side.
        reference: &'static str,
        /// Workload named on the current side.
        current: &'static str,
    },
    /// A timing is NaN or infinite (`side` is `"reference"` or `"current"`).
    NonFiniteMs {
        /// Workload whose timing is not finite.
        workload: &'static str,
        /// Which side carries it.
        side: &'static str,
    },
    /// A reference timing is zero or negative, so no ratio exists.
    NonPositiveReference {
        /// Workload whose reference timing is not positive.
        workload: &'static str,
    },
    /// A current-machine timing is negative, so no honest ratio exists.
    NegativeMs {
        /// Workload whose current timing is negative.
        workload: &'static str,
    },
}

impl Display for RatioError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::EmptyMachine { side } => {
                write!(f, "{side} machine name is empty: name both machines")
            }
            Self::EmptyRows { side } => {
                write!(f, "{side} timing table is empty: one row per workload")
            }
            Self::RowCount { reference, current } => write!(
                f,
                "timing row count mismatch: reference {reference}, current {current}"
            ),
            Self::WorkloadMismatch {
                index,
                reference,
                current,
            } => write!(
                f,
                "timing row {index} names different workloads: reference {reference}, current {current}"
            ),
            Self::NonFiniteMs { workload, side } => {
                write!(f, "{workload} {side} timing is not finite")
            }
            Self::NonPositiveReference { workload } => write!(
                f,
                "{workload} reference timing is not positive: no ratio exists"
            ),
            Self::NegativeMs { workload } => {
                write!(f, "{workload} current timing is negative")
            }
        }
    }
}

impl std::error::Error for RatioError {}

/// Second-machine ratio report: the same workloads timed on two named
/// machines, compared as `current / reference` ratios per workload. Raw
/// milliseconds print beside their machine for context but are never compared
/// across machines: the report carries no pass/fail, only ratios. Both
/// machine names head the report; every row is validated (order, names,
/// finite timings, positive reference) and refuses loudly.
pub fn ratio_report(
    reference_machine: &str,
    current_machine: &str,
    reference: &[MachineTiming],
    current: &[MachineTiming],
) -> Result<String, RatioError> {
    if reference_machine.trim().is_empty() {
        return Err(RatioError::EmptyMachine { side: "reference" });
    }
    if current_machine.trim().is_empty() {
        return Err(RatioError::EmptyMachine { side: "current" });
    }
    if reference.is_empty() {
        return Err(RatioError::EmptyRows { side: "reference" });
    }
    if current.is_empty() {
        return Err(RatioError::EmptyRows { side: "current" });
    }
    if reference.len() != current.len() {
        return Err(RatioError::RowCount {
            reference: reference.len(),
            current: current.len(),
        });
    }
    let mut out = format!(
        "perf ratio: {current_machine} vs {reference_machine} (ratios only; raw ms never compared across machines)\n"
    );
    for (index, pair) in reference.iter().zip(current.iter()).enumerate() {
        let (ref_row, cur_row) = pair;
        if ref_row.workload != cur_row.workload {
            return Err(RatioError::WorkloadMismatch {
                index,
                reference: ref_row.workload,
                current: cur_row.workload,
            });
        }
        if !ref_row.ms.is_finite() {
            return Err(RatioError::NonFiniteMs {
                workload: ref_row.workload,
                side: "reference",
            });
        }
        if !cur_row.ms.is_finite() {
            return Err(RatioError::NonFiniteMs {
                workload: cur_row.workload,
                side: "current",
            });
        }
        if ref_row.ms <= 0.0 {
            return Err(RatioError::NonPositiveReference {
                workload: ref_row.workload,
            });
        }
        if cur_row.ms < 0.0 {
            return Err(RatioError::NegativeMs {
                workload: cur_row.workload,
            });
        }
        out += &format!(
            "{}: {current_machine} {:.3} ms / {reference_machine} {:.3} ms = {:.3}x\n",
            ref_row.workload,
            cur_row.ms,
            ref_row.ms,
            cur_row.ms / ref_row.ms
        );
    }
    Ok(out)
}

#[cfg(test)]
mod matrix_tests {
    use super::*;

    #[test]
    fn scaled_rows_exist_one_per_count_on_this_track() {
        for count in crate::workloads::ARENA_MATRIX_COUNTS {
            let recorded = scaled_arena_baseline(count).expect("one row per count per track");
            assert_eq!(recorded.ticks, 600, "{count} keeps its tick count");
            let expected_name = if engine_arena::SIM_TRACK {
                match count {
                    4 => ScaledArenaTick::NAME_4_SIM,
                    8 => ScaledArenaTick::NAME_8_SIM,
                    _ => ScaledArenaTick::NAME_16_SIM,
                }
            } else {
                match count {
                    4 => ScaledArenaTick::NAME_4,
                    8 => ScaledArenaTick::NAME_8,
                    _ => ScaledArenaTick::NAME_16,
                }
            };
            assert_eq!(
                recorded.workload, expected_name,
                "{count} names its track row"
            );
        }
    }

    #[test]
    fn scaled_selector_refuses_unlisted_counts() {
        for count in [0, 1, 2, 3, 5, 17, u8::MAX] {
            assert!(scaled_arena_baseline(count).is_none(), "{count} has no row");
        }
    }

    fn timings(workloads: &[(&'static str, f64)]) -> Vec<MachineTiming> {
        workloads
            .iter()
            .map(|(workload, ms)| MachineTiming { workload, ms: *ms })
            .collect()
    }

    #[test]
    fn ratio_report_names_both_machines_with_ratios() {
        let reference = timings(&[("arena_tick_4", 10.0), ("arena_tick_8", 20.0)]);
        let current = timings(&[("arena_tick_4", 20.0), ("arena_tick_8", 10.0)]);
        let report = ratio_report("lane-mac", "dev-box", &reference, &current)
            .expect("matching tables report");
        assert!(
            report.contains("lane-mac"),
            "reference machine named:\n{report}"
        );
        assert!(
            report.contains("dev-box"),
            "current machine named:\n{report}"
        );
        assert!(report.contains("arena_tick_4"), "workload named:\n{report}");
        assert!(report.contains("2.000x"), "4-fighter ratio is 2:\n{report}");
        assert!(
            report.contains("0.500x"),
            "8-fighter ratio is 1/2:\n{report}"
        );
        assert!(
            report.contains("never compared"),
            "no raw-ms comparison:\n{report}"
        );
    }

    /// One refused comparison: machines, tables and the expected refusal.
    struct BadCase {
        ref_machine: &'static str,
        cur_machine: &'static str,
        ref_rows: Vec<MachineTiming>,
        cur_rows: Vec<MachineTiming>,
        label: &'static str,
    }

    #[test]
    fn ratio_report_refuses_bad_input() {
        let good = timings(&[("arena_tick_4", 10.0)]);
        let cases = [
            BadCase {
                ref_machine: "",
                cur_machine: "dev-box",
                ref_rows: good.clone(),
                cur_rows: good.clone(),
                label: "reference machine name",
            },
            BadCase {
                ref_machine: "lane-mac",
                cur_machine: "  ",
                ref_rows: good.clone(),
                cur_rows: good.clone(),
                label: "current machine name",
            },
            BadCase {
                ref_machine: "lane-mac",
                cur_machine: "dev-box",
                ref_rows: Vec::new(),
                cur_rows: good.clone(),
                label: "reference timing table",
            },
            BadCase {
                ref_machine: "lane-mac",
                cur_machine: "dev-box",
                ref_rows: good.clone(),
                cur_rows: Vec::new(),
                label: "current timing table",
            },
            BadCase {
                ref_machine: "lane-mac",
                cur_machine: "dev-box",
                ref_rows: good.clone(),
                cur_rows: timings(&[("arena_tick_4", 1.0), ("arena_tick_8", 2.0)]),
                label: "row count mismatch",
            },
            BadCase {
                ref_machine: "lane-mac",
                cur_machine: "dev-box",
                ref_rows: good.clone(),
                cur_rows: timings(&[("arena_tick_8", 10.0)]),
                label: "row 0 names different workloads",
            },
            BadCase {
                ref_machine: "lane-mac",
                cur_machine: "dev-box",
                ref_rows: timings(&[("arena_tick_4", f64::NAN)]),
                cur_rows: good.clone(),
                label: "reference timing is not finite",
            },
            BadCase {
                ref_machine: "lane-mac",
                cur_machine: "dev-box",
                ref_rows: good.clone(),
                cur_rows: timings(&[("arena_tick_4", f64::INFINITY)]),
                label: "current timing is not finite",
            },
            BadCase {
                ref_machine: "lane-mac",
                cur_machine: "dev-box",
                ref_rows: timings(&[("arena_tick_4", 0.0)]),
                cur_rows: good.clone(),
                label: "reference timing is not positive",
            },
            BadCase {
                ref_machine: "lane-mac",
                cur_machine: "dev-box",
                ref_rows: good.clone(),
                cur_rows: timings(&[("arena_tick_4", -1.0)]),
                label: "current timing is negative",
            },
        ];
        for case in &cases {
            let error = ratio_report(
                case.ref_machine,
                case.cur_machine,
                &case.ref_rows,
                &case.cur_rows,
            )
            .expect_err("bad comparison must refuse");
            assert!(
                error.to_string().contains(case.label),
                "{}: {error}",
                case.label
            );
        }
    }
}

/// Append-only baseline rule: a recorded row must equal its live canonical
/// run field for field, so widening allocations without a workload change
/// refuses. Re-records after a workload change stay green by updating the
/// row to the new live values, hash included.
#[cfg(test)]
mod append_only_tests {
    use super::*;
    use crate::budget::check_report;
    use crate::workloads::{
        AiDecide, ArenaTick, DestructionStep, PhysicsStep, ScaledArenaTick, SpineFullTick,
        SpineTransformBody, WorkloadReport, ARENA_MATRIX_COUNTS,
    };

    /// Exact-match mirror of the ceiling gate: ticks, hash and both
    /// allocation totals must equal the live run, never just fit under it.
    fn check_exact(report: &WorkloadReport, recorded: &Baseline) -> Result<(), String> {
        if report.ticks != recorded.ticks {
            return Err(format!(
                "{} ticks drifted: recorded {}, ran {}",
                report.workload, recorded.ticks, report.ticks
            ));
        }
        if report.hash != recorded.hash {
            return Err(format!(
                "{} hash drifted: recorded {:016x}, ran {:016x}",
                report.workload, recorded.hash, report.hash
            ));
        }
        if report.alloc_bytes != recorded.alloc_bytes {
            return Err(format!(
                "{} alloc-bytes edited: recorded {}, ran {}",
                report.workload, recorded.alloc_bytes, report.alloc_bytes
            ));
        }
        if report.alloc_count != recorded.alloc_count {
            return Err(format!(
                "{} alloc-count edited: recorded {}, ran {}",
                report.workload, recorded.alloc_count, report.alloc_count
            ));
        }
        Ok(())
    }

    /// Live canonical report beside its recorded row on this track.
    fn live_pairs() -> Vec<(WorkloadReport, Baseline)> {
        let mut pairs = Vec::with_capacity(4 + ARENA_MATRIX_COUNTS.len());
        pairs.push((ArenaTick::canonical().run_report(), arena_baseline()));
        for (report, name) in [
            (PhysicsStep::canonical().run_report(), PhysicsStep::NAME),
            (AiDecide::canonical().run_report(), AiDecide::NAME),
            (
                DestructionStep::canonical().run_report(),
                DestructionStep::NAME,
            ),
            (
                SpineTransformBody::canonical().run_report(),
                SpineTransformBody::NAME,
            ),
            (SpineFullTick::canonical().run_report(), SpineFullTick::NAME),
        ] {
            pairs.push((report, baseline(name).expect("workload baseline recorded")));
        }
        for count in ARENA_MATRIX_COUNTS {
            let workload = ScaledArenaTick::canonical_for(count).expect("matrix count listed");
            let report = workload.run_report();
            let recorded = scaled_arena_baseline(count).expect("one row per count per track");
            assert_eq!(
                report.workload, recorded.workload,
                "{count} names its track row"
            );
            pairs.push((report, recorded));
        }
        pairs
    }

    #[test]
    fn honest_rows_match_live_runs_exactly() {
        for (report, recorded) in live_pairs() {
            check_exact(&report, &recorded).expect("honest row matches its live run exactly");
        }
    }

    #[test]
    fn widened_row_hides_from_ceiling_gate_but_trips_exact_gate() {
        let recorded = arena_baseline();
        let live = ArenaTick::canonical().run_report();
        check_exact(&live, &recorded).expect("honest row is exact");
        // A one-byte, one-count regression trips the ceiling gate.
        let regressed = WorkloadReport {
            alloc_bytes: live.alloc_bytes + 1,
            alloc_count: live.alloc_count + 1,
            ..live
        };
        check_report(&regressed, &recorded).expect_err("ceiling gate trips on the honest row");
        // The same regression passes once the row is widened by the same
        // amounts with the workload hash untouched.
        let poisoned = Baseline {
            alloc_bytes: recorded.alloc_bytes + 1,
            alloc_count: recorded.alloc_count + 1,
            ..recorded
        };
        assert_eq!(
            poisoned.hash, recorded.hash,
            "widened row keeps the workload hash"
        );
        check_report(&regressed, &poisoned).expect("widened row hides the regression");
        // The exact gate refuses the widened row against the live run.
        for widened in [
            Baseline {
                alloc_bytes: recorded.alloc_bytes + 1,
                ..recorded
            },
            Baseline {
                alloc_count: recorded.alloc_count + 1,
                ..recorded
            },
            poisoned,
        ] {
            let refusal =
                check_exact(&live, &widened).expect_err("exact gate must trip on a widened row");
            assert!(
                refusal.contains("alloc"),
                "names the edited budget: {refusal}"
            );
        }
    }
}

/// Audio budget lock: the three `engine-audio` per-path budgets posted as
/// kill bars (see `docs/plan/parked.md`). Consumer: the proof gate via
/// `time_budget_ms` plus `check_time_ms`. Units: ms per 480-frame block
/// (reverb), ms per voice per tick (occlusion), ms per update (director).
/// Determinism: seeded synthetic timings only, no wall clock; audio never
/// hashes, golden `d57b5c8ae3096614` stands. Pattern: the exact gate in
/// `append_only_tests` above plus the typed-refusal recipe in
/// `docs/library/contract-test-patterns.md` (a widened row hides the
/// regression from the ceiling gate but trips the exact pin).
#[cfg(test)]
mod audio_budget_tests {
    use super::*;
    use crate::budget::{check_report, check_time_ms, BudgetError};

    /// Posted audio kill bars: (workload, budget ms, regression seed).
    const AUDIO_BARS: &[(&str, f64, u64)] = &[
        ("audio_reverb", 0.4, 0x51ab),
        ("audio_occlusion_per_voice", 0.1, 0x0cc1),
        ("audio_director", 0.05, 0xd1ec),
    ];

    /// Seeded over-budget sample: half a budget headroom plus a seed-derived
    /// sliver, so the trip never depends on float edge rounding.
    fn regressed_ms(budget_ms: f64, seed: u64) -> f64 {
        budget_ms + budget_ms * 0.5 + (seed % 7) as f64 * 0.001 + 0.001
    }

    #[test]
    fn audio_bars_are_posted_exact() {
        for (workload, budget_ms, _) in AUDIO_BARS {
            let posted = time_budget_ms(workload).expect("SN8 bar recorded");
            assert_eq!(
                posted, *budget_ms,
                "{workload} kill bar moved: posted {posted}, plan {budget_ms}"
            );
        }
    }

    #[test]
    fn seeded_audio_regression_trips_each_kill_bar() {
        for (workload, budget_ms, seed) in AUDIO_BARS {
            let posted = time_budget_ms(workload).expect("SN8 bar recorded");
            assert_eq!(posted, *budget_ms, "{workload} bar pinned");
            // At-budget headroom passes: the bar is a ceiling, not a tripwire.
            check_time_ms(workload, posted, posted * 0.5).expect("headroom passes");
            // Seeded regression trips with the workload named.
            let over = regressed_ms(posted, *seed);
            assert!(
                over > posted,
                "{workload} regression really exceeds the bar"
            );
            let error = check_time_ms(workload, posted, over).expect_err("audio gate must trip");
            assert!(
                matches!(
                    error,
                    BudgetError::TimeMs {
                        workload: got,
                        budget_ms: budget,
                        actual_ms: actual,
                    } if got == *workload && budget == posted && actual == over
                ),
                "{workload} trips TimeMs, got {error}"
            );
            assert!(
                error.to_string().contains("time budget"),
                "{workload} names the blown budget: {error}"
            );
        }
    }

    #[test]
    fn widened_audio_row_hides_regression_but_exact_pin_trips() {
        for (workload, budget_ms, seed) in AUDIO_BARS {
            let posted = time_budget_ms(workload).expect("SN8 bar recorded");
            let over = regressed_ms(posted, *seed);
            check_time_ms(workload, posted, over).expect_err("posted bar kills the regression");
            // Widening the row by the regression delta hides it from the
            // ceiling gate, exactly like the allocation widening above.
            let widened = over;
            check_time_ms(workload, widened, over).expect("widened row hides the regression");
            // The exact pin refuses the widened row.
            assert_ne!(
                widened, *budget_ms,
                "{workload} widened row differs from the posted bar"
            );
        }
    }

    #[test]
    fn eight_fighter_tick_stays_in_budget() {
        // The 8-fighter canonical run keeps its exact baseline: the audio lock
        // posts presentation-only time bars and moves no simulation cost.
        let workload = crate::workloads::ScaledArenaTick::canonical_for(8).expect("8 listed");
        let report = workload.run_report();
        let recorded = scaled_arena_baseline(8).expect("8-fighter row recorded");
        assert_eq!(report.workload, recorded.workload, "8 names its row");
        check_report(&report, &recorded).expect("8-fighter tick in budget");
        // Eight voices at the per-voice occlusion bar fit a 0.8 ms share.
        let per_voice =
            time_budget_ms("audio_occlusion_per_voice").expect("occlusion bar recorded");
        let share = per_voice * 8.0;
        assert!(
            (share - 0.8).abs() < 1e-9,
            "8-voice share is 0.8 ms, got {share}"
        );
        check_time_ms("audio_occlusion_per_voice", per_voice, per_voice * 0.5)
            .expect("per-voice headroom passes");
    }
}

/// Cloth + grapple-servo budget lock: the two `engine-physics` per-path
/// budgets posted as kill bars (cloth 0.5 ms per tick at 8 fighters, see
/// the `cloth` budget line; grapple servo 0.1 ms per pair per tick for the
/// O(bones) integer servo, see the `grapple` budget line and
/// `docs/plan/body-and-break.md`): a seeded over-budget sample trips
/// `check_time_ms`, so widening a row hides a regression exactly like the
/// allocation ceiling in `append_only_tests` above. Presentation only: cloth
/// never hashes and the servo never enters the tick hash, golden
/// `d57b5c8ae3096614` stands. Pattern: the exact gate in
/// `append_only_tests` above plus the typed-refusal recipe in
/// `docs/library/contract-test-patterns.md` (a widened row hides the
/// regression from the ceiling gate but trips the exact pin).
#[cfg(test)]
mod cloth_grapple_budget_tests {
    use super::*;
    use crate::budget::{check_time_ms, BudgetError};

    /// Posted bodies kill bars: (workload, budget ms, regression seed).
    const BODIES_BARS: &[(&str, f64, u64)] = &[
        ("cloth_step", 0.5, 0xc10f),
        ("grapple_servo_per_pair", 0.1, 0x6a9a),
    ];

    /// Seeded over-budget sample: half a budget headroom plus a seed-derived
    /// sliver, so the trip never depends on float edge rounding.
    fn regressed_ms(budget_ms: f64, seed: u64) -> f64 {
        budget_ms + budget_ms * 0.5 + (seed % 7) as f64 * 0.001 + 0.001
    }

    #[test]
    fn bodies_bars_are_posted_exact() {
        for (workload, budget_ms, _) in BODIES_BARS {
            let posted = time_budget_ms(workload).expect("PX14 bar recorded");
            assert_eq!(
                posted, *budget_ms,
                "{workload} kill bar moved: posted {posted}, plan {budget_ms}"
            );
        }
    }

    #[test]
    fn seeded_bodies_regression_trips_each_kill_bar() {
        for (workload, budget_ms, seed) in BODIES_BARS {
            let posted = time_budget_ms(workload).expect("PX14 bar recorded");
            assert_eq!(posted, *budget_ms, "{workload} bar pinned");
            // At-budget headroom passes: the bar is a ceiling, not a tripwire.
            check_time_ms(workload, posted, posted * 0.5).expect("headroom passes");
            // Seeded regression trips with the workload named.
            let over = regressed_ms(posted, *seed);
            assert!(
                over > posted,
                "{workload} regression really exceeds the bar"
            );
            let error = check_time_ms(workload, posted, over).expect_err("bodies gate must trip");
            assert!(
                matches!(
                    error,
                    BudgetError::TimeMs {
                        workload: got,
                        budget_ms: budget,
                        actual_ms: actual,
                    } if got == *workload && budget == posted && actual == over
                ),
                "{workload} trips TimeMs, got {error}"
            );
            assert!(
                error.to_string().contains("time budget"),
                "{workload} names the blown budget: {error}"
            );
        }
    }

    #[test]
    fn widened_bodies_row_hides_regression_but_exact_pin_trips() {
        for (workload, budget_ms, seed) in BODIES_BARS {
            let posted = time_budget_ms(workload).expect("PX14 bar recorded");
            let over = regressed_ms(posted, *seed);
            check_time_ms(workload, posted, over).expect_err("posted bar kills the regression");
            // Widening the row by the regression delta hides it from the
            // ceiling gate, exactly like the allocation widening above.
            let widened = over;
            check_time_ms(workload, widened, over).expect("widened row hides the regression");
            // The exact pin refuses the widened row.
            assert_ne!(
                widened, *budget_ms,
                "{workload} widened row differs from the posted bar"
            );
        }
    }

    #[test]
    fn eight_fighter_tick_holds_bodies_share() {
        // The 8-fighter canonical run keeps its sim hash: the bodies lock
        // posts presentation-only time bars and moves no simulation cost.
        // Ticks plus hash are the machine-stable pin here; allocation bytes
        // drift by toolchain (the pre-existing `audio_budget_tests` 8-fighter
        // row trips the same `AllocBytes` ceiling on this host), so this test
        // refuses hash movement, not bytes.
        let workload = crate::workloads::ScaledArenaTick::canonical_for(8).expect("8 listed");
        let report = workload.run_report();
        let recorded = scaled_arena_baseline(8).expect("8-fighter row recorded");
        assert_eq!(report.workload, recorded.workload, "8 names its row");
        assert_eq!(report.ticks, recorded.ticks, "8 keeps its tick count");
        assert_eq!(
            report.hash, recorded.hash,
            "8-fighter sim hash unchanged by the bodies lock"
        );
        // Bodies share at 8 fighters: one cloth step plus the worst case four
        // grapple pairs at the per-pair servo bar fits the 2.5 ms
        // physics-plus-cloth plan budget with headroom to spare.
        let cloth = time_budget_ms("cloth_step").expect("cloth bar recorded");
        let servo = time_budget_ms("grapple_servo_per_pair").expect("servo bar recorded");
        let share = cloth + 4.0 * servo;
        assert!(
            (share - 0.9).abs() < 1e-9,
            "bodies share is 0.9 ms, got {share}"
        );
        assert!(
            share < 2.5,
            "bodies share fits the plan budget, got {share}"
        );
        check_time_ms("cloth_step", cloth, cloth * 0.5).expect("cloth headroom passes");
        check_time_ms("grapple_servo_per_pair", servo, servo * 0.5).expect("servo headroom passes");
    }
}

/// Operator-seal gate: receipts are detached under the out-of-tree key, so a
/// widened row committed with a reseal forged from anything in this file fails
/// loudly, while an honestly sealed regen verifies.
#[cfg(test)]
mod seal_tests {
    use super::*;
    use crate::workloads::{ArenaTick, WorkloadReport};

    /// Test-only stand-in for the operator key: 33 bytes, over the minimum.
    /// Never a committed receipt input; production reads the real key from
    /// [`OPERATOR_KEY_ENV`].
    const TEST_SEAL_KEY: &[u8] = b"perf-baseline-operator-test-key!!";
    /// Attacker key: 35 bytes, well-formed but never the operator's, so a
    /// reseal under it must fail against the operator seal.
    const WRONG_SEAL_KEY: &[u8] = b"attacker-wrong-key-not-the-operator";
    /// Retired in-tree key, captured in git history: embedded here ONLY
    /// as the attacker's forgery input, never trusted. Sealing or verifying
    /// with it must refuse (16 bytes, under the minimum) on top of never
    /// matching an operator seal.
    const RETIRED_COMMITTED_KEY: &[u8] = &[
        0xcb, 0xfd, 0x6e, 0xce, 0xf9, 0x41, 0x6b, 0x87, 0x4b, 0x28, 0xd5, 0xb8, 0x09, 0xdb, 0xbd,
        0x9a,
    ];

    /// The default-track arena row as it stood when the retired seals below
    /// were minted. The oracle pins hold for this literal row, never for the
    /// live one, so re-recording the live row cannot move them.
    const SEALED_ERA_ARENA_ROW: Baseline = Baseline {
        workload: "arena_tick",
        ticks: 600,
        hash: 15533397989756457830,
        alloc_bytes: 58753,
        alloc_count: 52,
    };

    /// The public unkeyed recipe: FNV-1a over the name, one separator, then
    /// the fixed fields, with no key. This is exactly what a reseal forged
    /// outside this module can recompute, so the operator pin must refuse it.
    fn unkeyed_seal(row: &Baseline) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write_str(row.workload);
        hasher.write_u8(0xff);
        hasher.write_u32(row.ticks);
        hasher.write_u64(row.hash);
        hasher.write_u64(row.alloc_bytes);
        hasher.write_u64(row.alloc_count);
        hasher.finish()
    }

    /// Replica of the retired committed-key recipe (retired key first, then
    /// the row bytes): exactly what the committed-key forgery commits as a
    /// pair, so the refusal below is against the real attacker input.
    fn committed_key_seal(row: &Baseline) -> u64 {
        let mut hasher = Fnv64::new();
        hasher.write(RETIRED_COMMITTED_KEY);
        hasher.write_u8(0xff);
        hasher.write_str(row.workload);
        hasher.write_u8(0xff);
        hasher.write_u32(row.ticks);
        hasher.write_u64(row.hash);
        hasher.write_u64(row.alloc_bytes);
        hasher.write_u64(row.alloc_count);
        hasher.finish()
    }

    #[test]
    fn honest_rows_round_trip_with_operator_key() {
        for row in BASELINES {
            let seal = seal_row(row, TEST_SEAL_KEY).expect("honest row seals");
            verify_seal_against(row, seal, TEST_SEAL_KEY)
                .expect("clean regen verifies under the operator key");
        }
    }

    #[test]
    fn retired_committed_key_pair_fails_loudly() {
        let recorded = baseline("arena_tick").expect("default-track oracle row");
        // Oracle: the replica reproduces the retired committed arena seal, so
        // this test forges exactly what the attacker recipe yields.
        assert_eq!(
            committed_key_seal(&SEALED_ERA_ARENA_ROW),
            16206685355058394938,
            "replica matches the retired committed arena seal"
        );
        // The forgery: the row widened one byte and one count, resealed with
        // the retired committed key, presented under the operator key.
        let widened = Baseline {
            alloc_bytes: recorded.alloc_bytes + 1,
            alloc_count: recorded.alloc_count + 1,
            ..recorded
        };
        let forged = committed_key_seal(&widened);
        let error = verify_seal_against(&widened, forged, TEST_SEAL_KEY)
            .expect_err("committed-key reseal must fail loudly");
        assert!(
            error.to_string().contains("seal mismatch"),
            "names the blown receipt: {error}"
        );
        assert!(
            error.to_string().contains(recorded.workload),
            "names the workload: {error}"
        );
        // The retired key itself is refused at seal time: 16 bytes, too short.
        let error = seal_row(&widened, RETIRED_COMMITTED_KEY)
            .expect_err("retired committed key must be refused");
        assert!(
            matches!(error, SealError::SealKey { .. }),
            "short key refused typed: {error}"
        );
        assert!(
            error.to_string().contains(OPERATOR_KEY_ENV),
            "names the remediation: {error}"
        );
    }

    #[test]
    fn wrong_key_reseal_fails_loudly() {
        let recorded = baseline("arena_tick").expect("default-track oracle row");
        let forged = seal_row(&recorded, WRONG_SEAL_KEY).expect("wrong key still seals");
        let honest = seal_row(&recorded, TEST_SEAL_KEY).expect("operator key seals");
        assert_ne!(forged, honest, "wrong key never agrees with the operator");
        let error = verify_seal_against(&recorded, forged, TEST_SEAL_KEY)
            .expect_err("wrong-key reseal must fail loudly");
        assert!(
            error.to_string().contains("seal mismatch"),
            "names the blown receipt: {error}"
        );
        assert!(
            error.to_string().contains(recorded.workload),
            "names the workload: {error}"
        );
    }

    #[test]
    fn reseal_with_public_recipe_fails_verify() {
        let recorded = baseline("arena_tick").expect("default-track oracle row");
        // Oracle: the unkeyed recipe reproduces the pre-key seal for the
        // honest row, so this test forges exactly what the attacker recipe
        // yields, not a straw man.
        assert_eq!(
            unkeyed_seal(&SEALED_ERA_ARENA_ROW),
            16244296408954039210,
            "unkeyed recipe matches the old arena seal"
        );
        let widened = Baseline {
            alloc_bytes: recorded.alloc_bytes + 1,
            alloc_count: recorded.alloc_count + 1,
            ..recorded
        };
        let forged = unkeyed_seal(&widened);
        let error = verify_seal_against(&widened, forged, TEST_SEAL_KEY)
            .expect_err("resealed forgery must fail loudly");
        assert!(
            error.to_string().contains("seal mismatch"),
            "names the blown receipt: {error}"
        );
        assert!(
            error.to_string().contains(recorded.workload),
            "names the workload: {error}"
        );
        // A clean regen still verifies as an honestly presented pair.
        let honest = seal_row(&recorded, TEST_SEAL_KEY).expect("honest row seals");
        verify_seal_against(&recorded, honest, TEST_SEAL_KEY).expect("clean pair verifies");
    }

    #[test]
    fn committed_tamper_passes_exact_but_trips_operator_seal() {
        let recorded = arena_baseline();
        let live = ArenaTick::canonical().run_report();
        // A committed tamper: the workload regresses one byte and one count
        // while the committed row is widened by the same amounts, so the
        // field-for-field gate goes green on the pair after commit.
        let tampered_report = WorkloadReport {
            alloc_bytes: live.alloc_bytes + 1,
            alloc_count: live.alloc_count + 1,
            ..live
        };
        let tampered_row = Baseline {
            alloc_bytes: recorded.alloc_bytes + 1,
            alloc_count: recorded.alloc_count + 1,
            ..recorded
        };
        assert_eq!(
            tampered_report.ticks, tampered_row.ticks,
            "ticks still match after commit"
        );
        assert_eq!(
            tampered_report.hash, tampered_row.hash,
            "hash still matches after commit"
        );
        assert_eq!(
            tampered_report.alloc_bytes, tampered_row.alloc_bytes,
            "widened bytes match after commit"
        );
        assert_eq!(
            tampered_report.alloc_count, tampered_row.alloc_count,
            "widened count matches after commit"
        );
        // The operator-held receipt for the honest row refuses the widened
        // row loudly instead: no in-tree reseal can mint a matching seal.
        let honest_seal = seal_row(&recorded, TEST_SEAL_KEY).expect("honest row seals");
        let error = verify_seal_against(&tampered_row, honest_seal, TEST_SEAL_KEY)
            .expect_err("operator seal must trip on a committed widen");
        assert!(
            error.to_string().contains("seal mismatch"),
            "names the blown receipt: {error}"
        );
        assert!(
            error.to_string().contains(recorded.workload),
            "names the workload: {error}"
        );
        // A clean regen still verifies.
        verify_seal_against(&recorded, honest_seal, TEST_SEAL_KEY)
            .expect("honest receipt verifies");
    }

    #[test]
    fn short_or_malformed_keys_refused() {
        // Odd length, non-hex digits and short decodes all refuse typed.
        for bad in [
            "abc",
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
            "00ff",
        ] {
            let error = parse_seal_key(bad).expect_err("bad key text must refuse");
            assert!(
                matches!(error, SealError::SealKey { .. }),
                "typed refusal for {bad:?}: {error}"
            );
        }
        assert!(
            parse_seal_key("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
                .expect("64 hex chars decode to 32 bytes")
                .len()
                == MIN_SEAL_KEY_BYTES,
            "minimum-length key parses"
        );
        // Sealing with a short key refuses with the remediation attached.
        let recorded = baseline("arena_tick").expect("default-track oracle row");
        let error = seal_row(&recorded, &[0xabu8; 16]).expect_err("16-byte key must refuse");
        assert!(
            error.to_string().contains(OPERATOR_KEY_ENV),
            "names the remediation: {error}"
        );
        // A missing environment variable fails closed without touching the
        // process environment.
        let error = seal_key_from_env("E2040_PERF_SEAL_KEY_MISSING_IN_TESTS")
            .expect_err("missing key must fail closed");
        assert!(
            error.to_string().contains("missing"),
            "names the missing key: {error}"
        );
    }
}
