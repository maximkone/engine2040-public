//! bait: the duelist's resting stance, and the timed window it opens.
//!
//! Consumer: a depth brain asks [`BaitLadder::step`] once per tick and gets one
//! [`BaitTactic`] back; the ladder is opt-in, so no shipped policy reads it
//! yet (SW131). Data owner: the ladder owns its own roll counter, window and
//! reason-free state; the caller owns the percept. Units: ticks for the window
//! and the roll, permille for the open chance. Determinism: an integer counter
//! and division only - no clock, no float, no model - so the same seed and the
//! same percept tape replay the same windows on every machine. Errors: total
//! functions, every input maps to a legal tactic. Budget: O(1) integer ops per
//! tick, zero allocation and no steady-state allocation.
//!
//! PORT (ideas reimplemented, zero bytes vendored): altf4/SmashBot
//! `Strategies/bait.py` (MIT): `Bait.step` rolls `random.randint(0, 120) == 0`
//! to open an approach, holds it for a 60-frame `approach_frame` and resets the
//! window the moment `opponent_state.invulnerability_left > 0`; the tactic
//! gates are ordered, so `Punish` is asked before `Defend`. The engine rolls a
//! counter instead of a PRNG, so a replay is exact: one open per
//! [`BAIT_ROLL_MOD`] ticks, which is
//! [`BAIT_OPEN_CHANCE_PERMILLE_PER_TICK`] permille a tick.
//!
//! Fighting-game rule: bait is the resting stance, not a passive idle. The
//! duelist holds its spacing ([`BaitTactic::Wait`]) until its own window opens,
//! moves in only inside that window, and drops the plan the instant the foe
//! turns invulnerable: a punish then lands on a whiffed answer, never on a coin
//! flip.

/// Chance one tick opens the approach window, permille (about 8: once in 121
/// ticks, roughly every two seconds at 60 Hz).
pub const BAIT_OPEN_CHANCE_PERMILLE_PER_TICK: u32 = 1000 / 121;

/// Ticks the approach roll divides into: `1000 / chance`, so the counter rolls
/// one window per 121 ticks.
pub const BAIT_ROLL_MOD: u32 = 121;

/// How long one approach window stays open, ticks (60 = one second at 60 Hz).
pub const APPROACH_WINDOW_TICKS: u32 = 60;

/// A foe that turns invulnerable resets the window and the roll, so the bait
/// starts over instead of resuming a window the foe already answered.
pub const BAIT_RESET_ON_INVULNERABLE: bool = true;

/// What the duelist does this tick, in the ladder's priority order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BaitTactic {
    /// Absorb or slip a committed strike.
    Mitigate,
    /// Hold spacing and keep baiting: the resting stance.
    Wait,
    /// Take the punish on a recovering foe.
    Punish,
    /// Block or retreat because the foe is striking now.
    Defend,
    /// Contest the exchange on purpose.
    Challenge,
    /// The window is open: move in.
    Approach,
    /// Too far to do anything but close the gap outside a window.
    KeepDistance,
}

impl BaitTactic {
    /// Short name for logs, reasons and digests.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Mitigate => "mitigate",
            Self::Wait => "wait",
            Self::Punish => "punish",
            Self::Defend => "defend",
            Self::Challenge => "challenge",
            Self::Approach => "approach",
            Self::KeepDistance => "keep_distance",
        }
    }
}

/// The fixed priority ladder, most urgent first. The order is the contract:
/// `mitigate > wait > punish > defend > challenge > approach > keep_distance`.
pub const BAIT_LADDER: [BaitTactic; 7] = [
    BaitTactic::Mitigate,
    BaitTactic::Wait,
    BaitTactic::Punish,
    BaitTactic::Defend,
    BaitTactic::Challenge,
    BaitTactic::Approach,
    BaitTactic::KeepDistance,
];

/// What the duelist can see about the exchange this tick.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BaitPercept {
    /// A committed strike can be absorbed or slipped now.
    pub mitigate: bool,
    /// Still baiting: hold spacing while no window is open.
    pub wait: bool,
    /// The foe is recovering and can be punished.
    pub punish: bool,
    /// The foe is striking now, so block or retreat.
    pub defend: bool,
    /// The exchange is worth contesting on purpose.
    pub challenge: bool,
}

/// The bait stance: a roll counter, the open window and the tick inside it.
#[derive(Clone, Debug, Default)]
pub struct BaitLadder {
    bait_rng: u32,
    approach_window: bool,
    approach_tick: u32,
    opens: u32,
}

impl BaitLadder {
    /// Fresh ladder at seed 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// Fresh ladder at a chosen counter seed; the same seed replays the same
    /// windows.
    pub fn with_seed(seed: u32) -> Self {
        Self {
            bait_rng: seed,
            ..Self::default()
        }
    }

    /// True while the approach window is open.
    pub fn approach_window(&self) -> bool {
        self.approach_window
    }

    /// Ticks spent inside the current window; 0 while closed.
    pub fn approach_tick(&self) -> u32 {
        self.approach_tick
    }

    /// The roll counter as it stands now.
    pub fn bait_rng(&self) -> u32 {
        self.bait_rng
    }

    /// Windows this bout has opened so far.
    pub fn opens(&self) -> u32 {
        self.opens
    }

    /// Advance one tick and answer the tactic for it.
    ///
    /// A foe that turns invulnerable aborts the window before anything else
    /// is considered, so no approach survives an invulnerable answer.
    pub fn step(&mut self, percept: BaitPercept, foe_invulnerable: bool) -> BaitTactic {
        if foe_invulnerable {
            if self.approach_window {
                self.close_window();
            }
            if BAIT_RESET_ON_INVULNERABLE {
                self.bait_rng = 0;
            }
            // Never approach a foe the duelist cannot touch; bait again.
            return if percept.mitigate {
                BaitTactic::Mitigate
            } else {
                BaitTactic::Wait
            };
        }
        // The roll runs every tick, window or not, so the open cadence stays
        // one window per BAIT_ROLL_MOD ticks: a window that just closed does
        // not push the next one out.
        self.roll_window();
        let tactic = self.choose(percept);
        if self.approach_window {
            self.approach_tick = self.approach_tick.saturating_add(1);
            if self.approach_tick >= APPROACH_WINDOW_TICKS {
                self.close_window();
            }
        }
        tactic
    }

    /// Walk the fixed ladder and return the first tactic whose gate holds.
    /// The open window lifts the baiting hold, so `wait` yields to `approach`
    /// and to everything above it.
    fn choose(&self, percept: BaitPercept) -> BaitTactic {
        let window = self.approach_window;
        for tactic in BAIT_LADDER {
            let ready = match tactic {
                BaitTactic::Mitigate => percept.mitigate,
                BaitTactic::Wait => percept.wait && !window,
                BaitTactic::Punish => percept.punish,
                BaitTactic::Defend => percept.defend,
                BaitTactic::Challenge => percept.challenge,
                BaitTactic::Approach => window,
                BaitTactic::KeepDistance => true,
            };
            if ready {
                return tactic;
            }
        }
        BaitTactic::KeepDistance
    }

    /// One roll: the counter moves, and every [`BAIT_ROLL_MOD`]th tick opens
    /// the window.
    fn roll_window(&mut self) {
        self.bait_rng = self.bait_rng.wrapping_add(1);
        if self.bait_rng.is_multiple_of(BAIT_ROLL_MOD) {
            self.approach_window = true;
            self.approach_tick = 0;
            self.opens = self.opens.saturating_add(1);
        }
    }

    /// Close the window and leave the tick counter at rest.
    fn close_window(&mut self) {
        self.approach_window = false;
        self.approach_tick = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The resting percept: still baiting, nothing else available.
    fn baiting() -> BaitPercept {
        BaitPercept {
            wait: true,
            ..BaitPercept::default()
        }
    }

    /// SW131 Accept 1: bait is the resting stance and the approach is a closed
    /// window. Over 600 ticks the seeded counter opens it 4..=6 times, each
    /// window at most 60 ticks, and it is never open outside a window.
    #[test]
    fn bait_is_the_resting_stance_and_approach_is_a_closed_window() {
        let mut ladder = BaitLadder::with_seed(0);
        let mut windows = 0;
        let mut run = 0;
        for _ in 0..600 {
            match ladder.step(baiting(), false) {
                BaitTactic::Approach => {
                    run += 1;
                    assert!(
                        run <= APPROACH_WINDOW_TICKS,
                        "a window never outlives {APPROACH_WINDOW_TICKS} ticks"
                    );
                }
                other => {
                    assert_eq!(
                        other,
                        BaitTactic::Wait,
                        "outside the window the duelist keeps baiting"
                    );
                    if run > 0 {
                        windows += 1;
                        run = 0;
                    }
                }
            }
        }
        if run > 0 {
            windows += 1;
        }
        assert!(
            (4..=6).contains(&windows),
            "600 ticks open 4..=6 windows, saw {windows}"
        );
        assert_eq!(windows, ladder.opens(), "the counter matches the windows");
        assert_eq!(
            BAIT_ROLL_MOD, 121,
            "one window per 121 ticks is about {:?} permille",
            BAIT_OPEN_CHANCE_PERMILLE_PER_TICK
        );
        assert_eq!(BAIT_OPEN_CHANCE_PERMILLE_PER_TICK, 8);
    }

    /// SW131 Accept 2: a foe that turns invulnerable aborts the window at once
    /// and the bait starts over.
    #[test]
    fn foe_invulnerability_aborts_the_approach_window() {
        let mut ladder = BaitLadder::with_seed(0);
        while !ladder.approach_window() {
            ladder.step(baiting(), false);
        }
        assert_eq!(ladder.step(baiting(), false), BaitTactic::Approach);
        let aborted = ladder.step(baiting(), true);
        assert_eq!(
            aborted,
            BaitTactic::Wait,
            "an invulnerable foe is never approached"
        );
        assert!(!ladder.approach_window(), "the window closes at once");
        assert_eq!(ladder.approach_tick(), 0);
        assert_eq!(ladder.bait_rng(), 0, "the roll starts over");
        const { assert!(BAIT_RESET_ON_INVULNERABLE) }
        // The next ticks bait again, and the roll has to run its course.
        for _ in 0..BAIT_ROLL_MOD - 1 {
            assert_eq!(ladder.step(baiting(), false), BaitTactic::Wait);
            assert!(!ladder.approach_window());
        }
        // One more tick opens a fresh window from the reset counter.
        assert_eq!(ladder.step(baiting(), false), BaitTactic::Approach);
        assert_eq!(ladder.opens(), 2, "the abort did not spend a window");
    }

    /// The ladder order is the contract: every gate is asked in order, and the
    /// open window lifts the baiting hold.
    #[test]
    fn bait_ladder_priority_is_fixed() {
        let names: Vec<&str> = BAIT_LADDER.iter().map(|t| t.name()).collect();
        assert_eq!(
            names,
            vec![
                "mitigate",
                "wait",
                "punish",
                "defend",
                "challenge",
                "approach",
                "keep_distance"
            ]
        );
        let mut ladder = BaitLadder::with_seed(0);
        let all = BaitPercept {
            mitigate: true,
            wait: true,
            punish: true,
            defend: true,
            challenge: true,
        };
        assert_eq!(ladder.step(all, false), BaitTactic::Mitigate);
        let no_mitigate = BaitPercept {
            mitigate: false,
            ..all
        };
        assert_eq!(ladder.step(no_mitigate, false), BaitTactic::Wait);
        let no_wait = BaitPercept {
            wait: false,
            ..no_mitigate
        };
        assert_eq!(ladder.step(no_wait, false), BaitTactic::Punish);
        let only_punish = BaitPercept {
            punish: true,
            ..BaitPercept::default()
        };
        assert_eq!(ladder.step(only_punish, false), BaitTactic::Punish);
        // Nothing available and no window: close the gap, never a phantom move in.
        assert_eq!(
            ladder.step(BaitPercept::default(), false),
            BaitTactic::KeepDistance
        );
        // Same seed and tape twice: same windows and the same tactic count.
        let tape = || {
            let mut ladder = BaitLadder::with_seed(7);
            let mut opens = 0;
            let mut approaches = 0;
            for _ in 0..600 {
                if ladder.step(baiting(), false) == BaitTactic::Approach {
                    approaches += 1;
                }
                opens = ladder.opens();
            }
            (opens, approaches)
        };
        assert_eq!(tape(), tape(), "a replay without a model drifts");
    }
}
