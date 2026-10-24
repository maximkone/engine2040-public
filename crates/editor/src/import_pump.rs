//! Import pump (E10): the drop handler's per-frame poll, one tick at a time.
//!
//! The window host owns the frame loop, so it hands the pump the tick and an
//! `accepted` predicate (the host's own verdict for a dropped file), then
//! pushes the returned activity rows into the feed. The editor never imports
//! and never reads file contents: a refused path only records its row.
//! Units: ticks are caller frames, so no wall clock enters the pump.
//! Determinism: the watch reports in watch order, so rows follow that order.
//! Errors: [`WatchError`] from [`ImportPump::watch`]; a refused watch changes
//! nothing.

use std::path::Path;

use crate::activity::Activity;
use crate::watch::{
    drop_report, receipt_activities, ReimportReceipt, Watch, WatchError, DEFAULT_WINDOW_TICKS,
};

/// Rows the pump has reported, split by verdict.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PumpTally {
    /// Rows the predicate accepted.
    pub accepted: usize,
    /// Rows the predicate refused.
    pub refused: usize,
}

/// The drop handler: a watch, the host's verdict and the last report.
pub struct ImportPump {
    watch: Watch,
    check: Box<dyn Fn(&Path) -> bool>,
    tally: PumpTally,
    last: Vec<ReimportReceipt>,
}

impl ImportPump {
    /// Pump over a fresh watch with the default coalescing window.
    pub fn new(check: Box<dyn Fn(&Path) -> bool>) -> Self {
        Self {
            watch: Watch::new(DEFAULT_WINDOW_TICKS),
            check,
            tally: PumpTally::default(),
            last: Vec::new(),
        }
    }

    /// Add one watched file; `Ok(false)` when it was already watched.
    pub fn watch(&mut self, path: &Path) -> Result<bool, WatchError> {
        self.watch.watch(path)
    }

    /// Replace the host's verdict; existing watches stay.
    pub fn set_check(&mut self, check: Box<dyn Fn(&Path) -> bool>) {
        self.check = check;
    }

    /// Rows reported so far, split by verdict.
    pub fn tally(&self) -> PumpTally {
        self.tally
    }

    /// Receipts of the last frame that reported a change.
    pub fn last(&self) -> &[ReimportReceipt] {
        &self.last
    }

    /// One frame: poll at `now_ticks` and answer the activity rows.
    ///
    /// A frame with no reported change returns an empty `Vec` and allocates
    /// nothing else, so an idle frame costs one poll.
    pub fn poll(&mut self, now_ticks: u64) -> Vec<Activity> {
        let (_, receipts) = drop_report(&mut self.watch, now_ticks, &*self.check);
        if receipts.is_empty() {
            return Vec::new();
        }
        for receipt in &receipts {
            if receipt.accepted {
                self.tally.accepted += 1;
            } else {
                self.tally.refused += 1;
            }
        }
        let rows = receipt_activities(&receipts);
        self.last = receipts;
        rows
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_idle_frame_answers_no_rows() {
        let mut pump = ImportPump::new(Box::new(|_| true));
        assert!(pump.poll(5).is_empty());
        assert_eq!(pump.tally(), PumpTally::default());
        assert!(pump.last().is_empty());
        assert_eq!(pump.poll(6).len(), 0, "two idle frames in a row stay empty");
    }

    #[test]
    fn one_frame_reports_an_accepted_row_and_a_refused_row() {
        let dir = std::env::temp_dir().join(format!("e2040-import-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let taken = dir.join("taken.glb");
        let blocked = dir.join("blocked.glb");
        std::fs::write(&taken, b"a").unwrap();
        std::fs::write(&blocked, b"a").unwrap();
        let mut pump = ImportPump::new(Box::new(|path: &Path| path.ends_with("taken.glb")));
        pump.watch(&taken).unwrap();
        pump.watch(&blocked).unwrap();
        assert!(!pump.watch(&taken).unwrap(), "a second watch is a no-op");

        // The drop handler sees both writes, inside the coalescing window.
        std::fs::write(&taken, b"aa").unwrap();
        std::fs::write(&blocked, b"bb").unwrap();
        assert!(pump.poll(0).is_empty(), "inside the window");
        let rows = pump.poll(2);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].verb, "import.accepted");
        assert_eq!(rows[1].verb, "import.refused");
        assert_eq!(
            pump.tally(),
            PumpTally {
                accepted: 1,
                refused: 1
            }
        );
        assert_eq!(pump.last().len(), 2);
        assert!(pump.last()[0].accepted);
        assert!(!pump.last()[1].accepted);
        assert_eq!(pump.last()[0].at_tick, 2, "the report tick, not the write");

        // Coalesced: the next frame has nothing left to report.
        assert!(pump.poll(3).is_empty());
        assert_eq!(
            pump.tally(),
            PumpTally {
                accepted: 1,
                refused: 1
            }
        );

        // A later write keeps its own verdict.
        std::fs::write(&blocked, b"bbb").unwrap();
        assert!(pump.poll(4).is_empty());
        let rows = pump.poll(6);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].verb, "import.refused");

        // The check is a host seam: flipping it flips the next verdict.
        pump.set_check(Box::new(|_| true));
        std::fs::write(&blocked, b"bbbb").unwrap();
        assert!(pump.poll(7).is_empty());
        let rows = pump.poll(9);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].verb, "import.accepted");
        assert_eq!(
            pump.tally(),
            PumpTally {
                accepted: 2,
                refused: 2
            }
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
