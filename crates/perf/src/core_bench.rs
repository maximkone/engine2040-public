//! Core baseline kernels: tick/hash/RNG math on the 600-tick seed-2040
//! fixture plus the 10M-push accumulator soak and the 1M-u64 RNG stream.
//! Consumer: the runtime decisions in `docs/plan/runtime.md` (a second RNG
//! stream stays dropped unless the RNG proves the bottleneck).
//!
//! Determinism: every kernel returns a checksum; the same seed twice gives
//! the same checksum (asserted by `tests/it/core_bench_vectors.rs`). Timing never
//! enters a hash. Units: ns/op, ms/run, bytes. No new dependency (std-only).
//!
//! Kernels cover the packet baseline: `run_ticks` + `fold_hash`/`TickState`
//! + `Rng::next_u64`/`fork` + `Fixed` trig/sqrt + `hash_bytes` (FNV-1a64).

use crate::alloc;
use engine_core::fixed::Fixed;
use engine_core::{hash_bytes, IntAcc, Rng, TickState};
use std::hint::black_box;
use std::time::Instant;

/// 600-tick legacy golden shape: seed 2040, trivial step.
pub const TICKS_600: u64 = 600;
/// Legacy golden seed.
pub const SEED_2040: u64 = 2040;
/// Soak length: 10M accumulator pushes.
pub const ACCUM_PUSHES: u64 = 10_000_000;
/// RNG stream length: 1M u64 draws.
pub const RNG_STREAM: usize = 1_000_000;
/// Fixed trig corpus per sample (angle-domain sweep slice).
pub const TRIG_OPS: usize = 200_000;
/// Fixed sqrt corpus per sample.
pub const SQRT_OPS: usize = 200_000;
/// FNV hash_bytes iterations per sample (64-byte buffer).
pub const HASH_ITERS: usize = 200_000;
/// Fork corpus per sample.
pub const FORK_OPS: usize = 50_000;

/// Raw angle domain size for Q47.16 TAU (411776 distinct inputs).
const TAU_RAW: i64 = 411_776;

/// One deterministic kernel result: checksum plus allocation footprint.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KernelAlloc {
    /// Checksum of the kernel (deterministic per seed).
    pub checksum: u64,
    /// Total requested bytes on the run thread.
    pub alloc_bytes: u64,
    /// Allocation count on the run thread.
    pub alloc_count: u64,
    /// Upper bound on the live high-water mark: the shared probe counts
    /// requests, not frees, so this equals the true peak for kernels that
    /// free nothing mid-run (every kernel here).
    pub peak_bytes: u64,
}

/// Run `f` under the counting allocator, capturing totals plus the peak bound.
pub fn measure_kernel(f: impl FnOnce() -> u64) -> KernelAlloc {
    let (checksum, observed) = alloc::measure(f);
    KernelAlloc {
        checksum,
        alloc_bytes: observed.bytes,
        alloc_count: observed.count,
        peak_bytes: observed.bytes,
    }
}

/// 600-tick `run_ticks` with the trivial `|t| t.0` step (fold + TickState path).
pub fn kernel_run_ticks_600() -> u64 {
    let hashes = engine_core::run_ticks(TICKS_600, SEED_2040, |t, _| t.0);
    let mut acc = 0u64;
    for h in hashes {
        acc ^= black_box(h);
    }
    acc
}

/// Bare 600-step `TickState` fold with per-tick step hashes 0..600.
pub fn kernel_tickstate_fold() -> u64 {
    let mut state = TickState::new(SEED_2040);
    for t in 0..TICKS_600 {
        black_box(state.step(t));
    }
    state.hash()
}

/// Pure `fold_hash` chain without the TickState wrapper (same 600 steps).
pub fn kernel_fold_hash() -> u64 {
    let mut acc = SEED_2040;
    for t in 0..TICKS_600 {
        acc = engine_core::fold_hash(acc, t, black_box(t.wrapping_mul(0x9E37_79B9)));
    }
    acc
}

/// 1M-u64 RNG stream folded with XOR (the RNG stream-cost input).
pub fn kernel_rng_stream() -> u64 {
    let mut rng = Rng::new(SEED_2040);
    let mut acc = 0u64;
    for _ in 0..RNG_STREAM {
        acc ^= black_box(rng.next_u64());
    }
    acc
}

/// `FORK_OPS` keyed forks from one parent (the fork-cost input).
pub fn kernel_rng_fork() -> u64 {
    let base = Rng::new(SEED_2040);
    let mut acc = 0u64;
    for i in 0..FORK_OPS as u64 {
        acc ^= black_box(base.fork(i).state());
    }
    acc
}

/// `TRIG_OPS` Fixed sin+cos over the angle domain (the trig baseline).
pub fn kernel_fixed_trig() -> u64 {
    let mut acc = 0u64;
    for i in 0..TRIG_OPS as i64 {
        let raw = (i.wrapping_mul(7919)).rem_euclid(TAU_RAW);
        let angle = Fixed::from_bits(black_box(raw));
        let (s, c) = angle.sin_cos();
        acc ^= black_box(s.to_bits() as u64).wrapping_add(black_box(c.to_bits() as u64));
    }
    acc
}

/// `SQRT_OPS` Fixed sqrt over 1..1000 cycling (the sqrt baseline). The fold
/// mixes the index so the checksum is order-sensitive and nonzero (a plain
/// XOR over an even number of identical cycles cancels to zero).
pub fn kernel_fixed_sqrt() -> u64 {
    let mut acc = 0u64;
    for i in 0..SQRT_OPS {
        let v = Fixed::from_int(black_box((i % 1000) as i64 + 1));
        acc = acc.wrapping_add(
            black_box(v.sqrt().to_bits() as u64) ^ (i as u64).wrapping_mul(0x9E37_79B9),
        );
    }
    acc
}

/// `HASH_ITERS` FNV `hash_bytes` over a 64-byte buffer (the digest input).
/// The fold mixes the iteration index so the checksum is order-sensitive
/// and nonzero (a plain XOR over an even repetition count cancels to zero).
pub fn kernel_hash_bytes() -> u64 {
    let mut buf = [0u8; 64];
    for (i, slot) in buf.iter_mut().enumerate() {
        *slot = (i as u64).wrapping_mul(0x9E37_79B9).to_le_bytes()[0];
    }
    let mut acc = 0u64;
    for i in 0..HASH_ITERS {
        acc = acc.wrapping_add(
            black_box(hash_bytes(black_box(&buf))) ^ (i as u64).wrapping_mul(0x9E37_79B9),
        );
    }
    acc
}

/// 10M-push accumulator soak: push exactly 10 ms (integer ns), count steps,
/// fold the integer residual. 10M x 10 ms is 6_000_000 ticks with zero
/// residual, so this kernel pins the step contract.
pub fn kernel_accum_soak() -> u64 {
    let mut acc = IntAcc::new();
    for _ in 0..ACCUM_PUSHES {
        black_box(acc.push_ns(black_box(10_000_000)).unwrap_or(0));
    }
    black_box(accum_checksum(&acc))
}

/// Detail for the jittered soak: total steps, the final integer residual in
/// ns and its render-interpolation alpha. The retired f32 accumulator ended
/// this pattern 0.745 ms off true (RC30 measurement); the integer one reads
/// 0 ns.
pub fn accum_jitter_detail() -> (u64, u64, f32) {
    let acc = jitter_run();
    (acc.tick(), acc.buffered_ns(), acc.alpha())
}

/// Jittered 10M-push soak: an alternating 16 ms / 17 ms pattern (a
/// millisecond-quantized wall clock, the realistic poll input). Folds
/// (steps, residual). The total is exactly 9_900_000 ticks with 0 ns left.
pub fn kernel_accum_jitter() -> u64 {
    black_box(accum_checksum(&jitter_run()))
}

fn jitter_run() -> IntAcc {
    let mut acc = IntAcc::new();
    for i in 0..ACCUM_PUSHES {
        let frame_ns = if i % 2 == 0 { 16_000_000 } else { 17_000_000 };
        black_box(acc.push_ns(black_box(frame_ns)).unwrap_or(0));
    }
    acc
}

fn accum_checksum(acc: &IntAcc) -> u64 {
    acc.tick()
        ^ hash_bytes(&acc.buffered_ns().to_le_bytes())
        ^ hash_bytes(&acc.alpha().to_le_bytes())
}

/// Tick loop with a per-tick RNG draw (stream-A/B comparator): 600 ticks,
/// each step hashes `rng.next_u64()`. Compare against `kernel_run_ticks_600`
/// to isolate the RNG share of tick cost.
pub fn kernel_tick_with_rng() -> u64 {
    let hashes = engine_core::run_ticks(TICKS_600, SEED_2040, |_, rng| black_box(rng.next_u64()));
    let mut acc = 0u64;
    for h in hashes {
        acc ^= black_box(h);
    }
    acc
}

/// Bare 600-step `TickState` fold with one `Rng::next_u64` draw folded per
/// tick. Paired with `kernel_tickstate_fold` (same loop, constant step
/// hash), this isolates the RNG share of tick cost with zero allocation
/// noise — the primary stream-A/B comparator. The `run_ticks` pair above
/// is the secondary comparator (it carries one 4800-byte Vec alloc each).
pub fn kernel_tickstate_with_rng() -> u64 {
    let mut state = TickState::new(SEED_2040);
    let mut rng = Rng::new(SEED_2040);
    for _ in 0..TICKS_600 {
        black_box(state.step(black_box(rng.next_u64())));
    }
    state.hash()
}

/// Timing distribution over samples in milliseconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Timing {
    /// Sample count.
    pub n: usize,
    /// Warmup runs discarded before sampling.
    pub warmup: usize,
    /// Arithmetic mean in ms.
    pub mean_ms: f64,
    /// Sorted percentiles in ms.
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub p99_ms: f64,
    /// Extremes in ms.
    pub min_ms: f64,
    pub max_ms: f64,
    /// Nanoseconds per op (when `ops_per_sample` is Some).
    pub ns_per_op: Option<f64>,
}

/// Time `f` with `warmup` discards then `n` samples; `ops_per_sample`
/// converts the mean into ns/op.
pub fn time_kernel(
    warmup: usize,
    n: usize,
    ops_per_sample: Option<u64>,
    mut f: impl FnMut() -> u64,
) -> Timing {
    for _ in 0..warmup {
        black_box(f());
    }
    let mut samples = Vec::with_capacity(n);
    for _ in 0..n {
        let start = Instant::now();
        black_box(f());
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(f64::total_cmp);
    let mean_ms = samples.iter().sum::<f64>() / samples.len().max(1) as f64;
    let ns_per_op = ops_per_sample.map(|ops| mean_ms * 1_000_000.0 / ops as f64);
    Timing {
        n,
        warmup,
        mean_ms,
        p50_ms: crate::latency::nearest_rank_sorted(&samples, 50.0),
        p95_ms: crate::latency::nearest_rank_sorted(&samples, 95.0),
        p99_ms: crate::latency::nearest_rank_sorted(&samples, 99.0),
        min_ms: samples.first().copied().unwrap_or(0.0),
        max_ms: samples.last().copied().unwrap_or(0.0),
        ns_per_op,
    }
}
