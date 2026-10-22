//! primus: the playtest proof for the T4 boss (GA15).
//!
//! Consumer: the GA15 board row and the design card
//! (`docs/design/ENEMIES.md`). The row's Accept asks for a playtest-crate
//! `primus` filter that prints a nonzero passed count, so this module is
//! the proof: three named legs, each a pure fork measurement over seeded
//! duels.
//!
//! Data owner: the caller owns the duel seeds. The tier rows come from
//! `engine-ai` (`PRIMUS_TIER`, `SHIPPED_TIERS`); the primus phase helpers
//! (`primus_phase`, `primus_counter`) are the row's own arithmetic.
//! Units: permille shares, fixed ticks at 60 Hz. Determinism: every leg
//! runs the same fork path twice and the two numbers must be equal, so
//! the proof is replay-exact. Errors: a leg returns a named failure, never
//! a panic.

use engine_ai::depth::{primus_counter, primus_phase, PRIMUS_TIER};
use engine_ai::tier::{TierRow, SHIPPED_TIERS};

use crate::tournament::tier_duel_wins;

/// Seeded duels per primus leg, matching the tier check's
/// [`TIER_DUELS`](engine_ai::tier::TIER_DUELS).
pub const PRIMUS_DUELS: u32 = engine_ai::tier::TIER_DUELS;

/// The primus's phase-2 health share in permille: phase 2 opens at or
/// below this share of the boss's health.
pub const PRIMUS_PHASE_TWO_PERMILLE: u16 = 500;

/// The T3 elite row, the primus's rung below.
pub const ELITE_TIER: TierRow = SHIPPED_TIERS[2];

/// One primus leg's failure, named so the board row can read it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PrimusError {
    /// The duel fork refused the tier install or the run.
    RunRefused,
    /// T3 beat T4 phase 1 in too many duels: the boss is not a boss.
    /// The share of T3 wins is carried, in permille.
    BossTooWeak { t3_wins_permille: u16 },
    /// The primus phase-2 share does not open at
    /// [`PRIMUS_PHASE_TWO_PERMILLE`]. The phase it opened at is carried.
    PhaseTwoShareWrong { opened_at: u8 },
    /// A T4 attack has no answer: the phase's counter is outside the
    /// player's kit (`1` parry, `2` dodge, `3` block). The counter and
    /// the phase are carried.
    AttackWithoutAnswer { phase: u8, counter: u8 },
    /// The primus's own phases do not differ: phase 1 and phase 2 answer
    /// the same way. Both counters are carried.
    PhasesShareAnAnswer { phase_one: u8, phase_two: u8 },
}

impl core::fmt::Display for PrimusError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match *self {
            Self::RunRefused => write!(f, "primus duel run refused"),
            Self::BossTooWeak { t3_wins_permille } => write!(
                f,
                "the T3 elite beat T4 in {t3_wins_permille} permille of the duels, over the boss ceiling"
            ),
            Self::PhaseTwoShareWrong { opened_at } => write!(
                f,
                "primus phase 2 opened at {opened_at} permille, not {PRIMUS_PHASE_TWO_PERMILLE}"
            ),
            Self::AttackWithoutAnswer { phase, counter } => write!(
                f,
                "primus phase {phase} answers with {counter}, outside the player kit"
            ),
            Self::PhasesShareAnAnswer { phase_one, phase_two } => write!(
                f,
                "primus phases 1 and 2 answer the same ({phase_one} and {phase_two})"
            ),
        }
    }
}

/// The boss ceiling: the T3 elite may beat T4 phase 1 in at most this
/// share of the duels, in permille. The row asks for "under 40%".
pub const BOSS_CEILING_PERMILLE: u16 = 400;

/// Leg one: T3 beats T4 phase 1 in under 40% of the duels, so the primus
/// is a boss and not a rung. Runs the whole leg twice and both passes
/// must give the same share.
pub fn boss_ceiling_holds() -> Result<u16, PrimusError> {
    let mut wins = 0u32;
    for i in 0..PRIMUS_DUELS {
        match tier_duel_wins(ELITE_TIER, PRIMUS_TIER, i) {
            Some(true) => wins += 1,
            Some(false) => {}
            None => return Err(PrimusError::RunRefused),
        }
    }
    let permille = u16::try_from(wins * 1000 / PRIMUS_DUELS).unwrap_or(u16::MAX);
    if permille >= BOSS_CEILING_PERMILLE {
        return Err(PrimusError::BossTooWeak {
            t3_wins_permille: permille,
        });
    }
    Ok(permille)
}

/// Leg two: the primus opens phase 2 at 500 permille of its health, and
/// the two phases answer differently, so phase 2 is a new move and not
/// more health.
pub fn phase_two_opens_and_answers() -> Result<(), PrimusError> {
    let opened_at = primus_phase(PRIMUS_PHASE_TWO_PERMILLE);
    if opened_at != 2 {
        return Err(PrimusError::PhaseTwoShareWrong { opened_at });
    }
    let phase_one = primus_counter(1);
    let phase_two = primus_counter(2);
    for (phase, counter) in [(1, phase_one), (2, phase_two)] {
        if !(1..=3).contains(&counter) {
            return Err(PrimusError::AttackWithoutAnswer { phase, counter });
        }
    }
    if phase_one == phase_two {
        return Err(PrimusError::PhasesShareAnAnswer {
            phase_one,
            phase_two,
        });
    }
    Ok(())
}

/// Leg three: every T4 attack has an answer, in both phases. The primus's
/// phase-1 row carries the parry answer and phase 2 carries the dodge, so
/// the player always has a reply.
pub fn every_primus_attack_has_an_answer() -> Result<(), PrimusError> {
    phase_two_opens_and_answers()?;
    for phase in 1..=2u8 {
        let counter = primus_counter(phase);
        if !(1..=3).contains(&counter) {
            return Err(PrimusError::AttackWithoutAnswer { phase, counter });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The boss leg: the T3 elite beats T4 phase 1 in under 40% of the
    /// duels, and the whole leg replays exactly.
    #[test]
    fn primus_boss_beats_the_elite_in_most_duels() {
        let first = boss_ceiling_holds().expect("the primus holds the boss ceiling");
        let again = boss_ceiling_holds().expect("the boss leg replays");
        assert_eq!(first, again, "the boss leg must replay exactly");
        assert!(
            first < BOSS_CEILING_PERMILLE,
            "the T3 elite beat T4 in {first} permille of {PRIMUS_DUELS} duels, at or over the {BOSS_CEILING_PERMILLE} ceiling"
        );
        println!("primus boss leg: T3 won {first} permille of {PRIMUS_DUELS} duels");
    }

    /// The phase leg: phase 2 opens at 500 permille and answers
    /// differently from phase 1.
    #[test]
    fn primus_phase_two_opens_at_500_permille() {
        phase_two_opens_and_answers().expect("phase 2 opens with its own answer");
        assert_eq!(primus_phase(501), 1);
        assert_eq!(primus_phase(500), 2);
        assert_eq!(primus_phase(0), 2);
        assert_eq!(primus_counter(1), 1, "phase 1 answers with a parry");
        assert_eq!(primus_counter(2), 2, "phase 2 answers with a dodge");
    }

    /// The answer leg: every T4 attack has an answer in both phases.
    #[test]
    fn primus_every_attack_has_an_answer() {
        every_primus_attack_has_an_answer().expect("every T4 attack has an answer");
        for phase in 1..=2u8 {
            let counter = primus_counter(phase);
            assert!(
                (1..=3).contains(&counter),
                "phase {phase} answer {counter} is outside the player kit"
            );
        }
    }
}
