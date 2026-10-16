//! Public-benchmark mirror: 30 tasks in the shape of three public game-agent
//! suites, run through the eval door on mock, local and remote, scored by
//! deterministic checks, printed beside each suite's published headline and
//! the rival MCP doors we did not run.
//!
//! Consumer: the AI-door proof (`cargo test -p engine-playtest --lib mirror`
//! prints the table with `--nocapture`). Owner: this module only aggregates;
//! each slice owns its tasks and published figure. Backends are the eval
//! labels in recorded-replay mode: the contract, policy, revision and
//! fallback paths are the real provider crate, the proposals are replayed
//! from tapes, and no model, socket or clock is read.
//! Units: counts of tasks. Determinism: the same build prints the same text.
//! Honesty: a published figure measures other tasks, models and scaffolds, so
//! it sits beside ours and is never subtracted from it; a rival we did not
//! run prints `UNKNOWN`.

use crate::mirror_kit::{triple, SliceReport, RIVALS};
use crate::{mirror_craft, mirror_gdb, mirror_logic};

/// Tasks per slice the proof expects.
pub const SLICE_TASKS: usize = 10;

/// The 30-task mirror: one report per suite slice.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct MirrorReport {
    /// Slice reports in suite order.
    pub slices: Vec<SliceReport>,
}

/// Run all three slices on every backend.
pub fn run_mirror() -> MirrorReport {
    MirrorReport {
        slices: vec![mirror_gdb::run(), mirror_craft::run(), mirror_logic::run()],
    }
}

impl MirrorReport {
    /// Tasks across the slices.
    pub fn tasks(&self) -> usize {
        self.slices.iter().map(SliceReport::tasks).sum()
    }

    /// Reference submissions that solve, on one backend.
    pub fn solved(&self, backend: &str) -> usize {
        self.slices.iter().map(|slice| slice.solved(backend)).sum()
    }

    /// Weak submissions the checkers reject, on one backend.
    pub fn caught(&self, backend: &str) -> usize {
        self.slices.iter().map(|slice| slice.caught(backend)).sum()
    }

    /// Tasks with identical outcomes on every backend, plus the divergent ids.
    pub fn parity(&self) -> (usize, Vec<&'static str>) {
        let mut identical = 0;
        let mut divergent = Vec::new();
        for slice in &self.slices {
            let (same, split) = slice.parity();
            identical += same;
            divergent.extend(split);
        }
        (identical, divergent)
    }

    /// Rival cells known and unknown across every suite.
    pub fn rival_cells(&self) -> (usize, usize) {
        let cells = self.slices.iter().flat_map(|slice| slice.suite.rivals);
        let unknown = cells.clone().filter(|cell| cell.is_unknown()).count();
        (cells.count() - unknown, unknown)
    }

    /// The score table: one line per suite, totals, published figures, then
    /// every slice's own report.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "AI door mirror: {} tasks, {} suites, mock/local/remote, recorded replay, no model call\n",
            self.tasks(),
            self.slices.len()
        ));
        out.push_str(&format!(
            "{:<16} {:>5}  {:<14} {:<14} {:>6}  {}\n",
            "suite",
            "tasks",
            "solved m/l/r",
            "caught m/l/r",
            "parity",
            RIVALS.join(" | ")
        ));
        for slice in &self.slices {
            let rivals: Vec<String> = slice
                .suite
                .rivals
                .iter()
                .map(|claim| claim.text())
                .collect();
            out.push_str(&format!(
                "{:<16} {:>5}  {:<14} {:<14} {:>6}  {}\n",
                slice.suite.name,
                slice.tasks(),
                triple(|label| slice.solved(label)),
                triple(|label| slice.caught(label)),
                format!("{}/{}", slice.parity().0, slice.tasks()),
                rivals.join(" | ")
            ));
        }
        let (identical, divergent) = self.parity();
        let divergent = if divergent.is_empty() {
            "none".to_owned()
        } else {
            divergent.join(",")
        };
        out.push_str(&format!(
            "{:<16} {:>5}  {:<14} {:<14} {:>6}\n",
            "total",
            self.tasks(),
            triple(|label| self.solved(label)),
            triple(|label| self.caught(label)),
            format!("{identical}/{}", self.tasks()),
        ));
        let (known, unknown) = self.rival_cells();
        out.push_str(&format!(
            "rival cells: {known} known, {unknown} UNKNOWN (not run, never estimated); divergent tasks: {divergent}\n"
        ));
        out.push_str("published headline of each suite (other tasks, models and scaffolds; not comparable to the columns above):\n");
        for slice in &self.slices {
            out.push_str(&format!(
                "  {}: {}\n",
                slice.suite.name,
                slice.suite.published.text()
            ));
        }
        for slice in &self.slices {
            out.push('\n');
            out.push_str(&slice.to_text());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mirror_kit::Claim;

    #[test]
    fn thirty_tasks_solve_and_match_on_every_backend() {
        let report = run_mirror();
        assert_eq!(report.tasks(), 3 * SLICE_TASKS);
        for backend in ["mock", "local", "remote"] {
            assert_eq!(report.solved(backend), 30, "{backend} solved");
            assert_eq!(report.caught(backend), 30, "{backend} caught");
        }
        assert_eq!(report.parity(), (30, Vec::new()));
    }

    #[test]
    fn task_ids_are_unique_across_the_three_slices() {
        let report = run_mirror();
        let mut ids: Vec<&str> = report
            .slices
            .iter()
            .flat_map(|slice| slice.rows.iter())
            .filter(|row| row.backend == "mock")
            .map(|row| row.task)
            .collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(total, 30);
        assert_eq!(ids.len(), 30, "a task id is used twice");
    }

    #[test]
    fn rivals_stay_unknown_and_published_figures_carry_a_source() {
        let report = run_mirror();
        assert_eq!(report.rival_cells(), (0, 9), "no rival was run");
        for slice in &report.slices {
            assert!(slice.suite.rivals.iter().all(|claim| claim.is_unknown()));
            let Claim::Quoted { text, source } = slice.suite.published else {
                panic!("{} publishes a headline we quote", slice.suite.name);
            };
            assert!(source.starts_with("https://arxiv.org/abs/"), "{source}");
            assert!(!text.is_empty());
        }
        let text = report.to_text();
        assert!(text.matches("UNKNOWN").count() >= 9, "{text}");
    }

    #[test]
    fn the_report_reproduces_byte_for_byte() {
        let first = run_mirror();
        let second = run_mirror();
        assert_eq!(first.to_text(), second.to_text());
        assert_eq!(first, second);
    }

    #[test]
    fn a_diverging_backend_shows_up_in_the_aggregate() {
        let mut report = run_mirror();
        let row = report.slices[1]
            .rows
            .iter_mut()
            .find(|row| row.task == "cr03" && row.backend == "remote")
            .expect("cr03 remote row");
        row.reference.hash ^= 1;
        assert_eq!(report.parity(), (29, vec!["cr03"]));
        assert!(report.to_text().contains("divergent tasks: cr03"));
    }

    #[test]
    fn report_prints_the_score_table() {
        let text = run_mirror().to_text();
        println!("{text}");
        assert!(text.starts_with("AI door mirror: 30 tasks, 3 suites"));
        for name in ["GameDevBench", "CraftBench-UE", "GameLogicBench"] {
            assert!(text.contains(name), "table must name {name}");
        }
        for rival in RIVALS {
            assert!(text.contains(rival), "table must name {rival}");
        }
    }
}
