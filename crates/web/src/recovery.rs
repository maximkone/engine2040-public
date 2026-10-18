//! How often the shell starts over after a lost device before it gives up.
//!
//! A lost device is normal now and then (a driver reset, a GPU process restart): the shell
//! asks for a fresh adapter and goes on. A device that is lost again and again is not
//! recoverable, and starting over each time would spin forever, so a short run of losses ends
//! in the fatal page instead.
//!
//! Three ways a device ends, kept apart (QW8): the browser takes it (a real loss: start over),
//! the page's own test hook destroys it to prove starting over works (a forced loss: start
//! over), and the page destroys it on purpose because it is going away (a requested destroy:
//! never start over). The receipt carries the browser's own `GPUDeviceLostInfo` (reason and
//! message) beside who ended the device.

use std::sync::Mutex;

/// How many times the shell asks for a fresh adapter after a loss before it gives up, and the
/// pause (milliseconds, times the attempt number) before each: a browser may need a moment to
/// bring its GPU process back.
pub const ATTEMPTS: u32 = 4;
pub const PAUSE_MS: i32 = 500;

/// The most device losses allowed inside [`WINDOW_MS`].
pub const MAX_LOSSES: usize = 3;

/// The span in which [`MAX_LOSSES`] losses count as a storm.
pub const WINDOW_MS: f64 = 10_000.0;

/// Longest loss message the fatal page repeats (the browser's text can run long).
const MESSAGE_MAX: usize = 200;

/// Who ended a device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    /// Nobody on the page: the browser or the driver took the device. A real loss.
    Browser,
    /// The page's test hook (`lose_device`) destroyed the device to prove the page starts over.
    Forced,
    /// The page destroyed the device on purpose, for good (`destroy_device`): not a loss.
    Requested,
}

impl Cause {
    /// The word the receipt prints.
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Browser => "browser",
            Self::Forced => "forced",
            Self::Requested => "requested",
        }
    }
}

/// `GPUDeviceLostReason` as WebGPU spells it.
#[must_use]
pub fn reason_word(reason: wgpu::DeviceLostReason) -> &'static str {
    match reason {
        wgpu::DeviceLostReason::Unknown => "unknown",
        wgpu::DeviceLostReason::Destroyed => "destroyed",
    }
}

/// What a lost device reported (`GPUDeviceLostInfo`) and who ended it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LossNote {
    pub reason: String,
    pub message: String,
    pub cause: Cause,
}

/// The mailbox of a device's lost callback: the callback may run at any time, the frame loop
/// reads it once a frame. The first report stands.
#[derive(Debug, Default)]
pub struct LostBox(Mutex<Option<(String, String)>>);

impl LostBox {
    /// Called by the lost callback with the browser's reason and message.
    pub fn set(&self, reason: &str, message: &str) {
        if let Ok(mut slot) = self.0.lock() {
            slot.get_or_insert_with(|| (reason.to_string(), message.to_string()));
        }
    }

    /// The report, once the device is lost.
    #[must_use]
    pub fn get(&self) -> Option<(String, String)> {
        self.0.lock().ok().and_then(|slot| slot.clone())
    }
}

/// What to do about a lost device.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Verdict {
    /// Start over on a fresh adapter.
    Retry,
    /// Too many losses too fast: the fatal page.
    GiveUp,
    /// The page ended the device on purpose: stay stopped.
    Stop,
}

/// Times of recent losses, in the browser's millisecond clock, and the last one's note.
#[derive(Clone, Debug, Default)]
pub struct LossBudget {
    recent: Vec<f64>,
    total: u32,
    last: Option<LossNote>,
}

impl LossBudget {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            recent: Vec::new(),
            total: 0,
            last: None,
        }
    }

    /// Record a loss at `now_ms`. True when the shell may start over, false when this loss is
    /// the [`MAX_LOSSES`]th inside [`WINDOW_MS`].
    pub fn record(&mut self, now_ms: f64) -> bool {
        self.recent.retain(|t| now_ms - t < WINDOW_MS);
        self.recent.push(now_ms);
        self.total += 1;
        self.recent.len() < MAX_LOSSES
    }

    /// Decide what to do about a device lost at `now_ms`. A requested destroy is not a loss: it
    /// is not counted and never retried. Any other end is recorded with its note.
    pub fn judge(&mut self, now_ms: f64, note: LossNote) -> Verdict {
        if note.cause == Cause::Requested {
            return Verdict::Stop;
        }
        let retry = self.record(now_ms);
        self.last = Some(note);
        if retry {
            Verdict::Retry
        } else {
            Verdict::GiveUp
        }
    }

    /// Losses so far (a requested destroy is not one).
    #[must_use]
    pub fn total(&self) -> u32 {
        self.total
    }

    /// The last loss, with the browser's reason and message.
    #[must_use]
    pub fn last(&self) -> Option<&LossNote> {
        self.last.as_ref()
    }

    /// What the fatal page says after [`Verdict::GiveUp`]: how many losses in how long, and
    /// the browser's reason for the last.
    #[must_use]
    pub fn give_up_detail(&self) -> String {
        let storm = format!(
            "the device was lost {MAX_LOSSES} times within {} s; the browser cannot keep one",
            WINDOW_MS / 1000.0
        );
        match &self.last {
            Some(note) => {
                let message: String = note.message.chars().take(MESSAGE_MAX).collect();
                format!(
                    "{storm}. Last loss: reason={} cause={} message={message}",
                    note.reason,
                    note.cause.word()
                )
            }
            None => format!("{storm}."),
        }
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

    fn note(cause: Cause) -> LossNote {
        LossNote {
            reason: "unknown".into(),
            message: "the GPU process crashed".into(),
            cause,
        }
    }

    #[test]
    fn a_forced_loss_and_a_browser_loss_start_over_and_leave_a_note() {
        let mut b = LossBudget::default();
        assert_eq!(b.judge(1_000.0, note(Cause::Forced)), Verdict::Retry);
        assert_eq!(b.total(), 1);
        let last = b.last().expect("the loss is noted");
        assert_eq!(
            (last.reason.as_str(), last.cause),
            ("unknown", Cause::Forced)
        );
        assert_eq!(b.judge(30_000.0, note(Cause::Browser)), Verdict::Retry);
        assert_eq!(b.total(), 2);
        assert_eq!(b.last().map(|n| n.cause), Some(Cause::Browser));
    }

    #[test]
    fn a_requested_destroy_is_not_a_loss_and_is_never_retried() {
        let mut b = LossBudget::default();
        let destroyed = LossNote {
            reason: "destroyed".into(),
            message: String::new(),
            cause: Cause::Requested,
        };
        assert_eq!(b.judge(1_000.0, destroyed.clone()), Verdict::Stop);
        assert_eq!((b.total(), b.last()), (0, None), "it leaves no trace");
        // It does not eat into the storm budget either.
        for t in [1_100.0, 1_200.0] {
            assert_eq!(b.judge(t, note(Cause::Browser)), Verdict::Retry);
            assert_eq!(b.judge(t, destroyed.clone()), Verdict::Stop);
        }
        assert_eq!(b.judge(1_300.0, note(Cause::Browser)), Verdict::GiveUp);
    }

    #[test]
    fn three_losses_in_ten_seconds_give_up_and_the_fatal_text_names_the_reason() {
        let mut b = LossBudget::default();
        assert_eq!(b.judge(0.0, note(Cause::Browser)), Verdict::Retry);
        assert_eq!(b.judge(4_000.0, note(Cause::Browser)), Verdict::Retry);
        let last = LossNote {
            reason: "unknown".into(),
            message: "driver reset".into(),
            cause: Cause::Browser,
        };
        assert_eq!(b.judge(9_000.0, last), Verdict::GiveUp);
        let text = b.give_up_detail();
        assert!(
            text.contains("lost 3 times within 10 s")
                && text.contains("reason=unknown cause=browser message=driver reset"),
            "{text}"
        );
        assert_eq!(b.total(), 3);
    }

    #[test]
    fn a_long_message_is_cut_for_the_fatal_page() {
        let mut b = LossBudget::default();
        b.judge(
            0.0,
            LossNote {
                reason: "unknown".into(),
                message: "x".repeat(5_000),
                cause: Cause::Browser,
            },
        );
        assert!(b.give_up_detail().len() < 400);
        assert!(LossBudget::new().give_up_detail().ends_with('.'));
    }

    #[test]
    fn the_lost_box_keeps_the_first_report_and_the_words_are_webgpus() {
        let lost = LostBox::default();
        assert_eq!(lost.get(), None);
        lost.set("destroyed", "first");
        lost.set("unknown", "second");
        assert_eq!(lost.get(), Some(("destroyed".into(), "first".into())));
        assert_eq!(reason_word(wgpu::DeviceLostReason::Unknown), "unknown");
        assert_eq!(reason_word(wgpu::DeviceLostReason::Destroyed), "destroyed");
        let words = [Cause::Browser, Cause::Forced, Cause::Requested].map(Cause::word);
        assert_eq!(words, ["browser", "forced", "requested"]);
    }
}
