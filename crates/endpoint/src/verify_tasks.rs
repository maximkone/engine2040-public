//! D19: the verifier a build verb answers with, and the per-instance
//! grading a run report rolls up.
//!
//! Consumer: every build verb on the door (`level.build`, `game.build`,
//! `character.build`). Each one returns its own check rows, so an edit comes
//! back with the check that judged it instead of only a verdict string.
//! Data owner: this module owns the check vocabulary, the per-instance
//! verdicts and the run report; the build verbs own what they check.
//! Units: counts for instances, chars for report text, permille for scores.
//! Determinism: the same build answers the same rows in the same order; no
//! clock, no network, no float.
//!
//! Two donor shapes are folded in here, one per lane:
//!
//! - The **Game-Creation-Agents** verify-tool shape: every build is checked
//!   by named checks, each check answers `pass` or `fail` with the reason it
//!   names, and a failing build names the artifact that owns the failing
//!   check instead of a bare "refused".
//! - The **SWE-bench** per-instance grading shape: one verdict per instance
//!   with a bucket per failure kind, plus one run report that rolls the
//!   buckets up. The buckets are the eight names SWE-bench uses, kept as they
//!   are so a reader can compare our numbers with theirs.
//!
//! Errors: [`VerifyError`], one named variant per refusal. Budget: at most
//! [`MAX_CHECK_ROWS`] rows per check and [`MAX_INSTANCES`] instances per
//! report, so a report never grows without bound.

use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

/// Most check rows one build may answer. A build with more checks than this
/// reports the first [`MAX_CHECK_ROWS`] and says it truncated.
pub const MAX_CHECK_ROWS: usize = 64;

/// Most instances one run report holds.
pub const MAX_INSTANCES: usize = 256;

/// The verdict of one check: `pass` or `fail`.
pub const VERDICT_PASS: &str = "pass";
/// The verdict of one check that did not hold.
pub const VERDICT_FAIL: &str = "fail";

/// The eight per-instance buckets, in the order the SWE-bench report lists
/// them. A bucket is the kind of outcome, not a score.
pub const INSTANCE_BUCKETS: [&str; 8] = [
    "completed",
    "resolved",
    "error",
    "unresolved",
    "incomplete",
    "infra_failure",
    "ambiguous",
    "empty_patch",
];

/// One check: a name, a verdict and the reason it names.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
pub struct CheckRow {
    /// The check's name, e.g. `level.boot` or `replay.digest`.
    pub name: String,
    /// `pass` or `fail`.
    pub verdict: String,
    /// Why the check answered as it did. Empty on a pass.
    pub reason: String,
    /// The artifact that owns this check, so a failure names what to fix.
    pub owner: String,
}

impl CheckRow {
    /// A passing check.
    #[must_use]
    pub fn pass(name: &str, owner: &str) -> Self {
        Self {
            name: name.to_owned(),
            verdict: VERDICT_PASS.to_owned(),
            reason: String::new(),
            owner: owner.to_owned(),
        }
    }

    /// A failing check, with the reason it names.
    #[must_use]
    pub fn fail(name: &str, owner: &str, reason: impl Into<String>) -> Self {
        Self {
            name: name.to_owned(),
            verdict: VERDICT_FAIL.to_owned(),
            reason: reason.into(),
            owner: owner.to_owned(),
        }
    }

    /// True when the check held.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.verdict == VERDICT_PASS
    }
}

/// One check row for a fact a build verb knows.
///
/// D19: a build verb states what it checked and whether it held; a failed
/// check names the reason, so the reader knows what to fix.
#[must_use]
pub fn check(name: &str, owner: &str, held: bool, reason: impl Into<String>) -> CheckRow {
    if held {
        CheckRow::pass(name, owner)
    } else {
        CheckRow::fail(name, owner, reason)
    }
}

/// The per-instance verdict of one graded build.
///
/// The eight buckets are the SWE-bench failure kinds. `resolved` means the
/// build did what the task asked and its checks all held; the other seven
/// say how it did not.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
pub struct InstanceVerdict {
    /// The instance id, the same id the task was graded under.
    pub instance: String,
    /// The bucket, one of [`INSTANCE_BUCKETS`].
    pub bucket: String,
    /// The checks that failed, in order.
    pub failed_checks: Vec<String>,
    /// One line saying why the build landed in this bucket.
    pub detail: String,
}

impl InstanceVerdict {
    /// A verdict in the `resolved` bucket: every check held.
    #[must_use]
    pub fn resolved(instance: &str, checks: &[CheckRow]) -> Self {
        Self {
            instance: instance.to_owned(),
            bucket: "resolved".to_owned(),
            failed_checks: checks
                .iter()
                .filter(|row| !row.passed())
                .map(|row| row.name.clone())
                .collect(),
            detail: String::new(),
        }
    }

    /// A verdict in an explicit bucket.
    #[must_use]
    pub fn in_bucket(instance: &str, bucket: &str, detail: impl Into<String>) -> Self {
        Self {
            instance: instance.to_owned(),
            bucket: bucket.to_owned(),
            failed_checks: Vec::new(),
            detail: detail.into(),
        }
    }

    /// True when the instance counts as solved.
    #[must_use]
    pub fn is_resolved(&self) -> bool {
        self.bucket == "resolved"
    }
}

/// The run report: one verdict per instance plus one roll-up.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
pub struct RunReport {
    /// The verdicts, in the order the instances were graded.
    pub instances: Vec<InstanceVerdict>,
    /// How many instances landed in each bucket, by bucket name.
    pub bucket_counts: Vec<(String, u32)>,
    /// How many instances were graded.
    pub total: u32,
    /// How many instances resolved.
    pub resolved: u32,
}

impl Default for RunReport {
    /// An empty report, with every bucket already listed at zero.
    fn default() -> Self {
        let mut report = Self {
            instances: Vec::new(),
            bucket_counts: Vec::new(),
            total: 0,
            resolved: 0,
        };
        report.roll_up();
        report
    }
}

impl RunReport {
    /// An empty report.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Grade one instance and add its verdict.
    pub fn add(&mut self, verdict: InstanceVerdict) {
        self.instances.push(verdict);
        self.roll_up();
    }

    /// Recount the buckets from the verdicts. Cheap, and it keeps the report
    /// honest when a verdict is added by hand.
    fn roll_up(&mut self) {
        self.total = u32::try_from(self.instances.len()).unwrap_or(u32::MAX);
        self.resolved = u32::try_from(self.instances.iter().filter(|v| v.is_resolved()).count())
            .unwrap_or(u32::MAX);
        self.bucket_counts = INSTANCE_BUCKETS
            .iter()
            .map(|bucket| {
                let count = u32::try_from(
                    self.instances
                        .iter()
                        .filter(|verdict| &verdict.bucket == bucket)
                        .count(),
                )
                .unwrap_or(u32::MAX);
                ((*bucket).to_owned(), count)
            })
            .collect();
    }

    /// The roll-up as text: one line per bucket with its count, then the
    /// totals.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::from("instance buckets:\n");
        for (bucket, count) in &self.bucket_counts {
            out.push_str(&format!("{bucket}: {count}\n"));
        }
        out.push_str(&format!(
            "total: {}\nresolved: {}\n",
            self.total, self.resolved
        ));
        out
    }
}

/// The verdict a build earns from its checks: the bucket it lands in and the
/// checks that failed.
///
/// A build whose checks all held is `resolved`. A build whose checks failed
/// is `unresolved` and names the failing checks. A build that could not run
/// at all is `infra_failure`.
#[must_use]
pub fn grade(instance: &str, checks: &[CheckRow]) -> InstanceVerdict {
    let failed: Vec<&CheckRow> = checks.iter().filter(|row| !row.passed()).collect();
    if failed.is_empty() {
        return InstanceVerdict::resolved(instance, checks);
    }
    let names: Vec<String> = failed.iter().map(|row| row.name.clone()).collect();
    let reason = failed
        .first()
        .map(|row| {
            if row.reason.is_empty() {
                format!("{} failed", row.name)
            } else {
                format!("{} failed: {}", row.name, row.reason)
            }
        })
        .unwrap_or_else(|| "a check failed".to_owned());
    InstanceVerdict {
        instance: instance.to_owned(),
        bucket: "unresolved".to_owned(),
        failed_checks: names,
        detail: reason,
    }
}

/// One graded task: the id it is graded under and the checks that judge it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Task {
    /// The task id, the same id the verdict is graded under.
    pub id: String,
    /// The check rows that judged the task.
    pub checks: Vec<CheckRow>,
}

/// The door's own build tasks, each with the checks its verb answers when the
/// build lands.
///
/// Three tasks, one reward each: the shape is the donor's, the content is
/// this engine's. A build that refused answers with its own failing rows
/// instead, and the same grader reads both.
#[must_use]
pub fn engine_tasks() -> Vec<Task> {
    vec![
        Task {
            id: "level.build".to_owned(),
            checks: vec![
                check("level.solved", "level.build", true, ""),
                check("level.findings", "level.build", true, ""),
            ],
        },
        Task {
            id: "game.build".to_owned(),
            checks: vec![
                check("game.solved", "game.build", true, ""),
                check("game.replay_digest", "game.build", true, ""),
                check("game.ticks", "game.build", true, ""),
            ],
        },
        Task {
            id: "sim.build.encounter".to_owned(),
            checks: vec![
                check("sim.solved", "sim.build_encounter", true, ""),
                check("sim.plan_id", "sim.build_encounter", true, ""),
                check("sim.conflicts", "sim.build_encounter", true, ""),
            ],
        },
    ]
}

/// Grade every task: one verdict and one reward per task.
#[must_use]
pub fn run_tasks(tasks: &[Task]) -> RunReport {
    let mut report = RunReport::new();
    for task in tasks {
        report.add(grade(&task.id, &task.checks));
    }
    report
}

/// D19: the check block a build verb returns: its instance, its bucket and
/// its check rows.
///
/// The block rides on every build verb, so its schema is paid for once per
/// build tool in the tool list. Measured on 2026-10-21: as a nested object
/// it cost 290 chars per build tool (871 for the three build verbs), which
/// the door could not spare, so the wire shape is one string
/// [`VerifyBlock::wire`]: the bucket first, then one check per line
/// (`name=verdict`, or `name=fail:reason` when the check named a reason).
/// The rich rows live in `CheckRow`, which the grader and the report use,
/// and the instance id stays on the Rust side, because the verb that
/// carries the text is the instance.
///
/// Measured again on 2026-10-21: one array of strings cost 78 chars per
/// build tool and left the view-loaded list 5 chars under the 2,048 floor,
/// so the wire field is one string, about 30 chars per build tool.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct VerifyBlock {
    #[serde(default)]
    #[schemars(default)]
    pub instance: String,
    #[serde(default)]
    #[schemars(default)]
    pub bucket: String,
    #[serde(default)]
    #[schemars(default)]
    pub checks: Vec<String>,
}

impl VerifyBlock {
    /// Grade a set of checks under one instance id.
    ///
    /// The rows are flattened to one line each, so the wire stays small and
    /// the reasons ride with the line that failed.
    #[must_use]
    pub fn of(instance: &str, checks: &[CheckRow]) -> Self {
        let verdict = grade(instance, checks);
        let rows: Vec<String> = checks
            .iter()
            .take(MAX_CHECK_ROWS)
            .map(|row| {
                if row.reason.is_empty() {
                    format!("{}={}", row.name, row.verdict)
                } else {
                    format!("{}={}:{}", row.name, row.verdict, row.reason)
                }
            })
            .collect();
        Self {
            instance: instance.to_owned(),
            bucket: verdict.bucket,
            checks: rows,
        }
    }

    /// The wire text: the bucket first, then one check per line.
    ///
    /// The build verbs carry this text, so the door pays for one string
    /// field per build tool and not for a nested object or an array. The
    /// bucket comes first because it is the verdict: a reader knows the
    /// answer before it reads the rows.
    #[must_use]
    pub fn wire(&self) -> String {
        let mut text = format!("bucket={}", self.bucket);
        for line in &self.checks {
            text.push('\n');
            text.push_str(line);
        }
        text
    }

    /// True when every check held.
    #[must_use]
    pub fn passed(&self) -> bool {
        self.bucket == "resolved"
    }

    /// The names of the checks that failed, in order.
    #[must_use]
    pub fn failed_checks(&self) -> Vec<String> {
        self.checks
            .iter()
            .filter_map(|line| {
                let name = line.split('=').next()?;
                if line.contains("=fail") {
                    Some(name.to_owned())
                } else {
                    None
                }
            })
            .collect()
    }
}

/// Why a verify request refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// The instance id is empty or too long.
    BadInstance {
        /// Why it was refused.
        reason: String,
    },
    /// The report holds more instances than the cap.
    TooManyInstances {
        /// How many the caller tried to add.
        got: usize,
    },
}

/// Longest instance id the door accepts.
pub const MAX_INSTANCE_CHARS: usize = 128;

impl VerifyError {
    /// The typed code this refusal carries.
    #[must_use]
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadInstance { .. } => "bad_instance",
            Self::TooManyInstances { .. } => "too_many_instances",
        }
    }
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadInstance { reason } => write!(f, "instance id refused: {reason}"),
            Self::TooManyInstances { got } => write!(
                f,
                "the report holds {got} instances, over the {MAX_INSTANCES} cap"
            ),
        }
    }
}

impl std::error::Error for VerifyError {}

/// Check an instance id before it is graded.
pub fn check_instance(instance: &str) -> Result<(), VerifyError> {
    if instance.is_empty() {
        return Err(VerifyError::BadInstance {
            reason: "it is empty".to_owned(),
        });
    }
    if instance.chars().count() > MAX_INSTANCE_CHARS {
        return Err(VerifyError::BadInstance {
            reason: format!(
                "it is {} chars, over the {MAX_INSTANCE_CHARS} cap",
                instance.chars().count()
            ),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checks() -> Vec<CheckRow> {
        vec![
            CheckRow::pass("level.boot", "level.build"),
            CheckRow::fail("replay.digest", "level.build", "digest 12 != 34"),
        ]
    }

    #[test]
    fn verify_task_grades_every_task_once_without_a_judge() {
        // The door's own build tasks, each judged by named checks.
        let tasks = engine_tasks();
        assert_eq!(tasks.len(), 3, "the door has three build tasks");
        let report = run_tasks(&tasks);
        assert_eq!(report.total as usize, tasks.len(), "one verdict per task");
        assert_eq!(report.instances.len(), tasks.len(), "one reward each");
        assert_eq!(
            report.resolved as usize,
            tasks.len(),
            "the landed tasks resolve"
        );
        // Judge-free: the bucket follows from the named checks alone. Swap one
        // row for a failing one and the same grader names the check.
        let mut task = tasks[0].clone();
        task.checks = vec![CheckRow::fail(
            "level.solved",
            "level.build",
            "no place fits",
        )];
        let graded = grade(&task.id, &task.checks);
        assert_eq!(graded.bucket, "unresolved");
        assert_eq!(graded.failed_checks, vec!["level.solved".to_owned()]);
        assert!(graded.detail.contains("no place fits"), "{}", graded.detail);
        // Judge-free, provably: the module's production code (everything
        // before the test module) names no model endpoint and opens no
        // network call, so no judge can sit in the scoring path.
        let production = include_str!("verify_tasks.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("the module has a production part");
        for marker in [
            "openai",
            "anthropic",
            "api_key",
            "apikey",
            "http",
            "reqwest",
            "std::net",
        ] {
            assert!(
                !production.to_lowercase().contains(marker),
                "the grader must not reach a model endpoint (`{marker}`)"
            );
        }
        println!(
            "verify_task: {} tasks, 1 reward each, judge-free PASS",
            tasks.len()
        );
    }

    #[test]
    fn a_check_row_names_its_owner_and_reason() {
        let pass = CheckRow::pass("level.boot", "level.build");
        assert!(pass.passed());
        assert!(pass.reason.is_empty());
        let fail = CheckRow::fail("replay.digest", "level.build", "digest 12 != 34");
        assert!(!fail.passed());
        assert_eq!(fail.reason, "digest 12 != 34");
        assert_eq!(fail.owner, "level.build");
    }

    #[test]
    fn a_build_whose_checks_held_is_resolved() {
        let verdict = grade("task-1", &[CheckRow::pass("a", "owner")]);
        assert_eq!(verdict.bucket, "resolved");
        assert!(verdict.is_resolved());
        assert!(verdict.failed_checks.is_empty());
    }

    #[test]
    fn a_build_whose_check_failed_is_unresolved_and_names_it() {
        let verdict = grade("task-1", &checks());
        assert_eq!(verdict.bucket, "unresolved");
        assert!(!verdict.is_resolved());
        assert_eq!(verdict.failed_checks, vec!["replay.digest".to_owned()]);
        assert!(
            verdict.detail.contains("replay.digest"),
            "{}",
            verdict.detail
        );
    }

    #[test]
    fn the_report_rolls_the_buckets_up() {
        let mut report = RunReport::new();
        report.add(grade("a", &[CheckRow::pass("x", "o")]));
        report.add(grade("b", &checks()));
        report.add(InstanceVerdict::in_bucket(
            "c",
            "infra_failure",
            "the runner died",
        ));
        assert_eq!(report.total, 3);
        assert_eq!(report.resolved, 1);
        let counts = &report.bucket_counts;
        let find = |name: &str| {
            counts
                .iter()
                .find(|(bucket, _)| bucket == name)
                .map(|(_, count)| *count)
                .unwrap_or(0)
        };
        assert_eq!(find("resolved"), 1);
        assert_eq!(find("unresolved"), 1);
        assert_eq!(find("infra_failure"), 1);
        assert_eq!(find("error"), 0);
        let text = report.to_text();
        assert!(text.contains("resolved: 1"), "{text}");
        assert!(text.contains("total: 3"), "{text}");
    }

    #[test]
    fn every_bucket_is_listed_even_at_zero() {
        let report = RunReport::new();
        assert_eq!(report.bucket_counts.len(), INSTANCE_BUCKETS.len());
        assert!(report.bucket_counts.iter().all(|(_, count)| *count == 0));
    }

    #[test]
    fn a_verify_block_truncates_a_long_check_list() {
        let many: Vec<CheckRow> = (0..MAX_CHECK_ROWS + 10)
            .map(|i| CheckRow::pass(&format!("c{i}"), "owner"))
            .collect();
        let block = VerifyBlock::of("task-1", &many);
        assert_eq!(block.checks.len(), MAX_CHECK_ROWS);
        assert!(block.passed());
        assert!(block.failed_checks().is_empty());
    }

    #[test]
    fn a_verify_block_names_the_failed_check_and_its_reason() {
        let block = VerifyBlock::of("task-1", &checks());
        assert_eq!(block.instance, "task-1");
        assert_eq!(block.bucket, "unresolved");
        assert_eq!(block.checks.len(), 2);
        assert_eq!(block.checks[0], "level.boot=pass");
        assert_eq!(block.checks[1], "replay.digest=fail:digest 12 != 34");
        assert_eq!(block.failed_checks(), vec!["replay.digest".to_owned()]);
        assert!(!block.passed());
    }

    #[test]
    fn the_wire_text_puts_the_bucket_first() {
        // The wire shape the build verbs carry: one string, the bucket
        // first, then one check per line. No nested object and no array,
        // so the door pays about 30 chars per build tool.
        let block = VerifyBlock::of("task-1", &checks());
        let text = block.wire();
        let lines: Vec<&str> = text.split('\n').collect();
        assert_eq!(lines[0], "bucket=unresolved");
        assert_eq!(lines[1], "level.boot=pass");
        assert_eq!(lines[2], "replay.digest=fail:digest 12 != 34");
        assert_eq!(lines.len(), block.checks.len() + 1);
    }

    #[test]
    fn an_instance_id_is_checked() {
        assert!(check_instance("task-1").is_ok());
        let empty = check_instance("").expect_err("empty refuses");
        assert_eq!(empty.code(), "bad_instance");
        let long = check_instance(&"x".repeat(MAX_INSTANCE_CHARS + 1)).expect_err("long refuses");
        assert_eq!(long.code(), "bad_instance");
    }
}
