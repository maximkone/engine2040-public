//! Oracle-normalized agent score with a deterministic bootstrap interval.
//!
//! Idea ported from agentick's `leaderboard/scoring.py`, in integers:
//! `score = (agent - baseline) / (anchor - baseline)`, clipped to `0..=1000`
//! permille. The baseline is an idle policy's mean return; the anchor is the
//! mean return of the best SCRIPTED policy on the same task. A fight has no
//! closed-form optimum, so the anchor is a named policy, never an invented
//! maximum: [`NormScore::line`] prints which policy sets it.
//!
//! Consumer: [`crate::tournament`] builds one [`TaskReturns`] per difficulty
//! and prints [`NormScore::line`] in the balance-tournament report. Data
//! owner: the caller owns the per-episode returns; this module keeps no state.
//! Units: integer return points (damage per match), means in milli-points
//! (`sum * 1000 / n`, floor), permille scores `0..=1000`.
//! Determinism: the interval resamples the agent's episodes with
//! [`engine_core::Rng`] forked per resample from [`BOOTSTRAP_SEED`], so the
//! same inputs give the same bytes on every machine. No wall clock, no hash
//! order. Errors: [`NormError`] for empty tasks, policies or resamples.
//!
//! Interval: the 2.5th and 97.5th percentiles by rank over `resamples` sorted
//! draws. Only the agent's episodes are resampled; baseline and anchor stay
//! at their point means, so the interval is not a full uncertainty bound.

use std::fmt;

use engine_core::Rng;

/// Bootstrap resamples behind the interval.
pub const BOOTSTRAP_RESAMPLES: u32 = 200;

/// Fixed interval seed (house seed): the same returns give the same interval.
pub const BOOTSTRAP_SEED: u64 = 2040;

/// One task's episode returns: the agent, the idle or random baseline, and
/// each scripted policy as `(name, returns)`, in policy order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskReturns {
    /// Task label, printed beside its anchor (a difficulty name).
    pub name: String,
    /// Agent per-episode returns: the sample the interval resamples.
    pub agent: Vec<i64>,
    /// Idle or random policy per-episode returns.
    pub baseline: Vec<i64>,
    /// Scripted policies with their per-episode returns.
    pub scripted: Vec<(String, Vec<i64>)>,
}

/// One task's normalized score and the policy that set its anchor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskScore {
    /// Task label, copied from [`TaskReturns::name`].
    pub name: String,
    /// Name of the best scripted policy (highest mean, first on ties).
    pub anchor: String,
    /// Point score for this task in permille, `0..=1000`.
    pub permille: u16,
}

/// Agent score across tasks: the per-task scores averaged, with a bootstrap
/// interval. Every field is an integer, so the report line is byte-stable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NormScore {
    /// Point estimate: the per-task permille scores averaged (floor).
    pub permille: u16,
    /// Lower bound of the 95 percent interval, permille.
    pub ci_lo: u16,
    /// Upper bound of the 95 percent interval, permille.
    pub ci_hi: u16,
    /// Resamples behind the interval.
    pub resamples: u32,
    /// Per-task scores, in task order, each naming its anchor policy.
    pub tasks: Vec<TaskScore>,
}

impl NormScore {
    /// The report line: `norm=<permille> ci=[lo,hi] anchor=<task>=<policy>,...`.
    pub fn line(&self) -> String {
        let anchors: Vec<String> = self
            .tasks
            .iter()
            .map(|task| format!("{}={}", task.name, task.anchor))
            .collect();
        format!(
            "norm={} ci=[{},{}] anchor={}",
            self.permille,
            self.ci_lo,
            self.ci_hi,
            anchors.join(",")
        )
    }
}

/// Typed refusals: nothing scores without tasks, resamples and episodes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NormError {
    /// No tasks to average.
    NoTasks,
    /// Zero bootstrap resamples: no interval.
    NoResamples,
    /// A task has no scripted policy, so no anchor exists.
    NoScripted {
        /// The task without an anchor.
        task: String,
    },
    /// A policy series is empty: no mean exists for it.
    EmptyReturns {
        /// The task that holds the empty series.
        task: String,
        /// `agent`, `baseline`, or the scripted policy name.
        policy: String,
    },
}

impl fmt::Display for NormError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoTasks => write!(f, "no tasks to normalize"),
            Self::NoResamples => write!(f, "bootstrap needs at least one resample"),
            Self::NoScripted { task } => write!(f, "task {task} has no scripted anchor"),
            Self::EmptyReturns { task, policy } => {
                write!(f, "task {task} policy {policy} has no episodes")
            }
        }
    }
}

impl std::error::Error for NormError {}

/// Normalized score for one task from milli-point means.
///
/// When the anchor equals the baseline the gap is zero: the score is `1000`
/// when the agent mean reaches the anchor and `0` otherwise. Otherwise the
/// score is `(agent - baseline) * 1000 / (anchor - baseline)` floored and
/// clipped to `0..=1000`.
pub fn task_permille(agent_m: i128, baseline_m: i128, anchor_m: i128) -> u16 {
    if anchor_m == baseline_m {
        return if agent_m >= anchor_m { 1000 } else { 0 };
    }
    let scaled = (agent_m - baseline_m) * 1000 / (anchor_m - baseline_m);
    u16::try_from(scaled.clamp(0, 1000)).unwrap_or(1000)
}

/// Score every task, average them, and bootstrap the 95 percent interval.
///
/// The anchor of each task is its best scripted policy by mean return. Fails
/// without scoring when there are no tasks, no resamples, or any task lacks
/// episodes for the agent, the baseline or a scripted policy.
pub fn score(tasks: &[TaskReturns], resamples: u32, seed: u64) -> Result<NormScore, NormError> {
    if tasks.is_empty() {
        return Err(NormError::NoTasks);
    }
    if resamples == 0 {
        return Err(NormError::NoResamples);
    }
    let mut prepared = Vec::with_capacity(tasks.len());
    for task in tasks {
        prepared.push(prepare(task)?);
    }
    let task_count = u32::try_from(prepared.len()).unwrap_or(u32::MAX);
    let mut per_task = Vec::with_capacity(prepared.len());
    let mut point_total: u32 = 0;
    for item in &prepared {
        let agent_m = mean_milli(&item.task.agent).unwrap_or(0);
        let permille = task_permille(agent_m, item.baseline_m, item.anchor_m);
        point_total += u32::from(permille);
        per_task.push(TaskScore {
            name: item.task.name.clone(),
            anchor: item.anchor.to_owned(),
            permille,
        });
    }
    let mut draws = Vec::with_capacity(usize::try_from(resamples).unwrap_or(usize::MAX));
    for index in 0..resamples {
        let mut rng = Rng::new(seed).fork(u64::from(index));
        let mut total: u32 = 0;
        for item in &prepared {
            let agent_m = resampled_mean_milli(&item.task.agent, &mut rng);
            total += u32::from(task_permille(agent_m, item.baseline_m, item.anchor_m));
        }
        draws.push(u16::try_from(total / task_count).unwrap_or(1000));
    }
    draws.sort_unstable();
    let tail = usize::try_from(u64::from(resamples) * 25 / 1000).unwrap_or(0);
    let last = draws.len().saturating_sub(1 + tail);
    Ok(NormScore {
        permille: u16::try_from(point_total / task_count).unwrap_or(1000),
        ci_lo: draws.get(tail).copied().unwrap_or(0),
        ci_hi: draws.get(last).copied().unwrap_or(1000),
        resamples,
        tasks: per_task,
    })
}

/// Mean return in milli-points (`sum * 1000 / n`, floor); `None` when empty.
pub fn mean_milli(returns: &[i64]) -> Option<i128> {
    let count = i128::try_from(returns.len()).ok()?;
    if count == 0 {
        return None;
    }
    let sum: i128 = returns.iter().map(|value| i128::from(*value)).sum();
    Some(sum * 1000 / count)
}

/// A task with its validated baseline mean and its chosen anchor.
struct Prepared<'a> {
    task: &'a TaskReturns,
    baseline_m: i128,
    anchor_m: i128,
    anchor: &'a str,
}

/// Validate one task and pick its anchor: the scripted policy with the
/// highest mean return; a tie keeps the policy listed first.
fn prepare(task: &TaskReturns) -> Result<Prepared<'_>, NormError> {
    let refuse = |policy: &str| NormError::EmptyReturns {
        task: task.name.clone(),
        policy: policy.to_owned(),
    };
    if task.agent.is_empty() {
        return Err(refuse("agent"));
    }
    let baseline_m = mean_milli(&task.baseline).ok_or_else(|| refuse("baseline"))?;
    if task.scripted.is_empty() {
        return Err(NormError::NoScripted {
            task: task.name.clone(),
        });
    }
    let mut best: Option<(&str, i128)> = None;
    for (policy, returns) in &task.scripted {
        let mean = mean_milli(returns).ok_or_else(|| refuse(policy))?;
        if best.is_none_or(|(_, top)| mean > top) {
            best = Some((policy.as_str(), mean));
        }
    }
    let (anchor, anchor_m) = best.ok_or_else(|| NormError::NoScripted {
        task: task.name.clone(),
    })?;
    Ok(Prepared {
        task,
        baseline_m,
        anchor_m,
        anchor,
    })
}

/// Mean in milli-points of one bootstrap resample of `agent` (with
/// replacement). `agent` is non-empty: callers validate first.
fn resampled_mean_milli(agent: &[i64], rng: &mut Rng) -> i128 {
    let span = u64::try_from(agent.len()).unwrap_or(u64::MAX);
    let mut sum: i128 = 0;
    for _ in 0..agent.len() {
        let pick = rng.next_range(0, span);
        let index = usize::try_from(pick).unwrap_or(0);
        sum += i128::from(agent.get(index).copied().unwrap_or(0));
    }
    let count = i128::try_from(agent.len()).unwrap_or(1);
    sum * 1000 / count
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(name: &str, agent: &[i64], baseline: &[i64], anchor: &[i64]) -> TaskReturns {
        TaskReturns {
            name: name.to_owned(),
            agent: agent.to_vec(),
            baseline: baseline.to_vec(),
            scripted: vec![("spam:1".to_owned(), anchor.to_vec())],
        }
    }

    fn score_one(agent: &[i64], baseline: &[i64], anchor: &[i64]) -> NormScore {
        let tasks = [task("t", agent, baseline, anchor)];
        score(&tasks, BOOTSTRAP_RESAMPLES, BOOTSTRAP_SEED).expect("one task scores")
    }

    #[test]
    fn norm_known_vector_is_half_the_gap() {
        // Baseline 0, anchor 100, agent 50: halfway, 500 permille. One
        // episode leaves nothing to resample, so the interval is a point.
        let score = score_one(&[50], &[0], &[100]);
        assert_eq!(score.permille, 500);
        assert_eq!((score.ci_lo, score.ci_hi), (500, 500));
    }

    #[test]
    fn norm_clips_above_the_anchor_and_below_the_baseline() {
        assert_eq!(score_one(&[150], &[0], &[100]).permille, 1000);
        assert_eq!(score_one(&[-20], &[0], &[100]).permille, 0);
        assert_eq!(score_one(&[0], &[0], &[100]).permille, 0);
    }

    #[test]
    fn norm_zero_gap_rule_is_all_or_nothing() {
        // Anchor equal to baseline: 1000 only when the agent reaches it.
        assert_eq!(score_one(&[5], &[5], &[5]).permille, 1000);
        assert_eq!(score_one(&[6], &[5], &[5]).permille, 1000);
        assert_eq!(score_one(&[4], &[5], &[5]).permille, 0);
    }

    #[test]
    fn task_permille_vectors_in_milli_points() {
        // Means are milli-points: 50.000 is 50_000.
        assert_eq!(task_permille(50_000, 0, 100_000), 500);
        assert_eq!(task_permille(150_000, 0, 100_000), 1000);
        assert_eq!(task_permille(-20_000, 0, 100_000), 0);
        // An anchor below the baseline maps the same way, inverted.
        assert_eq!(task_permille(75_000, 100_000, 50_000), 500);
    }

    #[test]
    fn norm_averages_the_task_scores() {
        let tasks = [
            task("a", &[50], &[0], &[100]),
            task("b", &[200], &[0], &[100]),
        ];
        let score = score(&tasks, BOOTSTRAP_RESAMPLES, BOOTSTRAP_SEED).expect("two tasks");
        assert_eq!(score.permille, 750);
        let per_task: Vec<u16> = score.tasks.iter().map(|t| t.permille).collect();
        assert_eq!(per_task, vec![500, 1000]);
    }

    #[test]
    fn norm_anchor_is_the_best_scripted_mean_and_ties_keep_the_first() {
        let scripted = vec![
            ("spam:1".to_owned(), vec![10, 10]),
            ("spam:2".to_owned(), vec![25]),
            ("spam:3".to_owned(), vec![25]),
        ];
        let task = TaskReturns {
            name: "even".to_owned(),
            agent: vec![20],
            baseline: vec![0],
            scripted,
        };
        let score = score(&[task], BOOTSTRAP_RESAMPLES, BOOTSTRAP_SEED).expect("scores");
        assert_eq!(score.tasks[0].anchor, "spam:2");
        assert_eq!(score.tasks[0].permille, 800);
    }

    #[test]
    fn norm_ci_brackets_the_point_and_is_not_degenerate() {
        let agent = [10, 40, 25, 55, 70, 5, 35, 60, 45, 20];
        let score = score_one(&agent, &[0], &[100]);
        assert_eq!(score.permille, 365);
        assert!(score.ci_lo <= score.permille && score.permille <= score.ci_hi);
        assert!(score.ci_lo < score.ci_hi);
    }

    #[test]
    fn norm_more_episodes_narrow_the_ci() {
        let mut rng = Rng::new(7);
        let big: Vec<i64> = (0..160)
            .map(|_| i64::try_from(rng.next_range(0, 101)).unwrap_or(0))
            .collect();
        let narrow = score_one(&big, &[0], &[100]);
        let wide = score_one(&big[..10], &[0], &[100]);
        assert!(narrow.ci_hi - narrow.ci_lo < wide.ci_hi - wide.ci_lo);
    }

    #[test]
    fn norm_same_inputs_give_byte_identical_output() {
        let mut rng = Rng::new(11);
        let agent: Vec<i64> = (0..30)
            .map(|_| i64::try_from(rng.next_range(0, 90)).unwrap_or(0))
            .collect();
        let tasks = [task("t", &agent, &[3, 4], &[60, 80])];
        let first = score(&tasks, BOOTSTRAP_RESAMPLES, BOOTSTRAP_SEED).expect("scores");
        let second = score(&tasks, BOOTSTRAP_RESAMPLES, BOOTSTRAP_SEED).expect("scores");
        assert_eq!(first, second);
        assert_eq!(first.line(), second.line());
        assert_eq!(format!("{first:?}"), format!("{second:?}"));
    }

    #[test]
    fn norm_refuses_empty_or_unusable_inputs() {
        assert_eq!(
            score(&[], BOOTSTRAP_RESAMPLES, BOOTSTRAP_SEED),
            Err(NormError::NoTasks)
        );
        let ok = [task("t", &[1], &[0], &[2])];
        assert_eq!(score(&ok, 0, BOOTSTRAP_SEED), Err(NormError::NoResamples));
        let no_agent = [task("t", &[], &[0], &[2])];
        assert_eq!(
            score(&no_agent, BOOTSTRAP_RESAMPLES, BOOTSTRAP_SEED),
            Err(NormError::EmptyReturns {
                task: "t".to_owned(),
                policy: "agent".to_owned(),
            })
        );
        let no_scripted = [TaskReturns {
            name: "t".to_owned(),
            agent: vec![1],
            baseline: vec![0],
            scripted: Vec::new(),
        }];
        assert_eq!(
            score(&no_scripted, BOOTSTRAP_RESAMPLES, BOOTSTRAP_SEED),
            Err(NormError::NoScripted {
                task: "t".to_owned()
            })
        );
    }

    #[test]
    fn norm_line_names_the_anchor_per_task() {
        let score = score_one(&[50], &[0], &[100]);
        assert_eq!(score.line(), "norm=500 ci=[500,500] anchor=t=spam:1");
    }
}
