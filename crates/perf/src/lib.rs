//! perf: deterministic budget workloads plus a counting allocator.
//! Consumer: the profiler and the perf report via `WorkloadReport`.
//! Units: ticks, ms, bytes. Determinism: canonical runs own all state
//! locally, so concurrent runs hash identically (HASH-LAW, no shared state).
//! Errors: typed `BudgetError` naming the blown budget.
//!
//! The `profiler` module adds the opt-in puffin scopes, GPU timestamp-query
//! scaffolding and frame captures; profiling never enters simulation hashes.

/// Allocation counting and the heap-peak gate math.
pub mod alloc;
/// Bench-harness allocator rows (system and mimalloc candidate).
pub mod allocator;
/// Per-frame bump arena for presized transient scratch.
pub mod arena;
/// Recorded hash, allocation and millisecond baselines.
pub mod baselines;
/// Budget checks over measured workload reports.
pub mod budget;
/// Core tick, hash and RNG baseline kernels.
pub mod core_bench;
/// Named perf series: bounded history, O(1) mean, time-aware smoothing.
pub mod diagnostics;
/// Feel quantification: three-domain runs against the latency bar.
pub mod feel;
/// Frame-cost percentiles behind the fight frame gate.
pub mod fight_cost;
/// Input latency meter: event tick to first sim effect and present.
pub mod latency;
/// Opt-in puffin scopes, GPU timestamp scaffolding and frame captures.
pub mod profiler;
/// Packaged smoke: quick pass-path check for the frame-time gate.
pub mod smoke;
/// Hour soak: long-run stability check for the frame-time gate.
pub mod soak;
/// Pure integer VRAM budget math.
pub mod vram;
/// Canonical budget workloads: arena tick, physics, AI, destruction.
pub mod workloads;
