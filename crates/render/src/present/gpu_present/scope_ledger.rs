//! scope_ledger: validation-scope answers that arrive after the frame that asked.
//!
//! A browser answers `popErrorScope` from its event loop, so a frame can neither block on the
//! answer (the loop that would deliver it never gets to run) nor learn it before it returns.
//! The frame hands the answer here instead; the next frames poll it without waiting. Native
//! devices answer at once and keep the blocking pop in `submit::pop_scope`.
//!
//! Units: counts of answers and errors. Determinism: evidence only, never hashed.
//! Caller: `Presenter::pop_scope` (browser build) and `Presenter::settle_scopes`.
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

/// One scope's answer: the first validation error as text, or `None` when the scope was clean.
pub(crate) type ScopeAnswer = Pin<Box<dyn Future<Output = Option<String>>>>;

/// Most answers waiting at once. A device that stops answering (lost) cannot grow the list: the
/// oldest waits are dropped and counted.
pub(crate) const MAX_PENDING: usize = 64;

/// Answers still on their way, plus what the finished ones said.
#[derive(Default)]
pub(crate) struct ScopeLedger {
    pending: Vec<ScopeAnswer>,
    errors: u64,
    first: Option<String>,
    dropped: u64,
}

impl ScopeLedger {
    /// Wait for `answer` without blocking. Past [`MAX_PENDING`] the oldest wait is dropped.
    pub(crate) fn defer(&mut self, answer: ScopeAnswer) {
        if self.pending.len() >= MAX_PENDING {
            drop(self.pending.remove(0));
            self.dropped += 1;
        }
        self.pending.push(answer);
    }

    /// Poll every waiting answer once; finished ones leave the list and are counted.
    pub(crate) fn settle(&mut self) {
        let mut cx = Context::from_waker(Waker::noop());
        let mut still = Vec::with_capacity(self.pending.len());
        for mut answer in self.pending.drain(..) {
            match answer.as_mut().poll(&mut cx) {
                Poll::Ready(None) => {}
                Poll::Ready(Some(text)) => {
                    self.errors += 1;
                    self.first.get_or_insert(text);
                }
                Poll::Pending => still.push(answer),
            }
        }
        self.pending = still;
    }

    /// Validation errors reported by the answers that have arrived.
    pub(crate) fn errors(&self) -> u64 {
        self.errors
    }

    /// The first error text that arrived, kept for the page's receipt.
    pub(crate) fn first_error(&self) -> Option<&str> {
        self.first.as_deref()
    }

    /// Answers still waiting.
    pub(crate) fn pending(&self) -> usize {
        self.pending.len()
    }

    /// Waits dropped because the list was full.
    pub(crate) fn dropped(&self) -> u64 {
        self.dropped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    /// A future that stays pending until its shared flag flips, then yields `result`.
    struct After {
        ready: Rc<Cell<bool>>,
        result: Option<String>,
    }

    impl Future for After {
        type Output = Option<String>;
        fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            if self.ready.get() {
                Poll::Ready(self.result.take())
            } else {
                Poll::Pending
            }
        }
    }

    fn after(result: Option<&str>) -> (Rc<Cell<bool>>, ScopeAnswer) {
        let ready = Rc::new(Cell::new(false));
        let answer = After {
            ready: Rc::clone(&ready),
            result: result.map(str::to_owned),
        };
        (ready, Box::pin(answer))
    }

    #[test]
    fn an_answer_is_counted_only_once_it_arrives() {
        let mut ledger = ScopeLedger::default();
        let (ready, answer) = after(Some("bad bind group"));
        ledger.defer(answer);
        ledger.settle();
        assert_eq!(ledger.errors(), 0, "nothing arrived yet");
        assert_eq!(ledger.pending(), 1, "the wait is kept, not blocked on");
        ready.set(true);
        ledger.settle();
        assert_eq!(ledger.errors(), 1);
        assert_eq!(ledger.first_error(), Some("bad bind group"));
        assert_eq!(ledger.pending(), 0);
        ledger.settle();
        assert_eq!(ledger.errors(), 1, "an answer counts once");
    }

    #[test]
    fn a_clean_scope_leaves_no_error() {
        let mut ledger = ScopeLedger::default();
        let (ready, answer) = after(None);
        ledger.defer(answer);
        ready.set(true);
        ledger.settle();
        assert_eq!(ledger.errors(), 0);
        assert_eq!(ledger.first_error(), None);
        assert_eq!(ledger.pending(), 0);
    }

    #[test]
    fn the_first_error_text_is_kept_and_later_ones_only_count() {
        let mut ledger = ScopeLedger::default();
        for text in ["first", "second", "third"] {
            let (ready, answer) = after(Some(text));
            ready.set(true);
            ledger.defer(answer);
        }
        ledger.settle();
        assert_eq!(ledger.errors(), 3);
        assert_eq!(ledger.first_error(), Some("first"));
    }

    #[test]
    fn a_device_that_never_answers_cannot_grow_the_list() {
        let mut ledger = ScopeLedger::default();
        let mut gates = Vec::new();
        for _ in 0..MAX_PENDING + 5 {
            let (ready, answer) = after(Some("late"));
            gates.push(ready);
            ledger.defer(answer);
        }
        assert_eq!(ledger.pending(), MAX_PENDING, "the list stays capped");
        assert_eq!(ledger.dropped(), 5, "the overflow is counted, not hidden");
        for ready in &gates {
            ready.set(true);
        }
        ledger.settle();
        assert_eq!(
            ledger.errors(),
            MAX_PENDING as u64,
            "only the kept waits answer"
        );
    }
}
