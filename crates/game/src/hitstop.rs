//! hitstop: the look's read of the live fight's hit-stop count.
//!
//! Combat sets `FightState.hitstop` on impact (light 3, heavy 6, parry 8 under
//! v2; 4 under v1) and freezes the fight; the look holds the attacker's and
//! victim's clip clocks while the count is above zero (see
//! [`crate::pose_hold`]). Data owner: combat owns the freeze authority. The App
//! mirrors that count read-only each tick (`LiveMatch::hitstop`), so the look
//! and the feel hold exactly as long as the sim. This module owns only the pure
//! hold predicate and the presentation freeze cap (row QL7: the look holds at
//! most 100 ms in frames however long the sim freezes). Units: fixed ticks and
//! frames at 60 Hz. Determinism: pure integer logic, presentation only, never
//! hashed. Errors: none; every function is total.
//!
//! Caller: `App::hitstop_holds` (`crates/game/src/lib.rs`) and the look's
//! per-tick hold (`crates/game/src/fighter_look_acts.rs`).

/// True while hitstop holds clip clocks (`remaining > 0`). Pure over the
/// count; the look calls it with the app's mirrored count.
#[must_use]
pub fn holds(remaining: u32) -> bool {
    remaining > 0
}

/// Ceiling on the presentation freeze, in frames at 60 Hz: the row's 100 ms.
pub const FREEZE_MAX_FRAMES: u32 = 6;

/// The presentation freeze for one sim hit-stop count, in frames. The row
/// (QL7) caps the look's hold at 100 ms, so a parry's 8 ticks freeze the clock
/// six frames; the sim's own count is never rewritten, only read.
#[must_use]
pub fn freeze_frames(hitstop: u32) -> u32 {
    hitstop.min(FREEZE_MAX_FRAMES)
}

/// A freeze in wall milliseconds at the feel clock's 60 Hz, in integers so six
/// frames print 100, not 99.999. Saturates rather than wrapping.
#[must_use]
pub fn freeze_ms(frames: u32) -> u32 {
    frames.saturating_mul(1000) / 60
}

/// The receipt line one held freeze prints: `feel: hitstop hold=6 ms=100`.
/// Built once per hold, never per tick.
#[must_use]
pub fn hold_line(frames: u32) -> String {
    format!("feel: hitstop hold={frames} ms={}", freeze_ms(frames))
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

    #[test]
    fn hit_stop_freeze_is_capped_at_six_frames_and_one_hundred_ms() {
        // QL7: light 3, heavy 6 pass through; a parry's 8 stops at 6 frames,
        // which is 100 ms on the 60 Hz feel clock, never 133.
        assert_eq!(freeze_frames(0), 0);
        assert_eq!(freeze_frames(3), 3);
        assert_eq!(freeze_frames(6), 6);
        assert_eq!(freeze_frames(8), 6, "a parry's 8 frames stop at the cap");
        assert_eq!(freeze_frames(u32::MAX), FREEZE_MAX_FRAMES);
        assert_eq!(freeze_ms(freeze_frames(8)), 100, "100 ms, not 133");
        assert_eq!(freeze_ms(3), 50, "50 ms on a light blow");
        assert_eq!(freeze_ms(6), 100);
        assert_eq!(freeze_ms(u32::MAX), u32::MAX / 60, "saturates, no wrap");
        // The sim's 3/6/8 stay the sim's: this module only reads them.
        assert_eq!(
            [freeze_frames(3), freeze_frames(6), freeze_frames(8)],
            [3, 6, 6]
        );
        // The receipt line carries the capped frames and their milliseconds.
        assert_eq!(hold_line(6), "feel: hitstop hold=6 ms=100");
        assert_eq!(hold_line(freeze_frames(8)), "feel: hitstop hold=6 ms=100");
        assert_eq!(hold_line(3), "feel: hitstop hold=3 ms=50");
    }
}
