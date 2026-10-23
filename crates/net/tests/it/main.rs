//! One integration-test binary for engine-net.
//!
//! Each file in this folder is a module, so the crate links one test
//! executable, not one per file. Add a test file as `tests/it/<name>.rs`
//! plus one `mod <name>;` line below. Run one file with
//! `cargo test -p engine-net --test it <name>::`.

mod chaos;
mod checksum_dispute;
mod fork_varying_input;
mod g20_backroll_diff;
mod ggrs_loss;
mod mug_session_vectors;
mod packet_boundary_fuzz;
mod packet_corpus;
mod pred_window;
mod quic_parity;
mod relay;
mod rollback;
mod rollback_soak;
mod stepper_trace;
mod sync_vectors;
mod wan_fault_matrix;
