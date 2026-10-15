//! One integration-test binary for engine-net.
//!
//! Each file in this folder is a module, so the crate links one test
//! executable, not one per file. Add a test file as `tests/it/<name>.rs`
//! plus one `mod <name>;` line below. Run one file with
//! `cargo test -p engine-net --test it <name>::`.

mod a01_delay_bench;
mod chaos;
mod checksum_dispute;
mod command_log_replica;
mod fork_varying_input;
mod frag_vectors;
mod g20_backroll_diff;
mod ggrs_loss;
mod input_validators;
mod join_gate_vectors;
mod mug_session_vectors;
mod packet_corpus;
mod pred_window;
mod protocol_hash_vectors;
mod quic_parity;
mod relay;
mod rollback;
mod rollback_soak;
mod stepper_trace;
mod sync_vectors;
mod visibility_client_vectors;
mod wan_fault_matrix;
