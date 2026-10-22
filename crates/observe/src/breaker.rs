//! Circuit breaker and hedge budget over the [`supervisor`](crate::supervisor):
//! a rolling error-percent gate against retry storms and a per-code extra-load
//! cap against late primaries.
//!
//! Consumer: the supervisor (`enable_breaker`, `enable_hedge`) and the
//! provider, net and door retry paths that report outcomes; the vector tests
//! in `tests/it/circuit_vectors.rs` run against this module.
//! Data owner: the caller owns one breaker per fault code; this module owns
//! the window and the position.
//! Units: calls (`u64`), error percent (`u32`), latency and thresholds (`u64`
//! ms), counts (`u64`).
//! Clock: the caller's revision. A revision is a tick, a journal revision or
//! any other monotonic count the caller owns (`observe_at`); [`RollingBreaker::observe`]
//! uses the call index as its revision. No wall clock is read here, so the
//! same outcomes at the same revisions give the same state on every machine.
//! Std-only, integer, no float.
//! Errors: [`BreakerError`]; a breaker reject stops retries, it never panics.

use std::fmt;

/// Calls in one rolling breaker window before its error percent is judged.
pub const BREAKER_WINDOW_CALLS: usize = 10;

/// Error percent at or above which a full window opens the breaker.
pub const BREAKER_ERROR_PERCENT: u32 = 50;

/// Maximum half-open probe calls admitted per cool-down expiry.
pub const HALF_OPEN_MAX_CALLS: u32 = 1;

/// Breaker position. Closed admits retries, open rejects them fast,
/// half-open admits one recovery probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BreakerState {
    /// Retries flow; outcomes fill the rolling window.
    Closed,
    /// Retry storm guard: retries stop here without touching the budget.
    Open,
    /// One probe is admitted to test recovery.
    HalfOpen,
}

/// Typed breaker rejection: open means stop retrying, not try again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BreakerError {
    /// Breaker is open, or its one probe is already spent.
    Open,
}

impl fmt::Display for BreakerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open => write!(f, "circuit is open"),
        }
    }
}

/// Rolling error-percent breaker: opens at 50% or more errors over the last
/// 10 calls, then one half-open probe closes it on success or reopens it.
/// No clock is read: `opened_at` is the caller's revision, so the same
/// outcomes at the same revisions give the same state.
#[derive(Debug, Clone)]
pub struct RollingBreaker {
    /// Ring of the last [`BREAKER_WINDOW_CALLS`] outcomes (`true` is an error).
    window: [bool; BREAKER_WINDOW_CALLS],
    /// Next ring slot to overwrite.
    next: usize,
    /// Outcomes recorded in the current window (caps at the window size).
    filled: usize,
    /// Total calls observed (1-based, saturating).
    attempt: u64,
    /// Position of the breaker.
    state: BreakerState,
    /// Revision where the breaker last opened (`None` if never).
    opened_at: Option<u64>,
    /// Probe calls consumed while half-open (caps at [`HALF_OPEN_MAX_CALLS`]).
    probes: u32,
}

impl RollingBreaker {
    /// Closed breaker with an empty window.
    pub fn new() -> Self {
        Self {
            window: [false; BREAKER_WINDOW_CALLS],
            next: 0,
            filled: 0,
            attempt: 0,
            state: BreakerState::Closed,
            opened_at: None,
            probes: 0,
        }
    }

    /// Total calls observed so far.
    #[must_use]
    pub fn attempt(&self) -> u64 {
        self.attempt
    }

    /// True while new retries must reject fast.
    pub fn is_open(&self) -> bool {
        self.state == BreakerState::Open
    }

    /// True while the single recovery probe is admitted.
    pub fn is_half_open(&self) -> bool {
        self.state == BreakerState::HalfOpen
    }

    /// Revision where the breaker last opened, if it ever did.
    pub fn opened_at(&self) -> Option<u64> {
        self.opened_at
    }

    /// Gate one retry: `Ok` admits it, `Err` stops it without using window
    /// budget. Half-open admits here; `half_open_probe` is what spends the
    /// single probe.
    pub fn admit(&self) -> Result<(), BreakerError> {
        match self.state {
            BreakerState::Closed | BreakerState::HalfOpen => Ok(()),
            BreakerState::Open => Err(BreakerError::Open),
        }
    }

    /// Record one retry outcome (`ok` is success) with the call index as its
    /// revision. A closed window trips at 50% or more errors once full; a
    /// half-open probe closes on success or reopens on failure; an open
    /// breaker ignores outcomes.
    pub fn observe(&mut self, ok: bool) {
        self.observe_at(self.attempt.saturating_add(1), ok);
    }

    /// Record one retry outcome (`ok` is success) at the caller's `revision`.
    /// The revision is the breaker's clock: a tick, a journal revision or any
    /// other monotonic count. It is stamped into [`Self::opened_at`], so the
    /// same outcomes at the same revisions replay identically.
    pub fn observe_at(&mut self, revision: u64, ok: bool) {
        match self.state {
            BreakerState::HalfOpen => self.complete_probe(revision, ok),
            BreakerState::Open => {}
            BreakerState::Closed => {
                self.push(!ok);
                if self.filled == BREAKER_WINDOW_CALLS
                    && self.error_percent() >= BREAKER_ERROR_PERCENT
                {
                    self.state = BreakerState::Open;
                    self.opened_at = Some(revision);
                }
            }
        }
    }

    /// Move an open breaker to half-open and spend its one probe. A closed
    /// breaker admits without moving; a second probe in one half-open period
    /// is refused.
    pub fn half_open_probe(&mut self) -> Result<(), BreakerError> {
        match self.state {
            BreakerState::Closed => Ok(()),
            BreakerState::HalfOpen => {
                if self.probes >= HALF_OPEN_MAX_CALLS {
                    Err(BreakerError::Open)
                } else {
                    self.probes = self.probes.saturating_add(1);
                    Ok(())
                }
            }
            BreakerState::Open => {
                self.state = BreakerState::HalfOpen;
                self.probes = 1;
                Ok(())
            }
        }
    }

    /// Finish the half-open probe: success closes and clears the window,
    /// failure reopens at once at `revision`.
    fn complete_probe(&mut self, revision: u64, ok: bool) {
        if ok {
            self.state = BreakerState::Closed;
            self.window = [false; BREAKER_WINDOW_CALLS];
            self.next = 0;
            self.filled = 0;
            self.probes = 0;
        } else {
            self.state = BreakerState::Open;
            self.attempt = self.attempt.saturating_add(1);
            self.opened_at = Some(revision);
            self.probes = 0;
        }
    }

    /// Push one error flag into the ring and count the call.
    fn push(&mut self, is_error: bool) {
        self.window[self.next] = is_error;
        self.next = (self.next + 1) % BREAKER_WINDOW_CALLS;
        if self.filled < BREAKER_WINDOW_CALLS {
            self.filled = self.filled.saturating_add(1);
        }
        self.attempt = self.attempt.saturating_add(1);
    }

    /// Error percent over the filled window (0 while empty).
    pub fn error_percent(&self) -> u32 {
        if self.filled == 0 {
            return 0;
        }
        let errors =
            u32::try_from(self.window.iter().filter(|hit| **hit).count()).unwrap_or(u32::MAX);
        let filled = u32::try_from(self.filled).unwrap_or(u32::MAX);
        errors.saturating_mul(100) / filled.max(1)
    }
}

impl Default for RollingBreaker {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-code hedge budget: a retry that faults past `hedge_after_ms` may run
/// one backup at once, while backups stay within `max_extra_percent` of the
/// reported primaries. Units: ms (`u64`), counts (`u64`), percent (`u32`).
/// Determinism: integer counts only, no clock, same latencies give the same
/// decisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HedgeBudget {
    /// Latency past which a late retry owes a backup, in milliseconds.
    hedge_after_ms: u64,
    /// Extra-load cap as integer percent of reported primaries.
    max_extra_percent: u32,
    /// Reported primary attempts counted against the cap.
    primaries: u64,
    /// Backups spent so far, never above the cap.
    backups: u64,
}

impl HedgeBudget {
    /// Empty budget: no backups spent, cap applies to future reports.
    #[must_use]
    pub fn new(hedge_after_ms: u64, max_extra_percent: u32) -> Self {
        Self {
            hedge_after_ms,
            max_extra_percent,
            primaries: 0,
            backups: 0,
        }
    }

    /// Latency past which a late retry owes a backup, in milliseconds.
    #[must_use]
    pub fn hedge_after_ms(&self) -> u64 {
        self.hedge_after_ms
    }

    /// Extra-load cap as integer percent of reported primaries.
    #[must_use]
    pub fn max_extra_percent(&self) -> u32 {
        self.max_extra_percent
    }

    /// Reported primaries counted against the cap.
    #[must_use]
    pub fn primaries(&self) -> u64 {
        self.primaries
    }

    /// Backups spent so far.
    #[must_use]
    pub fn backups(&self) -> u64 {
        self.backups
    }

    /// Count one reported primary at `latency_ms`; `true` spends one backup
    /// when the sample is past the threshold and the cap still allows it.
    pub fn observe(&mut self, latency_ms: u64) -> bool {
        self.primaries = self.primaries.saturating_add(1);
        let allowed = self
            .primaries
            .saturating_mul(u64::from(self.max_extra_percent))
            / 100;
        let hedge = latency_ms > self.hedge_after_ms && self.backups < allowed;
        if hedge {
            self.backups = self.backups.saturating_add(1);
        }
        hedge
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn breaker_uses_the_caller_revision_as_its_clock() {
        let mut breaker = RollingBreaker::new();
        // Ten timeouts at ticks 100..109: the open revision is the tick.
        for tick in 100..110 {
            breaker.observe_at(tick, false);
        }
        assert!(breaker.is_open());
        assert_eq!(breaker.opened_at(), Some(109));
        assert_eq!(breaker.attempt(), 10);
        assert_eq!(breaker.admit(), Err(BreakerError::Open));

        // Same revisions again: same state and same open revision.
        let mut replay = RollingBreaker::new();
        for tick in 100..110 {
            replay.observe_at(tick, false);
        }
        assert_eq!(replay.opened_at(), breaker.opened_at());
        assert_eq!(replay.error_percent(), breaker.error_percent());
    }

    #[test]
    fn hedge_budget_caps_extra_load_at_percent() {
        let mut budget = HedgeBudget::new(15, 10);
        for _ in 0..9 {
            assert!(!budget.observe(40), "9 of 10 reports allow no backup");
        }
        assert!(budget.observe(40), "the tenth report allows one backup");
        assert_eq!(budget.primaries(), 10);
        assert_eq!(budget.backups(), 1);
        for _ in 0..9 {
            assert!(!budget.observe(40), "cap already spent");
        }
        assert!(budget.observe(40), "the twentieth allows the second");
        assert_eq!(budget.backups(), 2);
        assert_eq!(budget.hedge_after_ms(), 15);
        assert_eq!(budget.max_extra_percent(), 10);
    }
}
