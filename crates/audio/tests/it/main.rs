//! One integration-test binary for engine-audio.
//!
//! Each file in this folder is a module, so the crate links one test
//! executable, not one per file. Add a test file as `tests/it/<name>.rs`
//! plus one `mod <name>;` line below. Run one file with
//! `cargo test -p engine-audio --test it <name>::`.

mod device_heal;
mod dsp_rms;
mod feed_contention;
mod music_tension;
mod occlusion_xover;
mod presence;
mod strike_sound;
