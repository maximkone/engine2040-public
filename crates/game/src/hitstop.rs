//! hitstop: the look's read of the live fight's hit-stop count.
//!
//! Combat sets `FightState.hitstop` on impact (light 3, heavy 6, parry 8 under
//! v2; 4 under v1) and freezes the fight; the look holds the attacker's and
//! victim's clip clocks while the count is above zero (see
//! [`crate::pose_hold`]). Data owner: combat owns the freeze authority. The App
//! mirrors that count read-only each tick (`LiveMatch::hitstop`), so the look
//! and the feel hold exactly as long as the sim. This module owns only the pure
//! hold predicate. Units: fixed ticks. Determinism: pure integer logic,
//! presentation only, never hashed. Errors: none; every function is total.
//!
//! Caller: `App::hitstop_holds` (`crates/game/src/lib.rs`) and the look's
//! per-tick hold (`crates/game/src/fighter_look_acts.rs`).

/// True while hitstop holds clip clocks (`remaining > 0`). Pure over the
/// count; the look calls it with the app's mirrored count.
#[must_use]
pub fn holds(remaining: u32) -> bool {
    remaining > 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hitstop_holds_above_zero_only() {
        assert!(holds(8));
        assert!(holds(3));
        assert!(holds(1));
        assert!(!holds(0));
    }
}
