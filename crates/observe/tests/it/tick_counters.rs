// Donor: metrics-rs/metrics, MIT, pattern only (Key = name + labels, pluggable recorder).
// Link: https://github.com/metrics-rs/metrics
// Fresh std-only write: no donor bytes copied. Diagnostics only; never enters any simulation hash.
//
// Consumer: tick-stage counters for the observe door (per-stage counts over a
// fixed 600-tick loop). Data owner: this test file owns its local registry.
// Units: ticks (u64), counts (u64). Determinism scope: BTreeMap order,
// sorted labels, fixed loop bounds; no wall clock, no hash input.
use std::collections::BTreeMap;
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Key {
    name: &'static str,
    labels: Vec<(String, String)>,
}
impl Key {
    fn new(name: &'static str, labels: &[(&str, &str)]) -> Self {
        let mut sorted: Vec<(String, String)> = labels
            .iter()
            .map(|(k, v)| (String::from(*k), String::from(*v)))
            .collect();
        sorted.sort();
        Self {
            name,
            labels: sorted,
        }
    }
}
#[derive(Debug, Default)]
struct CounterRegistry {
    enabled: bool,
    counts: BTreeMap<Key, u64>,
}
impl CounterRegistry {
    fn enabled() -> Self {
        Self {
            enabled: true,
            counts: BTreeMap::new(),
        }
    }
    fn disabled() -> Self {
        Self {
            enabled: false,
            counts: BTreeMap::new(),
        }
    }
    fn increment(&mut self, key: Key, value: u64) {
        if !self.enabled {
            return;
        }
        let slot = self.counts.entry(key).or_insert(0);
        *slot = slot.saturating_add(value);
    }
    fn get(&self, key: &Key) -> u64 {
        self.counts.get(key).copied().unwrap_or(0)
    }
    fn total(&self) -> u64 {
        self.counts.values().copied().sum()
    }
    fn len(&self) -> usize {
        self.counts.len()
    }
    fn is_empty(&self) -> bool {
        self.counts.is_empty()
    }
}
#[derive(Debug, Default)]
struct NoopRecorder;
impl NoopRecorder {
    #[inline(always)]
    fn increment(&mut self, _key: Key, _value: u64) {}
}
fn record_maybe(registry: &mut CounterRegistry, key: Key, value: u64) {
    registry.increment(key, value);
}
fn record_const<const ENABLED: bool>(registry: &mut CounterRegistry, key: Key, value: u64) {
    if ENABLED {
        registry.increment(key, value);
    }
}
const TICKS: u64 = 600;
fn run_600_ticks(registry: &mut CounterRegistry) {
    let tick_total = Key::new("tick.total", &[]);
    let physics = Key::new("stage.count", &[("stage", "physics")]);
    let render = Key::new("stage.count", &[("stage", "render")]);
    for tick in 0..TICKS {
        record_maybe(registry, tick_total.clone(), 1);
        record_maybe(registry, physics.clone(), 1);
        if tick % 2 == 0 {
            record_maybe(registry, render.clone(), 1);
        }
    }
}
#[test]
fn tick_stage_counters_exact_over_600_ticks() {
    let mut registry = CounterRegistry::enabled();
    run_600_ticks(&mut registry);
    let tick_total = Key::new("tick.total", &[]);
    let physics = Key::new("stage.count", &[("stage", "physics")]);
    let render = Key::new("stage.count", &[("stage", "render")]);
    assert_eq!(registry.get(&tick_total), 600, "one total per tick");
    assert_eq!(registry.get(&physics), 600, "physics runs every tick");
    assert_eq!(registry.get(&render), 300, "render runs every 2nd tick");
    assert_eq!(registry.total(), 1500, "600 + 600 + 300");
    assert_eq!(registry.len(), 3, "exactly three keys");
    let ai = Key::new("stage.count", &[("stage", "ai")]);
    assert_eq!(registry.get(&ai), 0);
}
#[test]
fn disabled_path_records_zero() {
    let mut registry = CounterRegistry::disabled();
    run_600_ticks(&mut registry);
    assert!(registry.is_empty(), "disabled registry stores nothing");
    assert_eq!(registry.total(), 0);
    assert_eq!(registry.len(), 0);
    let tick_total = Key::new("tick.total", &[]);
    assert_eq!(registry.get(&tick_total), 0);
    run_600_ticks(&mut registry);
    assert!(registry.is_empty());
    assert_eq!(registry.total(), 0);
    let mut noop = NoopRecorder;
    for tick in 0..TICKS {
        noop.increment(Key::new("tick.total", &[]), 1);
        record_const::<false>(&mut registry, Key::new("tick.total", &[]), 1);
        let _ = tick;
    }
    assert!(registry.is_empty(), "const-disabled path stores nothing");
    assert_eq!(registry.total(), 0);
}
#[test]
fn key_labels_are_order_independent() {
    let a = Key::new("stage.count", &[("stage", "physics"), ("mode", "fixed")]);
    let b = Key::new("stage.count", &[("mode", "fixed"), ("stage", "physics")]);
    assert_eq!(a, b, "label order must not change identity");
    let mut registry = CounterRegistry::enabled();
    registry.increment(a, 2);
    assert_eq!(registry.get(&b), 2);
}
