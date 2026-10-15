//! post_aa_parity: jitter plus TAA resolve plus CAS parity vectors.
//!
//! Purpose: pin the RP1 front (`post.rs`) against the shape idea of the
//! Bevy anti-alias module (jittered projection, history resolve, CAS
//! sharpen): Halton jitter stays centered and deterministic, the static
//! TAA resolve is the identity, and the RCAS sharpen matches the current
//! post chain on flat fields while staying inside its neighbour lobe on
//! edges. Owner is the render lane; consumer is the WIN1/ED1 final frame
//! through the RA1 presenter and the Q1 clips. Units: pixels for sizes,
//! ms per pass in receipts (evidence only, never hashed). Determinism:
//! same complete inputs give bit-identical pixels twice; wall clock
//! appears in receipts only. Errors: bad caller input fails loudly via
//! `PostError`, never silent success.
//!
//! Attribution (idea-only, zero donor bytes): the jitter plus TAA-resolve
//! plus CAS module shape follows the *idea* of the `bevy_anti_alias`
//! module in bevyengine/bevy (MIT). No donor code, constants, or text
//! were copied; every line below calls the public `post.rs` functions
//! only and asserts fresh vectors against them. No `src` file was edited.
//!
//! Header read first: `crates/render/src/post.rs` lines 1-43 (pipeline
//! order jitter -> motion -> TAA resolve -> TAAU -> bloom/exposure/ACES/
//! grade, plus the RL1/RL2 receipt taps). This file calls public items
//! only: `jitter_offset`, `jitter_ndc`, `jitter_uniform`,
//! `jitter_projection`, `MotionField`, `TaaParams`, `taa_resolve`,
//! `taa_resolve_jittered`, `fsr_rcas_sharpen`, `apply_grade`,
//! `GradeConfig`, `PostChain`, `PostLights`, `run_frame`, `PassReceipt`.
//!
//! KAV: Halton-centered jitter vectors, static-resolve identity vectors,
//! RCAS lobe-clamp plus flat-field chain parity, per-pass ms receipts.

use engine_render::post::{
    apply_grade, fsr_rcas_sharpen, jitter_ndc, jitter_offset, jitter_projection, jitter_uniform,
    run_frame, taa_resolve, taa_resolve_jittered, GradeConfig, MotionField, PassReceipt, PostChain,
    PostLights, TaaParams,
};
use std::time::Instant;

const W: u32 = 8;
const H: u32 = 8;
const PX: usize = 64;
const FLAT: [f32; 3] = [0.4, 0.4, 0.4];
const EPS: f32 = 1e-6;

/// Row-major index for `w`-wide frames without numeric casts.
fn idx(w: u32, x: u32, y: u32) -> usize {
    usize::try_from(y * w + x).expect("aa parity texel index fits usize")
}

/// Lossless `u32` to `f32` for the small test dims (all fit `u16`).
fn f32_from_small(v: u32) -> f32 {
    f32::from(u16::try_from(v).expect("aa parity dim fits u16"))
}

/// Wall-clock ms since `start`; evidence only, never hashed.
fn ms_since(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}

/// One per-pass receipt assertion: finite, nonnegative wall time.
fn assert_receipt(ms: f64, pixels: u64, what: &str) -> PassReceipt {
    assert!(ms.is_finite(), "{what} ms is finite");
    assert!(ms >= 0.0, "{what} ms is nonnegative");
    PassReceipt {
        pass: "aa-parity",
        pixels,
        ms,
    }
}

#[test]
fn jitter_sequence_is_centered_deterministic_and_projected() {
    let started = Instant::now();
    let w_f = f32_from_small(1920);
    let h_f = f32_from_small(1080);
    let mut first = [0.0f32; 2];
    for frame in 0..8u64 {
        let j = jitter_offset(frame);
        assert!(j.iter().all(|c| c.is_finite()), "frame {frame} finite");
        assert!(
            j[0] >= -0.5 && j[0] < 0.5 && j[1] >= -0.5 && j[1] < 0.5,
            "frame {frame} jitter {j:?} centered"
        );
        assert_eq!(j, jitter_offset(frame), "frame {frame} deterministic");
        let ndc = jitter_ndc(frame, 1920, 1080);
        assert!(
            (ndc[0] - j[0] * 2.0 / w_f).abs() <= EPS && (ndc[1] - j[1] * 2.0 / h_f).abs() <= EPS,
            "frame {frame} ndc matches pixels"
        );
        let uni = jitter_uniform(frame, 1920, 1080);
        assert_eq!([uni[0], uni[1]], j, "frame {frame} uniform packs pixels");
        assert_eq!([uni[2], uni[3]], ndc, "frame {frame} uniform packs ndc");
        if frame == 0 {
            first = j;
        }
        if frame == 1 {
            assert_ne!(j, first, "sequence moves across frames");
        }
    }
    assert_eq!(jitter_ndc(3, 0, 1080), [0.0, 0.0], "zero width suspends");
    assert_eq!(jitter_ndc(3, 1920, 0), [0.0, 0.0], "zero height suspends");
    let ident = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let ndc = jitter_ndc(5, 1920, 1080);
    let out = jitter_projection(ident, ndc);
    assert_eq!((out[2][0], out[2][1]), (ndc[0], ndc[1]), "terms land");
    assert_eq!(out[0], ident[0], "row 0 untouched");
    assert_eq!(out[1], ident[1], "row 1 untouched");
    assert_eq!(out[3], ident[3], "row 3 untouched");
    let ms = ms_since(started);
    let pixels = u64::from(W) * u64::from(H);
    let receipt = assert_receipt(ms, pixels, "jitter");
    assert_eq!(receipt.pixels, pixels, "jitter receipt pixels");
}

#[test]
fn taa_resolve_static_scene_is_identity_with_timed_receipt() {
    let started = Instant::now();
    let current = vec![FLAT; PX];
    let history = vec![FLAT; PX];
    let motion = MotionField::zero(W, H).expect("zero motion builds");
    let params = TaaParams::default();
    let resolved =
        taa_resolve(&current, &history, &motion, W, H, &params).expect("static resolve runs");
    assert_eq!(resolved, current, "static scene resolves bit-identically");
    let jittered = taa_resolve_jittered(&current, &history, &motion, W, H, &params, [0.0, 0.0])
        .expect("zero jitter runs");
    assert_eq!(jittered, resolved, "zero jitter equals plain resolve");
    let again = taa_resolve(&current, &history, &motion, W, H, &params).expect("rerun");
    assert_eq!(again, resolved, "same inputs give identical pixels");
    // Failure case: blend outside 0..=1 is rejected, never clamped silently.
    let bad = TaaParams {
        history_blend: 2.0,
        ..params
    };
    assert!(taa_resolve(&current, &history, &motion, W, H, &bad).is_err());
    // Blend 0 follows current even when history disagrees.
    let dark = vec![[0.05, 0.05, 0.05]; PX];
    let follow = TaaParams {
        history_blend: 0.0,
        ..params
    };
    let out = taa_resolve(&current, &dark, &motion, W, H, &follow).expect("blend 0 runs");
    assert_eq!(out, current, "blend 0 follows current");
    let taa_ms = ms_since(started);
    let pixels = u64::from(W) * u64::from(H);
    let receipt = assert_receipt(taa_ms, pixels, "taa");
    assert_eq!(receipt.pixels, pixels, "taa receipt pixels");
    // Full-frame receipt: per-pass ms asserted, lights quoted, pixels exact.
    let chain = PostChain::default();
    let lights = PostLights {
        sun_ms: 1.0,
        cluster_ms: 2.0,
        ibl_ms: 3.0,
    };
    let frame = run_frame(
        &current, &history, &motion, W, H, W, H, &params, &chain, &lights,
    )
    .expect("frame runs");
    for (ms, name) in [
        (frame.receipt.taa.ms, "taa"),
        (frame.receipt.upsample.ms, "taau"),
        (frame.receipt.bloom.ms, "bloom"),
        (frame.receipt.exposure.ms, "exposure"),
        (frame.receipt.aces.ms, "aces"),
        (frame.receipt.grade.ms, "grade"),
    ] {
        assert!(ms.is_finite(), "{name} ms finite");
        assert!(ms >= 0.0, "{name} ms nonnegative");
    }
    assert!(
        (frame.receipt.total_ms()
            - (frame.receipt.taa.ms
                + frame.receipt.upsample.ms
                + frame.receipt.bloom.ms
                + frame.receipt.exposure.ms
                + frame.receipt.aces.ms
                + frame.receipt.grade.ms))
            .abs()
            <= 1e-9,
        "total is the pass sum"
    );
    assert_eq!(frame.receipt.taa.pixels, pixels, "taa pixels");
    assert_eq!(frame.receipt.upsample.pixels, pixels, "taau pixels");
    assert_eq!(frame.receipt.lights_ms(), 6.0, "lights quoted");
    assert_eq!(frame.ldr.len(), PX, "ldr length");
}

#[test]
fn cas_parity_matches_chain_on_flat_and_clamps_lobe_on_edge() {
    let started = Instant::now();
    let flat = vec![FLAT; PX];
    let identity = fsr_rcas_sharpen(&flat, W, H, 0.0).expect("sharp 0 runs");
    assert_eq!(identity, flat, "sharpness 0 is the identity");
    let soft = fsr_rcas_sharpen(&flat, W, H, 0.4).expect("sharp 0.4 runs");
    assert_eq!(soft, flat, "flat field survives sharpening");
    let again = fsr_rcas_sharpen(&flat, W, H, 0.4).expect("rerun");
    assert_eq!(again, soft, "same inputs give identical pixels");
    assert!(
        fsr_rcas_sharpen(&flat, W, H, 2.0).is_err(),
        "sharp >1 rejected"
    );
    assert!(
        fsr_rcas_sharpen(&flat, W, H, f32::NAN).is_err(),
        "NaN sharp rejected"
    );
    // CAS parity vs the current post chain: grading a sharpened flat
    // field equals grading the flat field directly.
    let grade = GradeConfig::default();
    let direct = apply_grade(FLAT, &grade).expect("grade runs");
    let via_cas = apply_grade(soft[idx(W, 3, 3)], &grade).expect("graded sharpened");
    assert_eq!(via_cas, direct, "cas plus grade matches grade on flat");
    // Edge probe: vertical step (dark left, bright right); every output
    // channel must stay inside its 4-neighbour lobe (no halo escapes).
    let mut edge = Vec::with_capacity(PX);
    for _y in 0..H {
        for x in 0..W {
            edge.push(if x < W / 2 {
                [0.1, 0.1, 0.1]
            } else {
                [0.9, 0.9, 0.9]
            });
        }
    }
    let sharp = fsr_rcas_sharpen(&edge, W, H, 1.0).expect("edge sharpen runs");
    assert_eq!(sharp.len(), PX, "edge length");
    let at = |img: &[[f32; 3]], x: u32, y: u32| img[idx(W, x, y)];
    for y in 0..H {
        for x in 0..W {
            let e = at(&edge, x, y);
            let up = at(&edge, x, y.saturating_sub(1));
            let down = at(&edge, x, (y + 1).min(H - 1));
            let left = at(&edge, x.saturating_sub(1), y);
            let right = at(&edge, (x + 1).min(W - 1), y);
            let o = at(&sharp, x, y);
            for c in 0..3 {
                let lo = up[c].min(down[c]).min(left[c]).min(right[c]);
                let hi = up[c].max(down[c]).max(left[c]).max(right[c]);
                assert!(
                    o[c] >= lo - EPS && o[c] <= hi + EPS,
                    "lobe holds at {x},{y} ch {c}: {} in [{lo},{hi}] vs src {}",
                    o[c],
                    e[c],
                );
            }
        }
    }
    // Bite probe: a clean step is a fixed point of the lobe clamp, so a
    // separate soft notch proves full sharpness still moves pixels that
    // sit strictly inside their lobe.
    let mut notch = vec![[0.5, 0.5, 0.5]; PX];
    notch[idx(W, 2, 3)] = [0.1, 0.1, 0.1];
    notch[idx(W, 3, 3)] = [0.6, 0.6, 0.6];
    notch[idx(W, 4, 3)] = [0.9, 0.9, 0.9];
    let bit = fsr_rcas_sharpen(&notch, W, H, 1.0).expect("notch sharpen runs");
    let center = at(&bit, 3, 3);
    for c in 0..3 {
        assert!(
            (center[c] - 0.6).abs() > 1e-6,
            "full sharpness moves the notch ch {c}: {}",
            center[c],
        );
        assert!(
            center[c] >= 0.1 - EPS && center[c] <= 0.9 + EPS,
            "notch stays in lobe ch {c}: {}",
            center[c],
        );
    }

    let ms = ms_since(started);
    let pixels = u64::from(W) * u64::from(H);
    let receipt = assert_receipt(ms, pixels, "cas");
    assert_eq!(receipt.pixels, pixels, "cas receipt pixels");
}
