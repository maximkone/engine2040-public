//! Release-mode timing proof (`--perf`): per-workload p50 against the
//! recorded millisecond budgets. Ignored in the default suite: wall-clock
//! gates never run beside unit tests. Run release, single-threaded:
//! `cargo test --release -p engine-perf -- --ignored --test-threads=1`.

use engine_perf::baselines::time_budget_ms;
use engine_perf::budget::check_time_ms;
use engine_perf::workloads::{
    AiDecide, ArenaTick, CrowdScale, DestructionStep, PhysicsStep, SpineFullTick,
    SpineTransformBody,
};
use std::time::Instant;

fn percentile(sorted: &mut [f64], p: f64) -> f64 {
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("timings are finite"));
    let end = sorted.len().saturating_sub(1);
    let index = ((p / 100.0) * end as f64).round() as usize;
    sorted[index.min(end)]
}

/// One timed workload: name plus its canonical run as a plain fn pointer.
struct Workload {
    name: &'static str,
    run: fn() -> u64,
}

fn arena_hash() -> u64 {
    ArenaTick::canonical().run_hash()
}

fn physics_hash() -> u64 {
    PhysicsStep::canonical().run_hash()
}

fn ai_hash() -> u64 {
    AiDecide::canonical().run_hash()
}

fn destruction_hash() -> u64 {
    DestructionStep::canonical().run_hash()
}

fn crowd_hash() -> u64 {
    CrowdScale::canonical().run_hash()
}

// The spine bars time the passes alone (`run_steps`); `run_hash` digests
// every pass, which is the equality proof and not the cost the bars name.
fn spine_body_steps() -> u64 {
    SpineTransformBody::canonical().run_steps()
}

fn spine_tick_steps() -> u64 {
    SpineFullTick::canonical().run_steps()
}

const WORKLOADS: &[Workload] = &[
    Workload {
        name: ArenaTick::NAME,
        run: arena_hash,
    },
    Workload {
        name: PhysicsStep::NAME,
        run: physics_hash,
    },
    Workload {
        name: AiDecide::NAME,
        run: ai_hash,
    },
    Workload {
        name: DestructionStep::NAME,
        run: destruction_hash,
    },
    Workload {
        name: CrowdScale::NAME,
        run: crowd_hash,
    },
    Workload {
        name: SpineTransformBody::NAME,
        run: spine_body_steps,
    },
    Workload {
        name: SpineFullTick::NAME,
        run: spine_tick_steps,
    },
];

#[test]
#[ignore = "timing: release-mode budget proof, run with --release -- --ignored --test-threads=1"]
fn perf_time_budgets() {
    for workload in WORKLOADS {
        (workload.run)();
        let mut samples = Vec::with_capacity(5);
        for _ in 0..5 {
            let start = Instant::now();
            let _ = (workload.run)();
            samples.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        let p50 = percentile(&mut samples, 50.0);
        let max = samples.iter().copied().fold(0.0f64, f64::max);
        let budget = time_budget_ms(workload.name).expect("time budget recorded");
        println!(
            "{}: p50 {p50:.3} ms, max {max:.3} ms, budget {budget:.1} ms",
            workload.name
        );
        check_time_ms(workload.name, budget, p50).expect("p50 within budget");
    }
}
