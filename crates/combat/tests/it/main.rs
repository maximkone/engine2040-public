//! One integration-test binary for engine-combat.
//!
//! Each file in this folder is a module, so the crate links one test
//! executable, not one per file. Add a test file as `tests/it/<name>.rs`
//! plus one `mod <name>;` line below. Run one file with
//! `cargo test -p engine-combat --test it <name>::`.

mod breakbench_shatter;
mod design_wired;
mod p2_feel_latency;
mod telegraph_floor;
