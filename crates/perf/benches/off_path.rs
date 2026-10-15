//! Off-path bench for steal P6 (EmbarkStudios/puffin@c81e9bb, MIT).
//!
//! What it proves: the disabled profiler gate costs <=2 ns/op (one relaxed
//! atomic load plus a branch, no clock/scope/sample), the canonical
//! ArenaTick 16-slot 600-tick hash repeats x2 with the profiler off, and the
//! off path adds zero allocations.
//!
//! Run: `cargo bench -p engine-perf --bench off_path` (release).
//! Kill: any golden drift, any alloc when off, or off-path over 10 ns/op;
//! target: off-path <=2 ns/op.

use std::hint::black_box;
use std::time::Instant;

use engine_perf::alloc::measure;
use engine_perf::profiler::{self, gate_on};
use engine_perf::workloads::{AiDecide, ArenaTick};

/// Iterations per timing repeat: large enough that the QPC granularity
/// (~100 ns) vanishes into the mean (10M ops x ~0.5 ns = ~5 ms per repeat).
const N: u64 = 10_000_000;
/// Repeats per side; the minimum wins (least scheduling noise).
const REPEATS: usize = 7;
/// Target bar (steal P6 accept) and hard kill (steal row kill line).
const TARGET_NS_PER_OP: f64 = 2.0;
const KILL_NS_PER_OP: f64 = 10.0;

fn timed_loop(mut work: impl FnMut()) -> f64 {
    let mut best = f64::INFINITY;
    for _ in 0..REPEATS {
        work(); // warm one full pass outside the clock where cheap
        let started = Instant::now();
        work();
        let ns = started.elapsed().as_nanos() as f64;
        best = best.min(ns / N as f64);
    }
    best
}

fn main() {
    profiler::set_enabled(false);

    // Golden held: canonical 16-slot 600-tick arena hash repeats x2 with the
    // profiler off and matches the recorded baseline row.
    let first = ArenaTick::canonical().run_hash();
    let second = ArenaTick::canonical().run_hash();
    assert_eq!(first, second, "arena hash repeats x2 with profiler off");
    assert_eq!(
        first,
        engine_perf::baselines::arena_baseline().hash,
        "off-gate hash matches the recorded baseline"
    );

    // Zero allocs when off: the disabled wrapper adds no counting work of
    // its own beyond the workload's own allocation.
    let (_, raw_alloc) = measure(|| AiDecide::canonical().run_hash());
    let (_, wrapped_alloc) = measure(profiler::profiled_ai_decide);
    assert_eq!(
        raw_alloc, wrapped_alloc,
        "disabled wrapper adds zero allocs"
    );

    // Off-path cost: relaxed atomic load plus a branch. Baseline loop pays
    // only the loop plus black_box; the gate loop adds is_enabled().
    let base_ns = timed_loop(|| {
        for _ in 0..N {
            black_box(0u8);
        }
    });
    let gate_ns = timed_loop(|| {
        for _ in 0..N {
            black_box(profiler::is_enabled());
        }
    });
    let cond_ns = timed_loop(|| {
        for _ in 0..N {
            black_box(gate_on(black_box(true)));
        }
    });
    let net_gate = (gate_ns - base_ns).max(0.0);
    let net_cond = (cond_ns - base_ns).max(0.0);
    let off_path = net_gate.max(net_cond);
    println!("off_path base={base_ns:.4}ns/op gate={gate_ns:.4}ns/op cond={cond_ns:.4}ns/op net_gate={net_gate:.4}ns/op net_cond={net_cond:.4}ns/op off_path={off_path:.4}ns/op target={TARGET_NS_PER_OP}ns/op");
    assert!(
        off_path <= KILL_NS_PER_OP,
        "off-path {off_path:.4}ns/op over kill {KILL_NS_PER_OP}ns/op"
    );
    assert!(
        off_path <= TARGET_NS_PER_OP,
        "off-path {off_path:.4}ns/op over target {TARGET_NS_PER_OP}ns/op"
    );
    println!("RESULT: off_path={off_path:.4}ns/op golden=x2 alloc=+0");
}
