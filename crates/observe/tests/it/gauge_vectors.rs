// Donor: rcrowley/go-metrics BSD-2-Clause https://github.com/rcrowley/go-metrics
// (gauge.go idea, fresh Rust; no donor bytes copied).
//
// Consumer: queue.depth gauge for the observe door (bounded depth signal
// over a fixed 600-tick loop). Data owner: this test file owns its local
// gauge. Units: depth (i64), ticks (u64). Determinism scope: fixed loop
// bounds, tick % 8 sequence; no wall clock, no hash input.
const QUEUE_DEPTH: &str = "queue.depth";

/// Last-value gauge: update overwrites, value reads back.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Gauge {
    value: i64,
}

impl Gauge {
    fn new() -> Self {
        Self { value: 0 }
    }

    fn update(&mut self, value: i64) {
        self.value = value;
    }

    #[must_use]
    fn value(&self) -> i64 {
        self.value
    }

    #[must_use]
    fn snapshot(&self) -> GaugeSnapshot {
        GaugeSnapshot { value: self.value }
    }
}

/// Frozen copy: later update calls never change this value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GaugeSnapshot {
    value: i64,
}

impl GaugeSnapshot {
    #[must_use]
    fn value(&self) -> i64 {
        self.value
    }
}

const TICKS: u64 = 600;

fn depth_at(tick: u64) -> i64 {
    i64::try_from(tick % 8).expect("tick % 8 fits in i64")
}

fn run_queue_depth(gauge: &mut Gauge) {
    debug_assert_eq!(QUEUE_DEPTH, "queue.depth");
    for tick in 0..TICKS {
        gauge.update(depth_at(tick));
    }
}

#[test]
fn queue_depth_last_value_is_7_at_tick_599() {
    let mut gauge = Gauge::new();
    assert_eq!(gauge.value(), 0);
    run_queue_depth(&mut gauge);
    // 599 % 8 == 7, so the last update leaves 7.
    assert_eq!(depth_at(599), 7);
    assert_eq!(gauge.value(), 7, "queue.depth last value must be 7");
    // Spot-check the overwrite sequence, not an accumulation.
    let mut probe = Gauge::new();
    probe.update(depth_at(0));
    assert_eq!(probe.value(), 0);
    probe.update(depth_at(7));
    assert_eq!(probe.value(), 7);
    probe.update(depth_at(8));
    assert_eq!(probe.value(), 0);
}

#[test]
fn snapshot_is_frozen_after_update_zero() {
    let mut gauge = Gauge::new();
    run_queue_depth(&mut gauge);
    let frozen = gauge.snapshot();
    assert_eq!(frozen.value(), 7);
    gauge.update(0);
    assert_eq!(gauge.value(), 0, "live gauge follows the latest update");
    assert_eq!(frozen.value(), 7, "snapshot stays frozen after Update(0)");
}

#[test]
fn update_overwrites_instead_of_accumulating() {
    let mut gauge = Gauge::new();
    gauge.update(5);
    assert_eq!(gauge.value(), 5);
    gauge.update(3);
    assert_eq!(gauge.value(), 3, "gauge sets, never adds");
    gauge.update(0);
    assert_eq!(gauge.value(), 0);
    let frozen = gauge.snapshot();
    gauge.update(7);
    assert_eq!(gauge.value(), 7);
    assert_eq!(frozen.value(), 0, "earlier snapshot keeps its value");
}
