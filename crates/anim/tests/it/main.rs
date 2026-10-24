//! One integration-test binary for engine-anim.
//!
//! Each file in this folder is a module, so the crate links one test
//! executable, not one per file. Add a test file as `tests/it/<name>.rs`
//! plus one `mod <name>;` line below. Run one file with
//! `cargo test -p engine-anim --test it <name>::`.

mod anim_budget;
mod anim_lod_pin;
mod motion_bake;
