//! One integration-test binary for engine-core.
//!
//! Each file in this folder is a module, so the crate links one test
//! executable, not one per file. Add a test file as `tests/it/<name>.rs`
//! plus one `mod <name>;` line below. Run one file with
//! `cargo test -p engine-core --test it <name>::`.
//! The `property_matrix` regression seeds stay in
//! `tests/property_matrix_regressions.txt`, found from the crate root.

mod deterministic_replay_vectors;
mod fixed_property;
mod fppe_vec3_vectors;
mod property_matrix;
mod turn_hash_journal;
