//! Windowed 2.5D ground proof: real window, PNG under target, one receipt line.

use std::path::{Path, PathBuf};

fn main() -> Result<(), engine_render::kam2_iso_proof::ProofError> {
    let proof = engine_render::kam2_iso_proof::run_proof(target_png())?;
    println!("KAM1A OK tiles={} png={}", proof.tiles, proof.png.display());
    Ok(())
}

fn target_png() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.ancestors().nth(2).unwrap_or(Path::new("."));
    root.join("target").join("kam1a.png")
}
