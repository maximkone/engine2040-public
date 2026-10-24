//! play_counters: the live play-job counters and their disk file (AX28).
//!
//! One count per [`JobStatus`] word, backed by `target/reports/play-jobs.json`
//! so a session that starts after a crash keeps the jobs the earlier session
//! counted. Split out of `play.rs` so that file stays inside the production
//! line cap; re-exported as `play::JobCounters`.

use rmcp::serde::{Deserialize, Serialize};

use super::McpToolError;
use super::JobStatus;

/// The job counters: one count per status word.
///
/// AX28: the counts are backed by disk. The table keeps them live and writes
/// them to `target/reports/play-jobs.json` when a job reaches a terminal
/// status, so a session that starts later reconciles against the file
/// instead of counting from zero. The file is the evidence; the live
/// counters are this session's view of it.
///
/// Units: jobs. Determinism: the same jobs in the same order give the same
/// counts and the same file body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
pub struct JobCounters {
    /// Jobs filed and not yet run.
    pub pending: u64,
    /// Jobs working.
    pub running: u64,
    /// Jobs that finished with a result.
    pub completed: u64,
    /// Jobs that finished with a refusal.
    pub failed: u64,
    /// Jobs the caller stopped.
    pub cancelled: u64,
    /// Jobs that outlived their poll budget.
    pub expired: u64,
}

impl JobCounters {
    /// Where the counters live, relative to the workspace root.
    pub const FILE: &'static str = "target/reports/play-jobs.json";

    /// Count one job in one status.
    pub fn bump(&mut self, status: JobStatus) {
        match status {
            JobStatus::Pending => self.pending += 1,
            JobStatus::Running => self.running += 1,
            JobStatus::Completed => self.completed += 1,
            JobStatus::Failed => self.failed += 1,
            JobStatus::Cancelled => self.cancelled += 1,
            JobStatus::Expired => self.expired += 1,
        }
    }

    /// The count for one status.
    #[must_use]
    pub fn count(&self, status: JobStatus) -> u64 {
        match status {
            JobStatus::Pending => self.pending,
            JobStatus::Running => self.running,
            JobStatus::Completed => self.completed,
            JobStatus::Failed => self.failed,
            JobStatus::Cancelled => self.cancelled,
            JobStatus::Expired => self.expired,
        }
    }

    /// Every count, one line per status, in vocabulary order.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for status in JobStatus::ALL {
            out.push_str(&format!("{}: {}\n", status.as_str(), self.count(status)));
        }
        out
    }

    /// The total number of jobs counted.
    #[must_use]
    pub fn total(&self) -> u64 {
        JobStatus::ALL
            .into_iter()
            .map(|status| self.count(status))
            .sum()
    }

    /// Write the counters under `root` and return the relative path.
    ///
    /// The write is best effort for the caller: a job that already happened
    /// is never undone because the disk said no.
    pub fn write(&self, root: &std::path::Path) -> Result<String, McpToolError> {
        let dir = root.join("target/reports");
        let failed = |why: std::io::Error| {
            McpToolError::new(
                "counters_unwritten",
                format!("could not write {} ({why})", Self::FILE),
            )
        };
        std::fs::create_dir_all(&dir).map_err(failed)?;
        let body = rmcp::serde_json::to_string_pretty(self).unwrap_or_default();
        std::fs::write(root.join(Self::FILE), body).map_err(failed)?;
        Ok(Self::FILE.to_owned())
    }

    /// Read the counters back from `root`; `None` when there is no file.
    #[must_use]
    pub fn read(root: &std::path::Path) -> Option<Self> {
        let text = std::fs::read_to_string(root.join(Self::FILE)).ok()?;
        rmcp::serde_json::from_str(&text).ok()
    }
}
