//! hitstop: read-only mirror of the sim's hitstop count for the look.
//!
//! Combat sets `state.hitstop` on impact and freezes the fight; the look
//! holds the attacker's and victim's clip clocks while the count is above
//! zero (see [`crate::pose_hold`]). Data owner: combat owns the freeze
//! authority; this module owns only the pure count law the app folds per
//! tick. Units: fixed ticks. Determinism: pure integer logic, presentation
//! only, never hashed. Errors: none; every function is total.
//!
//! Caller: `crates/game/src/lib.rs` (`App::simulate_tick`, `App::rebuild_live`).

use engine_combat::fight::HITSTOP_TICKS;
use engine_combat::FightEvent;

/// Next hitstop count from the tick's combat events: a Hit or Parried sets
/// [`HITSTOP_TICKS`], anything else (Blocked, Dodged, quiet) counts down by
/// one, never below zero.
#[must_use]
pub fn next(current: u32, events: &[FightEvent]) -> u32 {
    if events
        .iter()
        .any(|event| matches!(event, FightEvent::Hit { .. } | FightEvent::Parried { .. }))
    {
        HITSTOP_TICKS
    } else {
        current.saturating_sub(1)
    }
}

/// True while hitstop holds clip clocks (`remaining > 0`). Pure over the
/// count; the look calls it with the app's mirrored count.
#[must_use]
pub fn holds(remaining: u32) -> bool {
    remaining > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit() -> FightEvent {
        FightEvent::Hit {
            attacker: 0,
            victim: 1,
            damage: 120,
        }
    }

    #[test]
    fn hitstop_next_sets_four_on_hit_then_counts_down() {
        assert_eq!(next(0, &[hit()]), HITSTOP_TICKS);
        assert_eq!(next(HITSTOP_TICKS, &[]), HITSTOP_TICKS - 1);
        assert_eq!(next(0, &[]), 0, "countdown never goes below zero");
    }

    #[test]
    fn hitstop_next_ignores_blocked_and_dodged_sets_on_parry() {
        let blocked = FightEvent::Blocked {
            attacker: 2,
            victim: 3,
            chip: 10,
        };
        let dodged = FightEvent::Dodged {
            attacker: 2,
            victim: 3,
        };
        let parried = FightEvent::Parried {
            attacker: 1,
            victim: 0,
        };
        assert_eq!(next(0, &[blocked]), 0);
        assert_eq!(next(0, &[dodged]), 0);
        assert_eq!(next(0, &[parried]), HITSTOP_TICKS);
    }

    #[test]
    fn hitstop_holds_above_zero_only() {
        assert!(holds(HITSTOP_TICKS));
        assert!(holds(1));
        assert!(!holds(0));
    }
}
