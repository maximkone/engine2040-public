//! personality: a seeded, integer-only temperament for one brain.
//!
//! Consumer: `arena` derives one [`Personality`] per slot from the match seed
//! under rules v2 and installs it on each [`DepthBrain`](crate::depth::DepthBrain),
//! which carries it for the whole match. Data owner: the brain owns its copy;
//! the arena owns only the seed. Units: reaction in ticks, preferred range in
//! millimetres, aggression in permille. Determinism: a pure function of
//! `(seed, slot)` through the `engine_core` counter stream, no wall clock, no
//! float, so replays and snapshot forks derive the same temperaments.
//! Errors: none; constructors clamp into the legal bands.
//!
//! Why: with the symmetric v2 hit rule two identical brains close in lockstep
//! and trade blows until a same-tick double knockout. A temperament gives
//! each slot its own reaction time, spacing and nerve, so duels end with a
//! winner while the slot a temperament lands on stays a coin flip.

use engine_core::CounterRng;

/// Slowest reaction offset in ticks (the foe's blow is noticed this late).
pub const REACTION_MAX_TICKS: u8 = 5;
/// Shortest preferred swing range in millimetres.
pub const RANGE_MIN_MM: u16 = 1_000;
/// Longest preferred swing range in millimetres (inside the blade reach).
pub const RANGE_MAX_MM: u16 = 1_500;
/// Lowest seeded aggression in permille.
pub const AGGRESSION_MIN: u16 = 250;
/// Highest seeded aggression in permille.
pub const AGGRESSION_MAX: u16 = 750;
/// Neutral aggression in permille: no bias.
pub const AGGRESSION_NEUTRAL: u16 = 500;

/// Attack-lane penalty beyond the preferred range: the swing is held.
const HOLD_SWING_BIAS: i32 = 800;
/// Advance-lane bonus beyond the preferred range: the brain closes in.
const CLOSE_IN_BIAS: i32 = 350;

/// Domain name keeping the temperament stream apart from other seed uses.
const STREAM_NAME: &str = "temperament";

/// One brain's temperament. All fields are plain integers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Personality {
    reaction_ticks: u8,
    preferred_range_mm: u16,
    aggression_permille: u16,
}

impl Personality {
    /// Temperament from raw numbers, clamped into the legal bands.
    pub fn new(reaction_ticks: u8, preferred_range_mm: u16, aggression_permille: u16) -> Self {
        Self {
            reaction_ticks: reaction_ticks.min(REACTION_MAX_TICKS),
            preferred_range_mm: preferred_range_mm.clamp(RANGE_MIN_MM, RANGE_MAX_MM),
            aggression_permille: aggression_permille.min(1_000),
        }
    }

    /// Seeded temperament for `slot` of the match `seed`: one counter stream
    /// per slot, three draws. The same pair gives the same temperament on
    /// every platform; neighbouring seeds and slots draw unrelated values.
    pub fn seeded(seed: u64, slot: u8) -> Self {
        let stream = CounterRng::new(seed)
            .fork_named(STREAM_NAME)
            .fork(u64::from(slot));
        let span = |lo: u16, hi: u16, word: u64| {
            let width = u64::from(hi - lo) + 1;
            lo + u16::try_from(word % width).unwrap_or(0)
        };
        let reaction = span(0, u16::from(REACTION_MAX_TICKS), stream.sample(0));
        Self::new(
            u8::try_from(reaction).unwrap_or(0),
            span(RANGE_MIN_MM, RANGE_MAX_MM, stream.sample(1)),
            span(AGGRESSION_MIN, AGGRESSION_MAX, stream.sample(2)),
        )
    }

    /// Ticks between the foe's blow starting and this brain noticing it.
    pub const fn reaction_ticks(&self) -> u8 {
        self.reaction_ticks
    }

    /// Separation (mm) beyond which this brain holds its swing and closes in.
    pub const fn preferred_range_mm(&self) -> u16 {
        self.preferred_range_mm
    }

    /// Nerve in permille: above 500 swings and presses, below it guards.
    pub const fn aggression_permille(&self) -> u16 {
        self.aggression_permille
    }

    /// Additive score bias in utility order (attack, block, dodge, advance,
    /// retreat) at `dist_mm`: aggression leans the lanes, and beyond the
    /// preferred range the swing is held and the brain closes in instead.
    /// Bounded to `+/-1000`.
    pub fn score_bias(&self, dist_mm: i32) -> [i32; 5] {
        let nerve = i32::from(self.aggression_permille) - i32::from(AGGRESSION_NEUTRAL);
        let mut bias = [nerve * 2 / 3, -nerve / 4, -nerve / 4, nerve / 4, -nerve / 2];
        if dist_mm > i32::from(self.preferred_range_mm) {
            bias[0] = bias[0].saturating_sub(HOLD_SWING_BIAS);
            bias[3] = bias[3].saturating_add(CLOSE_IN_BIAS);
        }
        bias
    }

    /// Thinker weights for this nerve. Units: multiplier. Errors: none.
    /// Caller: crates/ai/src/gladiator.rs thinker path.
    pub fn weights(&self) -> Weights {
        Weights::from_morale(i32::from(self.aggression_permille()))
    }
}

/// Per-lane thinker multipliers in utility order (attack, block, dodge, advance, retreat) around 1.0. Units: multiplier. Errors: none. Deterministic.
/// Caller: crates/ai/src/gladiator.rs thinker path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Weights {
    /// Lane multipliers.
    pub lanes: [f32; 5],
}

impl Weights {
    /// Neutral nerve: no lean. Units: multiplier. Errors: none.
    pub const fn neutral() -> Self {
        Self { lanes: [1.0; 5] }
    }

    /// Bold nerve: swings and presses. Units: multiplier. Errors: none.
    pub const fn bold() -> Self {
        Self {
            lanes: [1.25, 0.85, 0.85, 1.15, 0.75],
        }
    }

    /// Shy nerve: guards and gives ground. Units: multiplier. Errors: none.
    pub const fn shy() -> Self {
        Self {
            lanes: [0.75, 1.25, 1.15, 0.85, 1.25],
        }
    }

    /// Nerve from 0..=1000 nerve (morale or aggression): above neutral bold, below shy. Units: multiplier. Errors: none.
    pub fn from_morale(nerve: i32) -> Self {
        if nerve > i32::from(AGGRESSION_NEUTRAL) {
            Self::bold()
        } else if nerve < i32::from(AGGRESSION_NEUTRAL) {
            Self::shy()
        } else {
            Self::neutral()
        }
    }

    /// Apply multipliers with clamp. Units: scores 0..1. Errors: none. Deterministic.
    pub fn apply(&self, scores: [f32; 5]) -> [f32; 5] {
        [
            crate::utility::clamp01(scores[0] * self.lanes[0]),
            crate::utility::clamp01(scores[1] * self.lanes[1]),
            crate::utility::clamp01(scores[2] * self.lanes[2]),
            crate::utility::clamp01(scores[3] * self.lanes[3]),
            crate::utility::clamp01(scores[4] * self.lanes[4]),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn personality_seeded_is_a_pure_function_of_seed_and_slot() {
        for seed in [0u64, 1, 2, 7, u64::MAX] {
            for slot in 0..8u8 {
                assert_eq!(
                    Personality::seeded(seed, slot),
                    Personality::seeded(seed, slot)
                );
            }
        }
    }

    #[test]
    fn personality_seeded_stays_inside_the_bands() {
        for seed in 0..500u64 {
            for slot in 0..8u8 {
                let p = Personality::seeded(seed, slot);
                assert!(p.reaction_ticks() <= REACTION_MAX_TICKS);
                assert!((RANGE_MIN_MM..=RANGE_MAX_MM).contains(&p.preferred_range_mm()));
                assert!((AGGRESSION_MIN..=AGGRESSION_MAX).contains(&p.aggression_permille()));
            }
        }
    }

    #[test]
    fn personality_slots_and_adjacent_seeds_differ() {
        let mut same_slots = 0u32;
        let mut same_seeds = 0u32;
        for seed in 0..400u64 {
            same_slots += u32::from(Personality::seeded(seed, 0) == Personality::seeded(seed, 1));
            same_seeds +=
                u32::from(Personality::seeded(seed, 0) == Personality::seeded(seed + 1, 0));
        }
        assert_eq!(
            same_slots, 0,
            "two slots of one match never share a temperament"
        );
        assert_eq!(same_seeds, 0, "seeds 2k and 2k+1 do not alias");
    }

    #[test]
    fn personality_draws_cover_every_reaction_and_both_sides_of_nerve() {
        let mut reactions = [0u32; 6];
        let (mut bold, mut shy) = (0u32, 0u32);
        for seed in 0..600u64 {
            let p = Personality::seeded(seed, 0);
            reactions[usize::from(p.reaction_ticks())] += 1;
            bold += u32::from(p.aggression_permille() > AGGRESSION_NEUTRAL);
            shy += u32::from(p.aggression_permille() < AGGRESSION_NEUTRAL);
        }
        assert!(reactions.iter().all(|n| *n > 40), "{reactions:?}");
        assert!(bold > 200 && shy > 200, "bold {bold} shy {shy}");
    }

    #[test]
    fn personality_new_clamps_out_of_band_numbers() {
        let p = Personality::new(200, 5, 4_000);
        assert_eq!(p.reaction_ticks(), REACTION_MAX_TICKS);
        assert_eq!(p.preferred_range_mm(), RANGE_MIN_MM);
        assert_eq!(p.aggression_permille(), 1_000);
        assert_eq!(
            Personality::new(0, 60_000, 0).preferred_range_mm(),
            RANGE_MAX_MM
        );
    }

    #[test]
    fn personality_bias_holds_the_swing_beyond_range_and_leans_with_nerve() {
        let calm = Personality::new(0, 1_200, AGGRESSION_NEUTRAL);
        assert_eq!(
            calm.score_bias(1_000),
            [0; 5],
            "neutral and in range is no bias"
        );
        let beyond = calm.score_bias(1_300);
        assert!(beyond[0] <= -800 && beyond[3] >= 350);
        let bold = Personality::new(0, 1_200, 750).score_bias(1_000);
        let shy = Personality::new(0, 1_200, 250).score_bias(1_000);
        assert!(bold[0] > 0 && bold[4] < 0, "bold swings and presses");
        assert!(
            shy[0] < 0 && shy[4] > 0 && shy[1] > 0,
            "shy guards and gives ground"
        );
    }
}
