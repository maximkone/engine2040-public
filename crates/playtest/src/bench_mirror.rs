//! Public-benchmark mirror report: the 30-task score table over the live HS20
//! execution, printed beside each suite's published headline and the rival
//! MCP doors we did not run.
//!
//! Consumer: the AI-door bench proof (`cargo test -p engine-playtest --lib
//! bench_mirror` prints the table with `--nocapture`). Owner: `mirror` owns
//! the execution, `tasks` owns the catalog; this module owns only the joined
//! table, one row per catalog task with its per-backend scores. Backends are
//! the eval labels in recorded-replay mode: no model, socket or clock is
//! read. Units: counts of tasks. Determinism: the same build prints the same
//! text. Honesty: a published figure measures other tasks, models and
//! scaffolds, so it sits beside ours and is never subtracted from it; a rival
//! we did not run prints `UNKNOWN`.

use crate::eval::{backend_label, BACKENDS};
use crate::mirror::run_mirror;
use crate::mirror_kit::{triple, SuiteInfo, RIVALS};
use crate::tasks::{self, TaskDef, TASK_COUNT};

/// Backends scored per row, in [`BACKENDS`] order.
pub const BACKEND_COUNT: usize = 3;

/// One task's score across backends, in [`BACKENDS`] order.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct BenchRow {
    /// Catalog descriptor (id, suite, bucket, shape).
    pub task: TaskDef,
    /// The reference submission solves, per backend.
    pub solved: [bool; BACKEND_COUNT],
    /// The weak submission is rejected, per backend.
    pub caught: [bool; BACKEND_COUNT],
}

impl BenchRow {
    /// True when every backend agrees on solve and reject.
    pub fn parity(&self) -> bool {
        self.solved[0] == self.solved[1]
            && self.solved[1] == self.solved[2]
            && self.caught[0] == self.caught[1]
            && self.caught[1] == self.caught[2]
    }

    /// True when the reference solves and the weak submission is rejected on
    /// every backend.
    pub fn green(&self) -> bool {
        self.solved == [true; BACKEND_COUNT] && self.caught == [true; BACKEND_COUNT]
    }
}

/// The joined score table: one row per catalog task plus the suite lines.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BenchReport {
    /// One row per task in catalog order.
    pub rows: Vec<BenchRow>,
    /// Suites behind the rows, in report order.
    pub suites: Vec<SuiteInfo>,
}

/// Slot of one backend label in the per-row arrays.
fn slot_of(backend: &str) -> Option<usize> {
    BACKENDS
        .iter()
        .position(|slot| backend_label(*slot) == backend)
}

/// Run the mirror and join every live row with its catalog descriptor.
/// Catalog ids always cover the live ids (both read the same slice tables),
/// so a live row without a descriptor keeps its scores under a blank shape
/// instead of vanishing from the table.
pub fn run_bench_mirror() -> BenchReport {
    debug_assert_eq!(BACKENDS.len(), BACKEND_COUNT);
    let mirror = run_mirror();
    let mut suites = Vec::with_capacity(mirror.slices.len());
    for slice in &mirror.slices {
        suites.push(*slice.suite);
    }
    let mut rows = Vec::with_capacity(TASK_COUNT);
    for def in tasks::tasks() {
        let mut solved = [false; BACKEND_COUNT];
        let mut caught = [false; BACKEND_COUNT];
        for (slot, backend) in BACKENDS.iter().enumerate() {
            let label = backend_label(*backend);
            if let Some(live) = mirror
                .slices
                .iter()
                .flat_map(|slice| slice.rows.iter())
                .find(|row| row.task == def.id && row.backend == label)
            {
                solved[slot] = live.reference.pass;
                caught[slot] = !live.weak.pass;
            }
        }
        rows.push(BenchRow {
            task: def,
            solved,
            caught,
        });
    }
    BenchReport { rows, suites }
}

impl BenchReport {
    /// Tasks in the table.
    pub fn tasks(&self) -> usize {
        self.rows.len()
    }

    /// Tasks whose reference solves, on one backend label.
    pub fn solved(&self, backend: &str) -> usize {
        let Some(slot) = slot_of(backend) else {
            return 0;
        };
        self.rows.iter().filter(|row| row.solved[slot]).count()
    }

    /// Tasks whose weak submission is rejected, on one backend label.
    pub fn caught(&self, backend: &str) -> usize {
        let Some(slot) = slot_of(backend) else {
            return 0;
        };
        self.rows.iter().filter(|row| row.caught[slot]).count()
    }

    /// Tasks with identical scores on every backend, plus the divergent ids.
    pub fn parity(&self) -> (usize, Vec<&'static str>) {
        let mut identical = 0;
        let mut divergent = Vec::new();
        for row in &self.rows {
            if row.parity() {
                identical += 1;
            } else {
                divergent.push(row.task.id);
            }
        }
        (identical, divergent)
    }

    /// Rival cells known and unknown across every suite.
    pub fn rival_cells(&self) -> (usize, usize) {
        let cells = self.suites.iter().flat_map(|suite| suite.rivals);
        let unknown = cells.clone().filter(|cell| cell.is_unknown()).count();
        (cells.count() - unknown, unknown)
    }

    /// The score table: one line per task, totals, published figures.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "bench mirror: {} tasks, {} suites, mock/local/remote, recorded replay, no model call\n",
            self.tasks(),
            self.suites.len()
        ));
        out.push_str(&format!(
            "{:<8} {:<14} {:<10} {:<14} {:<14} {:<6}  {}\n",
            "task",
            "suite",
            "category",
            "solved m/l/r",
            "caught m/l/r",
            "parity",
            RIVALS.join(" | ")
        ));
        for row in &self.rows {
            out.push_str(&format!(
                "{:<8} {:<14} {:<10} {:<14} {:<14} {:<6}  {}\n",
                row.task.id,
                row.task.suite,
                row.task.category,
                pass_triple(&row.solved),
                pass_triple(&row.caught),
                if row.parity() { "same" } else { "split" },
                row.task.shape,
            ));
        }
        let (identical, divergent) = self.parity();
        let divergent = if divergent.is_empty() {
            "none".to_owned()
        } else {
            divergent.join(",")
        };
        out.push_str(&format!(
            "total {} solved {} caught {} parity {identical}/{} divergent={divergent}\n",
            self.tasks(),
            triple(|label| self.solved(label)),
            triple(|label| self.caught(label)),
            self.tasks(),
        ));
        let (known, unknown) = self.rival_cells();
        out.push_str(&format!(
            "rival cells: {known} known, {unknown} UNKNOWN (not run, never estimated)\n"
        ));
        for suite in &self.suites {
            let rivals: Vec<String> = suite.rivals.iter().map(|claim| claim.text()).collect();
            out.push_str(&format!(
                "  {} rivals: {}\n",
                suite.name,
                rivals.join(" | ")
            ));
        }
        out.push_str("published headline of each suite (other tasks, models and scaffolds; not comparable to the columns above):\n");
        for suite in &self.suites {
            out.push_str(&format!("  {}: {}\n", suite.name, suite.published.text()));
        }
        out
    }
}

/// `pass/fail` per backend, in report order.
fn pass_triple(flags: &[bool; BACKEND_COUNT]) -> String {
    flags
        .iter()
        .map(|flag| if *flag { "pass" } else { "fail" }.to_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// `pass/fail` per backend, in report order.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::mirror_kit::Claim;

    #[test]
    fn thirty_tasks_solve_and_match_on_every_backend() {
        let report = run_bench_mirror();
        assert_eq!(report.tasks(), TASK_COUNT);
        assert_eq!(report.suites.len(), tasks::SUITE_COUNT);
        for backend in ["mock", "local", "remote"] {
            assert_eq!(report.solved(backend), TASK_COUNT, "{backend} solved");
            assert_eq!(report.caught(backend), TASK_COUNT, "{backend} caught");
        }
        assert_eq!(report.parity(), (TASK_COUNT, Vec::new()));
        assert!(report.rows.iter().all(BenchRow::green));
    }

    #[test]
    fn every_row_joins_the_catalog_and_the_live_execution() {
        let report = run_bench_mirror();
        let mirror = run_mirror();
        for row in &report.rows {
            assert_eq!(tasks::task(row.task.id), Some(row.task));
            let live = mirror
                .slices
                .iter()
                .flat_map(|slice| slice.rows.iter())
                .find(|live| live.task == row.task.id && live.backend == "mock")
                .expect("live mock row for every catalog task");
            assert_eq!(row.task.category, live.category, "bucket must match");
            assert_eq!(row.task.shape, live.shape, "shape must match");
        }
    }

    #[test]
    fn a_diverging_backend_shows_up_in_the_table() {
        let mut report = run_bench_mirror();
        let row = report
            .rows
            .iter_mut()
            .find(|row| row.task.id == "cr03")
            .expect("cr03 bench row");
        row.solved[2] = false;
        assert!(!row.parity());
        assert!(!row.green());
        assert_eq!(report.parity(), (TASK_COUNT - 1, vec!["cr03"]));
        assert!(report.to_text().contains("divergent=cr03"));
    }

    #[test]
    fn unknown_backends_score_zero_and_keep_their_shape() {
        let report = run_bench_mirror();
        assert_eq!(report.solved("zzz"), 0, "unknown backends solve nothing");
        assert_eq!(report.caught("zzz"), 0, "unknown backends catch nothing");
    }

    #[test]
    fn the_report_reproduces_byte_for_byte() {
        let first = run_bench_mirror();
        let second = run_bench_mirror();
        assert_eq!(first.to_text(), second.to_text());
        assert_eq!(first, second);
    }

    #[test]
    fn report_prints_the_score_table() {
        let report = run_bench_mirror();
        let text = report.to_text();
        println!("{text}");
        assert!(text.starts_with("bench mirror: 30 tasks, 3 suites"));
        for suite in &report.suites {
            assert!(text.contains(suite.name), "table must name {}", suite.name);
            let Claim::Quoted {
                text: figure,
                source,
            } = suite.published
            else {
                panic!("{} publishes a headline we quote", suite.name);
            };
            assert!(!figure.is_empty());
            assert!(source.starts_with("https://arxiv.org/abs/"), "{source}");
        }
        for rival in RIVALS {
            assert!(text.contains(rival), "table must name {rival}");
        }
        assert_eq!(report.rival_cells(), (0, 9), "no rival was run");
        assert!(text.matches("UNKNOWN").count() >= 9, "{text}");
    }
}
