//! Bounded startup/shutdown journal. Every lifecycle step appends one
//! entry; shutdown seals the journal and sealed journals refuse writes.
//!
//! Units: entries (bounded by config), sequence numbers (monotonic u64).
//! The journal is an ordering receipt only and never enters sim hashes.

use crate::error::AppError;

/// One recorded lifecycle step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEntry {
    /// Monotonic sequence number, 0-based.
    pub seq: u64,
    /// Dotted step name, e.g. `startup.scene_loaded`.
    pub step: String,
    /// Human-readable detail for the step.
    pub detail: String,
}

/// Append-only, capacity-bounded, sealable step log.
#[derive(Clone, Debug)]
pub struct Journal {
    entries: Vec<JournalEntry>,
    capacity: usize,
    sealed: bool,
    next_seq: u64,
}

impl Journal {
    /// Empty journal holding at most `capacity` entries.
    pub fn new(capacity: usize) -> Self {
        Self {
            entries: Vec::new(),
            capacity: capacity.max(1),
            sealed: false,
            next_seq: 0,
        }
    }

    /// Append one step; sealed or full journals refuse loudly.
    pub fn record(&mut self, step: &str, detail: String) -> Result<(), AppError> {
        if self.sealed {
            return Err(AppError::JournalSealed {
                step: leaked_step(step),
            });
        }
        if self.entries.len() >= self.capacity {
            return Err(AppError::JournalFull {
                step: leaked_step(step),
            });
        }
        self.entries.push(JournalEntry {
            seq: self.next_seq,
            step: step.to_string(),
            detail,
        });
        self.next_seq += 1;
        Ok(())
    }

    /// Seal the journal; every later write is refused.
    pub fn seal(&mut self) {
        self.sealed = true;
    }

    /// True once shutdown sealed the journal.
    pub fn is_sealed(&self) -> bool {
        self.sealed
    }

    /// Steps recorded so far.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True when nothing is recorded yet.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Recorded steps in sequence order.
    pub fn entries(&self) -> &[JournalEntry] {
        &self.entries
    }

    /// Dotted step names in sequence order (shutdown-order tests use this).
    pub fn steps(&self) -> Vec<&str> {
        self.entries
            .iter()
            .map(|entry| entry.step.as_str())
            .collect()
    }
}

/// Error variants carry `&'static str`, so owned step names from callers
/// map to a small static table; unknown steps fall back to a shared slot.
fn leaked_step(step: &str) -> &'static str {
    match step {
        "startup.config" => "startup.config",
        "startup.registries" => "startup.registries",
        "startup.systems_ordered" => "startup.systems_ordered",
        "startup.scene_loaded" => "startup.scene_loaded",
        "startup.frame_verified" => "startup.frame_verified",
        "startup.handover" => "startup.handover",
        "scene.staged" => "scene.staged",
        "shutdown.inputs_stopped" => "shutdown.inputs_stopped",
        "shutdown.workers_drained" => "shutdown.workers_drained",
        "shutdown.journal_sealing" => "shutdown.journal_sealing",
        _ => "journal.step",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_in_order_with_monotonic_seq() {
        let mut journal = Journal::new(8);
        journal
            .record("startup.config", "seed 2040".to_string())
            .unwrap();
        journal
            .record("startup.handover", "done".to_string())
            .unwrap();
        assert_eq!(journal.steps(), vec!["startup.config", "startup.handover"]);
        assert_eq!(journal.entries()[0].seq, 0);
        assert_eq!(journal.entries()[1].seq, 1);
    }

    #[test]
    fn sealed_journal_refuses_writes() {
        let mut journal = Journal::new(8);
        journal
            .record("startup.config", "seed 2040".to_string())
            .unwrap();
        journal.seal();
        assert!(journal.is_sealed());
        assert_eq!(
            journal.record("shutdown.inputs_stopped", "late".to_string()),
            Err(AppError::JournalSealed {
                step: "shutdown.inputs_stopped"
            })
        );
        assert_eq!(journal.len(), 1);
    }

    #[test]
    fn full_journal_refuses_writes() {
        let mut journal = Journal::new(2);
        journal.record("startup.config", "a".to_string()).unwrap();
        journal.record("startup.handover", "b".to_string()).unwrap();
        assert_eq!(
            journal.record("shutdown.inputs_stopped", "c".to_string()),
            Err(AppError::JournalFull {
                step: "shutdown.inputs_stopped"
            })
        );
    }
}
