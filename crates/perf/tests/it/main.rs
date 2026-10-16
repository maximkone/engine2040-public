//! One integration-test binary for engine-perf.
//!
//! Each file in this folder is a module, so the crate links one test
//! executable, not one per file. Add a test file as `tests/it/<name>.rs`
//! plus one `mod <name>;` line below. Run one file with
//! `cargo test -p engine-perf --test it <name>::`.
//!
//! `tests/profiler.rs` and `tests/w1_pf9a_paired.rs` stay separate binaries:
//! both flip the one process-wide profiler switch, and each guards it with its
//! own lock, so in one process they would race each other.

mod alloc;
mod alloc_compare;
mod allocator_baseline;
mod bench_export;
mod bet_wall_spike;
mod canonical;
mod concurrency;
mod core_bench;
mod core_bench_vectors;
mod heap_peak;
mod lib_wiring_vectors;
mod perf_proof;
mod regression;
mod sparkline_vectors;
mod w1_pf5_simd;
