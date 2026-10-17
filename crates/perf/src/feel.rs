//! Feel quantification: three-domain measurements against the shared
//! latency bar plus the keypoint rubric.
//! Status: test-only (no production caller reads [`DomainVerdict::summary`]
//! yet; exercised by the unit tests in this file and the headline check in
//! `crates/perf/tests/it/lib_wiring_vectors.rs`). The keypoint and
//! tournament harnesses named here do not exist yet, so counts arrive as
//! plain numbers in tests and this crate takes no new dependency edge.
//! Units: milliseconds for the latency legs, permille for shares and rates.
//! Determinism: pure over inputs; seeded fixtures replay exactly; no wall
//! clock, no GPU, no network. Errors: typed [`FeelError`].
//! Budget: report-time only; [`assess_domain`] allocates nothing.

use core::fmt::{self, Display, Formatter};

use engine_core::splitmix64;

use crate::latency::{
    FramePresent, InputEvent, LatencyReport, SimEffect, HEADLESS_SAMPLES, PRESENT_P95_BUDGET_MS,
};

/// Largest allowed rise of varied present p95 over baseline p95, in ms.
/// A larger rise kills the hunk even when the varied run clears the bar.
pub const KILL_RISE_MS: f64 = 10.0;
/// Smallest accepted keypoint pass rate, in permille (nine of ten).
pub const RUBRIC_PASS_PERMILLE: u32 = 900;
/// Lowest accepted tournament win share, in permille.
pub const WIN_LO_PERMILLE: u32 = 400;
/// Highest accepted tournament win share, in permille.
pub const WIN_HI_PERMILLE: u32 = 600;
/// Largest representable share, in permille; anything above refuses.
pub const SHARE_FULL_PERMILLE: u32 = 1000;

/// Which feel domain a measurement varied. The parameter rows stay owned
/// by the combat preset; this label only says which domain moved between
/// the baseline run and the varied run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FeelDomain {
    /// Numbers tuning: damage, timing and stamina rows.
    Tuning,
    /// Presentation juice: hit-stop, shake, flash and cue layers.
    Juicing,
    /// Input streamlining: buffering, commit and cancel windows.
    Streamlining,
}

impl Display for FeelDomain {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::Tuning => write!(f, "tuning"),
            Self::Juicing => write!(f, "juicing"),
            Self::Streamlining => write!(f, "streamlining"),
        }
    }
}

/// Per-domain stream code, xored into the fixture seed through the shared
/// splitter so neighboring domains never share one stream.
fn domain_code(domain: FeelDomain) -> u64 {
    match domain {
        FeelDomain::Tuning => 11,
        FeelDomain::Juicing => 22,
        FeelDomain::Streamlining => 33,
    }
}

/// Feel quantification failure: which input refused, with both sides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FeelError {
    /// A latency leg is unusable: NaN, infinite or negative milliseconds.
    BadSample {
        /// Which leg the sample came from.
        leg: &'static str,
        /// Offending milliseconds.
        value_ms: f64,
    },
    /// A win share above the full thousand permille; shares never exceed it.
    BadWinShare {
        /// Offending permille.
        permille: u32,
    },
    /// A rubric count with no denominator, or more passes than specs.
    BadRubric {
        /// Passed keypoints.
        passed: u32,
        /// Total keypoints.
        total: u32,
    },
}

impl Display for FeelError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match *self {
            Self::BadSample { leg, value_ms } => {
                write!(f, "feel: {leg} sample is unusable ({value_ms} ms)")
            }
            Self::BadWinShare { permille } => {
                write!(f, "feel: win share {permille} above 1000 permille")
            }
            Self::BadRubric { passed, total } => {
                write!(f, "feel: rubric {passed}/{total} has no passing rate")
            }
        }
    }
}

impl std::error::Error for FeelError {}

/// Verdict for one domain-varied run against its baseline run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DomainVerdict {
    /// Domain varied between the two runs.
    pub domain: FeelDomain,
    /// Baseline present p95 in ms.
    pub baseline_p95_ms: f64,
    /// Varied present p95 in ms.
    pub varied_p95_ms: f64,
    /// Varied minus baseline in ms; negative means the domain got faster.
    pub rise_ms: f64,
    /// True while the varied run clears the shared latency bar.
    pub latency_pass: bool,
    /// True while the rise stays within [`KILL_RISE_MS`].
    pub kill_pass: bool,
    /// True while the win share sits inside the accepted band.
    pub win_pass: bool,
    /// True while the rubric rate reaches [`RUBRIC_PASS_PERMILLE`].
    pub rubric_pass: bool,
    /// True while every bar above passes.
    pub overall_pass: bool,
}

impl DomainVerdict {
    /// One stable line for the perf report: the domain, both legs, the
    /// rise, each bar and the verdict. Numbers vary per input; names
    /// never do. Report text only; the gate path never calls this.
    #[must_use]
    pub fn summary(&self) -> String {
        let verdict = if self.overall_pass { "pass" } else { "FAIL" };
        format!(
            "feel domain={} baseline_p95={:.3}ms varied_p95={:.3}ms \
             rise={:.3}ms latency={} kill={} win={} rubric={} verdict={verdict}\n",
            self.domain,
            self.baseline_p95_ms,
            self.varied_p95_ms,
            self.rise_ms,
            push(self.latency_pass),
            push(self.kill_pass),
            push(self.win_pass),
            push(self.rubric_pass),
        )
    }
}

/// Verdict word for one bar inside [`DomainVerdict::summary`].
fn push(pass: bool) -> &'static str {
    if pass {
        "pass"
    } else {
        "FAIL"
    }
}

/// Score one domain-varied run: latency bar, kill-on-rise, win band and
/// rubric rate over plain numbers. Pure over inputs: no heap, no clock.
///
/// # Errors
///
/// [`FeelError::BadSample`] on a NaN, infinite or negative leg,
/// [`FeelError::BadWinShare`] past the full thousand permille,
/// [`FeelError::BadRubric`] with no denominator or more passes than specs.
pub fn assess_domain(
    domain: FeelDomain,
    baseline_p95_ms: f64,
    varied_p95_ms: f64,
    win_permille: u32,
    rubric_passed: u32,
    rubric_total: u32,
) -> Result<DomainVerdict, FeelError> {
    for (leg, value_ms) in [("baseline", baseline_p95_ms), ("varied", varied_p95_ms)] {
        if !value_ms.is_finite() || value_ms < 0.0 {
            return Err(FeelError::BadSample { leg, value_ms });
        }
    }
    if win_permille > SHARE_FULL_PERMILLE {
        return Err(FeelError::BadWinShare {
            permille: win_permille,
        });
    }
    if rubric_total == 0 || rubric_passed > rubric_total {
        return Err(FeelError::BadRubric {
            passed: rubric_passed,
            total: rubric_total,
        });
    }
    let latency_pass = varied_p95_ms <= PRESENT_P95_BUDGET_MS;
    let rise_ms = varied_p95_ms - baseline_p95_ms;
    let kill_pass = rise_ms <= KILL_RISE_MS;
    let win_pass = (WIN_LO_PERMILLE..=WIN_HI_PERMILLE).contains(&win_permille);
    let rate = u64::from(rubric_passed) * 1000 / u64::from(rubric_total);
    let rubric_pass = rate >= u64::from(RUBRIC_PASS_PERMILLE);
    let overall_pass = latency_pass && kill_pass && win_pass && rubric_pass;
    Ok(DomainVerdict {
        domain,
        baseline_p95_ms,
        varied_p95_ms,
        rise_ms,
        latency_pass,
        kill_pass,
        win_pass,
        rubric_pass,
        overall_pass,
    })
}

/// Score one domain from two ranked latency reports: the present p95 legs
/// rank through the shared ranker, so tick math and this rubric read
/// identically. Thin projection over [`assess_domain`]; same errors.
pub fn assess_reports(
    domain: FeelDomain,
    baseline: &LatencyReport,
    varied: &LatencyReport,
    win_permille: u32,
    rubric_passed: u32,
    rubric_total: u32,
) -> Result<DomainVerdict, FeelError> {
    assess_domain(
        domain,
        baseline.present_p95_ms,
        varied.present_p95_ms,
        win_permille,
        rubric_passed,
        rubric_total,
    )
}

/// Seeded rows for one domain: the shared three-frame fixture on a
/// per-domain stream. Same seed and domain give the same rows on every
/// machine; every domain ranks at the bar by construction, so the pass
/// path measures the ranker, never a restated constant.
#[must_use]
pub fn fixture_rows(
    seed: u64,
    domain: FeelDomain,
) -> (Vec<InputEvent>, Vec<SimEffect>, Vec<FramePresent>) {
    let spread = splitmix64(seed.wrapping_add(domain_code(domain)));
    crate::latency::fixture_3f(spread, HEADLESS_SAMPLES)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::latency::{self, LatencyError};

    const SEED: u64 = 7;
    const WIN_MID_PERMILLE: u32 = 500;
    const RUBRIC_PASSED: u32 = 27;
    const RUBRIC_TOTAL: u32 = 30;

    fn measured(domain: FeelDomain) -> LatencyReport {
        let (events, effects, presents) = fixture_rows(SEED, domain);
        latency::measure(&events, &effects, &presents).expect("fixture rows join")
    }

    fn varied_rows(
        events: &[InputEvent],
        sim_frames: u64,
        present_frames: u64,
    ) -> (Vec<SimEffect>, Vec<FramePresent>) {
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
        (effects, presents)
    }

    #[test]
    fn all_three_domains_pass_at_bar_x2() {
        for _ in 0..2 {
            for domain in [
                FeelDomain::Tuning,
                FeelDomain::Juicing,
                FeelDomain::Streamlining,
            ] {
                let baseline = measured(domain);
                let varied = measured(domain);
                assert_eq!(baseline.present_p95_ms, 50.0);
                let verdict = assess_reports(
                    domain,
                    &baseline,
                    &varied,
                    WIN_MID_PERMILLE,
                    RUBRIC_PASSED,
                    RUBRIC_TOTAL,
                )
                .expect("clean bars assess");
                assert!(verdict.latency_pass, "{verdict:?}");
                assert!(verdict.kill_pass, "{verdict:?}");
                assert!(verdict.win_pass, "{verdict:?}");
                assert!(verdict.rubric_pass, "{verdict:?}");
                assert!(verdict.overall_pass, "{verdict:?}");
                assert_eq!(verdict.rise_ms, 0.0);
                let text = verdict.summary();
                assert!(text.contains(&domain.to_string()), "{text}");
                assert!(text.contains("verdict=pass"), "{text}");
                println!("{text}");
            }
        }
    }

    #[test]
    fn four_frame_varied_fails_latency_and_kill() {
        let domain = FeelDomain::Juicing;
        let (events, _, _) = fixture_rows(SEED, domain);
        let baseline = measured(domain);
        let (effects, presents) = varied_rows(&events, 1, 4);
        let varied = latency::measure(&events, &effects, &presents).expect("rows join");
        assert_eq!(varied.present_p95_ms, 4.0 * 1000.0 / latency::TICK_HZ);
        let verdict = assess_reports(
            domain,
            &baseline,
            &varied,
            WIN_MID_PERMILLE,
            RUBRIC_PASSED,
            RUBRIC_TOTAL,
        )
        .expect("slow legs still assess");
        assert!(!verdict.latency_pass, "{verdict:?}");
        assert!(!verdict.kill_pass, "{verdict:?}");
        assert!(verdict.win_pass, "{verdict:?}");
        assert!(verdict.rubric_pass, "{verdict:?}");
        assert!(!verdict.overall_pass, "{verdict:?}");
        assert!(verdict.summary().contains("verdict=FAIL"));
        assert!(latency::check_p2_bar(&varied).is_err());
    }

    #[test]
    fn win_share_outside_band_fails_only_win() {
        let domain = FeelDomain::Tuning;
        let baseline = measured(domain);
        for share in [WIN_LO_PERMILLE - 1, WIN_HI_PERMILLE + 1] {
            let verdict = assess_reports(
                domain,
                &baseline,
                &baseline,
                share,
                RUBRIC_PASSED,
                RUBRIC_TOTAL,
            )
            .expect("off-band share still assesses");
            assert!(!verdict.win_pass, "{verdict:?}");
            assert!(verdict.latency_pass, "{verdict:?}");
            assert!(verdict.kill_pass, "{verdict:?}");
            assert!(verdict.rubric_pass, "{verdict:?}");
            assert!(!verdict.overall_pass, "{verdict:?}");
        }
        for share in [WIN_LO_PERMILLE, WIN_MID_PERMILLE, WIN_HI_PERMILLE] {
            let verdict = assess_reports(
                domain,
                &baseline,
                &baseline,
                share,
                RUBRIC_PASSED,
                RUBRIC_TOTAL,
            )
            .expect("in-band share assesses");
            assert!(verdict.win_pass, "{verdict:?}");
        }
    }

    #[test]
    fn rubric_below_ninety_fails_only_rubric() {
        let domain = FeelDomain::Streamlining;
        let baseline = measured(domain);
        let verdict = assess_reports(domain, &baseline, &baseline, WIN_MID_PERMILLE, 26, 30)
            .expect("low rubric still assesses");
        assert!(!verdict.rubric_pass, "{verdict:?}");
        assert!(!verdict.overall_pass, "{verdict:?}");
        assert!(verdict.latency_pass, "{verdict:?}");
        assert!(verdict.kill_pass, "{verdict:?}");
        assert!(verdict.win_pass, "{verdict:?}");
        let full = assess_reports(domain, &baseline, &baseline, WIN_MID_PERMILLE, 30, 30)
            .expect("full rubric assesses");
        assert!(full.rubric_pass, "{full:?}");
    }

    #[test]
    fn bad_inputs_refuse_loudly() {
        let domain = FeelDomain::Tuning;
        let baseline = measured(domain);
        for bad in [f64::NAN, f64::INFINITY, -1.0] {
            assert!(
                matches!(
                    assess_domain(domain, bad, 50.0, WIN_MID_PERMILLE, 27, 30),
                    Err(FeelError::BadSample { .. })
                ),
                "baseline {bad} refuses"
            );
            assert!(
                matches!(
                    assess_domain(domain, 50.0, bad, WIN_MID_PERMILLE, 27, 30),
                    Err(FeelError::BadSample { .. })
                ),
                "varied {bad} refuses"
            );
        }
        assert_eq!(
            assess_domain(domain, 50.0, 50.0, SHARE_FULL_PERMILLE + 1, 27, 30),
            Err(FeelError::BadWinShare {
                permille: SHARE_FULL_PERMILLE + 1
            })
        );
        assert_eq!(
            assess_domain(domain, 50.0, 50.0, WIN_MID_PERMILLE, 3, 0),
            Err(FeelError::BadRubric {
                passed: 3,
                total: 0
            })
        );
        assert_eq!(
            assess_domain(domain, 50.0, 50.0, WIN_MID_PERMILLE, 4, 3),
            Err(FeelError::BadRubric {
                passed: 4,
                total: 3
            })
        );
        let cases = [
            FeelError::BadSample {
                leg: "varied",
                value_ms: f64::NAN,
            },
            FeelError::BadWinShare { permille: 1001 },
            FeelError::BadRubric {
                passed: 1,
                total: 0,
            },
        ];
        for error in &cases {
            assert!(!error.to_string().is_empty(), "{error:?} explains itself");
            let _: &dyn std::error::Error = error;
        }
        assert_eq!(
            assess_reports(domain, &baseline, &baseline, WIN_MID_PERMILLE, 1, 0),
            Err(FeelError::BadRubric {
                passed: 1,
                total: 0
            })
        );
        assert_ne!(LatencyError::EmptyInput.to_string(), String::new());
    }

    #[test]
    fn scoring_allocates_nothing_x2() {
        let (_, seen) = crate::alloc::measure(|| std::hint::black_box(vec![0u8; 64]));
        assert!(seen.count >= 1, "alloc probe is live");
        for _ in 0..2 {
            let baseline = measured(FeelDomain::Tuning);
            let varied = measured(FeelDomain::Juicing);
            let (verdict, observed) = crate::alloc::measure(|| {
                assess_reports(
                    FeelDomain::Tuning,
                    &baseline,
                    &varied,
                    WIN_MID_PERMILLE,
                    RUBRIC_PASSED,
                    RUBRIC_TOTAL,
                )
            });
            assert!(verdict.is_ok());
            assert_eq!(observed.bytes, 0, "scoring allocates bytes");
            assert_eq!(observed.count, 0, "scoring allocates calls");
            let (_, observed) = crate::alloc::measure(|| {
                assess_domain(
                    FeelDomain::Streamlining,
                    50.0,
                    50.0,
                    WIN_MID_PERMILLE,
                    RUBRIC_PASSED,
                    RUBRIC_TOTAL,
                )
            });
            assert_eq!(observed.bytes, 0, "scalar scoring allocates bytes");
            assert_eq!(observed.count, 0, "scalar scoring allocates calls");
        }
    }

    #[test]
    fn same_rows_same_verdict() {
        let first = fixture_rows(SEED, FeelDomain::Tuning);
        let second = fixture_rows(SEED, FeelDomain::Tuning);
        assert_eq!(first, second, "seeded fixture replays exactly");
        let left = measured(FeelDomain::Tuning);
        let right = measured(FeelDomain::Tuning);
        assert_eq!(left, right, "same rows give the same report");
        let a = assess_reports(
            FeelDomain::Tuning,
            &left,
            &right,
            WIN_MID_PERMILLE,
            RUBRIC_PASSED,
            RUBRIC_TOTAL,
        )
        .expect("clean bars assess");
        let b = assess_reports(
            FeelDomain::Tuning,
            &left,
            &right,
            WIN_MID_PERMILLE,
            RUBRIC_PASSED,
            RUBRIC_TOTAL,
        )
        .expect("clean bars assess");
        assert_eq!(a, b, "same inputs give the same verdict");
    }

    #[test]
    fn neighboring_seeds_and_domains_do_not_alias() {
        let first = fixture_rows(SEED, FeelDomain::Tuning);
        let second = fixture_rows(SEED + 1, FeelDomain::Tuning);
        assert_ne!(first.0, second.0, "neighboring seeds spread apart");
        let tuning = fixture_rows(SEED, FeelDomain::Tuning);
        let juicing = fixture_rows(SEED, FeelDomain::Juicing);
        let streamlining = fixture_rows(SEED, FeelDomain::Streamlining);
        assert_ne!(tuning.0, juicing.0, "tuning and juicing streams differ");
        assert_ne!(juicing.0, streamlining.0, "juicing and streamlining differ");
        assert_eq!(FeelDomain::Tuning.to_string(), "tuning");
        assert_eq!(FeelDomain::Juicing.to_string(), "juicing");
        assert_eq!(FeelDomain::Streamlining.to_string(), "streamlining");
    }
}
