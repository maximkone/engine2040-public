//! flow: window match shell (Title, Match, Paused, Result).
//!
//! Consumer: the `arena` window binary drives menus through here
//! (`crates/game/src/bin/arena.rs`: `ticks_owed` plus Escape).
//! Data owner: this module owns the `Flow` state plus the `FlowEvent`
//! transition; the match authority stays in [`crate::App`]. Units:
//! frames and fixed ticks at the arena rate. Determinism scope: pure
//! transitions, no I/O and no hash input; pause holds the tick cursor
//! so the replay hash cannot drift. Errors: none; unknown transitions
//! hold the current state.
//!
//! Budget: O(1) time, no allocation per call.

/// Window shell state: exactly one drives the menus.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Flow {
    /// Title screen, before the bout starts.
    Title,
    /// Live bout, the only state that steps fixed ticks.
    Match,
    /// Paused bout, the accumulator holds and adds zero ticks.
    Paused,
    /// Finished bout, offering rematch or title.
    Result,
}

/// Window shell event: the single input to [`Flow::on`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum FlowEvent {
    /// Title starts the bout.
    Start,
    /// Escape flips Match and Paused.
    TogglePause,
    /// The last fixed tick ends the bout.
    MatchOver,
    /// Result starts the next bout with prior state cleared.
    Rematch,
    /// Result returns to the title.
    ToTitle,
}

impl Flow {
    /// True only in Match: pause, title and result add zero ticks.
    #[must_use]
    pub const fn ticks_run(self) -> bool {
        match self {
            Self::Match => true,
            Self::Title | Self::Paused | Self::Result => false,
        }
    }

    /// Next state for one event. Unknown pairs hold, so a stray event
    /// never strands the player. Pure.
    #[must_use]
    pub const fn on(self, event: FlowEvent) -> Self {
        match (self, event) {
            (Self::Title, FlowEvent::Start) => Self::Match,
            (Self::Match, FlowEvent::TogglePause) => Self::Paused,
            (Self::Paused, FlowEvent::TogglePause) => Self::Match,
            (Self::Match, FlowEvent::MatchOver) => Self::Result,
            (Self::Result, FlowEvent::Rematch) => Self::Match,
            (Self::Result, FlowEvent::ToTitle) => Self::Title,
            _ => self,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GameConfig, HUMAN_SLOT};
    use engine_input::{Buttons, FighterId, FighterInput};

    fn fixture() -> GameConfig {
        GameConfig {
            seed: 7,
            ticks: 64,
            fighters: 2,
            half_extent_m: 12.0,
        }
    }

    fn idle(tick: u32) -> FighterInput {
        FighterInput {
            tick,
            fighter: FighterId::new(HUMAN_SLOT).unwrap(),
            mx: 0,
            mz: 0,
            facing: 0,
            buttons: Buttons::NONE,
        }
    }

    /// Step one frame: a running shell ticks the match once, a held
    /// shell adds nothing. Pure over the test harness.
    fn step_frame(app: &mut crate::App, flow: Flow) -> u32 {
        if flow.ticks_run() {
            let tick = app.cursor();
            let outcome = app.tick(idle(tick));
            assert!(outcome.input_accepted, "fixture inputs validate");
            1
        } else {
            0
        }
    }

    #[test]
    fn flow_cycle_title_match_result_match() {
        // Title, Match, Result, Match again: the first runnable shell.
        let mut flow = Flow::Title;
        assert!(!flow.ticks_run());
        flow = flow.on(FlowEvent::Start);
        assert_eq!(flow, Flow::Match);
        assert!(flow.ticks_run());
        flow = flow.on(FlowEvent::TogglePause);
        assert_eq!(flow, Flow::Paused);
        assert!(!flow.ticks_run());
        flow = flow.on(FlowEvent::TogglePause);
        assert_eq!(flow, Flow::Match);
        flow = flow.on(FlowEvent::MatchOver);
        assert_eq!(flow, Flow::Result);
        assert!(!flow.ticks_run());
        flow = flow.on(FlowEvent::Rematch);
        assert_eq!(flow, Flow::Match);
        flow = flow.on(FlowEvent::MatchOver);
        assert_eq!(flow, Flow::Result);
        flow = flow.on(FlowEvent::ToTitle);
        assert_eq!(flow, Flow::Title);
        // Stray events hold instead of stranding the player.
        assert_eq!(Flow::Title.on(FlowEvent::TogglePause), Flow::Title);
        assert_eq!(Flow::Paused.on(FlowEvent::MatchOver), Flow::Paused);
        assert_eq!(Flow::Title.on(FlowEvent::Rematch), Flow::Title);
    }

    #[test]
    fn flow_ticks_run_only_in_match() {
        assert!(!Flow::Title.ticks_run());
        assert!(Flow::Match.ticks_run());
        assert!(!Flow::Paused.ticks_run());
        assert!(!Flow::Result.ticks_run());
    }

    #[test]
    fn flow_paused_frames_add_zero_ticks() {
        // Spike fixture: two sessions, seed 7, 64 ticks, 2 fighters.
        // 60 paused frames add 0 ticks, 60 match frames add 60.
        let mut paused_app = crate::App::new(fixture()).unwrap();
        let mut match_app = crate::App::new(fixture()).unwrap();
        let mut paused_added = 0u32;
        let mut match_added = 0u32;
        for _ in 0..60 {
            paused_added += step_frame(&mut paused_app, Flow::Paused);
            match_added += step_frame(&mut match_app, Flow::Match);
        }
        assert_eq!(paused_added, 0, "60 paused frames add 0 ticks");
        assert_eq!(paused_app.cursor(), 0);
        assert!(paused_app.checkpoints().is_empty());
        assert_eq!(match_added, 60, "60 match frames add 60 ticks");
        assert_eq!(match_app.cursor(), 60);
        assert_eq!(match_app.checkpoints().len(), 60);
    }

    #[test]
    fn flow_pause_keeps_final_hash() {
        // A bout with a pause in it ends on the same hash as one without.
        let mut straight = crate::App::new(fixture()).unwrap();
        for _ in 0..60 {
            step_frame(&mut straight, Flow::Match);
        }
        let mut with_pause = crate::App::new(fixture()).unwrap();
        for _ in 0..30 {
            step_frame(&mut with_pause, Flow::Match);
        }
        for _ in 0..60 {
            step_frame(&mut with_pause, Flow::Paused);
        }
        assert_eq!(with_pause.cursor(), 30, "pause holds the cursor");
        for _ in 0..30 {
            step_frame(&mut with_pause, Flow::Match);
        }
        assert_eq!(with_pause.cursor(), straight.cursor());
        assert_eq!(with_pause.checkpoints(), straight.checkpoints());
    }
}
