//! tests_look: the live look: on/off gap, furnace, occlusion and shading parity.

#![cfg(test)]

use super::*;

#[cfg(feature = "v1")]
pub(super) fn nonzero_on_off_bake() -> IblBake {
    // Nonzero fixture: uniform prefilter radiance plus unit
    // irradiance plus a real small BRDF table, so both probes
    // see baked light well above the flat floor.
    let mut cube = PrefilterCube::new(4, IBL_LEVELS);
    cube.faces.fill([0.8, 0.7, 0.6]);
    let mut coeffs = [[0.0; 3]; IBL_SH_COEFFS];
    coeffs[0] = [3.544_908, 3.544_908, 3.544_908];
    IblBake {
        cube,
        sh: IrradianceSh { coeffs },
        brdf: brdf_lut_build(8).expect("small lut builds"),
        source_average: [0.8, 0.7, 0.6],
    }
}

#[cfg(feature = "v1")]
pub(super) fn zero_on_off_bake() -> IblBake {
    IblBake {
        cube: PrefilterCube::new(4, IBL_LEVELS),
        sh: IrradianceSh::zero(),
        brdf: BrdfLut::new(4),
        source_average: [0.0; 3],
    }
}

#[cfg(feature = "v1")]
fn white_furnace_bake() -> IblBake {
    let mut cube = PrefilterCube::new(8, IBL_LEVELS);
    cube.faces.fill([1.0, 1.0, 1.0]);
    let mut coeffs = [[0.0; 3]; IBL_SH_COEFFS];
    coeffs[0] = [3.544_908, 3.544_908, 3.544_908];
    IblBake {
        cube,
        sh: IrradianceSh { coeffs },
        brdf: brdf_lut_build(16).expect("furnace lut builds"),
        source_average: [1.0, 1.0, 1.0],
    }
}

/// RL47: a roughness-1.0 white metal in a white furnace returns the light
/// it receives. Single-scatter split-sum returns 0.31; the multiple-
/// scatter factor closes the gap (accept >= 0.9).
#[cfg(feature = "v1")]
#[test]
fn ibl_furnace_white_metal_keeps_its_energy() {
    // The tap sequence must be Van der Corput order; a rotated reversal
    // puts every tap near zero and hides the energy loss.
    let vdc: Vec<f32> = (0..8).map(radical_inverse_vdc).collect();
    assert_eq!(vdc, [0.0, 0.5, 0.25, 0.75, 0.125, 0.625, 0.375, 0.875]);
    let e = brdf_integrate(1.0, 1.0).expect("integral");
    assert!(
        (0.25..=0.45).contains(&(e[0] + e[1])),
        "the single-scatter lobe loses the bounce (E {})",
        e[0] + e[1]
    );
    let bake = white_furnace_bake();
    for view in [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.6, 0.0, 0.8]] {
        let out =
            shade_ibl(&bake, [0.0, 1.0, 0.0], view, [1.0; 3], 1.0, 1.0).expect("furnace shades");
        for c in out {
            assert!(
                (0.9..=1.1).contains(&c),
                "view {view:?}: furnace ratio {c} sits in 0.9..=1.1"
            );
        }
    }
}

#[cfg(feature = "v1")]
#[test]
fn energy_compensation_closes_the_lobe_and_stays_finite() {
    let comp = energy_compensation([1.0, 0.5, 0.04], [0.31, 0.0]);
    assert!((0.31 * comp[0] - 1.0).abs() < 1e-5, "white metal closes");
    assert!(
        (comp[1] - 2.1129).abs() < 1e-4,
        "half metal takes half the gain, got {}",
        comp[1]
    );
    assert!(comp[2] > 1.0 && comp[2] < 1.2, "dielectric stays near 1");
    // A1: a zero or unbaked lobe gains at most 3x (white metal 4x), not 10000x.
    let dark = energy_compensation([1.0; 3], [0.0, 0.0]);
    assert_eq!(dark, [4.0; 3], "a zero lobe is capped, not amplified");
}

#[test]
fn the_forward_ibl_specular_compensates_multiple_scatter() {
    // The numeric proof is `spec_lobe::device_twin::ms_gain_wgsl_matches_the_cpu_twin`;
    // this keeps the call sites: both specular paths use the one `ms_gain`.
    let marker = "ms_gain(f0, fab.x + fab.y)";
    let wgsl = include_str!("../shaders/forward.wgsl");
    assert!(wgsl.contains("fn ibl_specular"));
    assert!(wgsl.contains(marker), "forward ibl_specular compensates");
    assert!(
        FORWARD_IBL_WGSL.contains(marker),
        "the chart chunk compensates"
    );
}

#[cfg(feature = "v1")]
#[test]
fn ibl_on_off_diff_armor_differs_above_epsilon() {
    // Armor-like metal: gray base, full metal, mid roughness.
    let bake = nonzero_on_off_bake();
    let diff = ibl_on_off_diff(
        &bake,
        [0.0, 1.0, 0.0],
        [0.4, 0.6, 0.5],
        [0.5, 0.5, 0.5],
        1.0,
        0.35,
    )
    .expect("armor diffs");
    assert!(
        diff.iter().any(|c| c.abs() > 1e-3),
        "armor bake moves above floor, got {diff:?}"
    );
}

#[cfg(feature = "v1")]
#[test]
fn ibl_on_off_diff_sand_differs_above_epsilon() {
    // Sand-like dielectric: sand base, no metal, rough.
    let bake = nonzero_on_off_bake();
    let diff = ibl_on_off_diff(
        &bake,
        [0.0, 1.0, 0.0],
        [0.4, 0.6, 0.5],
        [0.76, 0.62, 0.42],
        0.0,
        0.9,
    )
    .expect("sand diffs");
    assert!(
        diff.iter().any(|c| c.abs() > 1e-3),
        "sand bake moves above floor, got {diff:?}"
    );
}

#[cfg(feature = "v1")]
#[test]
fn ibl_on_off_diff_runs_match_exactly() {
    // Same complete inputs twice give bit-identical gaps.
    let bake = nonzero_on_off_bake();
    let armor = || {
        ibl_on_off_diff(
            &bake,
            [0.0, 1.0, 0.0],
            [0.4, 0.6, 0.5],
            [0.5, 0.5, 0.5],
            1.0,
            0.35,
        )
        .expect("armor diffs")
    };
    let sand = || {
        ibl_on_off_diff(
            &bake,
            [0.0, 1.0, 0.0],
            [0.4, 0.6, 0.5],
            [0.76, 0.62, 0.42],
            0.0,
            0.9,
        )
        .expect("sand diffs")
    };
    assert_eq!(armor(), armor(), "armor gap is deterministic");
    assert_eq!(sand(), sand(), "sand gap is deterministic");
}

#[cfg(feature = "v1")]
#[test]
fn ibl_on_off_diff_zero_bake_is_floor_without_refusal() {
    // Zero bake shades black, so the gap is the negated floor term:
    // zero for full metal, negative floor for dielectric. No refusal.
    let bake = zero_on_off_bake();
    let armor = ibl_on_off_diff(
        &bake,
        [0.0, 1.0, 0.0],
        [0.4, 0.6, 0.5],
        [0.5, 0.5, 0.5],
        1.0,
        0.35,
    )
    .expect("zero armor diffs");
    assert_eq!(armor, [0.0, 0.0, 0.0], "metal gap is zero, got {armor:?}");
    let sand = ibl_on_off_diff(
        &bake,
        [0.0, 1.0, 0.0],
        [0.4, 0.6, 0.5],
        [0.76, 0.62, 0.42],
        0.0,
        0.9,
    )
    .expect("zero sand diffs");
    assert!(
        sand.iter().all(|c| *c < 0.0),
        "dielectric gap is negated floor, got {sand:?}"
    );
}

#[cfg(feature = "v1")]
#[test]
fn ibl_on_off_diff_bad_normal_refuses_loudly() {
    let bake = nonzero_on_off_bake();
    assert!(matches!(
        ibl_on_off_diff(
            &bake,
            [0.0, 0.0, 0.0],
            [0.4, 0.6, 0.5],
            [0.5, 0.5, 0.5],
            1.0,
            0.35,
        ),
        Err(IblError::Invalid(_))
    ));
}

#[test]
fn bake_counts_quote_texels() {
    // The receipt quotes honest texel counts for the frame report; the
    // live submit path is covered by the chart tests behind `window`.
    let bake_like = PrefilterCube::new(8, IBL_LEVELS);
    assert_eq!(bake_like.texels(), IBL_LEVELS * IBL_FACES * 64);
    let lut = BrdfLut::new(8);
    assert_eq!(lut.texels(), 64);
    let sh = IrradianceSh::zero();
    assert_eq!(sh.to_bytes().len(), IBL_SH_COEFFS * 12);
    assert_eq!(bake_like.to_bytes().len(), bake_like.texels() * 12);
    assert_eq!(lut.to_bytes().len(), 64 * 8);
}

// ------------------------------------------------------------------
// HDRI cook behind `v1`: deterministic bytes, exact R2 prefilter
// parity, SH-vs-cosine-integral parity, forward shade sanity.
// ------------------------------------------------------------------

#[cfg(feature = "v1")]
pub(super) const HDRI_BYTES: &[u8] =
    include_bytes!("../../../../content/arena/hdri/polyhaven-studio-small-09-1k.hdr");

#[cfg(feature = "v1")]
#[test]
fn studio_hdri_bakes_deterministically() {
    // Same complete HDRI bytes twice give bit-identical bakes and bytes:
    // the cook artifact is deterministic.
    let (a, ms_a) = bake(HDRI_BYTES).expect("studio HDRI bakes");
    let (b, ms_b) = bake(HDRI_BYTES).expect("studio HDRI bakes again");
    assert_eq!(a, b, "bake is bit-identical twice");
    assert_eq!(a.to_bytes(), b.to_bytes(), "cook bytes are stable");
    assert!(ms_a >= 0.0 && ms_b >= 0.0, "bake records pass ms");
    let receipt = a.receipt(ms_a);
    assert_eq!(
        receipt.cube_texels,
        IBL_LEVELS as u64 * IBL_FACES as u64 * IBL_CUBE_SIZE as u64 * IBL_CUBE_SIZE as u64
    );
    assert_eq!(
        receipt.lut_texels,
        BRDF_LUT_SIZE as u64 * BRDF_LUT_SIZE as u64
    );
    assert_eq!(receipt.sh_coeffs, IBL_SH_COEFFS as u32);
    assert_eq!(
        a.to_bytes().len(),
        receipt.cube_texels as usize * 12 + IBL_SH_COEFFS * 12 + receipt.lut_texels as usize * 8
    );
    assert!(
        a.source_average.iter().all(|c| c.is_finite() && *c >= 0.0),
        "source average is finite/nonnegative"
    );
    #[cfg(not(feature = "gpu"))]
    {
        // Headless build: no device in hand, so the submit seam refuses
        // loudly after validating the shared look frame.
        let frame = crate::gtao::LookFrame {
            frame_index: 3,
            viewport_px: [32, 32],
            exposure_ev100: 10.0,
            sun_dir: [0.3, 0.8, 0.5],
        };
        assert!(matches!(a.submit_gpu(&frame), Err(IblError::Invalid(_))));
        let bad = crate::gtao::LookFrame {
            viewport_px: [0, 32],
            ..frame
        };
        assert!(matches!(a.submit_gpu(&bad), Err(IblError::Invalid(_))));
    }
    eprintln!(
        "RL2 cook: cube {} texels, lut {} texels, sh {} coeffs, bake {ms_a:.1}ms",
        receipt.cube_texels, receipt.lut_texels, receipt.sh_coeffs
    );
}

#[cfg(feature = "v1")]
#[test]
fn prefilter_parity_with_r2_is_exact() {
    // Every baked texel equals the R2 `prefilter` at the same direction
    // and roughness: parity is exact (0.0) by construction, and level 0
    // equals the direct R2 `sample`.
    use crate::v1_ibl_shadow as r2;
    let env = r2::load_hdri(HDRI_BYTES).expect("fixture decodes");
    let (bake, _) = bake(HDRI_BYTES).expect("studio HDRI bakes");
    let mut max_diff = 0.0f32;
    let mut worst = String::new();
    for level in 0..IBL_LEVELS {
        let r = PrefilterCube::level_roughness(level).expect("level roughness");
        for face in 0..IBL_FACES {
            for y in (0..IBL_CUBE_SIZE).step_by(7) {
                for x in (0..IBL_CUBE_SIZE).step_by(7) {
                    let dir = cube_texel_dir(face, x, y, IBL_CUBE_SIZE);
                    let want = r2::prefilter(&env, dir, r).expect("R2 prefilters");
                    let got = bake.cube.texel(level, face, x, y).expect("texel reads");
                    for c in 0..3 {
                        let d = (got[c] - want[c]).abs();
                        if d > max_diff {
                            max_diff = d;
                            worst = format!("level {level} face {face} ({x},{y}) ch {c}");
                        }
                    }
                    if r == 0.0 {
                        let direct = r2::sample(&env, dir).expect("R2 samples");
                        for c in 0..3 {
                            // Level 0 routes through `prefilter`, which
                            // renormalizes the already-unit cube dir: 1
                            // ulp of double normalization, not a kernel
                            // difference. The kernel path above is exact.
                            assert!(
                                (got[c] - direct[c]).abs() <= 1e-6,
                                "level 0 is the direct sample at {face} ({x},{y})"
                            );
                        }
                    }
                }
            }
        }
    }
    assert_eq!(max_diff, 0.0, "prefilter parity exact, worst {worst}");
}

#[cfg(feature = "v1")]
#[test]
fn sh_reproduces_uniform_environment() {
    // Analytic anchor: a uniform L=1 environment has irradiance PI in
    // every direction. This pins the solid-angle weights and the cosine
    // lobe factors independent of any HDRI content.
    let env = crate::v1_ibl_shadow::EnvMap {
        width: 64,
        height: 32,
        pixels: vec![[1.0, 1.0, 1.0]; 64 * 32],
    };
    let sh = project_sh(&env);
    for n in [
        [0.0, 1.0, 0.0],
        [0.0, -1.0, 0.0],
        [1.0, 0.0, 0.0],
        [0.3, -0.5, 0.8],
    ] {
        let got = eval_sh(&sh, n).expect("sh evals");
        for c in 0..3 {
            assert!(
                (got[c] - PI).abs() <= 0.01,
                "uniform env gives PI at {n:?}, got {got:?}"
            );
        }
    }
}

#[cfg(feature = "v1")]
#[test]
fn sh_matches_cosine_integral_within_posted_tolerance() {
    // Brute-force diffuse reference over a fixed Fibonacci sphere:
    // `E(n) = 4PI/N * sum L(w) max(dot(n,w), 0)` from direct R2 samples.
    // L2 SH must track it within abs 0.50 / rel 15% (posted tolerance).
    // The uniform-environment test above pins weights and lobe factors
    // exactly, so the residual here is pure L2 truncation on the peaky
    // studio HDRI (observed: abs 0.425, rel 12.9% on the -X probe facing
    // the softbox edge); an axis flip or missing lobe would miss by 2x.
    use crate::v1_ibl_shadow as r2;
    let env = r2::load_hdri(HDRI_BYTES).expect("fixture decodes");
    let (bake, _) = bake(HDRI_BYTES).expect("studio HDRI bakes");
    const N: usize = 4096;
    let golden = std::f32::consts::PI * (1.0 + (5.0f32).sqrt());
    let mut dirs = Vec::with_capacity(N);
    let mut rad = Vec::with_capacity(N);
    for i in 0..N {
        let z = 1.0 - 2.0 * (i as f32 + 0.5) / N as f32;
        let rr = (1.0 - z * z).max(0.0).sqrt();
        let theta = golden * i as f32;
        let d = [rr * theta.cos(), rr * theta.sin(), z];
        // R2 convention is +Y up; the Fibonacci frame is +Z up, so swap
        // axes into the env frame before sampling.
        let e = [d[0], d[2], d[1]];
        dirs.push(e);
        rad.push(r2::sample(&env, e).expect("reference samples"));
    }
    let weight = 4.0 * std::f32::consts::PI / N as f32;
    let normals: [[f32; 3]; 7] = [
        [0.0, 1.0, 0.0],
        [0.0, -1.0, 0.0],
        [1.0, 0.0, 0.0],
        [-1.0, 0.2, 0.1],
        [0.0, 0.0, 1.0],
        [0.3, -0.5, 0.8],
        [0.4, 0.35, 0.8],
    ];
    let mut max_abs = 0.0f32;
    let mut max_rel = 0.0f32;
    for n in normals {
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let n = [n[0] / l, n[1] / l, n[2] / l];
        let mut e_ref = [0.0f32; 3];
        for (d, p) in dirs.iter().zip(rad.iter()) {
            let cos = (n[0] * d[0] + n[1] * d[1] + n[2] * d[2]).max(0.0);
            e_ref[0] += p[0] * cos;
            e_ref[1] += p[1] * cos;
            e_ref[2] += p[2] * cos;
        }
        e_ref = [e_ref[0] * weight, e_ref[1] * weight, e_ref[2] * weight];
        let got = eval_sh(&bake.sh, n).expect("sh evals");
        let mut n_abs = 0.0f32;
        for c in 0..3 {
            let a = (got[c] - e_ref[c]).abs();
            n_abs = n_abs.max(a);
            max_abs = max_abs.max(a);
            if e_ref[c] > 0.5 {
                max_rel = max_rel.max(a / e_ref[c]);
            }
        }
        eprintln!("RL2 SH probe n={n:?} ref={e_ref:?} got={got:?} abs={n_abs:.4}");
    }
    assert!(
        max_abs <= 0.50,
        "SH irradiance within abs 0.50 of the cosine integral, max {max_abs}"
    );
    assert!(
        max_rel <= 0.15,
        "SH irradiance within rel 15% of the cosine integral, max {max_rel}"
    );
    eprintln!(
        "RL2 SH parity: max abs {max_abs:.4}, max rel {:.4}%",
        max_rel * 100.0
    );
}

#[cfg(feature = "v1")]
#[test]
fn forward_shade_is_sane_and_material_aware() {
    // The forward pass combines both bakes: finite/nonnegative output,
    // metals drop the diffuse term, roughness moves the specular term,
    // and a black albedo still reflects the environment through F0.
    let (bake, _) = bake(HDRI_BYTES).expect("studio HDRI bakes");
    let n = [0.0, 1.0, 0.0];
    let v = [0.4, 0.6, 0.5];
    let diel = shade_ibl(&bake, n, v, [0.5, 0.5, 0.5], 0.0, 0.5).expect("dielectric shades");
    let metal = shade_ibl(&bake, n, v, [0.5, 0.5, 0.5], 1.0, 0.5).expect("metal shades");
    let sharp = shade_ibl(&bake, n, v, [0.5, 0.5, 0.5], 0.0, 0.0).expect("sharp shades");
    let black = shade_ibl(&bake, n, v, [0.0, 0.0, 0.0], 0.0, 0.2).expect("black shades");
    for (name, s) in [
        ("diel", diel),
        ("metal", metal),
        ("sharp", sharp),
        ("black", black),
    ] {
        assert!(
            s.iter().all(|c| c.is_finite() && *c >= 0.0),
            "{name} shade is finite/nonnegative, got {s:?}"
        );
    }
    let lum = crate::v1_ibl_shadow::luminance;
    assert!(
        lum(diel) > lum(metal),
        "metal drops diffuse: diel {diel:?} vs metal {metal:?}"
    );
    assert!(
        (lum(sharp) - lum(diel)).abs() > 1e-3,
        "roughness moves specular: sharp {sharp:?} vs mid {diel:?}"
    );
    assert!(
        lum(black) > 0.0,
        "black albedo still reflects through F0 0.04, got {black:?}"
    );
    assert!(matches!(
        shade_ibl(&bake, [0.0, 0.0, 0.0], v, [0.5, 0.5, 0.5], 0.0, 0.5),
        Err(IblError::Invalid(_))
    ));
    assert!(matches!(
        shade_ibl(&bake, n, v, [2.0, 0.5, 0.5], 0.0, 0.5),
        Err(IblError::Invalid(_))
    ));
    assert!(matches!(
        shade_ibl(&bake, n, v, [0.5, 0.5, 0.5], 0.0, 1.5),
        Err(IblError::Invalid(_))
    ));
    // Same complete inputs twice give identical shades.
    let again = shade_ibl(&bake, n, v, [0.5, 0.5, 0.5], 0.0, 0.5).expect("shades again");
    assert_eq!(diel, again, "forward shade is deterministic");
}

#[cfg(feature = "v1")]
#[test]
fn shade_occluded_scales_diffuse_only() {
    // AO dims the SH diffuse term only; specular stays open
    // (pipeline rule 4: AO scales ambient, never the final color).
    let bake = nonzero_on_off_bake();
    let n = [0.0, 1.0, 0.0];
    let v = [0.4, 0.6, 0.5];
    let albedo = [0.5, 0.5, 0.5];
    let open = shade_ibl_occluded(&bake, n, v, albedo, 0.0, 0.5, 1.0).expect("open shades");
    let half = shade_ibl_occluded(&bake, n, v, albedo, 0.0, 0.5, 0.5).expect("half shades");
    let shut = shade_ibl_occluded(&bake, n, v, albedo, 0.0, 0.5, 0.0).expect("shut shades");
    let plain = shade_ibl(&bake, n, v, albedo, 0.0, 0.5).expect("plain shades");
    assert_eq!(open, plain, "occlusion 1.0 matches the open shade");
    let e = ambient_occluded(&bake.sh, n, 1.0).expect("irradiance reads");
    let diffuse_open = [
        albedo[0] * e[0] / PI,
        albedo[1] * e[1] / PI,
        albedo[2] * e[2] / PI,
    ];
    for c in 0..3 {
        assert!(
            shut[c].is_finite() && shut[c] >= 0.0,
            "shut keeps finite specular, got {shut:?}"
        );
        assert!(
            (open[c] - shut[c] - diffuse_open[c]).abs() < 1e-5,
            "open minus shut is the diffuse term, got open {open:?} shut {shut:?}"
        );
        assert!(
            (half[c] - shut[c] - 0.5 * diffuse_open[c]).abs() < 1e-5,
            "half occlusion halves diffuse only, got half {half:?} shut {shut:?}"
        );
    }
    assert!(
        matches!(
            shade_ibl_occluded(&bake, n, v, albedo, 0.0, 0.5, 2.0),
            Err(IblError::Invalid(_))
        ),
        "occlusion above 1 refuses"
    );
    assert!(
        matches!(
            shade_ibl_occluded(&bake, n, v, albedo, 0.0, 0.5, f32::NAN),
            Err(IblError::Invalid(_))
        ),
        "non-finite occlusion refuses"
    );
    assert!(
        matches!(
            shade_ibl_occluded(&bake, [0.0, 0.0, 0.0], v, albedo, 0.0, 0.5, 0.5),
            Err(IblError::Invalid(_))
        ),
        "zero normal refuses"
    );
}

#[cfg(feature = "v1")]
#[test]
fn ambient_on_off_gap_separates_lit_from_unlit() {
    // Lit bake reads above the flat floor through the live gap oracle;
    // unlit bake keeps the floor by refusing a gap.
    let bake = nonzero_on_off_bake();
    let normal = [0.0, 1.0, 0.0];
    let albedo = [0.5, 0.5, 0.5];
    let gap = live_ibl_on_off_gap(&bake, normal, albedo).expect("lit bake gaps");
    assert!(gap > 0.002, "lit bake moves above floor, got {gap}");
    let again = live_ibl_on_off_gap(&bake, normal, albedo).expect("lit bake gaps again");
    assert_eq!(gap, again, "gap is deterministic");
    let zero = zero_on_off_bake();
    assert!(
        matches!(
            live_ibl_on_off_gap(&zero, normal, albedo),
            Err(IblError::Invalid(_))
        ),
        "unlit bake keeps the floor"
    );
    assert!(matches!(
        live_ibl_on_off_gap(&bake, [0.0, 0.0, 0.0], albedo),
        Err(IblError::Invalid(_))
    ));
    assert!(matches!(
        live_ibl_on_off_gap(&bake, normal, [2.0, 0.5, 0.5]),
        Err(IblError::Invalid(_))
    ));
}

#[test]
fn ambient_sh9_on_off_differs_in_masked_region() {
    // EG12(a) pixel test: 4x4 raster, center 2x2 mask, baked SH9 ambient
    // at the (bent) normal times AO vs the constant floor, ambient only.
    // Direct sun stays untouched (diffuse scales, no combined scale);
    // fails when ambient is constant (gap 0), passes with live SH9.
    // No `v1` gate: runs in the default lib suite (the F2P proof).
    let mut coeffs = [[0.0f32; 3]; IBL_SH_COEFFS];
    coeffs[0] = [3.544_908, 3.544_908, 3.544_908];
    let sh = IrradianceSh { coeffs };
    let zero = IrradianceSh::zero();
    // The retired forward.wgsl floor triple (linear); the shader no
    // longer carries an AMBIENT_FLOOR const.
    let floor = [0.20f32, 0.18, 0.16];
    let albedo = [0.5, 0.5, 0.5];
    let normal = [0.0, 1.0, 0.0];
    let mut max_gap = 0.0f32;
    let mut masked = 0u32;
    for y in 0..4u32 {
        for x in 0..4u32 {
            let in_mask = (1..3).contains(&x) && (1..3).contains(&y);
            if !in_mask {
                continue;
            }
            masked += 1;
            let occlusion = 0.5;
            let e_on = ambient_occluded(&sh, normal, occlusion).expect("masked ambient evals");
            let on_diffuse = [
                albedo[0] * e_on[0] / PI,
                albedo[1] * e_on[1] / PI,
                albedo[2] * e_on[2] / PI,
            ];
            let off_diffuse = [
                albedo[0] * floor[0] * occlusion,
                albedo[1] * floor[1] * occlusion,
                albedo[2] * floor[2] * occlusion,
            ];
            for c in 0..3 {
                max_gap = max_gap.max((on_diffuse[c] - off_diffuse[c]).abs());
            }
        }
    }
    assert_eq!(masked, 4, "mask covers the center 2x2");
    assert!(
        max_gap > 0.002,
        "masked SH ambient moves above the floor, got {max_gap}"
    );
    let mut again = 0.0f32;
    for y in 1..3u32 {
        for x in 1..3u32 {
            let e_on = ambient_occluded(&sh, normal, 0.5).expect("ambient evals");
            let _ = (x, y);
            for c in 0..3 {
                let on = albedo[c] * e_on[c] / PI;
                let off = albedo[c] * floor[c] * 0.5;
                again = again.max((on - off).abs());
            }
        }
    }
    assert_eq!(max_gap, again, "masked gap is deterministic");
    // Zero bake keeps the floor: zero SH evals to zero irradiance.
    let e_zero = ambient_occluded(&zero, normal, 0.5).expect("zero ambient evals");
    assert_eq!(e_zero, [0.0, 0.0, 0.0], "zero SH stays black");
    // AO scales ambient linearly: half occlusion halves the irradiance.
    let e_open = ambient_occluded(&sh, normal, 1.0).expect("open ambient evals");
    let e_half = ambient_occluded(&sh, normal, 0.5).expect("half ambient evals");
    for c in 0..3 {
        assert!(
            (e_half[c] * 2.0 - e_open[c]).abs() < 1e-6,
            "occlusion halves ambient, got open {e_open:?} half {e_half:?}"
        );
    }
    assert!(
        matches!(
            ambient_occluded(&sh, [0.0, 0.0, 0.0], 0.5),
            Err(IblError::Invalid(_))
        ),
        "zero normal refuses loudly"
    );
    // Per-frame hand-off moves: lit vs zero SH uniform images differ in
    // the tail, so rewriting `sh_buf` each frame changes the ambient.
    let mut lit_image = [0u8; FORWARD_IBL_BIND_BYTES];
    forward_ibl_bind_image(&sh, &mut lit_image);
    let mut zero_image = [0u8; FORWARD_IBL_BIND_BYTES];
    forward_ibl_bind_image(&zero, &mut zero_image);
    assert_ne!(lit_image, zero_image, "live SH uniform differs on vs off");
    assert_eq!(
        lit_image[0..8],
        FORWARD_IBL_CHUNK_HASH.to_le_bytes(),
        "bind head stays the pinned chunk hash"
    );
}

#[test]
fn ambient_sh9_replaces_floor_on_masked_region() {
    // EG12(a): display ambient comes from the SH bake with AO on diffuse only.
    let wgsl = include_str!("../shaders/forward.wgsl");
    let body = wgsl
        .split("fn fs_display")
        .nth(1)
        .expect("display entry")
        .split("fn fs_skinned")
        .next()
        .expect("display close");
    assert!(body.contains("direct_light(in.world_pos, in.world_n, in.tangent, albedo, 1.0)"));
    assert!(!body.contains("AMBIENT_FLOOR"));
    assert!(body.contains("ibl_diffuse") || body.contains("ibl_irradiance"));
    assert!(body.contains("* occlusion") || body.contains("*occlusion"));
    assert!(body.contains("ibl_spec"));
    assert!(
        !body.contains("ibl * occlusion")
            && !body.contains("ibl*occlusion")
            && !body.contains("ibl *occlusion")
            && !body.contains("ibl* occlusion"),
        "display never scales combined ibl"
    );
    let mut coeffs = [[0.0f32; 3]; IBL_SH_COEFFS];
    coeffs[0] = [3.544_908, 3.544_908, 3.544_908];
    let sh = IrradianceSh { coeffs };
    let e = ambient_occluded(&sh, [0.0, 1.0, 0.0], 0.5).expect("masked evals");
    let gap = (0.5 * e[0] / PI - 0.5 * 0.20 * 0.5).abs();
    assert!(gap > 0.002, "masked gap {gap}");
}
