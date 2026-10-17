//! How often the shell starts over after a lost device before it gives up.
//!
//! A lost device is normal now and then (a driver reset, a GPU process restart): the shell
//! asks for a fresh adapter and goes on. A device that is lost again and again is not
//! recoverable, and starting over each time would spin forever, so a short run of losses ends
//! in the fatal page instead.

/// How many times the shell asks for a fresh adapter after a loss before it gives up, and the
/// pause (milliseconds, times the attempt number) before each: a browser may need a moment to
/// bring its GPU process back.
pub const ATTEMPTS: u32 = 4;
pub const PAUSE_MS: i32 = 500;

/// The most device losses allowed inside [`WINDOW_MS`].
pub const MAX_LOSSES: usize = 3;

/// The span in which [`MAX_LOSSES`] losses count as a storm.
pub const WINDOW_MS: f64 = 10_000.0;

/// Times of recent losses, in the browser's millisecond clock.
#[derive(Clone, Debug, Default)]
pub struct LossBudget {
    recent: Vec<f64>,
}

impl LossBudget {
    #[must_use]
    pub const fn new() -> Self {
        Self { recent: Vec::new() }
    }

    /// Record a loss at `now_ms`. True when the shell may start over, false when this loss is
    /// the [`MAX_LOSSES`]th inside [`WINDOW_MS`].
    pub fn record(&mut self, now_ms: f64) -> bool {
        self.recent.retain(|t| now_ms - t < WINDOW_MS);
        self.recent.push(now_ms);
        self.recent.len() < MAX_LOSSES
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_few_losses_in_a_row_are_survived_and_a_storm_is_not() {
        let mut b = LossBudget::default();
        assert!(b.record(1_000.0));
        assert!(b.record(1_050.0));
        assert!(
            !b.record(1_100.0),
            "the third loss inside the window gives up"
        );
    }

    #[test]
    fn losses_spread_out_never_add_up() {
        let mut b = LossBudget::default();
        for k in 0..50 {
            assert!(b.record(f64::from(k) * (WINDOW_MS + 1.0)), "loss {k}");
        }
    }

    #[test]
    fn old_losses_leave_the_window() {
        let mut b = LossBudget::default();
        assert!(b.record(0.0));
        assert!(b.record(100.0));
        // Both are older than the window by now, so this is the first of a new run.
        assert!(b.record(WINDOW_MS + 500.0));
        assert!(b.record(WINDOW_MS + 600.0));
        assert!(!b.record(WINDOW_MS + 700.0));
    }

    #[test]
    fn a_clock_that_steps_back_cannot_hide_a_storm() {
        let mut b = LossBudget::default();
        assert!(b.record(5_000.0));
        assert!(b.record(4_000.0));
        assert!(!b.record(4_500.0));
    }
}
