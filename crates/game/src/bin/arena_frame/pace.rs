//! arena_frame::pace: the fixed-dt harness pacer. The window loop asks
//! it when the next frame is due, so a perf run presents at a fixed rate
//! and pays the same number of fixed ticks on every run.
//!
//! Owner: engine-game bin; consumer: `arena.rs` (`about_to_wait`,
//! `pause_steps`). Units: the period in milliseconds, the deadlines in
//! `std::time::Instant`. Errors: none; the caller refuses a bad period
//! before the pacer is built.
//!
//! Presentation only: the pacer never reads or writes the sim, and the
//! tick it paces is the same fixed tick the window has always run.

use std::time::{Duration, Instant};

/// The fixed-dt pacer: one frame slot every `period`, counted from `start`.
/// The window presents the slot when it is due and skips the slots a late
/// frame missed, so a slow frame never queues a burst of catch-up frames.
#[derive(Clone, Copy, Debug)]
pub(crate) struct FixedDtPacer {
    /// The frame period.
    period: Duration,
    /// The instant the first slot starts at.
    start: Instant,
    /// The slot the window owes now.
    index: u32,
}

impl FixedDtPacer {
    /// A pacer whose first slot is due at `start`.
    pub(crate) fn new(period: Duration, start: Instant) -> Self {
        Self {
            period,
            start,
            index: 0,
        }
    }

    /// The frame period.
    pub(crate) fn period(&self) -> Duration {
        self.period
    }

    /// The instant the owed slot is due.
    pub(crate) fn deadline(&self) -> Instant {
        self.start + self.period * self.index
    }

    /// Take the slot after the one owed: the first slot at or after `now`,
    /// so a frame that ran late skips the slots it missed instead of
    /// drawing them back to back. A frame that ran early keeps its slot.
    pub(crate) fn advance(&mut self, now: Instant) {
        let late = now.saturating_duration_since(self.start);
        let period_ns = self.period.as_nanos().max(1);
        let periods = u64::try_from(late.as_nanos() / period_ns).unwrap_or(u64::MAX);
        let next = u32::try_from(periods.saturating_add(1)).unwrap_or(u32::MAX);
        self.index = self.index.saturating_add(1).max(next);
    }
}

/// Fixed ticks one presented frame owes under the fixed-dt harness: one,
/// clamped to the room left in the session. A window that cannot show a
/// frame owes none, like the wall-clock path. Pure over the three inputs,
/// so a test pins the tick count without a window.
pub(crate) fn fixed_dt_steps(shown: bool, ticks_done: u32, ticks_total: u32) -> u32 {
    u32::from(shown).min(ticks_total.saturating_sub(ticks_done))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn the_first_slot_is_due_at_the_start() {
        let start = Instant::now();
        let pacer = FixedDtPacer::new(ms(16), start);
        assert_eq!(pacer.deadline(), start);
        assert_eq!(pacer.index, 0);
        assert_eq!(pacer.period(), ms(16));
    }

    #[test]
    fn a_frame_on_time_keeps_the_next_slot_one_period_later() {
        let start = Instant::now();
        let mut pacer = FixedDtPacer::new(ms(16), start);
        // The slot is due: take the next one.
        pacer.advance(start);
        assert_eq!(pacer.index, 1);
        assert_eq!(pacer.deadline(), start + ms(16));
    }

    #[test]
    fn a_late_frame_skips_the_slots_it_missed() {
        let start = Instant::now();
        let mut pacer = FixedDtPacer::new(ms(16), start);
        // Three periods late: the next slot is the fourth, not a burst.
        pacer.advance(start + ms(48));
        assert_eq!(pacer.index, 4, "the missed slots are skipped");
        assert_eq!(pacer.deadline(), start + ms(64));
    }

    #[test]
    fn an_early_advance_keeps_the_next_slot_after_the_owed_one() {
        let start = Instant::now();
        let mut pacer = FixedDtPacer::new(ms(16), start);
        pacer.advance(start);
        // The frame came back early: the next slot is still the one after.
        pacer.advance(start + ms(1));
        assert_eq!(pacer.index, 2, "the slot after the owed one");
        assert_eq!(pacer.deadline(), start + ms(32));
    }

    #[test]
    fn a_clock_before_the_start_keeps_the_next_slot_after_the_owed_one() {
        let start = Instant::now();
        let mut pacer = FixedDtPacer::new(ms(16), start);
        pacer.advance(start);
        pacer.advance(start - ms(5));
        assert_eq!(pacer.index, 2, "a clock that went back never rewinds");
    }

    #[test]
    fn the_slot_count_grows_one_per_frame_at_the_period() {
        let start = Instant::now();
        let mut pacer = FixedDtPacer::new(ms(16), start);
        for frame in 0..10u32 {
            assert_eq!(pacer.index, frame, "frame {frame} owes its slot");
            pacer.advance(start + ms(u64::from(frame) * 16));
        }
    }

    #[test]
    fn the_fixed_dt_tick_count_is_one_per_presented_frame() {
        // 300 presented frames pay 300 ticks, so two runs of the same fixture
        // hash the same: the sim phase never rides the frame cost.
        let mut ticks = 0u32;
        for _ in 0..300 {
            ticks += fixed_dt_steps(true, ticks, 300);
        }
        assert_eq!(ticks, 300, "one tick per presented frame");
        // A window that cannot show a frame owes none.
        assert_eq!(fixed_dt_steps(false, 0, 300), 0);
        // The session's room bounds the count.
        assert_eq!(fixed_dt_steps(true, 299, 300), 1);
        assert_eq!(fixed_dt_steps(true, 300, 300), 0);
    }
}
