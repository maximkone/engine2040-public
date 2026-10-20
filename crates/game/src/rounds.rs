//! rounds: best-of-5 duel rounds over one-round arena sessions.
//! Consumer: [`crate::App`] decides each round from hp and resets hp.
//! Units: hp permille, rounds and wins as `u32`. Determinism: pure
//! integer folds; the same hp tape gives the same winner twice.
//! Errors: short inputs replay the round (`Tie`), never panic.

/// Round wins that take the match (best of 5, `rome-spec.md:41-47`).
pub const WINS_FOR_MATCH: u32 = 3;

/// One round's verdict: which side takes it, or replay on exact ties.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum RoundWinner {
    /// Slot 0 wins (duel) or the slot-0 side wins (1-vs-field).
    Slot0,
    /// Slot 1 wins (duel) or the field side wins (1-vs-field).
    Slot1,
    /// Exact tie: the round replays and neither side scores.
    Tie,
}

/// Round from hp (`rome-spec.md:45-46`): shutout wins early, else
/// higher total wins, exact tie replays; slot 0 vs summed field.
#[must_use]
pub fn decide_round(hp: &[u16], fighters: u8) -> RoundWinner {
    let n = usize::from(fighters);
    if n < 2 || hp.len() < n {
        return RoundWinner::Tie;
    }
    if n == 2 {
        if hp[0] > hp[1] {
            return RoundWinner::Slot0;
        }
        if hp[1] > hp[0] {
            return RoundWinner::Slot1;
        }
        return RoundWinner::Tie;
    }
    let mut field: u32 = 0;
    for health in &hp[1..n] {
        field = field.saturating_add(u32::from(*health));
    }
    let home = u32::from(hp[0]);
    if home > field {
        RoundWinner::Slot0
    } else if field > home {
        RoundWinner::Slot1
    } else {
        RoundWinner::Tie
    }
}

/// Best-of-5 fold: first to [`WINS_FOR_MATCH`]; ties replay, score nothing.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct MatchState {
    rounds: u32,
    wins0: u32,
    wins1: u32,
    ties: u32,
}

impl MatchState {
    /// Empty match. Pure.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            rounds: 0,
            wins0: 0,
            wins1: 0,
            ties: 0,
        }
    }

    /// Fold one verdict (tie: round counted, no score).
    pub fn apply(&mut self, winner: RoundWinner) {
        self.rounds = self.rounds.saturating_add(1);
        match winner {
            RoundWinner::Slot0 => self.wins0 = self.wins0.saturating_add(1),
            RoundWinner::Slot1 => self.wins1 = self.wins1.saturating_add(1),
            RoundWinner::Tie => self.ties = self.ties.saturating_add(1),
        }
    }

    /// Winner once a side reaches [`WINS_FOR_MATCH`]. Pure.
    #[must_use]
    pub fn winner(self) -> Option<RoundWinner> {
        if self.wins0 >= WINS_FOR_MATCH {
            Some(RoundWinner::Slot0)
        } else if self.wins1 >= WINS_FOR_MATCH {
            Some(RoundWinner::Slot1)
        } else {
            None
        }
    }

    /// Rounds played including tie replays. Pure.
    #[must_use]
    pub fn rounds_played(self) -> u32 {
        self.rounds
    }

    /// Round wins `(slot0, slot1)`. Pure.
    #[must_use]
    pub fn wins(self) -> (u32, u32) {
        (self.wins0, self.wins1)
    }

    /// Tie replays so far. Pure.
    #[must_use]
    pub fn ties(self) -> u32 {
        self.ties
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(hp_tape: &[(u16, u16)]) -> MatchState {
        let mut m = MatchState::new();
        for (a, b) in hp_tape {
            m.apply(decide_round(&[*a, *b], 2));
        }
        m
    }

    #[test]
    fn round_loop_duel_3_0_sweep() {
        let m = play(&[(800, 200), (700, 300), (900, 100)]);
        assert_eq!(m.wins(), (3, 0));
        assert_eq!(m.rounds_played(), 3);
        assert_eq!(m.winner(), Some(RoundWinner::Slot0));
    }

    #[test]
    fn round_loop_duel_3_2_split() {
        let m = play(&[(800, 200), (200, 800), (700, 300), (300, 700), (650, 400)]);
        assert_eq!(m.wins(), (3, 2));
        assert_eq!(m.rounds_played(), 5);
        assert_eq!(m.winner(), Some(RoundWinner::Slot0));
    }

    #[test]
    fn round_loop_tie_replays_round() {
        let m = play(&[(500, 500), (800, 200), (700, 300), (900, 100)]);
        assert_eq!(m.wins(), (3, 0));
        assert_eq!(m.ties(), 1);
        assert_eq!(m.rounds_played(), 4);
        assert_eq!(m.winner(), Some(RoundWinner::Slot0));
        assert_eq!(decide_round(&[], 2), RoundWinner::Tie);
        assert_eq!(decide_round(&[800], 2), RoundWinner::Tie);
        assert_eq!(decide_round(&[800, 200], 1), RoundWinner::Tie);
    }

    #[test]
    fn round_loop_app_close_and_reset() {
        let config = crate::GameConfig {
            seed: 7,
            ticks: 64,
            fighters: 2,
            half_extent_m: 12.0,
        };
        let mut app = crate::App::new(config).unwrap();
        assert_eq!(app.rounds_played(), 0);
        assert_eq!(app.match_winner(), None);
        assert_eq!(app.round_winner(), RoundWinner::Tie);
        assert_eq!(app.close_round(), RoundWinner::Tie);
        assert_eq!(app.rounds_played(), 1);
        assert_eq!(app.match_winner(), None);
        app.reset_for_next_round();
        assert_eq!(app.hp(), &[1000; engine_input::MAX_PARTICIPANTS as usize]);
        assert_eq!(app.rounds_played(), 1);
    }
}
