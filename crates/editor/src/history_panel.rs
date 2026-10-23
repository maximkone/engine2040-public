//! History panel: the visible undo list as rows, with a cursor and a jump.
//!
//! Owner: the dock (one [`HistoryPanel`] per window). The data owner stays the
//! ring in [`History`](crate::History); this panel only picks a row, so the
//! same state machine is testable without a window.
//! Units: row `0` is the newest step; [`History::activate`] takes that index.
//! Determinism: rows come straight from [`History::rows_newest_first`].
//! Errors: [`HistoryError`](crate::HistoryError); a refused jump leaves the
//! cursor where it was.

use crate::{History, HistoryEntry, HistoryError, HistoryJump, HistoryRow};

/// One history panel: a cursor over the newest-first rows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HistoryPanel {
    cursor: usize,
}

impl HistoryPanel {
    /// Fresh panel with the cursor on the newest step.
    pub fn new() -> Self {
        Self::default()
    }

    /// Row the cursor sits on (newest first).
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Move the cursor down (older) or up (newer), clamped to `len` rows.
    pub fn move_cursor(&mut self, down: bool, len: usize) {
        if down {
            if self.cursor + 1 < len {
                self.cursor += 1;
            }
        } else {
            self.cursor = self.cursor.saturating_sub(1);
        }
    }

    /// Rows for the panel, newest first.
    pub fn rows(&self, history: &History) -> Vec<HistoryRow> {
        history.rows_newest_first()
    }

    /// One line per row in panel order.
    pub fn row_texts(&self, history: &History) -> Vec<String> {
        history
            .rows_newest_first()
            .iter()
            .map(HistoryRow::text)
            .collect()
    }

    /// Enter: the jump the selected row asks the scene owner to make.
    pub fn activate(&self, history: &History) -> Result<HistoryJump, HistoryError> {
        let jump = history.activate(self.cursor)?;
        Ok(jump)
    }

    /// The entry under the cursor, when one exists.
    pub fn selected<'a>(&self, history: &'a History) -> Option<&'a HistoryEntry> {
        let len = history.len();
        let index = len.checked_sub(self.cursor.saturating_add(1))?;
        history.entry(index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::Author;

    fn entry(label: &str, layout: &str, revision: u64, tick: u64, depth: usize) -> HistoryEntry {
        HistoryEntry::new(label, layout, revision, tick, Author::Human, depth)
    }

    fn history() -> History {
        let mut history = History::new(100);
        history.push(entry("scene.place", "0\nhierarchy", 1, 10, 1));
        history.push(entry("scene.move", "1\nhierarchy", 2, 20, 2));
        history.push(entry("scene.rename", "2\nhierarchy\nbrowser", 3, 30, 3));
        history
    }

    #[test]
    fn rows_are_newest_first_with_label_revision_tick_and_name() {
        let mut history = history();
        history.name(2, "approved").unwrap();
        let panel = HistoryPanel::new();
        let rows = panel.rows(&history);
        assert_eq!(rows[0].label, "scene.rename");
        assert_eq!(rows[0].revision, 3);
        assert_eq!(rows[0].tick, 30);
        assert_eq!(rows[0].name.as_deref(), Some("approved"));
        assert_eq!(rows[2].label, "scene.place");
        assert_eq!(
            panel.row_texts(&history)[0],
            "scene.rename rev=3 t0030 name=approved"
        );
    }

    #[test]
    fn cursor_moves_and_clamps_and_enter_returns_the_jump() {
        let history = history();
        let mut panel = HistoryPanel::new();
        assert_eq!(panel.cursor(), 0);
        panel.move_cursor(false, history.len());
        assert_eq!(panel.cursor(), 0, "up at the top stays put");
        panel.move_cursor(true, history.len());
        panel.move_cursor(true, history.len());
        assert_eq!(panel.cursor(), 2);
        panel.move_cursor(true, history.len());
        assert_eq!(panel.cursor(), 2, "down at the bottom stays put");

        let jump = panel.activate(&history).unwrap();
        assert_eq!(jump.layout, "0\nhierarchy");
        assert_eq!(jump.ops, 2);
        assert!(jump.backward);
        assert_eq!(panel.selected(&history).unwrap().tick, 10);

        // An empty history refuses with the ring's own error.
        let empty = History::new(100);
        assert_eq!(
            HistoryPanel::new().activate(&empty),
            Err(HistoryError::OutOfRange { index: 0, len: 0 })
        );
        assert!(panel.selected(&empty).is_none());
    }
}
