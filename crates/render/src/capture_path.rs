//! capture_path: where a windowed proof writes its PNG. The KAM1a/b/c proofs
//! differ only in the file name they hand in, so the folder join lives here
//! instead of once in each `main`.
//!
//! Determinism: the path is fixed at build time from the crate's manifest dir;
//! nothing here reads the clock, the cwd or the environment.

use std::path::{Path, PathBuf};

/// Path of the proof capture `name` under the repository's `target/` folder.
///
/// `CARGO_MANIFEST_DIR` is `crates/render`, so the repository root is two
/// ancestors up; a path shorter than that falls back to the process cwd.
#[must_use]
pub fn target_png(name: &str) -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .ancestors()
        .nth(2)
        .unwrap_or_else(|| Path::new("."));
    root.join("target").join(name)
}
