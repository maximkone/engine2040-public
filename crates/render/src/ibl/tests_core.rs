//! tests_core: cube sampling, BRDF LUT, SH basis and the forward bind bytes.

#[cfg(feature = "v1")]
use super::tests_look::{nonzero_on_off_bake, zero_on_off_bake};
use super::*;

#[test]
fn cube_texel_dirs_are_unit_and_face_centers_hit_axes() {
    // Face centers reproduce the axis directions, and every texel dir is
    // unit: the bake basis covers the sphere with no degenerate taps.
    let size = 8u32;
    let centers = [
        (0, [1.0, 0.0, 0.0]),
        (1, [-1.0, 0.0, 0.0]),
        (2, [0.0, 1.0, 0.0]),
        (3, [0.0, -1.0, 0.0]),
        (4, [0.0, 0.0, 1.0]),
        (5, [0.0, 0.0, -1.0]),
    ];
    for (face, axis) in centers {
        for y in 0..size {
            for x in 0..size {
                let d = cube_texel_dir(face, x, y, size);
                let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                assert!((l - 1.0).abs() < 1e-6, "texel dir is unit, got {d:?}");
            }
        }
        // Even-sized faces straddle the axis: the 2x2 center quad must
        // agree with the axis on the dominant component.
        let h = size / 2;
        for (x, y) in [(h - 1, h - 1), (h, h - 1), (h - 1, h), (h, h)] {
            let d = cube_texel_dir(face, x, y, size);
            let dom = if axis[0] != 0.0 {
                d[0] * axis[0]
            } else if axis[1] != 0.0 {
                d[1] * axis[1]
            } else {
                d[2] * axis[2]
            };
            assert!(
                dom > 0.9,
                "face {face} center quad points along {axis:?}, got {d:?}"
            );
        }
    }
    assert_eq!(PrefilterCube::level_roughness(0).expect("level 0"), 0.0);
    assert_eq!(PrefilterCube::level_roughness(4).expect("level 4"), 1.0);
    assert!(matches!(
        PrefilterCube::level_roughness(5),
        Err(IblError::Invalid(_))
    ));
}

#[test]
fn cube_sample_roundtrips_texel_centers_exactly() {
    // Sampling a texel-center direction reproduces the stored texel: the
    // face/UV inverse is exact, so the forward pass reads what cook wrote.
    let mut cube = PrefilterCube::new(8, IBL_LEVELS);
    for (i, t) in cube.faces.iter_mut().enumerate() {
        let f = (i as f32 * 0.37).fract();
        *t = [f, 1.0 - f, 0.25 + f * 0.5];
    }
    let mut max_diff = 0.0f32;
    for level in [0, 2, 4] {
        let r = PrefilterCube::level_roughness(level).expect("level roughness");
        for face in 0..IBL_FACES {
            for (x, y) in [(0, 0), (7, 0), (0, 7), (7, 7), (3, 4), (5, 2)] {
                let dir = cube_texel_dir(face, x, y, 8);
                let got = sample_cube(&cube, dir, r).expect("center samples");
                let want = cube.texel(level, face, x, y).expect("texel reads");
                for c in 0..3 {
                    max_diff = max_diff.max((got[c] - want[c]).abs());
                }
            }
        }
    }
    assert_eq!(max_diff, 0.0, "texel centers round-trip exactly");
}

#[test]
fn cube_sample_rejects_bad_inputs() {
    let cube = PrefilterCube::new(8, IBL_LEVELS);
    assert!(matches!(
        sample_cube(&cube, [f32::NAN, 0.0, 0.0], 0.0),
        Err(IblError::Invalid(_))
    ));
    assert!(matches!(
        sample_cube(&cube, [0.0, 0.0, 0.0], 0.0),
        Err(IblError::Invalid(_))
    ));
    assert!(matches!(
        sample_cube(&cube, [0.0, 1.0, 0.0], 2.0),
        Err(IblError::Invalid(_))
    ));
    assert!(matches!(
        sample_cube(&cube, [0.0, 1.0, 0.0], f32::NAN),
        Err(IblError::Invalid(_))
    ));
    assert!(matches!(cube.texel(0, 0, 8, 0), Err(IblError::Invalid(_))));
    assert!(matches!(cube.texel(0, 6, 0, 0), Err(IblError::Invalid(_))));
    assert!(matches!(cube.texel(9, 0, 0, 0), Err(IblError::Invalid(_))));
}

#[test]
fn brdf_lut_is_deterministic_with_sane_corners() {
    // Same inputs twice give bit-identical LUTs; corners pin the
    // split-sum shape: smooth dielectric at normal incidence reflects
    // ~fully (scale near 1, bias near 0), grazing angles bias upward,
    // and rough surfaces spread energy out of the scale term.
    let (a, b) = (
        brdf_lut_build(16).expect("lut builds"),
        brdf_lut_build(16).expect("lut builds"),
    );
    assert_eq!(a, b, "brdf bake is bit-identical twice");
    assert_eq!(a.to_bytes(), b.to_bytes(), "lut bytes are stable");
    let smooth_normal = a.texel(15, 0).expect("corner reads");
    assert!(
        (smooth_normal[0] - 1.0).abs() < 0.05 && smooth_normal[1].abs() < 0.05,
        "smooth normal-incidence keeps energy in scale, got {smooth_normal:?}"
    );
    let grazing = a.texel(0, 0).expect("grazing reads");
    assert!(
        grazing[1] > 0.3,
        "grazing angles carry fresnel bias, got {grazing:?}"
    );
    let rough = a.texel(15, 15).expect("rough reads");
    assert!(
        rough[0] < smooth_normal[0],
        "roughness spreads energy out of scale: smooth {smooth_normal:?}, rough {rough:?}"
    );
    for t in &a.data {
        assert!(
            t.iter().all(|c| c.is_finite() && *c >= -0.01 && *c <= 1.01),
            "lut stays in range, got {t:?}"
        );
    }
    assert!(matches!(brdf_lut_build(0), Err(IblError::Invalid(_))));
    assert!(matches!(
        brdf_sample(&a, 2.0, 0.5),
        Err(IblError::Invalid(_))
    ));
    assert!(matches!(
        brdf_sample(&a, 0.5, f32::NAN),
        Err(IblError::Invalid(_))
    ));
}

#[test]
fn brdf_sample_matches_direct_integration() {
    // Bilinear sampling reproduces the integral it was baked from: cell
    // centers match to 1e-3, off-grid points to 1e-2 (the LUT is smooth).
    let lut = brdf_lut_build(32).expect("lut builds");
    let mut max_center = 0.0f32;
    for (x, y) in [(0, 0), (31, 0), (0, 31), (31, 31), (16, 16), (7, 23)] {
        let ndotv = (x as f32 + 0.5) / 32.0;
        let r = (y as f32 + 0.5) / 32.0;
        let direct = brdf_integrate(ndotv, r).expect("direct integrates");
        let got = brdf_sample(&lut, ndotv, r).expect("center samples");
        max_center = max_center.max((got[0] - direct[0]).abs());
        max_center = max_center.max((got[1] - direct[1]).abs());
    }
    assert!(
        max_center <= 1e-3,
        "cell centers match within 1e-3, got {max_center}"
    );
    let mut max_off = 0.0f32;
    for (ndotv, r) in [(0.33, 0.17), (0.9, 0.62), (0.05, 0.95), (0.71, 0.29)] {
        let direct = brdf_integrate(ndotv, r).expect("direct integrates");
        let got = brdf_sample(&lut, ndotv, r).expect("point samples");
        max_off = max_off.max((got[0] - direct[0]).abs());
        max_off = max_off.max((got[1] - direct[1]).abs());
    }
    assert!(max_off <= 1e-2, "off-grid within 1e-2, got {max_off}");
}

#[test]
fn sh_eval_rejects_bad_normals() {
    let sh = IrradianceSh::zero();
    let n = [0.0, 1.0, 0.0];
    assert_eq!(
        eval_sh(&sh, n),
        Ok([0.0, 0.0, 0.0]),
        "zero coefficients give zero irradiance"
    );
    assert!(matches!(
        eval_sh(&sh, [0.0, 0.0, 0.0]),
        Err(IblError::Invalid(_))
    ));
    assert!(matches!(
        eval_sh(&sh, [f32::INFINITY, 0.0, 0.0]),
        Err(IblError::Invalid(_))
    ));
    assert!(matches!(
        eval_sh(&sh, [1.0, 1.0, f32::NAN]),
        Err(IblError::Invalid(_))
    ));
}

/// Studio SH fixture for the bind lock: varied magnitudes per band and
/// per channel, so packing swaps or truncations move the bytes.
fn bind_lock_sh() -> IrradianceSh {
    IrradianceSh {
        coeffs: [
            [0.50, 0.40, 0.30],
            [0.10, -0.20, 0.05],
            [-0.15, 0.25, 0.35],
            [0.05, 0.10, -0.10],
            [0.20, 0.05, 0.15],
            [-0.05, 0.30, -0.25],
            [0.45, -0.35, 0.20],
            [0.12, 0.22, -0.02],
            [-0.28, 0.18, 0.08],
        ],
    }
}

#[test]
fn forward_ibl_chunk_is_pinned_for_the_forward_bind() {
    // The bound chunk is exactly the reviewed bytes: the hash pins drift,
    // the shape asserts pin silent rebinding (a stub chunk would miss the
    // entries, a renumbered bind group would miss the bindings).
    assert_eq!(
        super::forward_ibl_chunk_probe(),
        FORWARD_IBL_CHUNK_HASH,
        "forward chunk drifted without updating the pin"
    );
    assert!(!FORWARD_IBL_WGSL.is_empty(), "forward chunk ships");
    for binding in [
        "@group(1) @binding(0)",
        "@group(1) @binding(1)",
        "@group(1) @binding(2)",
        "@group(1) @binding(3)",
    ] {
        assert!(
            FORWARD_IBL_WGSL.contains(binding),
            "forward bind carries {binding}"
        );
    }
    for entry in [
        "ibl_sample",
        "ibl_irradiance",
        "ibl_sh_basis",
        "array<vec4<f32>, 9>",
        "textureSampleLevel",
    ] {
        assert!(
            FORWARD_IBL_WGSL.contains(entry),
            "forward chunk keeps {entry}"
        );
    }
    // The basis literals the GPU dots are named here; the oracle test
    // below pins their values against pi, so a flipped sign fails there.
    for literal in ["0.282095", "0.488603", "1.092548", "0.315392", "0.546274"] {
        assert!(
            FORWARD_IBL_WGSL.contains(literal),
            "forward chunk carries basis {literal}"
        );
    }
}

#[test]
fn forward_ibl_bind_image_carries_hash_and_sh_bytes() {
    // Compiled forward bind: hash head plus SH tail, bit-exact pack into
    // caller-owned storage (steady-state repacks grow nothing).
    let sh = bind_lock_sh();
    let mut image = [0u8; FORWARD_IBL_BIND_BYTES];
    forward_ibl_bind_image(&sh, &mut image);
    let mut head = [0u8; 8];
    head.copy_from_slice(&image[0..8]);
    assert_eq!(
        head,
        FORWARD_IBL_CHUNK_HASH.to_le_bytes(),
        "bind head is the pinned chunk hash"
    );
    let mut packed = [0u8; FORWARD_IBL_SH_UNIFORM_BYTES];
    sh_uniform_bytes_into(&sh, &mut packed);
    assert_eq!(
        &image[8..FORWARD_IBL_BIND_BYTES],
        &packed[..],
        "bind tail is the SH uniform image"
    );
    // Layout: RGB little-endian floats with zero w pads; decode
    // round-trips the coefficients exactly, so field swaps fail here.
    for (i, c) in sh.coeffs.iter().enumerate() {
        let base = 8 + i * 16;
        for (lane, v) in c.iter().enumerate() {
            let bytes: [u8; 4] = image[base + lane * 4..base + lane * 4 + 4]
                .try_into()
                .expect("uniform lanes are 4 bytes wide");
            assert_eq!(
                f32::from_le_bytes(bytes),
                *v,
                "coefficient {i} lane {lane} round-trips"
            );
        }
        assert_eq!(
            &image[base + 12..base + 16],
            &[0, 0, 0, 0],
            "coefficient {i} w pad stays zero"
        );
    }
    // Deterministic twice, sensitive once, zero baseline last.
    let mut again = [0u8; FORWARD_IBL_BIND_BYTES];
    forward_ibl_bind_image(&sh, &mut again);
    assert_eq!(image, again, "same SH binds the same bytes twice");
    let mut zero = [0u8; FORWARD_IBL_BIND_BYTES];
    forward_ibl_bind_image(&IrradianceSh::zero(), &mut zero);
    let mut zero_head = [0u8; 8];
    zero_head.copy_from_slice(&zero[0..8]);
    assert_eq!(
        zero_head,
        FORWARD_IBL_CHUNK_HASH.to_le_bytes(),
        "zero SH keeps the chunk hash head"
    );
    assert!(
        zero[8..].iter().all(|b| *b == 0),
        "zero SH packs a zero tail"
    );
    assert_ne!(image, zero, "distinct SH binds distinctly");
}

/// L2 real-SH basis from pi only (independent of the f32 literals in
/// [`crate::gi::sh_basis`] and the WGSL chunk): order L00, L1y, L10,
/// L11, L2-2, L2-1, L20, L21, L22.
fn basis_oracle(x: f64, y: f64, z: f64) -> [f64; 9] {
    use std::f64::consts::PI;
    let s1 = 0.5 * (1.0 / PI).sqrt();
    let s3 = (3.0 / (4.0 * PI)).sqrt();
    let s15 = 0.5 * (15.0 / PI).sqrt();
    let s5 = 0.25 * (5.0 / PI).sqrt();
    let s15q = 0.25 * (15.0 / PI).sqrt();
    [
        s1,
        -s3 * y,
        s3 * z,
        -s3 * x,
        s15 * x * y,
        -s15 * y * z,
        s5 * (3.0 * z * z - 1.0),
        -s15 * x * z,
        s15q * (x * x - y * y),
    ]
}

#[test]
fn forward_sh_matches_independent_oracle_within_gpu_tol() {
    // Headless parity leg for the forward bind: the CPU SH the bind
    // carries tracks an independent f64 oracle within the posted 2e-5
    // GPU-fusing budget (reference-GPU chart observation: shade abs
    // 0.00002), so a flipped basis sign or a dropped band fails here
    // before any device.
    const TOL: f32 = 0.00002;
    let sh = bind_lock_sh();
    let normals: [[f32; 3]; 7] = [
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
        [0.3, -0.5, 0.8],
        [0.4, 0.35, 0.8],
        [-1.0, 0.2, 0.1],
        [0.2, 0.9, -0.4],
    ];
    let mut max_abs = 0.0f32;
    for n in normals {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        let n = [n[0] / len, n[1] / len, n[2] / len];
        let got = eval_sh(&sh, n).expect("fixture SH evals");
        // WGSL-order f32 emulation: sequential `e += sh[i] * b[i]` per
        // channel, clamped like `ibl_irradiance`. `eval_sh` renormalizes
        // once more inside `sh_irradiance`, so this trails by float
        // rounding (~1e-7); the 2e-5 oracle gate below is the budget.
        let basis = crate::gi::sh_basis(n);
        let mut emu = [0.0f32; 3];
        for (coeff, b) in sh.coeffs.iter().zip(basis.iter()) {
            for (slot, v) in emu.iter_mut().zip(coeff.iter()) {
                *slot += *v * *b;
            }
        }
        emu = [emu[0].max(0.0), emu[1].max(0.0), emu[2].max(0.0)];
        let mut emu_abs = 0.0f32;
        for (e, g) in emu.iter().zip(got.iter()) {
            emu_abs = emu_abs.max((*e - *g).abs());
        }
        assert!(
            emu_abs <= 1e-6,
            "WGSL-order accumulation matches eval at {n:?}, worst {emu_abs}"
        );
        let (x, y, z) = (f64::from(n[0]), f64::from(n[1]), f64::from(n[2]));
        let oracle = basis_oracle(x, y, z);
        let mut exact = [0.0f64; 3];
        for (coeff, o) in sh.coeffs.iter().zip(oracle.iter()) {
            for (slot, v) in exact.iter_mut().zip(coeff.iter()) {
                *slot += f64::from(*v) * *o;
            }
        }
        for (g, e) in got.iter().zip(exact.iter()) {
            let e = e.max(0.0);
            max_abs = max_abs.max((*g - e as f32).abs());
        }
    }
    assert!(
        max_abs <= TOL,
        "forward SH within abs {TOL} of the oracle, worst {max_abs}"
    );
    eprintln!("GX12 forward SH parity: max abs {max_abs:.7} within {TOL}");
}

#[test]
fn ibl_bake_hands_bake_resources_to_forward() {
    // Unwired is FAIL: the compiled bake -> forward graph hands every
    // bake resource to forward with a Transition barrier, forward reads
    // all three, bake runs first, and the receipt keeps both passes'
    // draws.
    let compiled = compile_ibl_for_forward().expect("ibl wiring compiles");
    assert!(
        compiled.order_names_match(&[IBL_BAKE_PASS, IBL_FORWARD_PASS]),
        "bake runs before forward"
    );
    let (bake_id, forward_id) = (compiled.order()[0], compiled.order()[1]);
    assert_eq!(
        compiled.pass(bake_id).expect("dense ids").name.as_str(),
        IBL_BAKE_PASS
    );
    assert_eq!(
        compiled.pass(forward_id).expect("dense ids").name.as_str(),
        IBL_FORWARD_PASS
    );
    for resource in FORWARD_IBL_INPUTS {
        assert!(
            compiled.barriers().iter().any(|b| {
                b.resource == resource
                    && b.kind == crate::graph::BarrierKind::Transition
                    && b.from == bake_id
                    && b.to == forward_id
            }),
            "{resource} hands over to forward"
        );
    }
    let forward = compiled.pass(forward_id).expect("forward reads");
    for resource in FORWARD_IBL_INPUTS {
        assert!(
            forward.inputs.iter().any(|i| i == resource),
            "forward reads {resource}: {:?}",
            forward.inputs
        );
    }
    let mut compiled = compiled;
    let receipt = compiled.execute(&crate::graph::FrameInfo {
        frame_index: 7,
        width_px: 16,
        height_px: 16,
    });
    assert_eq!(receipt.draws, 3, "both passes record draws");
    assert_eq!(receipt.triangles, 10, "both passes record triangles");
    assert_eq!(receipt.per_pass_cpu_ms.len(), 2);
}

#[test]
fn chart_target_downscales_window_viewport_within_edge() {
    // The release window (1600x900) must submit a chart inside the edge
    // instead of failing the frame: uniform downscale, aspect preserved.
    assert_eq!(
        chart_target_size(32, 32).expect("test chart fits"),
        (32, 32),
        "fitting viewports pass through untouched"
    );
    let window = chart_target_size(1600, 900).expect("window downscales");
    assert_eq!(window, (256, 144), "1600x900 downscales to 256x144");
    assert!(
        window.0 <= IBL_CHART_MAX_EDGE && window.1 <= IBL_CHART_MAX_EDGE,
        "chart target stays within the edge, got {window:?}"
    );
    let wide = u64::from(window.0) * u64::from(900u32);
    let high = u64::from(window.1) * u64::from(1600u32);
    assert_eq!(wide, high, "downscale preserves aspect, got {window:?}");
    assert_eq!(
        chart_target_size(512, 32).expect("wide bar downscales"),
        (256, 16),
        "long edge lands exactly on the cap"
    );
    assert_eq!(
        chart_target_size(900, 1600).expect("portrait downscales"),
        (144, 256),
        "portrait keeps the tall edge on the cap"
    );
    assert_eq!(
        chart_target_size(2048, 2048).expect("full look downscales"),
        (256, 256),
        "largest look viewport still fits"
    );
    assert_eq!(
        chart_target_size(1600, 900).expect("deterministic"),
        window,
        "same viewport sizes the same target twice"
    );
    assert!(
        matches!(chart_target_size(0, 32), Err(IblError::Invalid(_))),
        "zero width refuses loudly"
    );
    assert!(
        matches!(chart_target_size(1600, 0), Err(IblError::Invalid(_))),
        "zero height refuses loudly"
    );
}

#[test]
fn chart_target_guard_refuses_over_edge_directly() {
    // Direct target sizes over the edge never reach a texture: the guard
    // refuses loudly with the cap named, so the presenter scope stays
    // clean.
    assert!(check_chart_target_size(32, 32).is_ok());
    assert!(check_chart_target_size(256, 144).is_ok());
    match check_chart_target_size(512, 32) {
        Err(IblError::Invalid(detail)) => {
            assert!(detail.contains("exceeds"), "names the cap: {detail}");
        }
        other => panic!("512x32 target must refuse, got {other:?}"),
    }
    match check_chart_target_size(1600, 900) {
        Err(IblError::Invalid(detail)) => {
            assert!(detail.contains("exceeds"), "names the cap: {detail}");
        }
        other => panic!("1600x900 target must refuse, got {other:?}"),
    }
    assert!(
        matches!(check_chart_target_size(0, 32), Err(IblError::Invalid(_))),
        "zero target refuses loudly"
    );
    assert!(
        matches!(check_chart_target_size(256, 257), Err(IblError::Invalid(_))),
        "one pixel over the edge refuses loudly"
    );
}

#[test]
fn forward_bind_assert_accepts_chunk_and_rejects_plain() {
    // The wiring gate accepts a forward shader that inlines the chunk
    // and refuses one that only ships it: a missing marker names itself
    // instead of passing silently.
    let stitched = FORWARD_IBL_MARKERS.join("\n");
    assert!(
        assert_forward_wgsl_binds_ibl(&stitched).is_ok(),
        "all five markers together bind the bake"
    );
    match assert_forward_wgsl_binds_ibl("fn fs() {}") {
        Err(IblError::Invalid(detail)) => {
            assert!(
                detail.contains("ibl_prefilter"),
                "names the first missing marker: {detail}"
            );
        }
        other => panic!("plain forward must refuse, got {other:?}"),
    }
    match assert_forward_wgsl_binds_ibl("") {
        Err(IblError::Invalid(detail)) => {
            assert!(detail.contains("inline"), "points at the chunk: {detail}");
        }
        other => panic!("empty forward must refuse, got {other:?}"),
    }
    let partial = stitched.replace("ibl_sample", "ibl_missing");
    match assert_forward_wgsl_binds_ibl(&partial) {
        Err(IblError::Invalid(detail)) => {
            assert!(
                detail.contains("ibl_sample"),
                "names the dropped entry point: {detail}"
            );
        }
        other => panic!("partial forward must refuse, got {other:?}"),
    }
}

#[test]
fn forward_ibl_live_replaces_ambient_floor() {
    // Live forward samples the bake every frame: the shipped forward
    // file carries every bind marker and its `fs` calls the sampling
    // entry with no flat floor term left.
    let wgsl = include_str!("../shaders/forward.wgsl");
    assert!(
        assert_forward_wgsl_binds_ibl(wgsl).is_ok(),
        "live forward binds the bake"
    );
    for marker in FORWARD_IBL_MARKERS {
        assert!(wgsl.contains(marker), "live forward carries {marker}");
    }
    let start = wgsl.find("fn fs(in: VOut)").expect("live forward has fs");
    let rest = &wgsl[start..];
    let end = super::fs_close(rest).expect("fs closes");
    let body = &rest[..end];
    assert!(
        body.contains("ibl_sample") || (body.contains("ibl_diffuse") && body.contains("ibl_spec")),
        "fs samples the bake via ibl_sample or the diffuse/specular split"
    );
    assert!(
        !body.contains("AMBIENT_FLOOR"),
        "fs keeps no flat floor term"
    );
    assert!(
        !body.contains("diffuse_color_term(albedo, mat.metallic) * AMBIENT_FLOOR"),
        "flat add is gone from fs"
    );
}

#[test]
fn forward_ibl_diffuse_only_occlusion() {
    // EG12: window fs scales baked diffuse by occlusion, specular open.
    let wgsl = include_str!("../shaders/forward.wgsl");
    assert!(
        assert_forward_ibl_scales_diffuse_only(wgsl).is_ok(),
        "live fs scales diffuse only"
    );
    let bad = "fn fs(in: VOut) -> FsOut {\n let ibl = ibl_sample(n, v, f0, r, a, m);\n let o = 1.0;\n let lit = ibl * occlusion;\n}\n";
    match assert_forward_ibl_scales_diffuse_only(bad) {
        Err(IblError::Invalid(detail)) => {
            assert!(detail.contains("combined ibl"), "names whole-ibl: {detail}");
        }
        other => panic!("whole-ibl fixture must refuse, got {other:?}"),
    }
}

#[test]
fn live_ibl_samples_bake_only_when_bound_and_nonzero() {
    // Decision rule only: unbound keeps the floor even with a bright
    // bake; a zero bake keeps the floor even when bound.
    assert!(!live_ibl_should_sample(false, true));
    assert!(!live_ibl_should_sample(true, false));
    assert!(live_ibl_should_sample(true, true));
    // LF2 accept: a live 64x64 encode floors to 1 us so the per-pass
    // table shows `ibl` above 0 ms with 0 validation errors; zero-size
    // stays skipped at 0. CPU-only, no adapter, deterministic.
    let live = live_ibl_receipt(0.0, 64, 64);
    assert_eq!(live.0, 0, "live IBL rides forward; chart count stays 0");
    assert_eq!(live.1, 0, "live IBL keeps 0 validation errors");
    assert_eq!(live.2, 1, "live encode floors to 1 us, got {live:?}");
    let timed = live_ibl_receipt(0.42, 64, 64);
    assert!(timed.2 > 0, "timed live encode stays above 0 us");
    assert_eq!(live_ibl_receipt(0.42, 0, 64), (0, 0, 0));
    assert_eq!(live_ibl_receipt(0.42, 64, 0), (0, 0, 0));
    // The tuple feeds `FrameReceipt::per_pass_us` as `ibl.2`: a floored
    // 1 us formats above `0.000ms`, while skipped `(0, 0, 0)` is `0.000ms`.
    let live_ms = format!("{}.{:03}", live.2 / 1000, live.2 % 1000);
    assert_ne!(live_ms, "0.000", "live ibl reads above 0 ms");
    let skipped = live_ibl_receipt(0.42, 0, 64);
    let skipped_ms = format!("{}.{:03}", skipped.2 / 1000, skipped.2 % 1000);
    assert_eq!(skipped_ms, "0.000", "skipped ibl stays at 0 ms");
}

#[cfg(feature = "v1")]
#[test]
fn bake_has_light_separates_zero_from_lit() {
    // Missing accessor for the live decision: a lit bake samples, a
    // zero bake keeps the flat floor. Fails before the accessor lands.
    assert!(super::bake_has_light(&nonzero_on_off_bake()));
    assert!(!super::bake_has_light(&zero_on_off_bake()));
}

#[cfg(feature = "v1")]
#[test]
fn bake_has_light_needs_sh_and_cube() {
    // Fail-closed LF2 slice: a lit `source_average` alone must not sample;
    // the SH and the prefilter cube must carry light too, or the frame
    // keeps the flat floor. Fails before the hardening lands.
    let mut no_sh = nonzero_on_off_bake();
    no_sh.sh = super::IrradianceSh::zero();
    assert!(
        !super::bake_has_light(&no_sh),
        "lit source with zero SH must keep the floor"
    );
    let mut no_cube = nonzero_on_off_bake();
    no_cube.cube = super::PrefilterCube::new(4, super::IBL_LEVELS);
    assert!(
        !super::bake_has_light(&no_cube),
        "lit source with zero cube must keep the floor"
    );
}

#[test]
fn forward_ibl_skinned_replaces_ambient_floor() {
    // Skinned capture samples the live bake like the window frame: the
    // shipped forward file's `fs_skinned` calls the sampling entry with
    // no flat floor term left; magenta stays the missing-skin signal.
    let wgsl = include_str!("../shaders/forward.wgsl");
    let start = wgsl
        .find("fn fs_skinned(in: VOut)")
        .expect("skinned capture has fs_skinned");
    let rest = &wgsl[start..];
    let end = super::fs_close(rest).expect("fs_skinned closes");
    let body = &rest[..end];
    assert!(body.contains("ibl_sample"), "fs_skinned samples the bake");
    assert!(
        !body.contains("AMBIENT_FLOOR"),
        "fs_skinned keeps no flat floor term"
    );
    assert!(
        !body.contains("diffuse_color_term(albedo, mat.metallic) * AMBIENT_FLOOR"),
        "flat add is gone from fs_skinned"
    );
    assert!(
        body.contains("has_skin == 0u"),
        "fs_skinned keeps the missing-skin guard"
    );
    assert!(
        body.contains("1.0, 0.0, 1.0"),
        "missing skin still shades magenta"
    );
}

#[test]
fn forward_ibl_skinned_scales_diffuse_only() {
    // LF2 rule-4 parity for the skinned capture: baked diffuse scales by
    // occlusion, specular stays open (mirrors window `fs` plus CPU
    // `shade_ibl_occluded`). Fails before the split lands.
    let wgsl = include_str!("../shaders/forward.wgsl");
    let start = wgsl
        .find("fn fs_skinned(in: VOut)")
        .expect("skinned capture has fs_skinned");
    let rest = &wgsl[start..];
    let end = super::fs_close(rest).expect("fs_skinned closes");
    let body = &rest[..end];
    assert!(
        !body.contains("ibl * occlusion")
            && !body.contains("ibl*occlusion")
            && !body.contains("ibl *occlusion")
            && !body.contains("ibl* occlusion"),
        "fs_skinned scales combined ibl by occlusion: scale diffuse only"
    );
    let has_diffuse = body.contains("ibl_diffuse") || body.contains("ibl_irradiance");
    let scales = body.contains("* occlusion") || body.contains("*occlusion");
    assert!(
        has_diffuse && scales,
        "fs_skinned misses diffuse-by-occlusion scale"
    );
    assert!(body.contains("ibl_spec"), "fs_skinned misses open specular");
}

#[test]
fn forward_skinned_samples_orm_like_window() {
    // EG12: fs_skinned had no ORM read, so rough/metal factors and the
    // baked occlusion never reached the skinned capture. Both forward
    // entries must sample orm_map; the row's proof is this count.
    let wgsl = include_str!("../shaders/forward.wgsl");
    assert_eq!(
        wgsl.matches("textureSample(orm_map").count(),
        2,
        "fs and fs_skinned both sample the ORM map"
    );
    let start = wgsl
        .find("fn fs_skinned(in: VOut)")
        .expect("skinned capture has fs_skinned");
    let rest = &wgsl[start..];
    let end = super::fs_close(rest).expect("fs_skinned closes");
    let body = &rest[..end];
    assert!(body.contains("orm.x"), "occlusion reads orm.x");
    assert!(
        body.contains("mat.roughness * orm.y"),
        "roughness scales by orm.y"
    );
    assert!(
        body.contains("mat.metallic * orm.z"),
        "metallic scales by orm.z"
    );
}
