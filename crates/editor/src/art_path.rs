//! Guided art path (W8): the fixed five-step ladder from import to score.
//!
//! Owner: the editor session. The host supplies the readiness of each step
//! (a file landed, a material is bound, a light exists, a capture ran, a score
//! is in), and the ladder only says which step comes next and refuses a step
//! taken out of order. Nothing here imports, renders or scores.
//! Determinism: the ladder is a fixed array; readiness is a bool per step.
//! Errors: [`StepError`]; a refused advance changes nothing.

/// One rung of the guided path, in ladder order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ArtStep {
    /// Bring the asset into the content tree.
    Import,
    /// Bind a material to it.
    Material,
    /// Place and aim a light.
    Light,
    /// Render the capture.
    Capture,
    /// Score the capture against the look reference.
    Score,
}

/// The ladder, lowest rung first.
pub const STEPS: [ArtStep; 5] = [
    ArtStep::Import,
    ArtStep::Material,
    ArtStep::Light,
    ArtStep::Capture,
    ArtStep::Score,
];

/// Panel labels, indexed like [`STEPS`].
const NAMES: [&str; 5] = ["Import", "Material", "Light", "Capture", "Score"];
/// One hint per rung, indexed like [`STEPS`].
const HINTS: [&str; 5] = [
    "drop the glb into content/ and let intake write its receipt",
    "pick a material and bind it to the imported mesh",
    "place a sun or a lamp and aim it at the subject",
    "capture the viewport and keep the image beside the asset",
    "score the capture against the look reference and keep the verdict",
];

impl ArtStep {
    /// Panel label.
    pub fn name(self) -> &'static str {
        NAMES[self.index()]
    }

    /// One line the panel shows for this step.
    pub fn hint(self) -> &'static str {
        HINTS[self.index()]
    }

    /// Position on the ladder, `0` for [`ArtStep::Import`].
    pub fn index(self) -> usize {
        match self {
            Self::Import => 0,
            Self::Material => 1,
            Self::Light => 2,
            Self::Capture => 3,
            Self::Score => 4,
        }
    }
}

/// The ladder plus one readiness flag per rung.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArtPath {
    ready: [bool; STEPS.len()],
}

impl ArtPath {
    /// Fresh path: nothing done, so [`ArtPath::next`] is Import.
    pub fn new() -> Self {
        Self::default()
    }

    /// True when the host reports the rung satisfied.
    pub fn is_ready(&self, step: ArtStep) -> bool {
        self.ready[step.index()]
    }

    /// The first rung the host has not satisfied, or `None` when all are done.
    pub fn next(&self) -> Option<ArtStep> {
        STEPS.into_iter().find(|step| !self.is_ready(*step))
    }

    /// Take one rung; it must be the next one.
    ///
    /// Skipping ahead, or advancing when everything is done, refuses with
    /// [`StepError::OutOfOrder`] carrying both names, and changes nothing.
    pub fn advance(&mut self, step: ArtStep) -> Result<(), StepError> {
        let Some(want) = self.next() else {
            return Err(StepError::OutOfOrder {
                want: step,
                got: step,
            });
        };
        if want != step {
            return Err(StepError::OutOfOrder { want, got: step });
        }
        self.ready[step.index()] = true;
        Ok(())
    }

    /// Mark one rung ready from the host's own predicate (a re-check), without
    /// claiming the steps below it.
    pub fn set_ready(&mut self, step: ArtStep, ready: bool) {
        self.ready[step.index()] = ready;
    }

    /// True when every rung is ready.
    pub fn is_done(&self) -> bool {
        self.next().is_none()
    }

    /// One row per rung: `[x] Import` or `[ ] Import`.
    pub fn rows(&self) -> Vec<String> {
        STEPS
            .iter()
            .map(|step| {
                let mark = if self.is_ready(*step) { "x" } else { " " };
                format!("[{mark}] {}", step.name())
            })
            .collect()
    }
}

/// What refused an advance. Nothing changed on error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepError {
    /// The ladder wanted one rung and got another.
    OutOfOrder {
        /// Rung the ladder asked for.
        want: ArtStep,
        /// Rung the caller tried.
        got: ArtStep,
    },
}

impl std::fmt::Display for StepError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutOfOrder { want, got } => write!(
                f,
                "guided path wants {} next, not {}; finish it in ladder order",
                want.name(),
                got.name()
            ),
        }
    }
}

impl std::error::Error for StepError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_path_starts_at_import_and_walks_in_order() {
        let mut path = ArtPath::new();
        assert_eq!(path.next(), Some(ArtStep::Import));
        assert!(!path.is_done());
        path.advance(ArtStep::Import).unwrap();
        assert_eq!(path.next(), Some(ArtStep::Material));
        path.advance(ArtStep::Material).unwrap();
        path.advance(ArtStep::Light).unwrap();
        path.advance(ArtStep::Capture).unwrap();
        assert_eq!(path.next(), Some(ArtStep::Score));
        path.advance(ArtStep::Score).unwrap();
        assert_eq!(path.next(), None);
        assert!(path.is_done());
        // Two panels: done then a re-check.
        assert_eq!(
            path.advance(ArtStep::Score),
            Err(StepError::OutOfOrder {
                want: ArtStep::Score,
                got: ArtStep::Score
            })
        );
    }

    #[test]
    fn skipping_ahead_names_both_rungs() {
        let mut path = ArtPath::new();
        path.advance(ArtStep::Import).unwrap();
        let error = path.advance(ArtStep::Light).unwrap_err();
        assert_eq!(
            error,
            StepError::OutOfOrder {
                want: ArtStep::Material,
                got: ArtStep::Light
            }
        );
        assert_eq!(
            error.to_string(),
            "guided path wants Material next, not Light; finish it in ladder order"
        );
        assert_eq!(path.next(), Some(ArtStep::Material), "nothing advanced");
        // A host re-check can satisfy a rung directly.
        path.set_ready(ArtStep::Material, true);
        assert_eq!(path.next(), Some(ArtStep::Light));
        assert_eq!(
            path.rows(),
            vec![
                "[x] Import",
                "[x] Material",
                "[ ] Light",
                "[ ] Capture",
                "[ ] Score"
            ]
        );
    }

    #[test]
    fn every_rung_has_a_hint() {
        for step in STEPS {
            assert!(!step.hint().is_empty(), "{} needs a hint", step.name());
            assert!(!step.name().is_empty());
        }
        assert_eq!(STEPS[0].index(), 0);
        assert_eq!(STEPS[4].index(), 4);
    }
}
