//! perception: what the AI can see, one reaction beat late.
//!
//! Consumer: [`OpponentTable::observe_sighted`](crate::opponent::OpponentTable),
//! which reads this ring instead of the live staged frame. Both are opt-in:
//! the default brain still reads the foe the tick it is told, so no shipped
//! decision or hash moves until a later row swaps the read.
//!
//! A [`Sight`] holds only what a man watching could name: which way the body
//! is turned, which action is showing and for how long, the separation and the
//! visible health bar. It never holds the foe's intent, stamina or cooldown,
//! so a press cannot be answered on the tick it is made.
//!
//! Determinism: integer only, no clock, no float, no heap; the same staged
//! frames and seed read back the same sight twice. Units: ticks at 60 Hz,
//! millimetres of separation, permille of the health bar.

use crate::reaction_delay::ObsRing;

/// Ticks between the staged frame and the sight read: 8 ticks is 133 ms at
/// 60 Hz, about one human reaction beat.
pub const PERCEPTION_TICKS: usize = 8;

/// Slots the ring keeps: two perception beats plus headroom, fixed at 16 so a
/// long bout never grows the AI's memory.
pub const RING_SLOTS: usize = 16;

/// The action a foe is visibly in. A pose, never the intent behind it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StagedAction {
    /// Standing, weight on both feet.
    Idle,
    /// Winding up; the readable part of an attack.
    Windup,
    /// The blow is out.
    Strike,
    /// Pulling the weapon or arm back after a blow.
    Recover,
    /// Guard up.
    Block,
    /// Slipping aside.
    Dodge,
}

impl StagedAction {
    /// Stable word for a log, a tell row or a test failure message.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Windup => "windup",
            Self::Strike => "strike",
            Self::Recover => "recover",
            Self::Block => "block",
            Self::Dodge => "dodge",
        }
    }
}

/// One frame as a watching man sees it: pose, separation and the visible bar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sight {
    /// True when the foe's body is turned toward the perceiver.
    pub facing: bool,
    /// The visible action.
    pub action: StagedAction,
    /// Ticks the visible action has been showing.
    pub action_age: u16,
    /// Separation in millimetres; 0 is touching.
    pub dist_mm: u16,
    /// Visible health, permille of the bar.
    pub hp_permille: u16,
}

impl Sight {
    /// One frame the caller stages each tick; every field is observable.
    pub const fn new(
        facing: bool,
        action: StagedAction,
        action_age: u16,
        dist_mm: u16,
        hp_permille: u16,
    ) -> Self {
        Self {
            facing,
            action,
            action_age,
            dist_mm,
            hp_permille,
        }
    }
}

/// The perceiver's ring: frames staged as they happen, one sight read a beat
/// behind. The ring is the only memory, so a feint made inside the last
/// [`PERCEPTION_TICKS`] cannot be in the read.
#[derive(Clone, Copy, Debug)]
pub struct Perception {
    ring: ObsRing<Sight, RING_SLOTS>,
}

impl Perception {
    /// Empty ring: nothing staged yet, nothing seen.
    pub fn new() -> Self {
        Self {
            ring: ObsRing::new(),
        }
    }

    /// Stage the frame this tick shows. One call per tick.
    pub fn stage(&mut self, sight: Sight) {
        self.ring.push(sight);
    }

    /// The sight a man would have now: the frame staged
    /// [`PERCEPTION_TICKS`] ago, `None` until the ring is that deep.
    pub fn read(&self) -> Option<Sight> {
        self.ring.read(PERCEPTION_TICKS)
    }

    /// Frames staged so far, capped at [`RING_SLOTS`] by the ring itself.
    pub fn staged(&self) -> usize {
        self.ring.len()
    }

    /// Drop every staged frame (a new bout with a new foe).
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

impl Default for Perception {
    fn default() -> Self {
        Self::new()
    }
}

/// How fast the perceiver can answer, in milliseconds at 60 Hz.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ReactionBand {
    /// Below this age an action is never answered (a feint lives here).
    pub lo_ms: u32,
    /// At or above this age an action is always answered.
    pub hi_ms: u32,
}

impl ReactionBand {
    /// The shipped band: 220 ms floor, 420 ms certainty.
    pub const DEFAULT: Self = Self {
        lo_ms: 220,
        hi_ms: 420,
    };

    /// One millisecond is this many microseconds at 60 Hz; ages are ticks.
    pub const MS_PER_TICK: u32 = 16;

    /// True when an action this old is answered. Under `lo_ms` never, at or
    /// over `hi_ms` always, between them a seeded coin whose odds rise with
    /// the age. The same age and seed answer the same way twice.
    pub fn answers(&self, action_age: u32, seed: u64) -> bool {
        let age_ms = action_age.saturating_mul(Self::MS_PER_TICK);
        if age_ms < self.lo_ms {
            return false;
        }
        if age_ms >= self.hi_ms {
            return true;
        }
        let span = self.hi_ms.saturating_sub(self.lo_ms).max(1);
        let chance = (age_ms.saturating_sub(self.lo_ms)).saturating_mul(1000) / span;
        engine_core::splitmix64(seed ^ u64::from(age_ms)) % 1000 < u64::from(chance)
    }
}

impl Default for ReactionBand {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// One adaptation the AI ships in answer to what it saw: the counter and the
/// tell that announces it. The tell is required, so a counter cannot ship
/// silently; a text without one is refused at construction.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Adaptation {
    /// The counter the AI takes.
    pub answer: &'static str,
    /// What the player reads before the counter lands.
    pub tell: &'static str,
}

impl Adaptation {
    /// One adaptation row, or `None` when either the answer or the tell is
    /// empty: an unannounced counter is not a row.
    pub fn new(answer: &'static str, tell: &'static str) -> Option<Self> {
        if answer.trim().is_empty() || tell.trim().is_empty() {
            return None;
        }
        Some(Self { answer, tell })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(action: StagedAction, dist_mm: u16) -> Sight {
        Sight::new(true, action, 0, dist_mm, 1000)
    }

    #[test]
    fn perception_read_is_the_frame_staged_eight_ticks_ago() {
        let mut seen = Perception::new();
        for tick in 0..20u16 {
            seen.stage(frame(StagedAction::Idle, 1000 + tick));
        }
        // Tick 19 is the newest staged frame, so the read is tick 11.
        assert_eq!(seen.read(), Some(frame(StagedAction::Idle, 1000 + 11)));
        assert_eq!(seen.read(), seen.read(), "the same ring reads the same");
        // A ring that is not deep enough yet answers nothing.
        let mut shallow = Perception::new();
        for _ in 0..PERCEPTION_TICKS {
            shallow.stage(frame(StagedAction::Idle, 1000));
        }
        assert_eq!(shallow.read(), None, "no sight before one full beat");
    }

    #[test]
    fn perception_a_feint_inside_the_window_is_never_seen() {
        let mut seen = Perception::new();
        for _ in 0..PERCEPTION_TICKS {
            seen.stage(frame(StagedAction::Idle, 1200));
        }
        // The feint is staged inside the last beat: a wind-up that collapses
        // into idle. The read still shows the frame from before it.
        seen.stage(frame(StagedAction::Windup, 1200));
        seen.stage(frame(StagedAction::Idle, 1200));
        let read = seen.read().expect("one beat of frames is staged");
        assert_eq!(read.action, StagedAction::Idle, "the feint is unseen");
        // And even once the wind-up is old enough to see, a young action is
        // never answered: no seed turns a 3-tick wind-up into a reaction.
        let band = ReactionBand::DEFAULT;
        for seed in 0..64u64 {
            assert!(
                !band.answers(3, seed),
                "a 48 ms wind-up is inside the reaction floor"
            );
        }
    }

    #[test]
    fn perception_ring_never_grows_past_sixteen() {
        let mut seen = Perception::new();
        for tick in 0..100u16 {
            seen.stage(frame(StagedAction::Idle, 500 + tick));
        }
        assert_eq!(seen.staged(), RING_SLOTS, "the ring holds sixteen slots");
        // The oldest readable slot is 15 back; anything older left the ring.
        assert_eq!(
            seen.read(),
            Some(frame(StagedAction::Idle, 500 + 91)),
            "the read is eight back"
        );
    }

    #[test]
    fn perception_same_seed_answers_the_same_way_twice() {
        let band = ReactionBand::DEFAULT;
        let ages = [14, 18, 22, 26];
        for age in ages {
            for seed in [0u64, 1, 2040] {
                assert_eq!(
                    band.answers(age, seed),
                    band.answers(age, seed),
                    "same age {age} and seed {seed} twice"
                );
            }
        }
        // The band's ends are certain: below lo never, at hi always.
        assert!(!band.answers(13, 2040), "208 ms is inside the floor");
        assert!(band.answers(27, 0), "432 ms is past the ceiling");
        assert!(band.answers(27, u64::MAX));
        // A second band with the same numbers answers identically.
        let copy = ReactionBand {
            lo_ms: band.lo_ms,
            hi_ms: band.hi_ms,
        };
        for age in ages {
            assert_eq!(band.answers(age, 2040), copy.answers(age, 2040));
        }
    }

    #[test]
    fn perception_every_adaptation_row_carries_a_tell() {
        let row = Adaptation::new(
            "parry the recovered wind-up",
            "shoulder drops before the cut",
        )
        .expect("a told counter is a row");
        assert_eq!(row.answer, "parry the recovered wind-up");
        assert!(!row.tell.is_empty());
        assert!(
            Adaptation::new("parry", "").is_none(),
            "a silent counter cannot ship"
        );
        assert!(Adaptation::new("", "a tell with no answer").is_none());
        assert!(Adaptation::new("  ", "spaces are not a tell").is_none());
    }

    #[test]
    fn perception_opponent_reads_the_delayed_frame_not_the_live_one() {
        use crate::opponent::OpponentTable;
        let mut table = OpponentTable::new();
        let mut seen = Perception::new();
        assert_eq!(
            table.observe_sighted(seen.read()),
            None,
            "an empty ring answers nothing, so the caller keeps its read"
        );
        for _ in 0..=PERCEPTION_TICKS {
            seen.stage(frame(StagedAction::Windup, 1500));
        }
        assert!(
            table.observe_sighted(seen.read()).is_some(),
            "a seen frame answers through the table"
        );
        // The frame staged last tick is not in the read yet.
        seen.stage(frame(StagedAction::Strike, 400));
        assert_eq!(
            seen.read().map(|sight| sight.dist_mm),
            Some(1500),
            "the ring still holds the older frame"
        );
    }
}
