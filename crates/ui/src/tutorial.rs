//! tutorial: guided first fight for a fresh profile.
//!
//! Consumer: the windowed loop draws [`Tutorial::menu`] under the prompts
//! and runs [`Tutorial::launch_command`] through the menu dispatch once the
//! guide finishes; the packaged executable owns that loop, so this module
//! never blocks on it. Data owner: `engine-ui`. Units: steps count guide
//! actions, prompts are short single lines. Determinism: fixed row order,
//! integer state only, same inputs give the same [`Tutorial::state_digest`]
//! on every machine. Errors: [`TutorialError`]; refusals change nothing.
//! Budget: constant time per input, no heap use on the step path.
//!
//! Two lessons share one guide: the first fight ([`SCRIPT`]) and the defense
//! lesson ([`DEFENSE_SCRIPT`]: block, parry, riposte). Both run the same
//! action stream, so a lesson replays from its recorded inputs like a fight.

use super::menus::{FightRequest, Menu, MenuCommand, Screen};

/// One guide action: the footwork, strike, guard and pause beats taught by
/// the first fight. Small Copy input shared by the script rows and the step
/// entry point, so a row and its input can never drift apart.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TutorialAction {
    /// Footwork beat: hold a direction to reach the marker.
    Move,
    /// Strike beat: press strike to land one hit.
    Attack,
    /// Guard beat: hold guard through the incoming hit.
    Block,
    /// Pause beat: open the pause menu, then resume.
    Pause,
    /// Parry beat: answer the incoming hit inside the parry window.
    Parry,
    /// Riposte beat: strike back inside the window the parry opened.
    Riposte,
}

impl core::fmt::Display for TutorialAction {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let label = match self {
            Self::Move => "move",
            Self::Attack => "attack",
            Self::Block => "block",
            Self::Pause => "pause",
            Self::Parry => "parry",
            Self::Riposte => "riposte",
        };
        f.write_str(label)
    }
}

/// One script row: stable id, one-line prompt and the awaited action.
/// Pure data; the loop renders the prompt and waits for the action.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TutorialStep {
    /// Stable id used by telemetry and the journal.
    pub id: &'static str,
    /// One-line prompt shown above the fight, ASCII, at most 80 chars.
    pub prompt: &'static str,
    /// Action that clears this row.
    pub want: TutorialAction,
}

/// Guide rows in fixed order: footwork, strike, guard, pause. The only
/// script the guide runs; replay feeds these wants back in order.
pub static SCRIPT: [TutorialStep; 4] = [
    TutorialStep {
        id: "move",
        prompt: "Move: hold a direction to reach the marker",
        want: TutorialAction::Move,
    },
    TutorialStep {
        id: "attack",
        prompt: "Attack: press strike to land one hit",
        want: TutorialAction::Attack,
    },
    TutorialStep {
        id: "block",
        prompt: "Block: hold guard through the incoming hit",
        want: TutorialAction::Block,
    },
    TutorialStep {
        id: "pause",
        prompt: "Pause: open the pause menu, then resume",
        want: TutorialAction::Pause,
    },
];

/// Defense lesson rows in fixed order: guard, parry, riposte. Block alone
/// soaks a hit; the parry answers it on time; the riposte spends the window
/// the parry opened, so the rows teach the beats in the order a fight uses them.
pub static DEFENSE_SCRIPT: [TutorialStep; 3] = [
    TutorialStep {
        id: "guard",
        prompt: "Block: hold guard through the incoming hit",
        want: TutorialAction::Block,
    },
    TutorialStep {
        id: "parry",
        prompt: "Parry: tap guard just before the hit lands",
        want: TutorialAction::Parry,
    },
    TutorialStep {
        id: "riposte",
        prompt: "Riposte: strike back while the foe is open",
        want: TutorialAction::Riposte,
    },
];

/// Which lesson a guide teaches.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Lesson {
    FirstFight,
    Defense,
}

impl Lesson {
    /// The rows of this lesson.
    const fn script(self) -> &'static [TutorialStep] {
        match self {
            Self::FirstFight => &SCRIPT,
            Self::Defense => &DEFENSE_SCRIPT,
        }
    }
}

/// Effect of one guide input. Pure output, no side effects.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TutorialEffect {
    /// Row cleared; rows left carried.
    Advanced {
        /// Rows still open after this input.
        remaining: usize,
    },
    /// Last row cleared; the fight request the loop should start carried.
    Completed(FightRequest),
}

/// What refused a guide event. Refusals change nothing.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TutorialError {
    /// Input does not match the current row; expected and seen carried.
    UnexpectedInput {
        /// Action the current row waits for.
        expected: TutorialAction,
        /// Action the player gave.
        got: TutorialAction,
    },
    /// The guide is still running; finishing calls wait for the last row.
    NotFinished,
    /// The guide already finished; further inputs are refused.
    AlreadyFinished,
    /// The guide was skipped; the clean menu is already out.
    AlreadySkipped,
}

impl core::fmt::Display for TutorialError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnexpectedInput { expected, got } => {
                write!(f, "tutorial wants {expected}, got {got}")
            }
            Self::NotFinished => write!(f, "tutorial is not finished yet"),
            Self::AlreadyFinished => write!(f, "tutorial already finished"),
            Self::AlreadySkipped => write!(f, "tutorial was skipped"),
        }
    }
}

impl std::error::Error for TutorialError {}

// Guide phase: arming rows, handed a fight, or handed back a clean menu.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Phase {
    Guiding,
    Finished,
    Skipped,
}

/// Guided first fight: fixed script rows over menu commands, skippable.
///
/// The guide owns the menu the loop draws at all times: the main screen
/// while guiding, a clean main screen after finish or skip. Finishing hands
/// the loop the exact menu command a manual start would run, so the fight
/// starts with no console on the path. Pure transitions, constant time.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tutorial {
    lesson: Lesson,
    done: u8,
    phase: Phase,
    menu: Menu,
}

impl Tutorial {
    /// Fresh profile: first row armed, clean main menu, nothing done. Pure.
    #[must_use]
    pub const fn fresh() -> Self {
        Self::start(Lesson::FirstFight)
    }

    /// Fresh profile on the defense lesson: block, parry, riposte. Pure.
    #[must_use]
    pub const fn defense() -> Self {
        Self::start(Lesson::Defense)
    }

    const fn start(lesson: Lesson) -> Self {
        Self {
            lesson,
            done: 0,
            phase: Phase::Guiding,
            menu: Menu::main(),
        }
    }

    /// Rows of the lesson this guide runs. Pure.
    #[must_use]
    pub const fn script(&self) -> &'static [TutorialStep] {
        self.lesson.script()
    }

    /// Current effect without feeding input: progress while guiding, the
    /// pending fight once finished. Skipped guides have no effect. Pure.
    fn status(&self) -> Result<TutorialEffect, TutorialError> {
        match self.phase {
            Phase::Guiding => Ok(TutorialEffect::Advanced {
                remaining: self.script().len().saturating_sub(usize::from(self.done)),
            }),
            Phase::Finished => Ok(TutorialEffect::Completed(FightRequest::default_fight())),
            Phase::Skipped => Err(TutorialError::AlreadySkipped),
        }
    }

    /// Feed one guide action: clear the current row when the input matches
    /// it, finish on the last row, refuse anything else with state kept.
    /// Pure, constant time, no heap use.
    ///
    /// # Errors
    ///
    /// Returns [`TutorialError::UnexpectedInput`] when the input misses the
    /// current row, [`TutorialError::AlreadyFinished`] past the last row, or
    /// [`TutorialError::AlreadySkipped`] after a skip.
    pub fn step(&mut self, input: TutorialAction) -> Result<TutorialEffect, TutorialError> {
        match self.phase {
            Phase::Finished => return Err(TutorialError::AlreadyFinished),
            Phase::Skipped => return Err(TutorialError::AlreadySkipped),
            Phase::Guiding => {}
        }
        let want = match self.script().get(usize::from(self.done)) {
            Some(row) => row.want,
            None => return Err(TutorialError::AlreadyFinished),
        };
        if input != want {
            return Err(TutorialError::UnexpectedInput {
                expected: want,
                got: input,
            });
        }
        self.done = self.done.saturating_add(1);
        if usize::from(self.done) >= self.script().len() {
            self.phase = Phase::Finished;
            self.menu = Menu::main();
        }
        self.status()
    }

    /// Feed a whole script slice in order; stops at the first refusal with
    /// state kept up to the refusal. An empty slice reports the current
    /// effect. Pure, one pass over the slice, no heap use.
    ///
    /// # Errors
    ///
    /// Returns the first [`TutorialError`] the slice hits, if any.
    pub fn apply_all(
        &mut self,
        inputs: &[TutorialAction],
    ) -> Result<TutorialEffect, TutorialError> {
        let mut effect = self.status()?;
        for input in inputs {
            effect = self.step(*input)?;
        }
        Ok(effect)
    }

    /// Abandon the guide and hand back a clean main menu; the stored menu
    /// resets to it too, so the loop keeps drawing a valid screen. Pure.
    ///
    /// # Errors
    ///
    /// Returns [`TutorialError::AlreadyFinished`] once the fight is won, or
    /// [`TutorialError::AlreadySkipped`] on a second skip.
    pub fn skip(&mut self) -> Result<Menu, TutorialError> {
        match self.phase {
            Phase::Guiding => {
                self.phase = Phase::Skipped;
                self.menu = Menu::main();
                Ok(self.menu)
            }
            Phase::Finished => Err(TutorialError::AlreadyFinished),
            Phase::Skipped => Err(TutorialError::AlreadySkipped),
        }
    }

    /// Menu command the loop runs to start the guided fight: the same
    /// request a manual start builds, available once the last row clears.
    /// Pure.
    ///
    /// # Errors
    ///
    /// Returns [`TutorialError::NotFinished`] while rows are open, or
    /// [`TutorialError::AlreadySkipped`] after a skip.
    pub fn launch_command(&self) -> Result<MenuCommand, TutorialError> {
        match self.phase {
            Phase::Finished => Ok(MenuCommand::StartFight(FightRequest::default_fight())),
            Phase::Guiding => Err(TutorialError::NotFinished),
            Phase::Skipped => Err(TutorialError::AlreadySkipped),
        }
    }

    /// Current row while guiding; none once finished or skipped. Pure.
    #[must_use]
    pub fn next(&self) -> Option<&'static TutorialStep> {
        match self.phase {
            Phase::Guiding => self.script().get(usize::from(self.done)),
            Phase::Finished | Phase::Skipped => None,
        }
    }

    /// Menu the loop draws under the guide: main while guiding, clean main
    /// after finish or skip. Pure.
    #[must_use]
    pub fn menu(&self) -> Menu {
        self.menu
    }

    /// True once the last row clears. Pure.
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.phase == Phase::Finished
    }

    /// True once the guide is skipped. Pure.
    #[must_use]
    pub fn is_skipped(&self) -> bool {
        self.phase == Phase::Skipped
    }

    /// Cleared row count. Pure.
    #[must_use]
    pub fn done_count(&self) -> usize {
        usize::from(self.done)
    }

    /// Rows still open; zero once finished or skipped. Pure.
    #[must_use]
    pub fn remaining(&self) -> usize {
        match self.phase {
            Phase::Guiding => self.script().len().saturating_sub(usize::from(self.done)),
            Phase::Finished | Phase::Skipped => 0,
        }
    }

    /// Guide receipt for replay checks: folds the cleared count, the phase,
    /// the menu screen and the highlight into one integer with a plain
    /// wrapping fold. This digest is a view receipt outside any simulation
    /// hash scope. Same inputs give the same digest. Pure.
    #[must_use]
    pub fn state_digest(&self) -> u64 {
        let phase_byte: u8 = match self.phase {
            Phase::Guiding => 0,
            Phase::Finished => 1,
            Phase::Skipped => 2,
        };
        let screen_byte: u8 = match self.menu.screen() {
            Screen::Main => 0,
            Screen::Pause => 1,
            Screen::Settings { .. } => 2,
        };
        let highlight_byte: u8 = u8::from(self.menu.selected() != 0);
        let lesson_byte: u8 = match self.lesson {
            Lesson::FirstFight => 0,
            Lesson::Defense => 1,
        };
        let mut digest: u64 = u64::from(lesson_byte);
        digest = digest.wrapping_mul(131).wrapping_add(u64::from(self.done));
        digest = digest.wrapping_mul(131).wrapping_add(u64::from(phase_byte));
        digest = digest
            .wrapping_mul(131)
            .wrapping_add(u64::from(screen_byte));
        digest = digest
            .wrapping_mul(131)
            .wrapping_add(u64::from(highlight_byte));
        digest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menus::{dispatch, FightStarted, MenuEffect, ModeHooks};

    fn start_fight(request: FightRequest) -> FightStarted {
        FightStarted {
            seed: request.seed,
            fighters: request.fighters,
        }
    }

    fn hooks() -> ModeHooks {
        ModeHooks {
            start_fight: Some(start_fight),
            resume: None,
            quit_to_main: None,
            quit_app: None,
        }
    }

    fn script_inputs() -> Vec<TutorialAction> {
        SCRIPT.iter().map(|row| row.want).collect()
    }

    fn finished_guide() -> Tutorial {
        let mut guide = Tutorial::fresh();
        guide
            .apply_all(&script_inputs())
            .expect("script must drive the guide home");
        guide
    }

    #[test]
    fn fresh_profile_finishes_guided_fight_with_no_console() {
        let mut guide = Tutorial::fresh();
        assert!(!guide.is_finished());
        assert!(!guide.is_skipped());
        assert_eq!(guide.done_count(), 0);
        assert_eq!(guide.menu(), Menu::main());

        let effect = guide
            .apply_all(&script_inputs())
            .expect("script must drive the guide home");
        match effect {
            TutorialEffect::Completed(request) => {
                assert_eq!(request, FightRequest::default_fight());
            }
            TutorialEffect::Advanced { .. } => panic!("full script must complete the guide"),
        }
        assert!(guide.is_finished());
        assert_eq!(guide.next(), None);
        assert_eq!(guide.remaining(), 0);
        assert_eq!(guide.menu(), Menu::main());

        // Completion hands the loop the exact menu command a manual start
        // runs: dispatch it with bound hooks, no console on the path.
        let cmd = guide.launch_command().expect("finished guide launches");
        assert!(matches!(cmd, MenuCommand::StartFight(_)));
        match dispatch(cmd, &hooks()).expect("bound hook starts the fight") {
            MenuEffect::FightStarted(started) => {
                assert_eq!(started.seed, FightRequest::default_fight().seed);
                assert_eq!(started.fighters, 2);
            }
            other => panic!("wrong effect: {other:?}"),
        }
    }

    #[test]
    fn rows_clear_one_by_one_with_remaining_counts() {
        let mut guide = Tutorial::fresh();
        assert_eq!(guide.remaining(), SCRIPT.len());
        let effect = guide
            .step(TutorialAction::Move)
            .expect("move clears the first row");
        assert!(matches!(effect, TutorialEffect::Advanced { remaining: 3 }));
        let effect = guide
            .step(TutorialAction::Attack)
            .expect("attack clears the second row");
        assert!(matches!(effect, TutorialEffect::Advanced { remaining: 2 }));
        let effect = guide
            .step(TutorialAction::Block)
            .expect("block clears the third row");
        assert!(matches!(effect, TutorialEffect::Advanced { remaining: 1 }));
        let effect = guide
            .step(TutorialAction::Pause)
            .expect("pause clears the last row");
        match effect {
            TutorialEffect::Completed(request) => {
                assert_eq!(request, FightRequest::default_fight());
            }
            TutorialEffect::Advanced { .. } => panic!("last row must complete the guide"),
        }
        assert!(guide.is_finished());
        assert_eq!(guide.done_count(), SCRIPT.len());
    }

    #[test]
    fn step_script_replays_exactly_twice() {
        let inputs = script_inputs();
        let mut first = Tutorial::fresh();
        let mut second = Tutorial::fresh();
        assert_eq!(first.state_digest(), second.state_digest());

        first.apply_all(&inputs).expect("first replay runs");
        second.apply_all(&inputs).expect("second replay runs");
        assert_eq!(first.state_digest(), second.state_digest());
        assert_eq!(first, second);

        // A third run from fresh lands on the same digest, so the replay is
        // stable rather than a lucky pair.
        let mut third = Tutorial::fresh();
        third.apply_all(&inputs).expect("third replay runs");
        assert_eq!(third.state_digest(), first.state_digest());
        assert_eq!(third, first);
    }

    #[test]
    fn digest_moves_with_progress() {
        let mut guide = Tutorial::fresh();
        let start = guide.state_digest();
        guide.step(TutorialAction::Move).expect("move clears");
        assert_ne!(guide.state_digest(), start);
        let mid = guide.state_digest();
        guide.skip().expect("guiding guide skips");
        assert_ne!(guide.state_digest(), mid);
    }

    #[test]
    fn skip_midway_leaves_a_clean_menu() {
        let mut guide = Tutorial::fresh();
        guide.step(TutorialAction::Move).expect("first row clears");
        assert!(guide.next().is_some());

        let menu = guide.skip().expect("guiding guide skips");
        assert_eq!(menu, Menu::main());
        assert_eq!(guide.menu(), Menu::main());
        assert!(guide.is_skipped());
        assert!(!guide.is_finished());
        assert_eq!(guide.next(), None);
        assert_eq!(guide.remaining(), 0);

        // After a skip the guide is spent: inputs, reskips and launches fail.
        assert_eq!(
            guide.step(TutorialAction::Attack),
            Err(TutorialError::AlreadySkipped)
        );
        assert_eq!(guide.skip(), Err(TutorialError::AlreadySkipped));
        assert_eq!(guide.launch_command(), Err(TutorialError::AlreadySkipped));
    }

    #[test]
    fn skip_from_fresh_leaves_a_clean_menu() {
        let mut guide = Tutorial::fresh();
        let menu = guide.skip().expect("fresh guide skips");
        assert_eq!(menu, Menu::main());
        assert_eq!(guide.menu(), Menu::main());
        assert!(guide.is_skipped());
    }

    #[test]
    fn wrong_input_refuses_and_keeps_state() {
        let mut guide = Tutorial::fresh();
        let before = guide.state_digest();
        let err = guide
            .step(TutorialAction::Attack)
            .expect_err("attack is not the first row");
        assert_eq!(
            err,
            TutorialError::UnexpectedInput {
                expected: TutorialAction::Move,
                got: TutorialAction::Attack,
            }
        );
        assert_eq!(err.to_string(), "tutorial wants move, got attack");
        assert_eq!(guide.state_digest(), before);
        assert_eq!(guide.done_count(), 0);
        assert_eq!(guide.next().map(|row| row.want), Some(TutorialAction::Move));
    }

    #[test]
    fn inputs_after_finish_fail_but_launch_waits() {
        let mut guide = finished_guide();
        assert_eq!(
            guide.step(TutorialAction::Pause),
            Err(TutorialError::AlreadyFinished)
        );
        assert_eq!(guide.skip(), Err(TutorialError::AlreadyFinished));
        // The launch command waits for the loop even after refused inputs.
        assert!(matches!(
            guide.launch_command(),
            Ok(MenuCommand::StartFight(_))
        ));
    }

    #[test]
    fn launch_before_finish_fails() {
        let mut guide = Tutorial::fresh();
        assert_eq!(guide.launch_command(), Err(TutorialError::NotFinished));
        guide.step(TutorialAction::Move).expect("first row clears");
        assert_eq!(guide.launch_command(), Err(TutorialError::NotFinished));
    }

    #[test]
    fn prompts_are_short_single_line_ascii() {
        assert_eq!(SCRIPT.len(), 4);
        let mut seen: Vec<&'static str> = Vec::new();
        for row in SCRIPT.iter() {
            assert!(!row.id.is_empty(), "every row needs a stable id");
            assert!(row.id.is_ascii(), "id must stay ASCII");
            assert!(!row.prompt.is_empty(), "every row needs a prompt");
            assert!(
                row.prompt.chars().count() <= 80,
                "prompt stays a one-line nudge"
            );
            assert!(!row.prompt.contains('\n'), "prompt stays one line");
            assert!(row.prompt.is_ascii(), "prompt stays ASCII");
            assert!(!seen.contains(&row.id), "row ids stay unique");
            seen.push(row.id);
        }
        // Wants cover every guide action exactly once, in guide order.
        assert_eq!(
            script_inputs(),
            vec![
                TutorialAction::Move,
                TutorialAction::Attack,
                TutorialAction::Block,
                TutorialAction::Pause,
            ]
        );
    }

    fn defense_inputs() -> Vec<TutorialAction> {
        DEFENSE_SCRIPT.iter().map(|row| row.want).collect()
    }

    #[test]
    fn fresh_profile_finishes_block_parry_riposte_with_no_console() {
        let mut guide = Tutorial::defense();
        assert_eq!(guide.script().len(), 3);
        assert_eq!(guide.remaining(), 3);
        assert_eq!(
            guide.next().map(|row| row.want),
            Some(TutorialAction::Block)
        );
        let effect = guide
            .apply_all(&defense_inputs())
            .expect("the defense rows drive the lesson home");
        assert_eq!(
            effect,
            TutorialEffect::Completed(FightRequest::default_fight())
        );
        assert!(guide.is_finished());
        let cmd = guide.launch_command().expect("finished lesson launches");
        assert!(matches!(
            dispatch(cmd, &hooks()),
            Ok(MenuEffect::FightStarted(_))
        ));
    }

    #[test]
    fn defense_lesson_replays_exactly_twice_and_differs_from_lesson_one() {
        let inputs = defense_inputs();
        let mut first = Tutorial::defense();
        let mut second = Tutorial::defense();
        first.apply_all(&inputs).expect("first replay");
        second.apply_all(&inputs).expect("second replay");
        assert_eq!(first, second);
        assert_eq!(first.state_digest(), second.state_digest());
        // Same cleared count, different lesson: the digest tells them apart.
        let mut lesson_one = Tutorial::fresh();
        lesson_one
            .apply_all(&script_inputs()[..3])
            .expect("three rows of lesson one");
        assert_eq!(lesson_one.done_count(), first.done_count());
        assert_ne!(lesson_one.state_digest(), first.state_digest());
    }

    #[test]
    fn riposte_before_parry_is_refused_naming_both() {
        let mut guide = Tutorial::defense();
        guide.step(TutorialAction::Block).expect("guard clears");
        let before = guide.state_digest();
        assert_eq!(
            guide.step(TutorialAction::Riposte),
            Err(TutorialError::UnexpectedInput {
                expected: TutorialAction::Parry,
                got: TutorialAction::Riposte,
            })
        );
        assert_eq!(guide.state_digest(), before);
        // Lesson one does not know the defense beats either.
        let mut lesson_one = Tutorial::fresh();
        assert_eq!(
            lesson_one.step(TutorialAction::Parry),
            Err(TutorialError::UnexpectedInput {
                expected: TutorialAction::Move,
                got: TutorialAction::Parry,
            })
        );
    }

    #[test]
    fn skipping_the_defense_lesson_leaves_a_clean_menu() {
        let mut guide = Tutorial::defense();
        guide.step(TutorialAction::Block).expect("guard clears");
        assert_eq!(guide.skip(), Ok(Menu::main()));
        assert_eq!(guide.menu(), Menu::main());
        assert_eq!(guide.remaining(), 0);
        assert_eq!(guide.launch_command(), Err(TutorialError::AlreadySkipped));
    }

    #[test]
    fn defense_rows_are_short_unique_ascii() {
        for row in DEFENSE_SCRIPT.iter() {
            assert!(row.id.is_ascii() && row.prompt.is_ascii());
            assert!(!row.prompt.is_empty() && row.prompt.chars().count() <= 80);
            assert!(SCRIPT.iter().all(|other| other.id != row.id), "{}", row.id);
        }
        assert_eq!(TutorialAction::Parry.to_string(), "parry");
        assert_eq!(TutorialAction::Riposte.to_string(), "riposte");
    }

    #[test]
    fn error_display_names_each_case() {
        assert_eq!(
            TutorialError::UnexpectedInput {
                expected: TutorialAction::Block,
                got: TutorialAction::Pause,
            }
            .to_string(),
            "tutorial wants block, got pause"
        );
        assert_eq!(
            TutorialError::NotFinished.to_string(),
            "tutorial is not finished yet"
        );
        assert_eq!(
            TutorialError::AlreadyFinished.to_string(),
            "tutorial already finished"
        );
        assert_eq!(
            TutorialError::AlreadySkipped.to_string(),
            "tutorial was skipped"
        );
        assert_eq!(TutorialAction::Move.to_string(), "move");
        assert_eq!(TutorialAction::Attack.to_string(), "attack");
        assert_eq!(TutorialAction::Block.to_string(), "block");
        assert_eq!(TutorialAction::Pause.to_string(), "pause");
    }
}
