//! Core baseline: determinism gate (default) plus the release-mode timing
//! proof (ignored). Consumer: the runtime decisions in
//! `docs/plan/runtime.md` (a second RNG stream stays dropped unless the RNG
//! exceeds 10% of tick cost).
//!
//! Default suite: every kernel hashes identically twice; allocation
//! footprints are reported (no timing asserts here).
//! Timing proof (release, single-threaded):
//! `cargo test --release -p engine-perf --test it core_bench:: -- --ignored --test-threads=1`
//! with `BENCH_OUT` pointing under the lane target dir; warmup 5, n = 30.

use engine_perf::core_bench::*;
use std::hint::black_box;

// Packet budget: warmup + n >= 30, release profile.
const WARMUP: usize = 5;
const SAMPLES: usize = 30;

/// One deterministic kernel plus its name.
type Kernel = (&'static str, fn() -> u64);

#[test]
fn core_kernels_deterministic_x2() {
    let pairs: &[Kernel] = &[
        ("run_ticks_600", kernel_run_ticks_600),
        ("tickstate_fold", kernel_tickstate_fold),
        ("tickstate_with_rng", kernel_tickstate_with_rng),
        ("fold_hash", kernel_fold_hash),
        ("rng_stream_1M", kernel_rng_stream),
        ("rng_fork", kernel_rng_fork),
        ("fixed_trig", kernel_fixed_trig),
        ("fixed_sqrt", kernel_fixed_sqrt),
        ("hash_bytes", kernel_hash_bytes),
        ("tick_with_rng", kernel_tick_with_rng),
        ("accum_jitter", kernel_accum_jitter),
    ];
    for (name, kernel) in pairs {
        let first = measure_kernel(|| black_box(kernel()));
        let second = measure_kernel(|| black_box(kernel()));
        assert_eq!(first.checksum, second.checksum, "{name} checksum repeats");
        assert_eq!(
            (first.alloc_bytes, first.alloc_count),
            (second.alloc_bytes, second.alloc_count),
            "{name} allocation footprint repeats"
        );
        println!(
            "{name}: checksum {0:016x}, alloc {1} bytes / {2} count, peak {3} bytes",
            first.checksum, first.alloc_bytes, first.alloc_count, first.peak_bytes
        );
    }
    // Accumulator soak runs once here (10M pushes); determinism of the full
    // soak is asserted in the timing proof to keep the default suite fast.
    // A short prefix still pins the push/step contract in the default suite.
    let mut acc = engine_core::LegacyFixedAcc::fixed();
    let mut steps = 0u32;
    for _ in 0..600 {
        steps += acc.push(engine_core::DT);
    }
    assert_eq!(steps, 600, "accumulator steps once per DT push");
}

#[test]
#[ignore = "timing: release-mode BENCH core baseline, run with --release -- --ignored --test-threads=1"]
fn core_bench_timing() {
    // Ordered to profile one machine exclusively: sequential, one thread.
    type TimedKernel = (&'static str, fn() -> u64, Option<u64>);
    let kernels: &[TimedKernel] = &[
        ("run_ticks_600", kernel_run_ticks_600, Some(600)),
        ("tickstate_fold", kernel_tickstate_fold, Some(600)),
        ("tickstate_with_rng", kernel_tickstate_with_rng, Some(600)),
        ("fold_hash", kernel_fold_hash, Some(600)),
        ("tick_with_rng", kernel_tick_with_rng, Some(600)),
        ("rng_stream_1M", kernel_rng_stream, Some(RNG_STREAM as u64)),
        ("rng_fork", kernel_rng_fork, Some(FORK_OPS as u64)),
        ("fixed_trig", kernel_fixed_trig, Some(TRIG_OPS as u64)),
        ("fixed_sqrt", kernel_fixed_sqrt, Some(SQRT_OPS as u64)),
        ("hash_bytes_64B", kernel_hash_bytes, Some(HASH_ITERS as u64)),
    ];
    let mut rows: Vec<String> = Vec::new();
    rows.push(
        "kernel,seed_or_input,ops_per_sample,n,warmup,mean_ms,p50_ms,p95_ms,p99_ms,min_ms,max_ms,ns_per_op,checksum,alloc_bytes,alloc_count,peak_bytes".to_string(),
    );
    let mut tick_plain_p50 = 0.0;
    let mut tick_rng_p50 = 0.0;
    let mut fold_plain_p50 = 0.0;
    let mut fold_rng_p50 = 0.0;
    for (name, kernel, ops) in kernels {
        let timing = time_kernel(WARMUP, SAMPLES, *ops, || black_box(kernel()));
        let measured = measure_kernel(|| black_box(kernel()));
        if *name == "run_ticks_600" {
            tick_plain_p50 = timing.p50_ms;
        }
        if *name == "tick_with_rng" {
            tick_rng_p50 = timing.p50_ms;
        }
        if *name == "tickstate_fold" {
            fold_plain_p50 = timing.p50_ms;
        }
        if *name == "tickstate_with_rng" {
            fold_rng_p50 = timing.p50_ms;
        }
        let ns = timing
            .ns_per_op
            .map(|v| format!("{v:.2}"))
            .unwrap_or_else(|| "-".to_string());
        println!(
            "{name}: n={} warmup={} mean={:.4}ms p50={:.4} p95={:.4} p99={:.4} min={:.4} max={:.4} ns/op={} checksum={:016x} alloc={}B/{} peak={}B",
            timing.n,
            timing.warmup,
            timing.mean_ms,
            timing.p50_ms,
            timing.p95_ms,
            timing.p99_ms,
            timing.min_ms,
            timing.max_ms,
            ns,
            measured.checksum,
            measured.alloc_bytes,
            measured.alloc_count,
            measured.peak_bytes,
        );
        rows.push(format!(
            "{name},seed-2040,{},{},{},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{},{:016x},{},{},{}",
            ops.unwrap_or(1),
            timing.n,
            timing.warmup,
            timing.mean_ms,
            timing.p50_ms,
            timing.p95_ms,
            timing.p99_ms,
            timing.min_ms,
            timing.max_ms,
            timing
                .ns_per_op
                .map(|v| format!("{v:.3}"))
                .unwrap_or_else(|| "-".to_string()),
            measured.checksum,
            measured.alloc_bytes,
            measured.alloc_count,
            measured.peak_bytes,
        ));
    }
    // Soak: single long sample (10M pushes) plus a sliced ns/push timing
    // with n=30 over 200k-push slices so the distribution stays comparable.
    let soak_timing = time_kernel(WARMUP, SAMPLES, Some(200_000), || {
        let mut acc = engine_core::LegacyFixedAcc::fixed();
        let mut steps = 0u64;
        for _ in 0..200_000 {
            steps += acc.push(black_box(engine_core::DT)) as u64;
        }
        black_box(steps)
    });
    let soak_full = measure_kernel(kernel_accum_soak);
    let jitter_full = measure_kernel(kernel_accum_jitter);
    println!(
        "accum_soak_10M: slice n={} mean={:.4}ms p50={:.4} p95={:.4} p99={:.4} ns/push={:.2} full_checksum={:016x} alloc={}B/{} peak={}B",
        soak_timing.n,
        soak_timing.mean_ms,
        soak_timing.p50_ms,
        soak_timing.p95_ms,
        soak_timing.p99_ms,
        soak_timing.ns_per_op.unwrap_or(0.0),
        soak_full.checksum,
        soak_full.alloc_bytes,
        soak_full.alloc_count,
        soak_full.peak_bytes,
    );
    let (jitter_steps, jitter_residual, jitter_alpha) = accum_jitter_detail();
    println!(
        "accum_jitter_10M: full_checksum={:016x} steps={} residual={:.9}s alpha={:.6} alloc={}B/{} peak={}B",
        jitter_full.checksum,
        jitter_steps,
        jitter_residual,
        jitter_alpha,
        jitter_full.alloc_bytes,
        jitter_full.alloc_count,
        jitter_full.peak_bytes,
    );
    rows.push(format!(
        "accum_soak_10M_slice200k,DT-1/60,200000,{},{},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{},{:016x},{},{},{}",
        soak_timing.n,
        soak_timing.warmup,
        soak_timing.mean_ms,
        soak_timing.p50_ms,
        soak_timing.p95_ms,
        soak_timing.p99_ms,
        soak_timing.min_ms,
        soak_timing.max_ms,
        soak_timing
            .ns_per_op
            .map(|v| format!("{v:.3}"))
            .unwrap_or_else(|| "-".to_string()),
        soak_full.checksum,
        soak_full.alloc_bytes,
        soak_full.alloc_count,
        soak_full.peak_bytes,
    ));
    rows.push(format!(
        "accum_jitter_10M,DT-0.5x/1.5x-alternating,10000000,1,0,-,-,-,-,-,-,-,{:016x},{},{},{}",
        jitter_full.checksum,
        jitter_full.alloc_bytes,
        jitter_full.alloc_count,
        jitter_full.peak_bytes,
    ));
    // Stream A/B verdict input: RNG share of the 600-tick loop. The primary
    // comparator is the zero-alloc TickState pair; the run_ticks pair is
    // secondary (each sample carries one 4800-byte Vec alloc).
    let share = |plain: f64, with_rng: f64| {
        if with_rng > 0.0 {
            (with_rng - plain).max(0.0) / with_rng * 100.0
        } else {
            0.0
        }
    };
    println!(
        "stream_AB primary (tickstate): plain_p50={:.4}ms rng_p50={:.4}ms rng_share={:.2}% (drop unless >10%)",
        fold_plain_p50,
        fold_rng_p50,
        share(fold_plain_p50, fold_rng_p50)
    );
    println!(
        "stream_AB secondary (run_ticks): plain_p50={:.4}ms rng_p50={:.4}ms rng_share={:.2}%",
        tick_plain_p50,
        tick_rng_p50,
        share(tick_plain_p50, tick_rng_p50)
    );
    let out = std::env::var("BENCH_OUT").unwrap_or_else(|_| "bench-core.csv".to_string());
    if let Some(parent) = std::path::Path::new(&out).parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).expect("bench out dir");
        }
    }
    std::fs::write(&out, rows.join("\n") + "\n").expect("bench csv writes");
    println!("raw csv: {out}");
}
