//! Integration leg: the public tape API runs a small deterministic
//! tape from outside the crate (the consumer fix-row harnesses will
//! call it the same way). Units: meters, ticks, heap-call counts.
//!
//! This binary installs the counting allocator so the probe sampler below
//! observes real heap use; without it the maxima would read zero.
#![cfg(feature = "rapier")]

use engine_core::alloc_probe;
use engine_physics::ragdoll::Ragdoll;
use engine_physics::ragdoll_tape::{
    StepCost, StepSampler, TapeConfig, TapeError, CLINCH_GAP_M, PENETRATION_CAP_M, TAPE_SEED,
};

#[cfg(test)]
#[global_allocator]
static TEST_ALLOC: alloc_probe::CountingAlloc = alloc_probe::CountingAlloc;

/// Counting sampler over the public [`StepSampler`] surface.
#[derive(Default)]
struct Probe {
    max_calls: u64,
    max_bytes: u64,
}

impl StepSampler for Probe {
    fn step(&mut self, ragdoll: &mut Ragdoll, dt: f32) -> Result<StepCost, TapeError> {
        let (result, count) = alloc_probe::measure(|| ragdoll.step(dt));
        result?;
        self.max_calls = self.max_calls.max(count.calls);
        self.max_bytes = self.max_bytes.max(count.bytes);
        Ok(StepCost {
            calls: count.calls,
            bytes: count.bytes,
        })
    }
}

#[test]
fn small_tape_deterministic_and_bounded() {
    let config = TapeConfig::new(TAPE_SEED, 2, 120, 30, 150).expect("valid small tape");
    assert_eq!(config.ticks(), 2 * (120 + 30) + 2 * (150 + 30));
    let mut first = Probe::default();
    let a = engine_physics::ragdoll_tape::run_tape_with(&config, &mut first).expect("tape runs");
    let mut second = Probe::default();
    let b = engine_physics::ragdoll_tape::run_tape_with(&config, &mut second).expect("tape reruns");
    assert_eq!(a.digest, b.digest, "small tape must repeat: {a} vs {b}");
    assert_eq!(a, b);
    assert!(
        (0.0..PENETRATION_CAP_M).contains(&a.interpenetration_max_m),
        "penetration out of bounds: {}",
        a.interpenetration_max_m
    );
    assert!(
        a.separation_min_m >= 0.0 && a.separation_min_m <= CLINCH_GAP_M,
        "separation out of bounds: {}",
        a.separation_min_m
    );
    assert_eq!(a.ticks, config.ticks());
    println!("{a}");
}

#[test]
fn small_tape_rejects_bad_configs() {
    assert_eq!(
        TapeConfig::new(TAPE_SEED, 0, 120, 30, 150),
        Err(TapeError::NoFalls)
    );
    assert_eq!(
        TapeConfig::new(TAPE_SEED, 2, 30, 30, 150),
        Err(TapeError::NoSteps)
    );
}
