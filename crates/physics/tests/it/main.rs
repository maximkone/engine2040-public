//! One integration-test binary for engine-physics.
//!
//! Each file in this folder is a module, so the crate links one test
//! executable, not one per file. Add a test file as `tests/it/<name>.rs`
//! plus one `mod <name>;` line below. Run one file with
//! `cargo test -p engine-physics --test it <name>::`.
//! Five files stay their own binaries in `tests/`: each sets a
//! `#[global_allocator]` to count allocations, and one binary allows one.

mod break_slice_vectors;
mod idmap_fxbench;
mod pd_drive;
mod probe;
mod trace_snapshot;
