//! One integration-test binary for engine-endpoint.
//!
//! Each file in this folder is a module, so the crate links one test
//! executable, not one per file. Add a test file as `tests/it/<name>.rs`
//! plus one `mod <name>;` line below. Run one file with
//! `cargo test -p engine-endpoint --test it <name>::`.

mod collapse_fixtures;
mod diagnostic_render_vectors;
mod game_build;
mod injection_door;
mod jsonrpc_conformance_vectors;
mod lib_std_wiring;
mod lib_wiring_vectors;
mod mcp_manifest_vectors;
mod mcp_stdio;
mod redteam;
mod rules_door;
mod rules_pack_vectors;
mod scene_drives_match;
mod scene_query_cone;
mod session_chaos;
mod step20x2;
mod tool_budget_exhaustion;
mod tool_count_pin;
