//! perf: deterministic budget workloads plus a counting allocator.
//! Consumer: the profiler and the perf report via `WorkloadReport`.
//! Units: ticks, ms, bytes. Determinism: canonical runs own all state
//! locally, so concurrent runs hash identically (HASH-LAW, no shared state).
//! Errors: typed `BudgetError` naming the blown budget.
//!
//! The `profiler` module adds the opt-in puffin scopes, GPU timestamp-query
//! scaffolding and frame captures; profiling never enters simulation hashes.

pub mod alloc;
pub mod allocator;
pub mod arena;
pub mod baselines;
pub mod budget;
pub mod core_bench;
pub mod fight_cost;
pub mod latency;
pub mod pgo_harness;
pub mod profiler;
pub mod server_budget;
pub mod smoke;
pub mod soak;
pub mod vram;
pub mod workloads;
