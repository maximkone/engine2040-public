//! Advisory agreement probe over versioned trace exports.
//!
//! Consumer: the operator comparing two green trace exports before any
//! offline refit decision; the probe only reports numbers and never tunes.
//! Data owner: the caller owns both reports; this module borrows them and
//! builds a fresh summary. Units: row counts and permille (`0..=1000`).
//! Determinism: pure in the two reports plus the held-out seed; the same
//! inputs twice give the same summary. Errors: typed [`EvalError`] only.

use engine_core::{splitmix64, Fnv64};

use super::eval::{export_trace, import_trace, EvalError, EvalReport, EvalRow};

/// Full agreement in permille: every compared row agrees.
pub const FULL_AGREEMENT_PERMILLE: usize = 1000;
/// Held-out divisor: hashes landing on zero count as held-out, so about one
/// row in five is held out deterministically per seed.
pub const HELD_OUT_DIVISOR: u64 = 5;

/// Advisory agreement summary over two trace exports.
///
/// `total_rows` anchors on the baseline: rows missing from the candidate
/// count as disagreements, while candidate-only rows are ignored (both sides
/// carry the same harness shape, so either case is a loud mismatch through
/// the counts). Verdicts compare completion, rejection and the privacy flag;
/// latency, cost and free text vary across runs and never enter agreement.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct AgreementReport {
    /// Baseline rows compared.
    pub total_rows: usize,
    /// Baseline rows whose verdict triple matches the candidate.
    pub agreed_rows: usize,
    /// Baseline rows assigned to the held-out split.
    pub held_out_rows: usize,
    /// Held-out rows whose verdict triple matches the candidate.
    pub held_out_agreed: usize,
    /// Baseline rows matching their designed outcome.
    pub baseline_green: usize,
    /// Candidate rows matching their designed outcome.
    pub candidate_green: usize,
    /// Baseline row count.
    pub baseline_total: usize,
    /// Candidate row count.
    pub candidate_total: usize,
    /// Held-out seed this summary was computed under.
    pub seed: u64,
}

impl AgreementReport {
    /// Agreement over all baseline rows in permille; empty baselines read as
    /// full agreement so the rule below stays total.
    pub const fn agreement_permille(self) -> usize {
        match self
            .agreed_rows
            .saturating_mul(FULL_AGREEMENT_PERMILLE)
            .checked_div(self.total_rows)
        {
            Some(permille) => permille,
            None => FULL_AGREEMENT_PERMILLE,
        }
    }

    /// Agreement over the held-out rows in permille; an empty split reads as
    /// full agreement for the same reason as [`Self::agreement_permille`].
    pub const fn held_out_permille(self) -> usize {
        match self
            .held_out_agreed
            .saturating_mul(FULL_AGREEMENT_PERMILLE)
            .checked_div(self.held_out_rows)
        {
            Some(permille) => permille,
            None => FULL_AGREEMENT_PERMILLE,
        }
    }

    /// Green rows lost moving baseline to candidate; zero means the candidate
    /// holds every success the baseline had.
    pub const fn success_loss(self) -> usize {
        self.baseline_green.saturating_sub(self.candidate_green)
    }

    /// The graduation rule, reported advisory only: both traces fully green,
    /// zero success loss, and full agreement overall and held-out. This
    /// function never tunes; it only names whether the numbers meet the bar.
    pub const fn meets_two_green_rule(self) -> bool {
        self.baseline_total == self.baseline_green
            && self.candidate_total == self.candidate_green
            && self.baseline_green == self.candidate_green
            && self.agreement_permille() == FULL_AGREEMENT_PERMILLE
            && self.held_out_permille() == FULL_AGREEMENT_PERMILLE
    }

    /// Reproducible advisory text: counts, permille and the rule verdict.
    /// Pure in the summary; the same summary twice gives the same bytes.
    /// This path allocates the output string; it never runs in the tick.
    pub fn to_text(self) -> String {
        let rule = if self.meets_two_green_rule() {
            "holds"
        } else {
            "broken"
        };
        format!(
            "agreement probe (advisory only: no tuning applied)\n\
             rows={} agreed={} agreement_permille={} green_baseline={} green_candidate={} success_loss={}\n\
             held_out seed={} rows={} agreed={} held_out_permille={}\n\
             two-green rule: {rule}\n",
            self.total_rows,
            self.agreed_rows,
            self.agreement_permille(),
            self.baseline_green,
            self.candidate_green,
            self.success_loss(),
            self.seed,
            self.held_out_rows,
            self.held_out_agreed,
            self.held_out_permille(),
        )
    }
}

/// True when the `(task, backend)` cell belongs to the held-out split under
/// `seed`. The assignment hashes the seed plus both labels through the shared
/// hash owner, so every machine and every run splits identically; different
/// seeds split differently. Pure in its inputs.
pub fn is_held_out(task: &str, backend: &str, seed: u64) -> bool {
    let mut hasher = Fnv64::new();
    hasher.write_u64(seed);
    hasher.write_str(task);
    // Separator keeps ("ab", "c") and ("a", "bc") apart.
    hasher.write_u8(0);
    hasher.write_str(backend);
    splitmix64(hasher.finish()).is_multiple_of(HELD_OUT_DIVISOR)
}

/// Find one row by `(task, backend)`; the harness shape holds at most one.
fn find_row<'rows>(report: &'rows EvalReport, task: &str, backend: &str) -> Option<&'rows EvalRow> {
    report
        .rows
        .iter()
        .find(|row| row.task == task && row.backend == backend)
}

/// Probe two reports under one held-out seed. Pure in both reports plus
/// the seed; row order never matters because rows match by key, and missing
/// candidate rows count as disagreements.
///
/// This function allocates nothing beyond the returned summary; the row
/// scans borrow both reports.
pub fn probe_reports(
    baseline: &EvalReport,
    candidate: &EvalReport,
    held_out_seed: u64,
) -> AgreementReport {
    let mut agreed = 0usize;
    let mut held_out_rows = 0usize;
    let mut held_out_agreed = 0usize;
    for row in &baseline.rows {
        let held_out = is_held_out(row.task, row.backend, held_out_seed);
        if held_out {
            held_out_rows += 1;
        }
        let Some(other) = find_row(candidate, row.task, row.backend) else {
            continue;
        };
        let same = row.completed == other.completed
            && row.rejected == other.rejected
            && row.privacy_failure == other.privacy_failure;
        if same {
            agreed += 1;
            if held_out {
                held_out_agreed += 1;
            }
        }
    }
    AgreementReport {
        total_rows: baseline.rows.len(),
        agreed_rows: agreed,
        held_out_rows,
        held_out_agreed,
        baseline_green: baseline.green_rows(),
        candidate_green: candidate.green_rows(),
        baseline_total: baseline.rows.len(),
        candidate_total: candidate.rows.len(),
        seed: held_out_seed,
    }
}

/// Re-import one trace text twice and require byte-identical re-exports.
/// The first re-export may differ from the input (imports tolerate one
/// trailing carriage return per line but never write it); the two
/// re-exports must match exactly.
///
/// Errors: [`EvalError::TraceVersion`] for foreign headers (propagated from
/// the import before any row is touched), [`EvalError::TraceParse`] for
/// malformed bodies or for re-exports that drift.
fn reexport_twice(text: &str, tag: &str) -> Result<(EvalReport, String), EvalError> {
    let first = import_trace(text)?;
    let first_text = export_trace(&first);
    let second = import_trace(&first_text)?;
    let second_text = export_trace(&second);
    if first_text != second_text {
        return Err(EvalError::TraceParse {
            reason: format!(
                "{tag} trace drifts across re-imports; the export must be byte-identical twice"
            ),
        });
    }
    Ok((first, first_text))
}

/// Probe two trace texts under one held-out seed: each side re-imports twice
/// (byte-identical), then the reports compare by verdict triple. Advisory
/// only: the returned summary carries agreement numbers and the rule
/// verdict, never a tuned artifact.
///
/// Errors: [`EvalError::TraceVersion`] for foreign headers,
/// [`EvalError::TraceParse`] for malformed or drifting bodies.
pub fn probe_traces(
    baseline_text: &str,
    candidate_text: &str,
    held_out_seed: u64,
) -> Result<AgreementReport, EvalError> {
    let (baseline, _) = reexport_twice(baseline_text, "baseline")?;
    let (candidate, _) = reexport_twice(candidate_text, "candidate")?;
    Ok(probe_reports(&baseline, &candidate, held_out_seed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::run_eval;

    /// First held-out seed for the two-seed acceptance check.
    const FIRST_SEED: u64 = 2040;
    /// Second held-out seed; partitions differ from [`FIRST_SEED`].
    const SECOND_SEED: u64 = 9077;

    /// Verdict-preserving timing shift standing in for a second run: the
    /// harness rows are fixture-driven, so latency is the run-varying column
    /// while verdicts stay fixed. Green counts only verdicts, so both sides
    /// stay green while the bytes differ.
    fn with_timing_shift(report: &EvalReport) -> EvalReport {
        let mut shifted = report.clone();
        for row in &mut shifted.rows {
            row.latency_ms = row.latency_ms.wrapping_add(1);
        }
        shifted
    }

    /// Import twice and require byte-identical re-exports, mirroring the
    /// probe's own gate so the test posts the acceptance numbers directly.
    fn expect_reimports_twice(text: &str, tag: &str) -> EvalReport {
        let first = import_trace(text).expect("export must re-import");
        let first_text = export_trace(&first);
        assert_eq!(first_text, text, "{tag} re-export must be byte-identical");
        let second = import_trace(&first_text).expect("re-export must re-import");
        let second_text = export_trace(&second);
        assert_eq!(
            second_text, text,
            "{tag} second re-export must be byte-identical"
        );
        second
    }

    #[test]
    fn two_green_exports_reimport_twice_and_agree_with_zero_loss() {
        let baseline = run_eval();
        let candidate = with_timing_shift(&baseline);
        let baseline_text = export_trace(&baseline);
        let candidate_text = export_trace(&candidate);
        assert_ne!(
            baseline_text, candidate_text,
            "the two exports must differ while both stay green"
        );
        let baseline_back = expect_reimports_twice(&baseline_text, "baseline");
        let candidate_back = expect_reimports_twice(&candidate_text, "candidate");
        for (report, tag) in [(&baseline_back, "baseline"), (&candidate_back, "candidate")] {
            assert_eq!(
                report.green_rows(),
                report.rows.len(),
                "{tag} export must be fully green"
            );
        }
        for seed in [FIRST_SEED, SECOND_SEED] {
            let summary = probe_traces(&baseline_text, &candidate_text, seed)
                .expect("two green traces must probe");
            assert_eq!(summary.total_rows, baseline.rows.len());
            assert_eq!(summary.agreed_rows, summary.total_rows);
            assert_eq!(summary.agreement_permille(), FULL_AGREEMENT_PERMILLE);
            assert_eq!(summary.held_out_permille(), FULL_AGREEMENT_PERMILLE);
            assert_eq!(summary.success_loss(), 0);
            assert!(
                summary.meets_two_green_rule(),
                "two green agreeing traces hold the rule: {}",
                summary.to_text()
            );
            let direct = probe_reports(&baseline, &candidate, seed);
            assert_eq!(direct, summary, "text and report probes must agree");
            let text = summary.to_text();
            assert!(text.contains("advisory only"), "probe is advisory: {text}");
            assert!(text.contains("success_loss=0"), "loss posted: {text}");
        }
    }

    #[test]
    fn flipped_verdict_reports_loss_and_breaks_the_rule() {
        let baseline = run_eval();
        let mut candidate = baseline.clone();
        let head = candidate.rows.first_mut().expect("the harness writes rows");
        head.completed = !head.completed;
        head.rejected = !head.rejected;
        let summary = probe_reports(&baseline, &candidate, FIRST_SEED);
        assert_eq!(summary.agreed_rows, summary.total_rows - 1);
        assert!(summary.agreement_permille() < FULL_AGREEMENT_PERMILLE);
        assert_eq!(summary.success_loss(), 1);
        assert!(
            !summary.meets_two_green_rule(),
            "one lost success breaks the rule: {}",
            summary.to_text()
        );
        // The text probe sees the same loss through the wire format.
        let probed = probe_traces(
            &export_trace(&baseline),
            &export_trace(&candidate),
            FIRST_SEED,
        )
        .expect("verdict flip still probes");
        assert_eq!(probed, summary);
    }

    #[test]
    fn foreign_version_and_truncated_bodies_refuse_typed() {
        let baseline_text = export_trace(&run_eval());
        let forged = baseline_text.replacen(" v1 ", " v2 ", 1);
        let error = probe_traces(&forged, &baseline_text, FIRST_SEED)
            .expect_err("foreign version must refuse");
        assert_eq!(error.code(), "trace_version");
        let mut lines: Vec<&str> = baseline_text.lines().collect();
        lines.pop();
        let dropped = lines.join("\n") + "\n";
        let error =
            probe_traces(&baseline_text, &dropped, FIRST_SEED).expect_err("dropped line must fail");
        assert_eq!(error.code(), "trace_parse");
        let no_trailing = baseline_text
            .strip_suffix('\n')
            .expect("export ends with newline");
        let error = probe_traces(no_trailing, &baseline_text, FIRST_SEED)
            .expect_err("missing newline must fail");
        assert_eq!(error.code(), "trace_parse");
    }

    #[test]
    fn held_out_split_is_deterministic_per_seed() {
        let report = run_eval();
        for row in &report.rows {
            let first_pass = is_held_out(row.task, row.backend, FIRST_SEED);
            let second_pass = is_held_out(row.task, row.backend, FIRST_SEED);
            assert!(
                first_pass == second_pass,
                "same seed twice gives the same cell"
            );
        }
        let held_out = report
            .rows
            .iter()
            .filter(|row| is_held_out(row.task, row.backend, FIRST_SEED))
            .count();
        assert!(
            (1..report.rows.len()).contains(&held_out),
            "the split must hold some out and keep some in"
        );
    }
}
