//! Windowed 2.5D ground proof: real window, PNG under target, one receipt line.

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), engine_render::kam2_iso_proof::ProofError> {
    let png = engine_render::capture_path::target_png("kam1a.png");
    let proof = engine_render::kam2_iso_proof::run_proof(png)?;
    println!("KAM1A OK tiles={} png={}", proof.tiles, proof.png.display());
    Ok(())
}

/// The proof writes a PNG and blocks on the GPU adapter: native only.
#[cfg(target_arch = "wasm32")]
fn main() {}
