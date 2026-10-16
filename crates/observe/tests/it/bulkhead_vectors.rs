// Donor: resilience4j/resilience4j, Apache-2.0,
// https://github.com/resilience4j/resilience4j (resilience4j-bulkhead
// SemaphoreBulkhead maxConcurrentCalls + fast-reject). Idea ported fresh:
// bound concurrent heals, reject the rest without work. No donor bytes.
//! Supervisor bulkhead vectors capping heal storms with fast reject.
//!
//! Owner: engine-observe diagnostics. Units: calls (`usize`), ticks (`u64`).
//! Determinism: fixed 32-fault fixture at one tick, no clock; same storm
//! twice gives the same counts. Errors: [`BulkheadFull`] typed rejection.
use engine_observe::{supervisor::Supervisor, EventCode, RingBuffer};
use std::fmt;

/// Storm size: 32 same-code faults at one tick.
const STORM_CALLS: usize = 32;

/// Tick every storm fault is reported at.
const STORM_TICK: u64 = 7;

/// Bulkhead cap: concurrent heals admitted before fast reject.
const BULKHEAD_MAX: usize = 2;

/// Storm code: budget 1, so the storm must cap after two taped rows.
const STORM_CODE: EventCode = EventCode::SAVE_CORRUPT;

/// Typed bulkhead rejection; full means drop the heal, not queue it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BulkheadFull {
    /// No permit free: the heal storm is capped.
    Full,
}

impl fmt::Display for BulkheadFull {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => write!(f, "bulkhead is full"),
        }
    }
}

impl std::error::Error for BulkheadFull {}

/// Fresh semaphore bulkhead: at most `max_concurrent` heals hold a permit;
/// the rest reject fast with [`BulkheadFull::Full`] and touch no budget.
#[derive(Debug, Clone)]
struct SemaphoreBulkhead {
    /// Permits admitted at once (`maxConcurrentCalls`).
    max_concurrent: usize,
    /// Permits currently held.
    inflight: usize,
    /// Fast rejections so far (saturating).
    rejected: usize,
}

impl SemaphoreBulkhead {
    /// Closed bulkhead with `max_concurrent` permits.
    fn new(max_concurrent: usize) -> Self {
        Self {
            max_concurrent,
            inflight: 0,
            rejected: 0,
        }
    }

    /// Permits currently held.
    fn inflight(&self) -> usize {
        self.inflight
    }

    /// Fast rejections so far.
    fn rejected(&self) -> usize {
        self.rejected
    }

    /// Take one permit or reject fast without work.
    fn try_acquire(&mut self) -> Result<(), BulkheadFull> {
        if self.inflight < self.max_concurrent {
            self.inflight = self.inflight.saturating_add(1);
            Ok(())
        } else {
            self.rejected = self.rejected.saturating_add(1);
            Err(BulkheadFull::Full)
        }
    }

    /// Give one permit back at the tick boundary.
    fn release(&mut self) {
        self.inflight = self.inflight.saturating_sub(1);
    }
}

/// One storm run: (fast_rejected, probes, tape_len).
fn run_storm() -> (usize, usize, usize) {
    let mut supervisor = Supervisor::new();
    let mut ring = RingBuffer::new(64);
    let mut fast_rejected = 0_usize;
    let mut probes = 0_usize;
    for _ in 0..STORM_CALLS {
        let tape_before = supervisor.tape_len();
        let failed_before = ring.query(Some(EventCode::HEAL_FAILED)).len();
        let was_half_open = supervisor.is_half_open(STORM_CODE);
        let decision = supervisor
            .report_fault(STORM_CODE, STORM_TICK, &mut ring)
            .expect("storm fault must return advice");
        if supervisor.is_half_open(STORM_CODE) && !was_half_open && !decision.is_quarantine() {
            probes = probes.saturating_add(1);
        } else if decision.is_quarantine()
            && supervisor.tape_len() == tape_before
            && ring.query(Some(EventCode::HEAL_FAILED)).len() == failed_before
        {
            fast_rejected = fast_rejected.saturating_add(1);
        }
    }
    (fast_rejected, probes, supervisor.tape_len())
}

#[test]
fn storm_of_32_same_tick_fast_rejects_at_least_90_percent() {
    let (fast_rejected, probes, tape_len) = run_storm();
    assert!(
        fast_rejected * 100 >= 90 * STORM_CALLS,
        "storm must fast-reject >=90%: {fast_rejected}/{STORM_CALLS}"
    );
    assert_eq!(fast_rejected, 30);
    assert!(probes <= 1, "storm must admit <=1 probe, got {probes}");
    assert_eq!(probes, 0);
    assert_eq!(tape_len, 2);
    println!("storm fast-reject {fast_rejected}/{STORM_CALLS} probes={probes} passed");
}

#[test]
fn storm_through_semaphore_bulkhead_caps_concurrency() {
    let mut gate = SemaphoreBulkhead::new(BULKHEAD_MAX);
    let mut supervisor = Supervisor::new();
    let mut ring = RingBuffer::new(64);
    let mut held = 0_usize;
    for _ in 0..STORM_CALLS {
        match gate.try_acquire() {
            Ok(()) => {
                supervisor
                    .report_fault(STORM_CODE, STORM_TICK, &mut ring)
                    .expect("admitted heal must return advice");
                held = held.saturating_add(1);
            }
            Err(BulkheadFull::Full) => {}
        }
    }
    let fast_rejected = gate.rejected();
    assert_eq!(gate.inflight(), BULKHEAD_MAX);
    assert_eq!(held, BULKHEAD_MAX);
    assert_eq!(fast_rejected, STORM_CALLS - BULKHEAD_MAX);
    assert!(
        fast_rejected * 100 >= 90 * STORM_CALLS,
        "bulkhead must fast-reject >=90%: {fast_rejected}/{STORM_CALLS}"
    );
    assert!(supervisor.tape_len() <= BULKHEAD_MAX);
    assert_eq!(BulkheadFull::Full.to_string(), "bulkhead is full");
    for _ in 0..held {
        gate.release();
    }
    assert_eq!(gate.inflight(), 0);
    println!("bulkhead cap {BULKHEAD_MAX} fast-reject {fast_rejected}/{STORM_CALLS} passed");
}

#[test]
fn storm_admits_at_most_one_probe_after_cooldown() {
    let mut supervisor = Supervisor::new();
    let mut ring = RingBuffer::new(64);
    for _ in 0..STORM_CALLS {
        supervisor
            .report_fault(STORM_CODE, STORM_TICK, &mut ring)
            .expect("storm fault must return advice");
    }
    assert!(supervisor.is_quarantined(STORM_CODE));
    let until = supervisor
        .quarantined_until(STORM_CODE)
        .expect("storm must quarantine with a cool-down");
    assert_eq!(until, STORM_TICK + 10);
    let probe = supervisor
        .report_fault(STORM_CODE, until, &mut ring)
        .expect("cool-down expiry must admit one probe");
    assert!(!probe.is_quarantine());
    assert_eq!(probe.attempt, 1);
    assert!(supervisor.is_half_open(STORM_CODE));
    let reopen = supervisor
        .report_fault(STORM_CODE, until + 1, &mut ring)
        .expect("failed probe must re-quarantine");
    assert!(reopen.is_quarantine());
    assert!(supervisor.is_quarantined(STORM_CODE));
    assert!(!supervisor.is_half_open(STORM_CODE));
    let tape_before = supervisor.tape_len();
    let failed_before = ring.query(Some(EventCode::HEAL_FAILED)).len();
    let fast = supervisor
        .report_fault(STORM_CODE, until + 2, &mut ring)
        .expect("capped fault must fast-reject");
    assert!(fast.is_quarantine());
    assert_eq!(supervisor.tape_len(), tape_before);
    assert_eq!(
        ring.query(Some(EventCode::HEAL_FAILED)).len(),
        failed_before
    );
    println!("single probe at {until} then fast-reject passed");
}

#[test]
fn same_storm_twice_gives_same_counts() {
    let first = run_storm();
    let second = run_storm();
    assert_eq!(first, second);
    assert_eq!(first.0, 30);
    assert_eq!(first.1, 0);
    assert_eq!(first.2, 2);
    println!("determinism x2 fast-reject 30/32 passed");
}
