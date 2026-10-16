//! Tick-length owed-histogram vectors separating stall shapes.
//!
//! Consumer: observe door tick pacing diagnostics (stall shape triage).
//! Data owner: this test file owns its local TickStats.
//! Units: nanoseconds (u64), tick-lengths (u64), histogram counts (u64).
//! Determinism scope: fixed 600-tick loops, integer division, Vec order;
//! no wall clock, no Instant, no hash input. Errors: none (inputs saturate).
//!
//! PORT(lichen79/sfemu:src/pace.rs) MIT, pattern only: idea of TickStats
//! with an owed histogram, worst_ns and drop events reimplemented fresh
//! here with elapsed_ns as an argument so the tick path takes no clock.
//! No donor bytes copied. License: MIT.
//!
//! Budget: one Vec<u64> of MAX_OWED + 1 buckets per tracker; no
//! per-tick allocation (buckets reused, slices in place).

/// Nominal tick length: one tick-length owed per 50 ms of elapsed time.
const TICK_NS: u64 = 50_000_000;
/// Largest owed value with its own bucket; larger values clamp here.
const MAX_OWED: usize = 64;
/// Nominal per-tick elapsed for healthy ticks (5 ms < TICK_NS, owes 0).
const NOMINAL_NS: u64 = 5_000_000;
/// Single long stall: 2 000 ms owes 40 tick-lengths.
const LONG_STALL_NS: u64 = 2_000_000_000;
/// Repeated short stall: 50 ms owes exactly 1 tick-length.
const SHORT_STALL_NS: u64 = 50_000_000;
/// Fixed loop length for both stall-shape fixtures.
const FIXTURE_TICKS: usize = 600;

/// Fresh std-only tick tracker: owed histogram plus worst-case and drops.
///
/// owed[n] counts ticks that owed n tick-lengths (owed[MAX_OWED]
/// also absorbs any larger overrun). worst_ns is the largest elapsed_ns
/// seen. drop_events counts ticks with at least one owed tick-length.
#[derive(Debug, Clone)]
struct TickStats {
    owed: Vec<u64>,
    worst_ns: u64,
    drop_events: u64,
}

impl TickStats {
    /// Empty tracker with zeroed buckets.
    #[must_use]
    fn new() -> Self {
        Self {
            owed: vec![0_u64; MAX_OWED + 1],
            worst_ns: 0,
            drop_events: 0,
        }
    }

    /// Record one tick that took elapsed_ns (caller-measured, no clock
    /// inside). Returns tick-lengths owed (elapsed_ns / TICK_NS).
    fn tick(&mut self, elapsed_ns: u64) -> u64 {
        let owed_ticks = elapsed_ns / TICK_NS;
        self.worst_ns = self.worst_ns.max(elapsed_ns);
        let bucket = usize::try_from(owed_ticks)
            .unwrap_or(usize::MAX)
            .min(MAX_OWED);
        self.owed[bucket] = self.owed[bucket].saturating_add(1);
        if owed_ticks > 0 {
            self.drop_events = self.drop_events.saturating_add(1);
        }
        owed_ticks
    }

    /// Total owed tick-lengths across all buckets (saturating sum).
    #[must_use]
    fn dropped_total(&self) -> u64 {
        let mut total: u64 = 0;
        for (index, count) in self.owed.iter().enumerate() {
            let width = u64::try_from(index).unwrap_or(u64::MAX);
            total = total.saturating_add(width.saturating_mul(*count));
        }
        total
    }
}

/// 600-tick run with one stall at tick 0, nominal elsewhere.
fn run_single_long_stall() -> TickStats {
    let mut stats = TickStats::new();
    for tick in 0..FIXTURE_TICKS {
        let elapsed_ns = if tick == 0 { LONG_STALL_NS } else { NOMINAL_NS };
        stats.tick(elapsed_ns);
    }
    stats
}

/// 600-tick run with forty 50 ms stalls at ticks 0..40, nominal elsewhere.
fn run_many_short_stalls() -> TickStats {
    let mut stats = TickStats::new();
    for tick in 0..FIXTURE_TICKS {
        let elapsed_ns = if tick < 40 {
            SHORT_STALL_NS
        } else {
            NOMINAL_NS
        };
        stats.tick(elapsed_ns);
    }
    stats
}

#[test]
fn single_long_stall_vs_many_small_same_dropped_total_different_histogram() {
    let single = run_single_long_stall();
    let many = run_many_short_stalls();
    // Same dropped total: 1 x 40 == 40 x 1.
    assert_eq!(single.dropped_total(), 40_u64);
    assert_eq!(many.dropped_total(), 40_u64);
    // Different histogram shapes separate the stall causes.
    assert_eq!(single.owed[40], 1_u64);
    assert_eq!(many.owed[40], 0_u64);
    assert_eq!(single.owed[1], 0_u64);
    assert_eq!(many.owed[1], 40_u64);
    assert_ne!(single.owed, many.owed);
    // Nominal buckets mirror the stall counts over the fixed 600 ticks.
    assert_eq!(single.owed[0], 599_u64);
    assert_eq!(many.owed[0], 560_u64);
    // Worst case and event counts also separate the shapes.
    assert_eq!(single.worst_ns, LONG_STALL_NS);
    assert_eq!(many.worst_ns, SHORT_STALL_NS);
    assert_eq!(single.drop_events, 1_u64);
    assert_eq!(many.drop_events, 40_u64);
}

#[test]
fn worst_ns_tracks_max_elapsed_without_clock() {
    let mut stats = TickStats::new();
    assert_eq!(stats.worst_ns, 0_u64);
    stats.tick(0_u64);
    assert_eq!(stats.worst_ns, 0_u64);
    stats.tick(NOMINAL_NS);
    assert_eq!(stats.worst_ns, NOMINAL_NS);
    stats.tick(LONG_STALL_NS);
    assert_eq!(stats.worst_ns, LONG_STALL_NS);
    // A later small tick never lowers the worst case.
    stats.tick(NOMINAL_NS);
    assert_eq!(stats.worst_ns, LONG_STALL_NS);
    // Same inputs twice give the same worst case (no hidden clock).
    let mut replay = TickStats::new();
    for elapsed_ns in [0_u64, NOMINAL_NS, LONG_STALL_NS, NOMINAL_NS] {
        replay.tick(elapsed_ns);
    }
    assert_eq!(replay.worst_ns, stats.worst_ns);
    assert_eq!(replay.owed, stats.owed);
}

#[test]
fn drop_events_counts_only_stall_ticks() {
    let mut stats = TickStats::new();
    assert_eq!(stats.drop_events, 0_u64);
    // Nominal ticks owe nothing and raise no drop event.
    for _ in 0..10 {
        let owed = stats.tick(NOMINAL_NS);
        assert_eq!(owed, 0_u64);
    }
    assert_eq!(stats.drop_events, 0_u64);
    assert_eq!(stats.owed[0], 10_u64);
    // Each stall tick raises exactly one drop event.
    assert_eq!(stats.tick(SHORT_STALL_NS), 1_u64);
    assert_eq!(stats.drop_events, 1_u64);
    assert_eq!(stats.tick(LONG_STALL_NS), 40_u64);
    assert_eq!(stats.drop_events, 2_u64);
    assert_eq!(stats.dropped_total(), 41_u64);
}
