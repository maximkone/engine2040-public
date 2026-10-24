//! trace: the tour's decision-trace station. The station hands slot 0 to a real gladiator
//! [`Brain`] with the AIN8 decision ring on, so the round is fought by the bot and not by the
//! training dummy; once a second the newest `BtTrace` line is kept, and the tour prints it.
//!
//! Owner: this module owns the bot and its ring; the brain, the decision log and the trace tape
//! belong to `engine-ai`. Presentation and script only: the inputs it returns go to the same match
//! the script drives, so nothing here is hashed and no score is written back.
//!
//! Determinism: the brain is deterministic over its percept, the ring is fixed capacity and the
//! line is refreshed on a fixed tick period, so one seed prints one trace. Budget: one brain, one
//! ring of [`DECISION_CAP`] rows, one string a second; nothing per tick.

use crate::FighterView;
use engine_ai::bt_trace::BtTrace;
use engine_ai::gladiator::{Brain, Percept};
use engine_input::{FighterId, FighterInput};

/// Rows the decision ring and the trace tape hold: two minutes of a 6-tick brain.
pub const DECISION_CAP: usize = 64;
/// The decision line is refreshed once a second at the fixed 60 Hz.
pub const LINE_PERIOD: u32 = 60;
/// Ticks between the brain's own evaluations: the gladiator's shipped period.
const BRAIN_PERIOD: u32 = 6;

/// A gladiator brain with its decision ring on, and the last line it printed.
#[derive(Clone, Debug)]
pub struct TraceBot {
    brain: Brain,
    trace: BtTrace,
    line: Option<String>,
    decisions: u32,
}

impl TraceBot {
    /// A bot for `slot` with the decision ring recording from its first decision.
    #[must_use]
    pub fn new(slot: FighterId) -> Self {
        let mut brain = Brain::new(slot, BRAIN_PERIOD);
        brain.record_decisions(DECISION_CAP);
        Self {
            brain,
            trace: BtTrace::new(DECISION_CAP),
            line: None,
            decisions: 0,
        }
    }

    /// One tick of the bot: a percept from the two drawn bodies, the brain's input, and once a
    /// second the newest decision line folded out of the ring.
    pub fn input(&mut self, tick: u32, me: &FighterView, foe: &FighterView) -> FighterInput {
        let to = [foe.pos_m[0] - me.pos_m[0], foe.pos_m[2] - me.pos_m[2]];
        let dist = to[0].hypot(to[1]);
        let percept = Percept::new(me.pos_m, me.yaw, foe.pos_m, false, dist, tick);
        let input = self.brain.decide(&percept);
        if self.line.is_none() || tick.is_multiple_of(LINE_PERIOD) {
            if let Some(log) = self.brain.decisions() {
                self.trace.note_decisions(log);
                self.decisions = u32::try_from(self.trace.len()).unwrap_or(u32::MAX);
                let from = tick.saturating_sub(LINE_PERIOD);
                let text = self.trace.lines_text(None, from, tick, 1);
                let text = text.trim();
                if !text.is_empty() && text != "no rows" {
                    self.line = Some(text.to_owned());
                }
            }
        }
        input
    }

    /// The newest decision line, once the bot has decided for a second.
    #[must_use]
    pub fn line(&self) -> Option<&str> {
        self.line.as_deref()
    }

    /// Rows on the trace tape.
    #[must_use]
    pub const fn decisions(&self) -> u32 {
        self.decisions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view(slot: u8, x: f32) -> FighterView {
        FighterView {
            slot,
            pos_m: [x, 0.0, 0.0],
            yaw: 0.0,
        }
    }

    /// F2P: the bot decides, the ring records, and a line prints within a second of the first
    /// decision; the same percepts print the same line.
    #[test]
    fn the_bot_prints_a_decision_line_within_a_second() {
        let me = view(0, 0.0);
        let foe = view(1, 2.0);
        let mut bot = TraceBot::new(FighterId::new(0).expect("slot"));
        let mut last = None;
        for tick in 0..180 {
            let input = bot.input(tick, &me, &foe);
            assert_eq!(input.fighter, FighterId::new(0).expect("slot"), "slot 0 decides");
            last = bot.line().map(str::to_owned).or(last);
        }
        let line = last.expect("a line prints");
        assert!(bot.decisions() > 0, "the ring holds rows: {}", bot.decisions());
        assert!(!line.is_empty());

        let mut again = TraceBot::new(FighterId::new(0).expect("slot"));
        for tick in 0..180 {
            again.input(tick, &me, &foe);
        }
        assert_eq!(
            again.line().map(str::to_owned),
            Some(line),
            "the same percepts print the same line"
        );
    }
}
