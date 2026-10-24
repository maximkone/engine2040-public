//! One integration-test binary for engine-game.
//!
//! Each file in this folder is a module, so the crate links one test
//! executable, not one per file. Add a test file as `tests/it/<name>.rs`
//! plus one `mod <name>;` line below. Run one file with
//! `cargo test -p engine-game --test it <name>::`.
//!
//! `tests/crash_hook.rs` stays a separate binary: it installs the process-wide
//! crash hook and re-runs its own executable as a child process.

mod camera_smoke;
mod cannae;
mod config_builder_vectors;
mod config_rows;
mod config_vectors;
mod config_verify_vectors;
mod design_combat;
mod design_controls;
mod design_cues;
mod design_enemy;
mod design_loop;
mod env_prefix_layers;
mod fps_view_tape;
mod ime_vectors;
mod knob_rules_vectors;
mod legionary_slice;
mod lobby_ui_flow;
mod rome_numbers;
mod rome_spec_drift;
mod scene_cli;
