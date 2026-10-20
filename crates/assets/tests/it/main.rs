//! One integration-test binary for engine-assets.
//!
//! Each file in this folder is a module, so the crate links one test
//! executable, not one per file. Add a test file as `tests/it/<name>.rs`
//! plus one `mod <name>;` line below. Run one file with
//! `cargo test -p engine-assets --test it <name>::`.
//! `tests/vfetch_vectors.rs` stays its own binary: its `import` leg builds
//! under `--features import` (proven 2026-10-16: 7 passed).

mod ai_props;
mod arena_cook;
mod blockcomp_cook;
mod crowd_cook;
mod extras_wiring;
mod gltf_cc0_corpus;
mod gltf_conformance;
mod gltf_corpus_vectors;
mod import_dedup_vectors;
mod meshlet_cull_vectors;
mod meshlet_lod_cook;
mod meshlet_vectors;
mod optimize_vectors;
mod pilot_corner;
mod quant_vectors;
mod quantize_grid_vectors;
mod quantize_parity;
mod registry_singleton_vectors;
mod retarget_batch_a;
mod retarget_soma;
mod sidecar_conformance;
mod sparse_order_vectors;
mod spatial_sort_vectors;
