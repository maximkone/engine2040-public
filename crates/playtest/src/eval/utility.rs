//! Efficiency-utility column of the EV1 report (AX11).
//!
//! Each of the 75 rows gets what it spent, model turns against engine tool
//! calls, next to the reference floor of its task, and an efficiency score
//! that only a green row (one that did its designed outcome) can earn.
//! Reported, never gated: nothing here returns a verdict and no test fails
//! on a score.
//!
//! One command reproduces the report text and its columns:
//! `cargo test -p engine-playtest --lib eval::utility -- --nocapture`.
//!
//! Spend model: an authoring row costs one model turn and a play row none.
//! Tool calls are what the row ran on the engine: a clean proposal one, a
//! neutral fork window two (it runs the window twice to prove replay), a
//! reseeded run two, and a refused row only the scripted fallback its detail
//! names. Floors live in `fixtures/eval-utility/floors.txt`.
//!
//! Failure mode named: `fallback_tool_overhead`, a row that is green because
//! the harness refused it (timeout, quota) yet still burned a scripted
//! fallback tool call the floor does not need; the plain completed/rejected
//! columns call that row perfect.

use super::*;

/// Floor fixture text, one `task turns tools` line per task.
const FLOORS: &str = include_str!("../../../../fixtures/eval-utility/floors.txt");

/// What a row spent, or what its floor allows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Spend {
    turns: u32,
    tools: u32,
}

/// Floor of one task from the fixture.
fn floor_of(task: &str) -> Option<Spend> {
    FLOORS
        .lines()
        .filter(|line| !line.starts_with('#'))
        .find_map(|line| {
            let mut words = line.split_whitespace();
            (words.next()? == task).then(|| Spend {
                turns: words.next().and_then(|w| w.parse().ok()).unwrap_or(0),
                tools: words.next().and_then(|w| w.parse().ok()).unwrap_or(0),
            })
        })
}

/// Spend of one row, from its task script and recorded detail.
fn spent(task: &EvalTask, row: &EvalRow) -> Spend {
    let turns = u32::from(!matches!(
        task.flow,
        Flow::Run | Flow::Capture | Flow::Verify
    ));
    let tools = if row.detail.starts_with("refused:") {
        u32::from(row.detail.contains(" fallback:"))
    } else {
        match task.script {
            TaskScript::ForkNeutral | TaskScript::RunReseed => 2,
            _ => 1,
        }
    };
    Spend { turns, tools }
}

/// Success-gated efficiency in permille: floor over spend (turns plus tool
/// calls), capped at 1000, zero unless the row is green.
fn efficiency_permille(green: bool, floor: Spend, spend: Spend) -> u32 {
    if !green {
        return 0;
    }
    let (want, got) = (floor.turns + floor.tools, spend.turns + spend.tools);
    if got == 0 {
        return 1000;
    }
    (want * 1000 / got).min(1000)
}

impl EvalReport {
    /// The utility columns for every row plus a summary, as text. Reported,
    /// never gated.
    pub fn utility_text(&self) -> String {
        let mut out = String::from("AX11 efficiency-utility column (reported, never gated)\n");
        out.push_str(
            "task backend green turns tools floor_turns floor_tools turn_over tool_over efficiency_permille\n",
        );
        let mut green_rows = 0u32;
        let mut score_sum = 0u32;
        let mut overhead = Vec::new();
        for row in &self.rows {
            let Some(task) = AUTHORING_TASKS
                .iter()
                .chain(PLAY_TASKS.iter())
                .find(|task| task.id == row.task)
            else {
                continue;
            };
            let Some(floor) = floor_of(row.task) else {
                continue;
            };
            let spend = spent(task, row);
            let green = Self::row_is_green(row);
            let score = efficiency_permille(green, floor, spend);
            let (turn_over, tool_over) = (
                spend.turns.saturating_sub(floor.turns),
                spend.tools.saturating_sub(floor.tools),
            );
            if green {
                green_rows += 1;
                score_sum += score;
                if row.detail.starts_with("refused:") && tool_over > 0 {
                    overhead.push(format!("{}/{}", row.task, row.backend));
                }
            }
            let shown = if green {
                score.to_string()
            } else {
                "-".to_owned()
            };
            out.push_str(&format!(
                "{} {} {} {} {} {} {} {} {} {}\n",
                row.task,
                row.backend,
                green,
                spend.turns,
                spend.tools,
                floor.turns,
                floor.tools,
                turn_over,
                tool_over,
                shown,
            ));
        }
        out.push_str(&format!(
            "green_rows={green_rows} mean_efficiency_permille={}\n",
            score_sum.checked_div(green_rows).unwrap_or(0),
        ));
        out.push_str(&format!(
            "failure_mode fallback_tool_overhead rows={} {}\n",
            overhead.len(),
            overhead.join(","),
        ));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_task_has_one_floor_line() {
        for task in AUTHORING_TASKS.iter().chain(PLAY_TASKS.iter()) {
            assert!(floor_of(task.id).is_some(), "{} has a floor", task.id);
        }
        let lines = FLOORS.lines().filter(|l| !l.starts_with('#')).count();
        assert_eq!(lines, AUTHORING_COUNT + PLAY_COUNT);
    }

    #[test]
    fn the_75_row_utility_report_reproduces_and_names_the_overhead_mode() {
        let report = run_eval();
        let text = report.utility_text();
        println!("{text}");
        assert_eq!(text, run_eval().utility_text(), "reproduces");
        let rows = text.lines().filter(|l| l.split(' ').count() == 10).count();
        assert_eq!(rows, 75 + 1, "75 rows plus the header");
        assert!(text.contains("never gated"));
        // Hand-checked: a10 (timeout) is green on every backend, spends one turn
        // and the scripted fallback tool against a floor of one turn and no tool,
        // so 1 / 2 = 500 permille and one tool over.
        assert!(text.contains("a10 mock true 1 1 1 0 0 1 500\n"), "{text}");
        // A stale refusal takes no fallback and matches its floor exactly.
        assert!(text.contains("a17 mock true 1 0 1 0 0 0 1000\n"), "{text}");
        // Malformed, timeout and quota refusals all fall back: 5 tasks x 3 backends.
        let mode = text.lines().last().expect("summary line");
        assert!(
            mode.starts_with("failure_mode fallback_tool_overhead rows=15 "),
            "{mode}"
        );
        for task in ["a07", "a10", "a11", "a15", "a20"] {
            assert!(mode.contains(&format!("{task}/remote")), "{mode}");
        }
        assert!(
            text.contains("green_rows=75 mean_efficiency_permille=900\n"),
            "{text}"
        );
    }

    #[test]
    fn a_failed_row_earns_nothing() {
        let floor = Spend { turns: 1, tools: 1 };
        assert_eq!(efficiency_permille(false, floor, floor), 0);
        assert_eq!(efficiency_permille(true, floor, floor), 1000);
        assert_eq!(
            efficiency_permille(true, floor, Spend { turns: 1, tools: 3 }),
            500
        );
        let free = Spend { turns: 0, tools: 0 };
        assert_eq!(efficiency_permille(true, free, free), 1000);
    }
}
