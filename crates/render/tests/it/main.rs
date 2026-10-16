//! One integration-test binary for engine-render.
//!
//! Each file in this folder is a module, so the crate links one test
//! executable, not one per file. Add a test file as `tests/it/<name>.rs`
//! plus one `mod <name>;` line below. Run one file with
//! `cargo test -p engine-render --test it <name>::`.
//!
//! `tests/gi_vectors.rs` stays a separate binary: it installs a
//! `#[global_allocator]`, and one binary allows only one allocator.

mod accel_golden;
mod cluster_vectors;
mod core_hash;
mod gi_bake;
mod lod;
mod material;
mod meshlet_cluster_vectors;
mod particles_seed;
mod post_aa_parity;
mod shadow_aces;
mod shadow_post;
mod splat_backdrop;
mod torch_cluster_vectors;
mod vfx_r5;
