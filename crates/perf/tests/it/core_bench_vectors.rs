//! Core bench vectors: the `core_bench` kernels as their callers see them.
//!
//! Consumer: the runtime choices in `docs/plan/runtime.md` (a second RNG
//! stream stays dropped unless the RNG proves the bottleneck) read these
//! kernels through `engine_perf::core_bench`. Units: ticks, draws, ops,
//! checksums, bytes. Determinism: same seed twice gives the same checksum;
//! timing never enters a hash. Errors are panics via asserts.
//!
//! Why this file exists: `crates/perf/src/core_bench.rs` changed 7 times in
//! 30 days yet held zero `#[test]` and no `tests/it` module named it, while
//! its docs cite `tests/core_bench.rs`, which does not exist (a new
//! top-level `tests/*.rs` fails `tools/workspace-check.mjs`). Each vector
//! below names the public path, so a dropped kernel, a drifted fixture, or
//! a stubbed checksum fails here first.

use engine_perf::core_bench::{
    accum_jitter_detail, kernel_accum_jitter, kernel_accum_soak, kernel_fixed_sqrt,
    kernel_fixed_trig, kernel_fold_hash, kernel_hash_bytes, kernel_rng_fork, kernel_rng_stream,
    kernel_run_ticks_600, kernel_tick_with_rng, kernel_tickstate_fold, kernel_tickstate_with_rng,
    measure_kernel, time_kernel, ACCUM_PUSHES, FORK_OPS, HASH_ITERS, RNG_STREAM, SEED_2040,
    SQRT_OPS, TICKS_600, TRIG_OPS,
};

/// The 600-tick seed-2040 fixture shape plus the soak/stream corpus sizes
/// stay pinned: a changed tick count or seed silently moves every checksum.
#[test]
fn fixture_shape_pins_ticks_seed_and_corpus() {
    assert_eq!(TICKS_600, 600, "600-tick golden shape");
    assert_eq!(SEED_2040, 2040, "legacy golden seed");
    assert_eq!(ACCUM_PUSHES, 10_000_000, "soak pushes");
    assert_eq!(RNG_STREAM, 1_000_000, "rng draws");
    assert_eq!(TRIG_OPS, 200_000, "trig corpus");
    assert_eq!(SQRT_OPS, 200_000, "sqrt corpus");
    assert_eq!(HASH_ITERS, 200_000, "hash corpus");
    assert_eq!(FORK_OPS, 50_000, "fork corpus");
}

/// Every cheap kernel repeats exactly and reads nonzero: determinism is the
/// caller-visible contract, and a zeroed stub must trip here, not in a
/// runtime decision.
#[test]
fn cheap_kernels_repeat_exactly_and_read_nonzero() {
    let pairs = [
        (
            "run_ticks_600",
            kernel_run_ticks_600(),
            kernel_run_ticks_600(),
        ),
        (
            "tickstate_fold",
            kernel_tickstate_fold(),
            kernel_tickstate_fold(),
        ),
        ("fold_hash", kernel_fold_hash(), kernel_fold_hash()),
        ("rng_stream", kernel_rng_stream(), kernel_rng_stream()),
        ("rng_fork", kernel_rng_fork(), kernel_rng_fork()),
        ("fixed_trig", kernel_fixed_trig(), kernel_fixed_trig()),
        ("fixed_sqrt", kernel_fixed_sqrt(), kernel_fixed_sqrt()),
        ("hash_bytes", kernel_hash_bytes(), kernel_hash_bytes()),
        (
            "tick_with_rng",
            kernel_tick_with_rng(),
            kernel_tick_with_rng(),
        ),
        (
            "tickstate_with_rng",
            kernel_tickstate_with_rng(),
            kernel_tickstate_with_rng(),
        ),
    ];
    for (name, first, second) in pairs {
        assert_eq!(first, second, "{name} repeats exactly");
        assert_ne!(first, 0, "{name} checksum is nonzero");
    }
}

/// Kernel families stay distinct: copying one body into another must trip.
/// The RNG pair also isolates the RNG share, so the with-RNG variant must
/// differ from its constant-step twin.
#[test]
fn kernel_families_stay_distinct() {
    let run_ticks = kernel_run_ticks_600();
    let tickstate = kernel_tickstate_fold();
    let fold = kernel_fold_hash();
    let stream = kernel_rng_stream();
    let trig = kernel_fixed_trig();
    let sqrt = kernel_fixed_sqrt();
    let digest = kernel_hash_bytes();
    let fork = kernel_rng_fork();
    let tick_rng = kernel_tick_with_rng();
    let tickstate_rng = kernel_tickstate_with_rng();

    assert_ne!(run_ticks, tickstate, "vec fold differs from state fold");
    assert_ne!(tickstate, fold, "state fold differs from bare fold");
    assert_ne!(stream, fold, "rng stream differs from fold chain");
    assert_ne!(trig, sqrt, "trig differs from sqrt");
    assert_ne!(digest, fold, "digest differs from fold chain");
    assert_ne!(fork, stream, "fork differs from stream");
    assert_ne!(tick_rng, run_ticks, "rng step differs from trivial step");
    assert_ne!(
        tickstate_rng, tickstate,
        "rng draw differs from constant step"
    );
}

/// `measure_kernel` reports the checksum it ran and the footprint the
/// caller budgets: pure kernels allocate nothing, the vec-backed tick loop
/// allocates.
#[test]
fn measure_kernel_reports_checksum_and_footprint() {
    let probe = measure_kernel(|| 0x1234_u64);
    assert_eq!(probe.checksum, 0x1234, "checksum passes through");
    assert_eq!(probe.peak_bytes, probe.alloc_bytes, "peak bounds the run");

    let pure = measure_kernel(kernel_tickstate_fold);
    assert_eq!(
        pure.checksum,
        kernel_tickstate_fold(),
        "measured checksum matches direct call"
    );
    assert_eq!(pure.alloc_bytes, 0, "state fold makes no heap call");
    assert_eq!(pure.alloc_count, 0, "state fold makes no heap call");

    let bare = measure_kernel(kernel_fold_hash);
    assert_eq!(bare.alloc_bytes, 0, "bare fold makes no heap call");
    assert_eq!(bare.alloc_count, 0, "bare fold makes no heap call");

    let vecked = measure_kernel(kernel_run_ticks_600);
    assert_eq!(
        vecked.checksum,
        kernel_run_ticks_600(),
        "vec checksum matches direct call"
    );
    assert!(
        vecked.alloc_bytes > 0 && vecked.alloc_count > 0,
        "vec tick loop reports its one Vec alloc ({} bytes, {} calls)",
        vecked.alloc_bytes,
        vecked.alloc_count
    );
}

/// `time_kernel` ranks a trivial closure: counts survive, the mean sits
/// inside the extremes, and the ns/op conversion follows the mean.
#[test]
fn time_kernel_ranks_trivial_closure() {
    let timing = time_kernel(0, 4, Some(100), || 0x9E37_79B9_u64);
    assert_eq!(timing.n, 4, "sample count survives");
    assert_eq!(timing.warmup, 0, "warmup count survives");
    assert!(timing.min_ms <= timing.max_ms, "extremes order");
    assert!(
        timing.mean_ms >= timing.min_ms && timing.mean_ms <= timing.max_ms,
        "mean {} inside [{}, {}]",
        timing.mean_ms,
        timing.min_ms,
        timing.max_ms
    );
    assert!(
        timing.p50_ms >= timing.min_ms && timing.p50_ms <= timing.max_ms,
        "p50 {} inside [{}, {}]",
        timing.p50_ms,
        timing.min_ms,
        timing.max_ms
    );
    let ns_per_op = timing.ns_per_op.expect("ops convert to ns/op");
    assert!(
        ns_per_op.is_finite() && ns_per_op >= 0.0,
        "ns/op {ns_per_op}"
    );
    let expected = timing.mean_ms * 1_000_000.0 / 100.0;
    assert!(
        (ns_per_op - expected).abs() <= expected.max(1.0) * 1e-9 + 1e-6,
        "ns/op {ns_per_op} follows mean {}",
        timing.mean_ms
    );
}

/// The 10M-push soak pair stays informative: both checksums repeat and read
/// nonzero, the jittered pattern differs from the exact-DT pattern, and the
/// jitter detail carries steps plus a finite interpolation alpha.
#[test]
fn accum_soak_pair_stays_informative() {
    let exact = kernel_accum_soak();
    assert_eq!(exact, kernel_accum_soak(), "exact soak repeats");
    assert_ne!(exact, 0, "exact soak checksum is nonzero");

    let jitter = kernel_accum_jitter();
    assert_eq!(jitter, kernel_accum_jitter(), "jitter soak repeats");
    assert_ne!(jitter, 0, "jitter soak checksum is nonzero");
    assert_ne!(exact, jitter, "jitter pattern differs from exact DT");

    let (steps, residual, alpha) = accum_jitter_detail();
    assert!(steps > 0, "jitter soak takes steps");
    assert!(residual.is_finite(), "residual {residual} is finite");
    assert!(
        alpha.is_finite() && (0.0..=1.0).contains(&alpha),
        "alpha {alpha} interpolates"
    );
}
