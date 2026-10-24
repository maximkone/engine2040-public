//! play_task: the task-shaped `play.run` contract (AX28).
//!
//! A client that declares [`TASKS_CAPABILITY`] gets a task for a `play.run`
//! start: the job's progress walks in [`TASK_SLICE_TICKS`] slices and
//! [`PlayJobs::task_for`] answers id, status text (`tick 240/600` while
//! running) and a done flag. A client that does not declare it keeps the
//! start and poll handles byte for byte. The result is still the eager one:
//! the manifest digest equals the blocking run's.
//!
//! Presentation of a session handle only: no clock, no wall time, the scene
//! document is never read or written. Units: ticks at 60 Hz.

use super::play::{PlayJob, PlayJobs};

/// The MCP capability a task-shaped client declares to receive task answers
/// (AX28); a client that does not declare it keeps the start/poll handles.
pub const TASKS_CAPABILITY: &str = "io.modelcontextprotocol/tasks";

/// True when a client's declared capabilities name [`TASKS_CAPABILITY`]
/// exactly. A near miss (`tasks` alone, a prefix) is not the extension.
#[must_use]
pub fn wants_tasks(declared: &[&str]) -> bool {
    declared.iter().any(|name| *name == TASKS_CAPABILITY)
}

/// Sim ticks a task-shaped job reports per poll (4 s at 60 Hz): a 600-tick run
/// shows `tick 240/600` after the first task poll, `tick 480/600` after the
/// second, and completes on the third. Deterministic, no wall clock.
pub const TASK_SLICE_TICKS: u32 = 240;

/// The task shape of one play job (AX28): the id the client polls, the status
/// it reads (`tick 240/600` while running, a job status word once terminal)
/// and whether the work is finished.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TaskPlayRun {
    /// The job's task id: the `job_id` a start filed.
    pub task_id: u64,
    /// `tick <reached>/<ticks>` while running, else the job status word.
    pub status: String,
    /// True once the job is terminal (completed, cancelled, expired, failed).
    pub done: bool,
}

impl PlayJobs {
    /// The task shape of `task_id` (AX28), or `None` for an unknown id.
    ///
    /// While the job runs the status is `tick <reached>/<ticks>` with
    /// `reached` the ticks its task polls have reported (see
    /// [`TASK_SLICE_TICKS`]); once terminal it is the job status word. Reading
    /// a task never advances it and never touches the scene.
    #[must_use]
    pub fn task_for(&self, task_id: u64) -> Option<TaskPlayRun> {
        let job = self.jobs.get(&task_id)?;
        let done = job.status.is_terminal();
        let status = match job.status {
            super::play::JobStatus::Running => {
                format!("tick {}/{}", reached_ticks(job), job.ticks)
            }
            other => other.as_str().to_owned(),
        };
        Some(TaskPlayRun {
            task_id,
            status,
            done,
        })
    }
}

/// Ticks a task-shaped job has reported so far: one [`TASK_SLICE_TICKS`] slice
/// per poll, never past the match end. A job that took no poll reported
/// nothing.
pub(crate) fn reached_ticks(job: &PlayJob) -> u32 {
    if job.polls_used == 0 {
        return 0;
    }
    let polls = u32::try_from(job.polls_used).unwrap_or(u32::MAX);
    polls.saturating_mul(TASK_SLICE_TICKS).min(job.ticks)
}

/// The task's reported fraction as `0.0..=1.0`, exact on a slice boundary
/// (240 of 600 reads 0.4).
pub(crate) fn task_progress(job: &PlayJob) -> f32 {
    if job.ticks == 0 {
        return 1.0;
    }
    let permille = reached_ticks(job) * 1000 / job.ticks;
    f32::from(u16::try_from(permille).unwrap_or(1000)) / 1000.0
}
