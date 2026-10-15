//! Steady-state allocation proofs. Physics steps and AI decisions allocate
//! nothing after warm-up; the instrument-sanity test proves a blind counter
//! could not fake these zeros.

use engine_perf::alloc::measure;
use engine_perf::arena::FrameTransients;
use engine_perf::workloads::{AiDecide, PhysicsStep, RenderCull};

#[test]
fn counting_sees_a_real_allocation() {
    // black_box keeps the optimizer from eliding the sink in release.
    let ((), observed) = measure(|| {
        std::hint::black_box(vec![0u8; 64]);
    });
    assert_eq!(observed.count, 1, "counter observes one alloc");
    assert_eq!(observed.bytes, 64, "counter observes 64 bytes");
}

#[test]
fn physics_step_is_steady_state_zero() {
    let workload = PhysicsStep::canonical();
    let (first, snap_first) = workload.steady(600);
    let (second, snap_second) = workload.steady(600);
    assert_eq!(first, second, "steady slices hash identically");
    for (label, snap) in [("first", snap_first), ("second", snap_second)] {
        assert_eq!(snap.bytes, 0, "{label} slice allocates no bytes");
        assert_eq!(snap.count, 0, "{label} slice makes no allocations");
    }
}

#[test]
fn ai_decide_is_steady_state_zero() {
    let workload = AiDecide::canonical();
    let (first, snap_first) = workload.steady(600);
    let (second, snap_second) = workload.steady(600);
    assert_eq!(first, second, "steady slices hash identically");
    for (label, snap) in [("first", snap_first), ("second", snap_second)] {
        assert_eq!(snap.bytes, 0, "{label} slice allocates no bytes");
        assert_eq!(snap.count, 0, "{label} slice makes no allocations");
    }
}

#[test]
fn frame_transients_are_steady_state_zero() {
    // Acceptance: per-frame transient scratch (render cull keys, anim
    // pose words, audio mix words) over a presized bump arena allocates
    // nothing in the steady state and repeats exactly.
    let workload = FrameTransients::canonical();
    let (first, snap_first) = workload.steady().expect("presized frame scratch fits");
    let (second, snap_second) = workload.steady().expect("repeat run fits");
    assert_eq!(first, second, "steady frames hash identically");
    for (label, snap) in [("first", snap_first), ("second", snap_second)] {
        assert_eq!(snap.bytes, 0, "{label} run allocates no bytes");
        assert_eq!(snap.count, 0, "{label} run makes no allocations");
    }
}

#[test]
fn render_cull_stages_arena_keys_with_no_steady_alloc() {
    // Call-site proof: the draw-order consumer stages arena-backed cull
    // keys into the real `ForwardPlus` draw list. Presized once and warmed,
    // steady frames allocate nothing and repeat exactly.
    let workload = RenderCull::canonical();
    let (first, snap_first) = workload.steady().expect("presized cull scratch fits");
    let (second, snap_second) = workload.steady().expect("repeat run fits");
    assert_eq!(first, second, "steady culls hash identically");
    for (label, snap) in [("first", snap_first), ("second", snap_second)] {
        assert_eq!(snap.bytes, 0, "{label} run allocates no bytes");
        assert_eq!(snap.count, 0, "{label} run makes no allocations");
    }
}
