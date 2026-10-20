//! Windowed sprite proof: field, walkers, tracking turret, PNG under target.

use std::path::{Path, PathBuf};

fn main() -> Result<(), engine_render::kam2_sprite_proof::SpriteProofError> {
    let proof = engine_render::kam2_sprite_proof::run_proof(target_png())?;
    println!(
        "KAM1B OK sprites={} png={}",
        proof.sprites,
        proof.png.display()
    );
    Ok(())
}

fn target_png() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .ancestors()
        .nth(2)
        .unwrap_or_else(|| Path::new("."));
    root.join("target").join("kam1b.png")
}
