//! One integration-test binary for engine-observe.
//!
//! Each file in this folder is a module, so the crate links one test
//! executable, not one per file. Add a test file as `tests/it/<name>.rs`
//! plus one `mod <name>;` line below. Run one file with
//! `cargo test -p engine-observe --test it <name>::`.

mod backoff_jitter_vectors;
mod bulkhead_vectors;
mod circuit_vectors;
mod counter_snapshot_vectors;
mod gauge_vectors;
mod heal_vectors;
mod hedge_vectors;
mod log_vectors;
mod retry_vectors;
mod tick_counters;
mod tick_histogram_vectors;
mod w1_pf9_rerun;
