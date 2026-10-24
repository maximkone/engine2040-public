//! Guide overlay state for the guided art path (W8).
//!
//! The ladder itself lives in [`crate::art_path`]: a fixed five-rung array, the
//! host's readiness per rung, `next`, `advance` and `hint`. This module only
//! builds the rows the overlay draws from that state, so the draw code stays
//! paint-only and the row shape is testable headlessly.
//! Determinism: rows follow ladder order, one per rung.
//! Errors: none; the ladder carries its own [`StepError`](crate::art_path::StepError).

use crate::art_path::{ArtPath, ArtStep};

/// One overlay row: the rung, whether it is the one to do now, and its hint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RungRow {
    /// Row text, `[x] Import` or `[ ] Import`.
    pub label: String,
    /// True for the first rung the host has not satisfied.
    pub current: bool,
    /// Hint line, present only on the current rung.
    pub hint: Option<&'static str>,
}

impl ArtPath {
    /// Rows the overlay draws: every rung in ladder order, the current one
    /// flagged and carrying its hint. A finished path has no current row.
    #[must_use]
    pub fn panel_rows(&self) -> Vec<RungRow> {
        let current = self.next();
        self.rows()
            .into_iter()
            .zip(crate::art_path::STEPS)
            .map(|(label, step)| RungRow {
                label,
                current: current == Some(step),
                hint: if current == Some(step) {
                    Some(step.hint())
                } else {
                    None
                },
            })
            .collect()
    }

    /// The rung to do now, named for a refusal line.
    #[must_use]
    pub fn current_name(&self) -> &'static str {
        match self.next() {
            Some(step) => step.name(),
            None => "done",
        }
    }
}

/// What refused the overlay's `N` key. Nothing changed on error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayError {
    /// Every rung is already ready, so the key has nothing to take.
    Done,
    /// The ladder refused the rung (it names both rungs).
    Step(crate::art_path::StepError),
}

impl std::fmt::Display for OverlayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Done => f.write_str("guided path is done; nothing left to advance"),
            Self::Step(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for OverlayError {}

/// The step the overlay's `N` key takes, or the refusal when the ladder is done.
///
/// `advance` refuses out of order with both names, so a key press can never
/// skip a rung; the overlay shows the returned error as its toast.
pub fn advance_on_key(path: &mut ArtPath) -> Result<ArtStep, OverlayError> {
    let Some(step) = path.next() else {
        return Err(OverlayError::Done);
    };
    path.advance(step)
        .map(|()| step)
        .map_err(OverlayError::Step)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art_path::ArtStep;

    #[test]
    fn rows_flag_the_current_rung_and_carry_its_hint() {
        let mut path = ArtPath::new();
        let rows = path.panel_rows();
        assert_eq!(rows.len(), 5);
        assert_eq!(rows[0].label, "[ ] Import");
        assert!(rows[0].current);
        assert_eq!(rows[0].hint, Some(ArtStep::Import.hint()));
        assert!(!rows[1].current);
        assert_eq!(rows[1].hint, None, "only the current rung carries a hint");
        assert_eq!(path.current_name(), "Import");
        path.advance(ArtStep::Import).unwrap();
        let rows = path.panel_rows();
        assert!(rows[1].current);
        assert_eq!(rows[1].hint, Some(ArtStep::Material.hint()));
        for step in [ArtStep::Material, ArtStep::Light, ArtStep::Capture] {
            path.advance(step).unwrap();
        }
        assert_eq!(path.current_name(), "Score");
        path.advance(ArtStep::Score).unwrap();
        assert_eq!(path.current_name(), "done");
        assert!(path.panel_rows().iter().all(|row| !row.current));
        assert!(path.panel_rows()[4].hint.is_none());
    }

    #[test]
    fn the_key_advances_one_rung_and_names_the_refusal() {
        let mut path = ArtPath::new();
        assert_eq!(advance_on_key(&mut path), Ok(ArtStep::Import));
        assert_eq!(path.current_name(), "Material");
        // A rung the host has not satisfied yet, pressed out of order.
        let mut stalled = ArtPath::new();
        let error = stalled.advance(ArtStep::Light).unwrap_err();
        assert_eq!(
            error.to_string(),
            "guided path wants Import next, not Light; finish it in ladder order"
        );
        // A done path refuses the key with its own line.
        let mut done = ArtPath::new();
        for step in crate::art_path::STEPS {
            done.advance(step).unwrap();
        }
        assert_eq!(advance_on_key(&mut done), Err(OverlayError::Done));
        assert_eq!(
            OverlayError::Done.to_string(),
            "guided path is done; nothing left to advance"
        );
    }
}
