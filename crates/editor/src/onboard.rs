//! First-run checklist over templates and play-in-editor (tutorial-1 flow).
//!
//! Consumer is the first-run editor session (the Shell/app onboarding panel
//! calls [`Checklist::note_template`] on each template apply and
//! [`Checklist::note_first_fight`] on play-in-editor start). Data owner is
//! [`Checklist`]: four completion bits plus fixed step ids, hints and
//! per-step second budgets. Units are steps and seconds; the whole flow must
//! fit [`TIME_TO_FIRST_FIGHT_BUDGET_SECS`]. Determinism scope: the same event
//! sequence gives the same completion on every machine; no wall clock, no
//! I/O. Errors are typed [`OnboardError`]; refusals change nothing.

use std::fmt;

use crate::templates::TemplateKind;

/// Tutorial id this checklist implements.
pub const TUTORIAL_ID: &str = "tutorial-1";
/// Measured time-to-first-fight budget in seconds (15 minutes).
pub const TIME_TO_FIRST_FIGHT_BUDGET_SECS: u64 = 900;
/// Longest hint in characters: hints are one-line nudges, never docs.
pub const MAX_HINT_CHARS: usize = 80;

/// One first-run step, in tutorial order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Step {
    /// Create the arena starter (the Arena template).
    Arena,
    /// Create the roster starter (the fighter template).
    Roster,
    /// Create a prop or a weapon starter (the Prop or Weapon template).
    Gear,
    /// Start in-viewport play over the built scene.
    FirstFight,
}

impl Step {
    /// Every step in tutorial order.
    pub const fn all() -> [Self; 4] {
        [Self::Arena, Self::Roster, Self::Gear, Self::FirstFight]
    }

    /// Stable id and one-line hint of this step, in one table.
    const fn text(self) -> (&'static str, &'static str) {
        match self {
            Self::Arena => ("new-arena", "Templates tab: pick Arena, press Create"),
            Self::Roster => (
                "add-roster",
                "Templates tab: pick the fighter template, press Create",
            ),
            Self::Gear => (
                "add-gear",
                "Templates tab: pick Prop or Weapon, press Create",
            ),
            Self::FirstFight => (
                "first-fight",
                "Press Play in the viewport to start the fight",
            ),
        }
    }

    /// Stable id used by the panel and the journal.
    pub const fn id(self) -> &'static str {
        self.text().0
    }

    /// One-line hint shown inline. Hints only: no links, no doc pages.
    pub const fn hint(self) -> &'static str {
        self.text().1
    }

    /// Per-step time budget in seconds; the sum must fit the 15-minute gate.
    pub const fn budget_secs(self) -> u64 {
        match self {
            Self::Arena => 180,
            Self::Roster => 120,
            Self::Gear => 120,
            Self::FirstFight => 180,
        }
    }

    /// Parse a stable id. Err on anything else.
    pub fn from_id(id: &str) -> Result<Self, OnboardError> {
        match id {
            "new-arena" => Ok(Self::Arena),
            "add-roster" => Ok(Self::Roster),
            "add-gear" => Ok(Self::Gear),
            "first-fight" => Ok(Self::FirstFight),
            _ => Err(OnboardError::UnknownStep { id: id.to_string() }),
        }
    }

    /// True when applying this template kind advances the step. The fight
    /// step never completes from a template: only play-in-editor starts it.
    /// Roster membership goes through [`TemplateKind::is_roster`] (this
    /// file is outside the genre ledger, so it never names the variant).
    pub fn accepts(self, kind: TemplateKind) -> bool {
        matches!(
            (self, kind),
            (Self::Arena, TemplateKind::Arena)
                | (Self::Gear, TemplateKind::Prop | TemplateKind::Weapon)
        ) || (matches!(self, Self::Roster) && kind.is_roster())
    }

    /// Bit index of the step in the completion mask.
    const fn bit(self) -> u8 {
        match self {
            Self::Arena => 0,
            Self::Roster => 1,
            Self::Gear => 2,
            Self::FirstFight => 3,
        }
    }
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

/// What refused an onboarding event. Refusals change nothing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OnboardError {
    /// The step id names no tutorial-1 step.
    UnknownStep {
        /// The refused id.
        id: String,
    },
    /// Play was not ready: the scene cannot fight yet.
    NotPlayReady,
}

impl fmt::Display for OnboardError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownStep { id } => write!(f, "unknown onboarding step `{id}`"),
            Self::NotPlayReady => write!(f, "scene is not ready to play yet"),
        }
    }
}

impl std::error::Error for OnboardError {}

/// First-run checklist: four completion bits over tutorial-1.
///
/// Event-driven (never per-tick): the panel feeds template applies into
/// [`note_template`](Self::note_template) and the play-in-editor start into
/// [`note_first_fight`](Self::note_first_fight). State is one byte, so the
/// steady-state cost is zero allocations and no clock reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Checklist {
    done: u8,
}

impl Checklist {
    /// Empty checklist: nothing done.
    pub fn new() -> Self {
        Self::default()
    }

    /// True when the step is done.
    pub fn is_done(self, step: Step) -> bool {
        self.done & (1 << step.bit()) != 0
    }

    /// Completed step count.
    pub fn done_count(self) -> usize {
        Step::all().iter().filter(|s| self.is_done(**s)).count()
    }

    /// Total step count (always 4).
    pub fn len(self) -> usize {
        Step::all().len()
    }

    /// True when the checklist holds no completion yet.
    pub fn is_empty(self) -> bool {
        self.done == 0
    }

    /// True when every step is done.
    pub fn is_complete(self) -> bool {
        self.done_count() == self.len()
    }

    /// Progress in whole percent (0, 25, 50, 75, 100).
    pub fn progress_pct(self) -> u8 {
        (self.done_count() as u8) * 25
    }

    /// Next unfinished step in tutorial order. None when complete.
    pub fn next(self) -> Option<Step> {
        Step::all().into_iter().find(|s| !self.is_done(*s))
    }

    /// Unfinished steps in tutorial order.
    pub fn remaining(self) -> Vec<Step> {
        Step::all()
            .into_iter()
            .filter(|s| !self.is_done(*s))
            .collect()
    }

    /// Complete one step by id. Ok(true) when newly completed, Ok(false)
    /// when already done; unknown ids refuse and change nothing.
    pub fn complete_by_id(&mut self, id: &str) -> Result<bool, OnboardError> {
        let step = Step::from_id(id)?;
        Ok(self.complete(step))
    }

    /// Complete one step. True when newly completed.
    pub fn complete(&mut self, step: Step) -> bool {
        if self.is_done(step) {
            return false;
        }
        self.done |= 1 << step.bit();
        true
    }

    /// Clear one step. True when it was done.
    pub fn reset(&mut self, step: Step) -> bool {
        if !self.is_done(step) {
            return false;
        }
        self.done &= !(1 << step.bit());
        true
    }

    /// Clear every step.
    pub fn reset_all(&mut self) {
        self.done = 0;
    }

    /// Feed one applied template into the checklist. Returns the newly
    /// completed step, if any; templates outside the tutorial map to nothing
    /// and change nothing.
    pub fn note_template(&mut self, kind: TemplateKind) -> Option<Step> {
        let step = Step::all().into_iter().find(|s| s.accepts(kind))?;
        self.complete(step).then_some(step)
    }

    /// Feed the play-in-editor start into the checklist: completes the fight step
    /// only when the scene is play-ready (the caller passes
    /// `TemplateService::play_ready`). A non-ready scene refuses with
    /// [`OnboardError::NotPlayReady`] and changes nothing.
    pub fn note_first_fight(&mut self, play_ready: bool) -> Result<bool, OnboardError> {
        if !play_ready {
            return Err(OnboardError::NotPlayReady);
        }
        Ok(self.complete(Step::FirstFight))
    }

    /// Estimated seconds for the whole tutorial (sum of step budgets).
    pub fn estimated_total_secs() -> u64 {
        Step::all().iter().map(|s| s.budget_secs()).sum()
    }

    /// Estimated seconds left for the unfinished steps.
    pub fn estimated_remaining_secs(self) -> u64 {
        self.remaining().iter().map(|s| s.budget_secs()).sum()
    }

    /// True when the whole tutorial fits the 15-minute gate.
    pub fn fits_budget() -> bool {
        Self::estimated_total_secs() <= TIME_TO_FIRST_FIGHT_BUDGET_SECS
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{Document, Journal};
    use crate::templates::TemplateService;

    fn ready_doc_with_starters() -> (Document, Journal, TemplateService) {
        let mut doc = Document::new();
        let mut journal = Journal::new();
        let mut service = TemplateService::new();
        service
            .apply(&mut doc, &mut journal, TemplateKind::Arena, "arena", 1)
            .unwrap();
        service
            .apply(&mut doc, &mut journal, TemplateKind::Fighter, "fighter", 2)
            .unwrap();
        service
            .apply(&mut doc, &mut journal, TemplateKind::Weapon, "weapon", 3)
            .unwrap();
        (doc, journal, service)
    }

    #[test]
    fn tutorial1_flow_reaches_first_fight_inside_fifteen_minutes() {
        assert_eq!(TUTORIAL_ID, "tutorial-1");
        assert!(Checklist::fits_budget(), "tutorial-1 must fit 15 minutes");
        assert!(
            Checklist::estimated_total_secs() <= TIME_TO_FIRST_FIGHT_BUDGET_SECS,
            "600 s estimated over the 900 s gate"
        );

        let (doc, _, _) = ready_doc_with_starters();
        let mut list = Checklist::new();
        assert!(list.is_empty());
        assert_eq!(list.next(), Some(Step::Arena));

        assert_eq!(list.note_template(TemplateKind::Arena), Some(Step::Arena));
        assert_eq!(
            list.note_template(TemplateKind::Fighter),
            Some(Step::Roster)
        );
        assert_eq!(list.note_template(TemplateKind::Weapon), Some(Step::Gear));
        assert_eq!(list.progress_pct(), 75);
        assert_eq!(list.next(), Some(Step::FirstFight));

        assert!(TemplateService::play_ready(&doc));
        assert_eq!(
            list.note_first_fight(TemplateService::play_ready(&doc)),
            Ok(true)
        );
        assert!(list.is_complete());
        assert_eq!(list.next(), None);
        assert_eq!(list.progress_pct(), 100);
        assert!(list.remaining().is_empty());
    }

    #[test]
    fn hints_are_hints_not_docs() {
        for step in Step::all() {
            let hint = step.hint();
            assert!(!hint.is_empty(), "{step} needs a hint");
            assert!(
                hint.chars().count() <= MAX_HINT_CHARS,
                "{step} hint over {MAX_HINT_CHARS} chars: `{hint}`"
            );
            for banned in ["http", "docs", "manual", "wiki"] {
                assert!(
                    !hint.to_lowercase().contains(banned),
                    "{step} hint must not point at docs: `{hint}`"
                );
            }
        }
        // Ids roundtrip through from_id; Display matches the id.
        for step in Step::all() {
            assert_eq!(Step::from_id(step.id()), Ok(step));
            assert_eq!(step.to_string(), step.id());
        }
    }

    #[test]
    fn unknown_step_refuses_and_changes_nothing() {
        let mut list = Checklist::new();
        list.complete(Step::Arena);
        let before = list;
        let error = list.complete_by_id("nope").unwrap_err();
        assert_eq!(
            error,
            OnboardError::UnknownStep {
                id: "nope".to_string(),
            }
        );
        assert!(error.to_string().contains("nope"));
        assert_eq!(list, before, "refusal changes nothing");
        // Empty and wrong-case ids refuse the same way.
        assert!(list.complete_by_id("").is_err());
        assert!(list.complete_by_id("New-Arena").is_err());
        assert_eq!(list, before);
    }

    #[test]
    fn first_fight_needs_a_play_ready_scene() {
        let mut list = Checklist::new();
        let empty = Document::new();
        assert!(!TemplateService::play_ready(&empty));
        let before = list;
        assert_eq!(
            list.note_first_fight(TemplateService::play_ready(&empty)),
            Err(OnboardError::NotPlayReady)
        );
        assert_eq!(list, before, "refusal changes nothing");
        assert!(!list.is_done(Step::FirstFight));
    }

    #[test]
    fn recompleting_is_idempotent_and_resets_reopen_steps() {
        let mut list = Checklist::new();
        assert!(list.complete(Step::Arena));
        assert!(!list.complete(Step::Arena));
        assert_eq!(list.done_count(), 1);
        assert_eq!(list.progress_pct(), 25);

        // A duplicate template apply completes nothing new.
        assert_eq!(list.note_template(TemplateKind::Arena), None);
        assert_eq!(list.done_count(), 1);

        // Gear accepts a prop or a weapon, exactly once.
        assert_eq!(list.note_template(TemplateKind::Prop), Some(Step::Gear));
        assert_eq!(list.note_template(TemplateKind::Weapon), None);

        // Reset reopens the step; reset_all clears the list.
        assert!(list.reset(Step::Gear));
        assert!(!list.reset(Step::Gear));
        assert_eq!(list.next(), Some(Step::Roster));
        list.reset_all();
        assert!(list.is_empty());
        assert_eq!(
            list.estimated_remaining_secs(),
            Checklist::estimated_total_secs()
        );
    }

    #[test]
    fn remaining_time_shrinks_as_steps_complete() {
        let mut list = Checklist::new();
        let total = Checklist::estimated_total_secs();
        assert_eq!(list.estimated_remaining_secs(), total);
        list.complete(Step::Arena);
        assert_eq!(
            list.estimated_remaining_secs(),
            total - Step::Arena.budget_secs()
        );
        // The fight step never completes from a template event.
        assert!(!Step::FirstFight.accepts(TemplateKind::Arena));
        assert!(!Step::FirstFight.accepts(TemplateKind::Fighter));
        assert!(!Step::FirstFight.accepts(TemplateKind::Prop));
        assert!(!Step::FirstFight.accepts(TemplateKind::Weapon));
        assert_eq!(
            list.note_template(TemplateKind::Fighter),
            Some(Step::Roster)
        );
        assert!(!list.is_done(Step::FirstFight));
    }
}
