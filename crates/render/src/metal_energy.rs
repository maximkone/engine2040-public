//! metal_energy: the QL2 white-furnace proof of the multiple-scatter gain on a
//! rough conductor. The gain `1 + F0 * (1 / E - 1)` (RL47 A1) already runs in
//! `shaders/forward.wgsl` (`ibl_specular`, `specular_term`) and in the CPU twin
//! `ibl::shade_ibl`; this file pins the energy it returns.
//!
//! Test-only: `lib.rs` declares it under `#[cfg(all(test, feature = "v1"))]`.
//! Before = the single-scatter albedo the split-sum lobe returns at F0 = 1,
//! roughness 1.0, normal view, under a uniform white environment: `scale + bias`
//! read from the same LUT the shade samples. After = `ibl::shade_ibl` on that
//! bake, which multiplies the lobe by the gain. Determinism: pure f32.

use crate::ibl::{
    brdf_lut_build, brdf_sample, shade_ibl, IblBake, IrradianceSh, PrefilterCube, IBL_LEVELS,
    IBL_SH_COEFFS,
};

/// Printed tolerance: the compensated white-metal albedo must land within this
/// of 1.0 (the gain closes the lobe, so the shipped value is 1.0 to f32 noise).
const TOLERANCE: f32 = 0.02;
/// The uncompensated albedo must sit at or below this (single-scatter 0.31).
const CLEARLY_BELOW: f32 = 0.5;

#[test]
fn furnace_white_conductor_before_and_after_compensation() {
    // Uniform white environment: every prefilter face and the SH DC term carry 1.
    let mut cube = PrefilterCube::new(8, IBL_LEVELS);
    cube.faces.fill([1.0, 1.0, 1.0]);
    let mut coeffs = [[0.0; 3]; IBL_SH_COEFFS];
    coeffs[0] = [3.544_908; 3];
    let bake = IblBake {
        cube,
        sh: IrradianceSh { coeffs },
        brdf: brdf_lut_build(16).expect("furnace lut builds"),
        source_average: [1.0; 3],
    };
    // Rough (roughness 1.0), F0 = 1 conductor, normal view: the worst case.
    let up = [0.0, 1.0, 0.0];
    let lut = brdf_sample(&bake.brdf, 1.0, 1.0).expect("lut samples");
    let before = lut[0] + lut[1];
    let after = shade_ibl(&bake, up, up, [1.0; 3], 1.0, 1.0).expect("furnace shades")[0];
    let recovered = (after - before) / (1.0 - before);
    println!(
        "QL2 furnace (GGX conductor, F0 1.0, roughness 1.0, white env): \
         before={before:.4} after={after:.4} tolerance={TOLERANCE} \
         recovered={:.1}% of the lost energy",
        recovered * 100.0
    );
    assert!(
        (after - 1.0).abs() <= TOLERANCE,
        "compensated albedo {after} is outside 1.0 +/- {TOLERANCE}"
    );
    assert!(
        before <= CLEARLY_BELOW,
        "uncompensated albedo {before} should sit clearly below 1.0 (<= {CLEARLY_BELOW})"
    );
}
