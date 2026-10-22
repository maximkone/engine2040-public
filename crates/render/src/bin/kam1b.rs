//! Windowed sprite proof: field, walkers, tracking turret, PNG under target.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), engine_render::kam2_sprite_proof::SpriteProofError> {
    let png = engine_render::capture_path::target_png("kam1b.png");
    let proof = engine_render::kam2_sprite_proof::run_proof(png)?;
    println!(
        "KAM1B OK sprites={} png={}",
        proof.sprites,
        proof.png.display()
    );
    Ok(())
}

/// The proof writes a PNG and blocks on the GPU adapter: native only.
#[cfg(target_arch = "wasm32")]
fn main() {}
