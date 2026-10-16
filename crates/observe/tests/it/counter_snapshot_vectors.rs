// Donor: rcrowley/go-metrics BSD-2-Clause, counter.go Inc/Dec/Clear/Snapshot idea.
// Link: https://github.com/rcrowley/go-metrics
// Fresh Rust write: no donor bytes copied. Diagnostics only; never enters any simulation hash.
//
// Consumer: observe tick counters (600-tick dec fixture plus frozen snapshot check).
// Data owner: this test file owns its local counter. Units: counts (i64),
// ticks (u64). Determinism scope: fixed 600-tick loop, no wall clock,
// no hash input. Errors: none; overflow saturates instead of wrapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct GoCounter {
    value: i64,
}

impl GoCounter {
    fn new() -> Self {
        Self { value: 0 }
    }

    fn inc(&mut self, delta: i64) {
        self.value = self.value.saturating_add(delta);
    }

    fn dec(&mut self, delta: i64) {
        self.value = self.value.saturating_sub(delta);
    }

    fn clear(&mut self) {
        self.value = 0;
    }

    fn count(&self) -> i64 {
        self.value
    }

    fn snapshot(&self) -> CounterSnapshot {
        CounterSnapshot { value: self.value }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CounterSnapshot {
    value: i64,
}

impl CounterSnapshot {
    fn value(&self) -> i64 {
        self.value
    }
}

const TICKS: u64 = 600;

fn run_600_dec_ticks(counter: &mut GoCounter) {
    for _ in 0..TICKS {
        counter.dec(1);
    }
}

#[test]
fn dec_per_tick_over_600_ticks_exact() {
    let mut counter = GoCounter::new();
    assert_eq!(counter.count(), 0);
    run_600_dec_ticks(&mut counter);
    assert_eq!(counter.count(), -600, "one dec per tick over 600 ticks");
    counter.dec(1);
    assert_eq!(counter.count(), -601);
    counter.inc(1);
    assert_eq!(counter.count(), -600);
}

#[test]
fn snapshot_at_600_frozen_after_clear() {
    let mut counter = GoCounter::new();
    run_600_dec_ticks(&mut counter);
    let frozen = counter.snapshot();
    assert_eq!(frozen.value(), -600, "snapshot at tick 600");
    counter.clear();
    assert_eq!(counter.count(), 0, "clear resets the live counter");
    assert_eq!(frozen.value(), -600, "snapshot stays frozen after clear");
    counter.inc(5);
    counter.dec(2);
    assert_eq!(counter.count(), 3);
    assert_eq!(frozen.value(), -600, "frozen copy is a value, not an alias");
}

#[test]
fn inc_dec_clear_counts_match_counter_semantics() {
    let mut counter = GoCounter::new();
    counter.inc(10);
    assert_eq!(counter.count(), 10);
    counter.dec(3);
    assert_eq!(counter.count(), 7);
    let snap = counter.snapshot();
    assert_eq!(snap.value(), 7);
    counter.clear();
    assert_eq!(counter.count(), 0);
    assert_eq!(snap.value(), 7, "clear must not touch taken snapshots");
    counter.inc(2);
    counter.dec(2);
    assert_eq!(counter.count(), 0);
}

#[test]
fn snapshot_isolated_and_saturates_on_overflow() {
    let mut counter = GoCounter::new();
    counter.inc(i64::MAX);
    assert_eq!(counter.count(), i64::MAX);
    counter.inc(1);
    assert_eq!(counter.count(), i64::MAX, "overflow saturates, never wraps");
    let top = counter.snapshot();
    counter.clear();
    assert_eq!(counter.count(), 0);
    assert_eq!(top.value(), i64::MAX, "snapshot keeps pre-clear value");
    counter.dec(i64::MAX);
    counter.dec(1);
    assert_eq!(counter.count(), i64::MIN, "underflow saturates at MIN");
    counter.dec(1);
    assert_eq!(counter.count(), i64::MIN, "further dec stays at MIN");
}
