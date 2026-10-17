//! post: engine-owned TAA/TAAU plus the GPU post chain (RP1).
//!
//! Owner: render lane; consumers WIN1/ED1 final frame through the RA1
//! presenter and the Q1 clips. Units: pixels for sizes and motion vectors,
//! ms per pass in receipts. Determinism: integer size math, fixed tap
//! orders, same complete inputs give bit-identical outputs; wall clock
//! appears in receipts, never in pixels. Errors are typed [`PostError`]:
//! bad caller input fails loudly, over-budget sizes degrade with
//! `BudgetExceeded`, and the no-device submit names the device-bound
//! `gpu::Rp1Submitter` to build first, never silent success.
//!
//! Pipeline order: jittered projection (per-frame Halton-2/3 offset into
//! the projection matrix) -> motion vectors (current minus history, in
//! pixels) -> TAA resolve (bilinear history reproject plus Karis-style
//! 3x3 neighborhood box clamp, then blend) -> temporal upsampling
//! (engine-owned bilinear TAAU present path; FSR1-inspired EASU plus RCAS
//! spatial path beside it, TAAU default frozen) ->
//! post chain (bloom extract/blur/add, exposure, ACES, grading). Exposure
//! is fixed stops by default; the auto-exposure state beside it histograms
//! the pre-exposure HDR (64 log bins, trimmed mean onto key) and slews
//! toward the retarget at 3 stops per second, so day sweeps track while
//! sparse highlights cannot pump the frame. The chain
//! reads the RL1/RL2 passes through [`PostLights`]: the caller hands the
//! `FrameLightsReceipt` (RL1) and `IblReceipt` (RL2) of the frame being
//! posted, and the output receipt quotes their pass ms alongside the post
//! pass ms, so the frame report names every light and post cost.
//!
//! Graph shape: [`build_rp1_graph`] declares the six RP1 passes
//! (`rp1-taa` -> `rp1-taau` -> `rp1-bloom` -> `rp1-exposure` ->
//! `rp1-aces` -> `rp1-grade`) on the RA2 frame graph, and the `window`
//! `gpu::Rp1Submitter` records the same six in compiled order on a live
//! device with motion-vector plus jitter reprojection and box-clamp
//! resolve, then the bloom/exposure/ACES/grade chain, all validation
//! clean. The CPU chain stays the parity oracle: same complete inputs
//! give bit-identical CPU pixels twice, and GPU pixels match it within
//! the posted tolerance.
//!
//! Provenance: Halton-2/3 jitter, box-clamp reprojection, soft-knee bloom,
//! Narkowicz ACES fit and contrast/saturation grading are textbook
//! formulations implemented directly (no upstream copy), so they carry no
//! `PORT()` marker. The spatial upscale reimplements the FSR1 EASU plus RCAS
//! shape (directional Lanczos-2 taps with 2x2 clamp plus lobe limiters) with
//! zero donor bytes; its items carry the donor `PORT()` marker.
//! Port shapes below reimplemented with zero donor bytes; behavior unchanged.
//! PORT(BabylonJS/Babylon.js@136feacc:packages/dev/core/src/ShadersWGSL/taa.fragment.fx): resolve shape.
//! PORT(BabylonJS/Babylon.js@136feacc:packages/dev/core/src/ShadersWGSL/tonemap.fragment.fx): aces shape.

use std::fmt;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Bounds: every external size is validated before allocation.
// ---------------------------------------------------------------------------

/// Largest accepted post dimension in pixels (mirrors the RA1 frame bound).
pub const MAX_POST_DIM: u32 = 2048;
/// Most bloom mip levels in the CPU chain.
pub const MAX_BLOOM_LEVELS: u32 = 6;
/// Default history blend: how much clamped history the resolve keeps.
pub const DEFAULT_HISTORY_BLEND: f32 = 0.9;
/// Contrast pivot (linear mid-grey), same convention as the R4 grade path.
pub const GRADE_MID_GREY: f32 = 0.18;
/// Posted ghosting gate: max trail-region luma after the TAA resolve.
pub const GHOST_MAX_TRAIL: f32 = 0.25;
/// Posted shimmer gate: max per-pixel temporal variance over the clip.
pub const SHIMMER_MAX_VAR: f32 = 0.01;

// ---------------------------------------------------------------------------
// Errors: typed; budget overruns degrade, no-device submit names the device
// path to build.
// ---------------------------------------------------------------------------

/// Post failures. `BudgetExceeded` is a degrade signal: the caller clamps
/// to `allowed` and keeps rendering. `Device` covers calls with no live
/// device handed over (use `gpu::Rp1Submitter` on a `window` device);
/// `Validation` carries a wgpu scope hit; neither path ever reports
/// silent success.
#[derive(Clone, PartialEq, Debug)]
pub enum PostError {
    /// Caller input is unusable; reason carried.
    Invalid(String),
    /// The request exceeded a bound; the caller clamps to `allowed`.
    BudgetExceeded {
        /// Requested amount.
        requested: u32,
        /// Amount the pass actually uses.
        allowed: u32,
    },
    /// No live device on this call; what to build first, carried.
    Device(String),
    /// A wgpu validation scope fired inside the submit; scope text carried.
    Validation(String),
}

impl fmt::Display for PostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "post input: {e}"),
            Self::BudgetExceeded { requested, allowed } => {
                write!(
                    f,
                    "post budget exceeded: {requested} requested, {allowed} used"
                )
            }
            Self::Device(e) => write!(f, "post device: {e}"),
            Self::Validation(e) => write!(f, "post validation: {e}"),
        }
    }
}

impl std::error::Error for PostError {}

// ---------------------------------------------------------------------------
// Receipts: per-pass ms plus pixel counts for the frame report.
// ---------------------------------------------------------------------------

/// One post pass receipt: what ran, how many pixels, how long.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PassReceipt {
    /// Pass name (`"taa"`, `"taau"`, `"bloom"`, `"exposure"`, `"aces"`, `"grade"`).
    pub pass: &'static str,
    /// Pixels written by this pass.
    pub pixels: u64,
    /// Wall-clock pass time in ms; evidence only, never hashed.
    pub ms: f64,
}

/// Frame-level post receipt: TAA resolve, TAAU upsample and the four chain
/// passes, plus the RL1/RL2 light ms the chain read.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FramePostReceipt {
    /// TAA resolve pass.
    pub taa: PassReceipt,
    /// Temporal upsample pass.
    pub upsample: PassReceipt,
    /// Bloom pass.
    pub bloom: PassReceipt,
    /// Exposure pass.
    pub exposure: PassReceipt,
    /// ACES tonemap pass.
    pub aces: PassReceipt,
    /// Grading pass.
    pub grade: PassReceipt,
    /// RL1 sun shadow pass ms read from the light receipt.
    pub sun_ms: f64,
    /// RL1 torch cluster pass ms read from the light receipt.
    pub cluster_ms: f64,
    /// RL2 IBL bake ms read from the IBL receipt.
    pub ibl_ms: f64,
}

impl FramePostReceipt {
    /// Total post pass time in ms for the frame report (lights excluded).
    pub fn total_ms(&self) -> f64 {
        self.taa.ms
            + self.upsample.ms
            + self.bloom.ms
            + self.exposure.ms
            + self.aces.ms
            + self.grade.ms
    }

    /// Total light pass time in ms read from RL1/RL2 for the frame report.
    pub fn lights_ms(&self) -> f64 {
        self.sun_ms + self.cluster_ms + self.ibl_ms
    }
}

fn receipt(pass: &'static str, pixels: u64, ms: f64) -> PassReceipt {
    PassReceipt { pass, pixels, ms }
}

// The post jitter and motion leaf lives in `post_jitter_motion`; re-exported
// here so every `post::` caller keeps its path.
pub use crate::post_jitter_motion::*;

mod taa;
pub use taa::*;
mod upscale;
pub use upscale::*;
mod chain;
pub use chain::*;
mod auto_exposure;
pub use auto_exposure::*;
mod rp1;
pub use rp1::*;

/// Pinned 720p-to-1080p spatial upscale (FC3): EASU-shape lobe plus gradient reject.
#[path = "upscale_easu.rs"]
pub mod upscale_easu;

// ---------------------------------------------------------------------------
// GPU descriptors behind `window`: resolve/post targets plus the WGSL
// chunks the RA1 presenter consumes. Allocation tests skip without an
// adapter, exactly like the RL1 presenter tests.
// ---------------------------------------------------------------------------

/// Post GPU descriptors and the TAA-resolve plus post sampling chunks for
/// the RA1 presenter. Engine-owned code (no port markers); the presenter
/// inlines these chunks when the post bind group lands.
#[cfg(feature = "window")]
pub mod gpu;

// ---------------------------------------------------------------------------
// Tests: jitter, motion, resolve/clamp, upsample, chain, scenes, failures.
// `v1` tests pin parity with the R4/R1 paths; `window` tests pin the GPU
// descriptors with the RA1 skip pattern.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    fn lights() -> PostLights {
        PostLights {
            sun_ms: 1.25,
            cluster_ms: 0.5,
            ibl_ms: 2.0,
        }
    }

    #[test]
    fn single_uniform_owner() {
        // Caller: Rp1Submitter uniform uploads (window path) via the single owner.
        // Behavior: post.rs must reuse the shared uniform helper in v1_gpu
        // and never carry its own copy.
        let src = [include_str!("post.rs"), include_str!("post/gpu.rs")].concat();
        let needle = ["fn", "uniform_buf("].join(" ");
        assert!(
            !src.contains(&needle),
            "post must reuse the shared uniform helper, found private duplicate"
        );
        assert!(
            src.contains("create_uniform_buffer"),
            "post must call the single uniform-buffer owner"
        );
    }

    #[test]
    fn taa_static_scene_resolves_exactly() {
        // History equals current: every sample reprojects 1:1 and the box
        // clamp is a no-op, so the resolve reproduces the frame exactly.
        let frame: Vec<[f32; 3]> = (0..64).map(|i| [i as f32 * 0.01, 0.2, 0.05]).collect();
        let motion = MotionField::zero(8, 8).expect("zero builds");
        let params = TaaParams::default();
        let out = taa_resolve(&frame, &frame, &motion, 8, 8, &params).expect("resolves");
        assert_eq!(out, frame, "static resolve is the identity");
        // Zero blend follows the current frame even with stale history.
        let stale = vec![[9.0, 9.0, 9.0]; 64];
        let follow = TaaParams {
            history_blend: 0.0,
            ..params
        };
        let out = taa_resolve(&frame, &stale, &motion, 8, 8, &follow).expect("resolves");
        assert_eq!(out, frame);
    }

    #[test]
    fn neighborhood_clamp_kills_ghosting() {
        // Moving-bar scene: the clamped resolve keeps the vacated trail
        // dark (posted gate GHOST_MAX_TRAIL), while the naive resolve
        // without the clamp smears history across the trail.
        let scene = GhostingScene::moving_bar().expect("scene builds");
        let params = TaaParams::default();
        let clamped = taa_resolve(
            &scene.current,
            &scene.prev,
            &scene.motion,
            scene.width,
            scene.height,
            &params,
        )
        .expect("resolves");
        let trail = GhostingScene::trail_luma(&clamped, scene.width, scene.trail);
        eprintln!("RP1 ghosting: clamped trail luma {trail:.4} (gate {GHOST_MAX_TRAIL})");
        assert!(
            trail <= GHOST_MAX_TRAIL,
            "ghost trail {trail} exceeds posted gate {GHOST_MAX_TRAIL}"
        );
        let naive = taa_resolve(
            &scene.current,
            &scene.prev,
            &scene.motion,
            scene.width,
            scene.height,
            &TaaParams {
                use_clamp: false,
                ..params
            },
        )
        .expect("naive resolves");
        let naive_trail = GhostingScene::trail_luma(&naive, scene.width, scene.trail);
        eprintln!("RP1 ghosting: naive trail luma {naive_trail:.4}");
        assert!(
            naive_trail > trail,
            "naive {naive_trail} must ghost more than clamped {trail}"
        );
        assert!(
            naive_trail > GHOST_MAX_TRAIL,
            "scene must actually ghost without the clamp, got {naive_trail}"
        );
        // Velocity rejection: a huge jump follows the current frame.
        let fast = MotionField::build(16, 16, vec![[500.0, 0.0]; 256]).expect("fast builds");
        let out =
            taa_resolve(&scene.current, &scene.prev, &fast, 16, 16, &params).expect("resolves");
        assert_eq!(out, scene.current, "fast movers follow current");
    }

    #[test]
    fn taa_upsample_is_exact_on_identity_and_solids() {
        // Same-size upscale is the bit-exact identity (weights collapse).
        let src = vec![[0.3, 0.6, 0.9]; 4 * 3];
        let same = taa_upsample(&src, 4, 3, 4, 3).expect("same size upscales");
        assert_eq!(same, src, "same-size TAAU is the identity");
        // Solid colors survive any upscale unchanged (no invented detail).
        let solid = vec![[0.7, 0.2, 0.1]; 2 * 2];
        let big = taa_upsample(&solid, 2, 2, 8, 6).expect("solid upscales");
        assert_eq!(big.len(), 8 * 6);
        assert!(big.iter().all(|p| *p == [0.7, 0.2, 0.1]));
        // 1x1 replicates to every destination texel.
        let one = taa_upsample(&[[1.0, 0.5, 0.0]], 1, 1, 3, 2).expect("1x1 upscales");
        assert_eq!(one, vec![[1.0, 0.5, 0.0]; 6]);
    }

    #[test]
    fn fsr_540p_to_1080p_posts_mtf50_shimmer_and_ms() {
        // 540p step edge to 1080p: spatial path must lift MTF50 over the
        // frozen bilinear TAAU at matched shimmer, with ms posted as evidence
        // (GPU bars need the reference box; CPU ms never gates here).
        let (src_w, src_h) = (960u32, 540u32);
        let (dst_w, dst_h) = (1920u32, 1080u32);
        let low = fsr_edge_lowres(src_w, src_h).expect("edge builds");
        let start = Instant::now();
        let base = taa_upsample(&low, src_w, src_h, dst_w, dst_h).expect("bilinear runs");
        let base_ms = start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        let cand =
            fsr_upscale(&low, src_w, src_h, dst_w, dst_h, FSR_DEFAULT_SHARPNESS).expect("fsr runs");
        let cand_ms = start.elapsed().as_secs_f64() * 1000.0;
        let base_w = fsr_edge_width_10_90(&base, dst_w, dst_h).expect("base width measures");
        let cand_w = fsr_edge_width_10_90(&cand, dst_w, dst_h).expect("cand width measures");
        let lift = fsr_mtf50_lift(base_w, cand_w).expect("lift computes");
        // Shimmer through the real pipeline: TAA settles the 16x16 checker,
        // then each settled frame upscales to 32x32. Both paths must hold the
        // spatial gate on the settled clip.
        let scene = ShimmerScene::checkerboard().expect("shimmer builds");
        let motion = MotionField::zero(scene.width, scene.height).expect("zero builds");
        let params = TaaParams::default();
        let mut history = vec![[0.5, 0.5, 0.5]; 256];
        let mut settled = Vec::new();
        for k in 0..12 {
            let current = &scene.frames[k % 2];
            let resolved = taa_resolve(
                current,
                &history,
                &motion,
                scene.width,
                scene.height,
                &params,
            )
            .expect("settles");
            history = resolved.clone();
            if k >= 8 {
                settled.push(resolved);
            }
        }
        let base_clip: Vec<Vec<[f32; 3]>> = settled
            .iter()
            .map(|f| taa_upsample(f, 16, 16, 32, 32).expect("base settled"))
            .collect();
        let cand_clip: Vec<Vec<[f32; 3]>> = settled
            .iter()
            .map(|f| fsr_upscale(f, 16, 16, 32, 32, FSR_DEFAULT_SHARPNESS).expect("cand settled"))
            .collect();
        let base_var = ShimmerScene::temporal_variance(&base_clip);
        let cand_var = ShimmerScene::temporal_variance(&cand_clip);
        // Present x2 identical: same inputs give bit-identical outputs.
        let again =
            fsr_upscale(&low, src_w, src_h, dst_w, dst_h, FSR_DEFAULT_SHARPNESS).expect("reruns");
        assert_eq!(cand, again, "spatial pixels bit-identical twice");
        eprintln!(
            "FSR 540p->1080p: base_width {base_w:.3}px cand_width {cand_w:.3}px lift {lift:.3} (gate {FSR_MTF50_LIFT_MIN}), base_var {base_var:.6} cand_var {cand_var:.6} (gate {FSR_SHIMMER_GATE}), base_ms {base_ms:.2}ms cand_ms {cand_ms:.2}ms"
        );
        assert!(
            lift >= FSR_MTF50_LIFT_MIN,
            "MTF50 lift {lift} under {FSR_MTF50_LIFT_MIN}"
        );
        assert!(
            cand_var <= FSR_SHIMMER_GATE,
            "shimmer {cand_var} over {FSR_SHIMMER_GATE}"
        );
        assert!(base_ms.is_finite() && cand_ms.is_finite(), "ms must post");
    }

    #[test]
    fn fsr_fails_loudly() {
        let ok = vec![[0.5, 0.5, 0.5]; 16];
        assert!(fsr_easu_con(0, 4, 8, 8).is_err());
        assert!(fsr_easu_upsample(&ok[..15], 4, 4, 8, 8).is_err());
        assert!(fsr_rcas_sharpen(&ok, 4, 4, 2.0).is_err());
        assert!(fsr_upscale(&ok, 4, 4, 4096, 4, FSR_DEFAULT_SHARPNESS).is_err());
        assert!(fsr_edge_width_10_90(&[[0.5, 0.5, 0.5]; 16], 4, 4).is_err());
        assert!(fsr_mtf50(0.0 - 1.0).is_err());
        assert!(fsr_mtf50_lift(-1.0, 1.0).is_err());
    }

    #[test]
    fn extract_glam_matches_scalar_bit_exact() {
        // SIMD slice: glam path must reproduce the scalar path
        // bit-identically on a mixed HDR field, plus an A/B timing line
        // for the receipt (wall clock only, never hashed).
        let cfg = BloomConfig::default();
        let mut hdr = vec![[0.1, 0.2, 0.3]; 32 * 32];
        hdr[5] = [8.0, 7.0, 6.0];
        hdr[999] = [0.0, 0.0, 0.0];
        let scalar = bright_extract_scalar(&hdr, cfg.threshold, cfg.knee);
        let dispatched = bright_extract(&hdr, cfg.threshold, cfg.knee);
        assert_eq!(scalar, dispatched, "extract paths agree bit-exactly");
        #[cfg(feature = "v1")]
        {
            let glam = bright_extract_glam(&hdr, cfg.threshold, cfg.knee);
            assert_eq!(scalar, glam, "glam SIMD matches scalar bit-exactly");
            let (scalar_ms, glam_ms) = bright_extract_ab(&hdr, cfg.threshold, cfg.knee);
            assert!(scalar_ms.is_finite() && glam_ms.is_finite());
            std::eprintln!("bright_extract A/B 32x32: scalar={scalar_ms:.3}ms glam={glam_ms:.3}ms");
        }
        // End-to-end bloom still flares through the dispatcher.
        let out = apply_bloom(&hdr, 32, 32, &cfg).expect("bloom runs");
        assert!(out[5][0] >= 8.0, "spark keeps its core");
    }

    #[test]
    fn bloom_fires_on_bright_not_dark() {
        // A lone HDR spark flares its neighbors; a dark field stays dark.
        let mut hdr = vec![[0.1, 0.1, 0.1]; 8 * 8];
        hdr[3 * 8 + 3] = [8.0, 8.0, 8.0];
        let cfg = BloomConfig::default();
        let out = apply_bloom(&hdr, 8, 8, &cfg).expect("bloom runs");
        assert!(out[3 * 8 + 3][0] >= 8.0, "spark keeps its core");
        let neighbor = out[3 * 8 + 4][0];
        assert!(
            neighbor > 0.1,
            "bloom lifts the spark neighbor, got {neighbor}"
        );
        let dark = vec![[0.1, 0.1, 0.1]; 8 * 8];
        let still = apply_bloom(&dark, 8, 8, &cfg).expect("dark blooms");
        let mut max_lift = 0.0f32;
        for (a, b) in dark.iter().zip(still.iter()) {
            max_lift = max_lift.max((b[0] - a[0]).abs());
        }
        assert_eq!(max_lift, 0.0, "sub-threshold field is untouched");
        // Zero intensity adds nothing back anywhere.
        let off = apply_bloom(
            &hdr,
            8,
            8,
            &BloomConfig {
                intensity: 0.0,
                ..cfg
            },
        )
        .expect("off blooms");
        assert_eq!(off, hdr, "zero intensity is the identity");
    }

    #[test]
    fn bloom_gate_skips_dark_frame_and_runs_on_torch() {
        // Caller: `apply_bloom` in this file (the LF4 live gate oracle).
        // Behavior: a sub-knee frame reports no halo and stays bit-identical
        // through the chain, while a torch spark reports a halo and lifts
        // its neighbor.
        let dark = vec![[0.05, 0.05, 0.06]; 8 * 8];
        let cfg = BloomConfig::default();
        assert_eq!(frame_max_luma(&dark), 0.06);
        assert!(!bloom_should_run(
            frame_max_luma(&dark),
            cfg.threshold,
            cfg.knee
        ));
        let still = apply_bloom(&dark, 8, 8, &cfg).expect("dark blooms");
        assert_eq!(still, dark, "gated dark frame stays bit-identical");
        let mut hdr = vec![[0.05, 0.05, 0.06]; 8 * 8];
        hdr[3 * 8 + 3] = [4.0, 2.2, 1.1];
        assert!(bloom_should_run(
            frame_max_luma(&hdr),
            cfg.threshold,
            cfg.knee
        ));
        let out = apply_bloom(&hdr, 8, 8, &cfg).expect("torch blooms");
        assert!(out[3 * 8 + 4][0] > hdr[3 * 8 + 4][0]);
        assert_eq!(out[0], hdr[0]);
    }

    #[test]
    fn bloom_gate_rejects_bad_inputs() {
        // Caller: `apply_bloom` in this file (fail-closed, never runs).
        // Behavior: non-finite luma, bad threshold and bad knee never run.
        assert!(!bloom_should_run(f32::NAN, 1.0, 0.5));
        assert!(!bloom_should_run(f32::INFINITY, 1.0, 0.5));
        assert!(!bloom_should_run(4.0, -1.0, 0.5));
        assert!(!bloom_should_run(4.0, f32::NAN, 0.5));
        assert!(!bloom_should_run(4.0, 1.0, -0.1));
        assert!(!bloom_should_run(4.0, 1.0, 1.1));
        assert!(!bloom_should_run(4.0, 1.0, f32::NAN));
        assert!(!bloom_should_run(0.05, 1.0, 0.5));
        assert!(bloom_should_run(4.0, 1.0, 0.5));
    }

    #[test]
    fn bloom_on_off_lift_torch_halo_lifts_neighbor_but_not_dark() {
        // Caller: future LF4 presenter gate (headless CPU mirror only).
        // Behavior: one helper call pins the torch halo above off at the
        // neighbor while the far corner stays identical.
        let mut hdr = vec![[0.05, 0.05, 0.06]; 8 * 8];
        hdr[3 * 8 + 3] = [4.0, 2.2, 1.1];
        let cfg = BloomConfig::default();
        let (halo_lift, dark_lift) =
            bloom_on_off_lift(&hdr, 8, 8, &cfg, (4, 3), (0, 0)).expect("lift runs");
        assert!(halo_lift > 0.0, "halo lifts above off, got {halo_lift}");
        assert_eq!(
            dark_lift, 0.0,
            "far corner stays identical, got {dark_lift}"
        );
    }

    #[test]
    fn bloom_on_off_lift_dark_field_is_identity_and_deterministic() {
        // Caller: future LF4 presenter gate (headless CPU mirror only).
        // Behavior: a sub-knee field lifts nothing anywhere, on-vs-off is
        // bit-identical, and x2 runs match exactly.
        let dark = vec![[0.05, 0.05, 0.06]; 8 * 8];
        let cfg = BloomConfig::default();
        let (halo_lift, dark_lift) =
            bloom_on_off_lift(&dark, 8, 8, &cfg, (4, 3), (0, 0)).expect("lift runs");
        assert_eq!((halo_lift, dark_lift), (0.0, 0.0), "dark lifts nothing");
        let on = apply_bloom(&dark, 8, 8, &cfg).expect("on blooms");
        let off = apply_bloom(
            &dark,
            8,
            8,
            &BloomConfig {
                intensity: BLOOM_OFF_INTENSITY,
                ..cfg
            },
        )
        .expect("off blooms");
        assert_eq!(on, off, "sub-knee on-vs-off is bit-identical");
        assert_eq!(on, dark, "sub-knee on is the identity");
        let again = bloom_on_off_lift(&dark, 8, 8, &cfg, (4, 3), (0, 0)).expect("reruns");
        assert_eq!((halo_lift, dark_lift), again, "x2 runs match exactly");
    }

    #[test]
    fn bloom_on_off_lift_rejects_bad_inputs() {
        // Caller: future LF4 presenter gate (headless CPU mirror only).
        // Behavior: short slices, out-of-range coords and bad configs
        // refuse loudly, never silent success.
        let cfg = BloomConfig::default();
        let hdr = vec![[0.05, 0.05, 0.06]; 8 * 8];
        assert!(matches!(
            bloom_on_off_lift(&hdr[..63], 8, 8, &cfg, (4, 3), (0, 0)),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            bloom_on_off_lift(&hdr, 8, 8, &cfg, (8, 3), (0, 0)),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            bloom_on_off_lift(&hdr, 8, 8, &cfg, (4, 3), (0, 8)),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            bloom_on_off_lift(
                &hdr,
                8,
                8,
                &BloomConfig {
                    intensity: 3.0,
                    ..cfg
                },
                (4, 3),
                (0, 0)
            ),
            Err(PostError::Invalid(_))
        ));
    }

    #[test]
    fn kino_curve_matches_donor_setup() {
        // Curve triple pins the donor setup at threshold 1.0, soft knee 0.5.
        let c = kino_curve(1.0, 0.5);
        assert!((c[0] - 0.5).abs() < 1e-4, "curve.x is threshold minus knee");
        assert!((c[1] - 1.0).abs() < 1e-4, "curve.y is twice the knee");
        assert!((c[2] - 0.5).abs() < 1e-3, "curve.z is 0.25 over knee");
    }

    #[test]
    fn kino_weight_is_zero_low_one_high() {
        // Weight is 0 below the knee foot, 1 far above, smooth between.
        let c = kino_curve(1.0, 0.5);
        assert_eq!(kino_prefilter_weight(0.2, 1.0, c), 0.0);
        let mid = kino_prefilter_weight(1.0, 1.0, c);
        assert!(mid > 0.0 && mid < 1.0, "mid knee is partial, got {mid}");
        let hi = kino_prefilter_weight(4.0, 1.0, c);
        assert!((hi - 1.0).abs() < 1e-4, "far above is full, got {hi}");
    }

    #[test]
    fn kino_iterations_clamp_to_cap() {
        // Depth grows with height and clamps to the cap.
        assert_eq!(kino_iterations(8, 2.5, 6), 1);
        assert_eq!(kino_iterations(1080, 2.5, 6), 4);
        assert_eq!(kino_iterations(4096, 7.0, 6), 6);
    }

    #[test]
    fn kino_karis_weight_favors_dim_taps() {
        // Karis weight falls as brightness rises.
        let dim = kino_karis_weight(0.1);
        let hot = kino_karis_weight(10.0);
        assert!(dim > hot, "dim tap must outweigh the firefly");
    }

    #[test]
    fn bloom_on_off_lift_kino_curve_torch_halo_and_subknee_identity() {
        // Caller: crates/render/src/present/gpu_present/live.rs plus submit.rs
        // Behavior: Kino curve lifts the torch neighbor above off while the
        // far corner stays bit-identical, and a sub-knee field stays the
        // identity. Pins the donor curve at threshold 1.0 alongside the lift.
        let c = kino_curve(1.0, 0.5);
        assert!((c[0] - 0.5).abs() < 1e-4, "curve.x pins the donor setup");
        assert!((c[2] - 0.5).abs() < 1e-3, "curve.z pins the donor setup");
        let mut hdr = vec![[0.05, 0.05, 0.06]; 8 * 8];
        hdr[3 * 8 + 3] = [4.0, 2.2, 1.1];
        let cfg = BloomConfig::default();
        let (halo_lift, dark_lift) =
            bloom_on_off_lift(&hdr, 8, 8, &cfg, (4, 3), (0, 0)).expect("lift runs");
        assert!(halo_lift > 0.0, "halo lifts above off, got {halo_lift}");
        assert_eq!(
            dark_lift, 0.0,
            "far corner stays identical, got {dark_lift}"
        );
        let dark = vec![[0.05, 0.05, 0.06]; 8 * 8];
        let (halo_dark, dark_dark) =
            bloom_on_off_lift(&dark, 8, 8, &cfg, (4, 3), (0, 0)).expect("dark lifts");
        assert_eq!((halo_dark, dark_dark), (0.0, 0.0), "sub-knee lifts nothing");
        let on = apply_bloom(&dark, 8, 8, &cfg).expect("on blooms");
        assert_eq!(on, dark, "sub-knee on is the identity");
    }

    #[test]
    fn exposure_aces_grade_are_monotonic_and_bounded() {
        assert!(approx(exposure_mult(1.0).expect("ev"), 2.0, 1e-6));
        assert!(approx(exposure_mult(-1.0).expect("ev"), 0.5, 1e-6));
        assert!(matches!(exposure_mult(9.0), Err(PostError::Invalid(_))));
        // ACES fit pins: 0->0, 1->~0.8, monotone, saturates at 1.
        assert_eq!(aces_fitted(0.0), 0.0);
        assert!((aces_fitted(1.0) - 0.8).abs() < 0.01);
        assert!(aces_fitted(0.5) < aces_fitted(1.0));
        assert!(aces_fitted(1.0) < aces_fitted(4.0));
        assert_eq!(aces_fitted(100.0), 1.0);
        let mid = tonemap_aces([0.5, 0.5, 0.5], 1.0, 11.2).expect("tonemaps");
        let hi = tonemap_aces([2.0, 2.0, 2.0], 1.0, 11.2).expect("tonemaps");
        assert!(hi[0] > mid[0] && hi[0] <= 1.0);
        assert!(matches!(
            tonemap_aces([0.5, 0.5, 0.5], 1.0, 0.0),
            Err(PostError::Invalid(_))
        ));
        // Identity grade keeps pixels (f32 rounding: within 1e-6).
        let ident = GradeConfig::default();
        let c = [0.4, 0.2, 0.1];
        let kept = apply_grade(c, &ident).expect("grades");
        for i in 0..3 {
            assert!(
                (kept[i] - c[i]).abs() < 1e-6,
                "identity grade keeps {c:?}, got {kept:?}"
            );
        }
        let grey = apply_grade(
            c,
            &GradeConfig {
                saturation: 0.0,
                ..ident
            },
        )
        .expect("desats");
        assert!(
            (grey[0] - grey[1]).abs() < 1e-6 && (grey[1] - grey[2]).abs() < 1e-6,
            "zero saturation greys the pixel, got {grey:?}"
        );
    }

    #[test]
    fn chain_reads_rl1_rl2_and_is_deterministic() {
        // The chain consumes the RL1/RL2 receipts of the frame and quotes
        // their ms in the output receipt; same inputs run bit-identical x2.
        let hdr: Vec<[f32; 3]> = (0..256)
            .map(|i| [i as f32 * 0.01 + 0.2, 0.3, 0.15])
            .collect();
        let chain = PostChain::default();
        let run = || run_chain(&hdr, 16, 16, &chain, &lights()).expect("chain runs");
        let (a, b) = (run(), run());
        assert_eq!(a.ldr, b.ldr, "chain pixels bit-identical twice");
        assert_eq!((a.width, a.height), (16, 16));
        assert_eq!(a.ldr.len(), 256);
        for p in &a.ldr {
            assert!(p.iter().all(|c| *c >= 0.0 && *c <= 2.0));
        }
        let r = &a.receipt;
        assert_eq!(
            (r.sun_ms, r.cluster_ms, r.ibl_ms),
            (1.25, 0.5, 2.0),
            "receipt reads the RL1/RL2 pass ms"
        );
        assert!((r.lights_ms() - 3.75).abs() < 1e-9);
        assert!(r.total_ms() >= 0.0);
        for (pass, pixels) in [
            (r.bloom, 256),
            (r.exposure, 256),
            (r.aces, 256),
            (r.grade, 256),
        ] {
            assert_eq!(pass.pixels, pixels, "pass {} quotes pixels", pass.pass);
            assert!(pass.ms >= 0.0, "pass {} quotes ms", pass.pass);
        }
        // The no-device seam names the device-bound submitter instead of
        // pretending success; the live device path is gpu::Rp1Submitter.
        assert!(matches!(chain.submit_gpu(), Err(PostError::Device(_))));
    }

    #[test]
    fn full_frame_names_every_pass_in_ms() {
        // TAA resolve plus TAAU upsample plus the chain: every receipt slot
        // is filled and the output is display-bounded.
        let scene = GhostingScene::moving_bar().expect("scene builds");
        let taa = TaaParams::default();
        let chain = PostChain::default();
        let out = run_frame(
            &scene.current,
            &scene.prev,
            &scene.motion,
            scene.width,
            scene.height,
            32,
            32,
            &taa,
            &chain,
            &lights(),
        )
        .expect("frame runs");
        assert_eq!((out.width, out.height, out.ldr.len()), (32, 32, 1024));
        assert_eq!(out.receipt.taa.pixels, 256);
        assert_eq!(out.receipt.upsample.pixels, 1024);
        assert!(out.receipt.total_ms() >= 0.0);
        assert!((out.receipt.lights_ms() - 3.75).abs() < 1e-9);
        let again = run_frame(
            &scene.current,
            &scene.prev,
            &scene.motion,
            scene.width,
            scene.height,
            32,
            32,
            &taa,
            &chain,
            &lights(),
        )
        .expect("frame runs again");
        assert_eq!(out.ldr, again.ldr, "frame pixels bit-identical twice");
    }

    #[test]
    fn shimmer_converges_under_the_posted_gate() {
        // Raw checkerboard pair flickers; the clamped TAA resolve over the
        // alternating clip converges to a stable mean under SHIMMER_MAX_VAR.
        let scene = ShimmerScene::checkerboard().expect("scene builds");
        let raw_var = ShimmerScene::temporal_variance(&scene.frames);
        eprintln!("RP1 shimmer: raw variance {raw_var:.4}");
        assert!(
            raw_var > SHIMMER_MAX_VAR,
            "scene must actually shimmer, got {raw_var}"
        );
        let motion = MotionField::zero(scene.width, scene.height).expect("zero builds");
        let params = TaaParams::default();
        // Cold start from a cleared mid-grey history (the honest
        // presenter path): the resolve then tracks the alternating pair
        // into its limit cycle instead of dragging frame 0 along.
        let mut history = vec![[0.5, 0.5, 0.5]; scene.width as usize * scene.height as usize];
        let mut clip = Vec::new();
        for k in 0..12 {
            let current = &scene.frames[k % 2];
            let resolved = taa_resolve(
                current,
                &history,
                &motion,
                scene.width,
                scene.height,
                &params,
            )
            .expect("resolves");
            clip.push(resolved.clone());
            history = resolved;
        }
        let settled = ShimmerScene::temporal_variance(&clip[8..]);
        eprintln!("RP1 shimmer: settled variance {settled:.6} (gate {SHIMMER_MAX_VAR})");
        assert!(
            settled <= SHIMMER_MAX_VAR,
            "settled variance {settled} exceeds posted gate {SHIMMER_MAX_VAR}"
        );
        assert!(settled < raw_var, "TAA must calm the shimmer");
    }

    #[test]
    fn invalid_inputs_fail_loudly() {
        let motion = MotionField::zero(4, 4).expect("zero builds");
        let params = TaaParams::default();
        let frame = vec![[0.5, 0.5, 0.5]; 16];
        assert!(matches!(
            taa_resolve(&frame[..15], &frame, &motion, 4, 4, &params),
            Err(PostError::Invalid(_))
        ));
        let mut bad = frame.clone();
        bad[3] = [f32::INFINITY, 0.0, 0.0];
        assert!(matches!(
            taa_resolve(&bad, &frame, &motion, 4, 4, &params),
            Err(PostError::Invalid(_))
        ));
        let wrong = MotionField::zero(8, 8).expect("zero builds");
        assert!(matches!(
            taa_resolve(&frame, &frame, &wrong, 4, 4, &params),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            taa_resolve(
                &frame,
                &frame,
                &motion,
                4,
                4,
                &TaaParams {
                    history_blend: 2.0,
                    ..params
                }
            ),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            taa_upsample(&frame, 4, 4, 0, 4),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            taa_upsample(&frame[..15], 4, 4, 8, 8),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            taa_upsample(&frame, 4, 4, 4096, 4),
            Err(PostError::BudgetExceeded { .. })
        ));
        assert!(matches!(
            PostChain {
                aces_white: 0.0,
                ..PostChain::default()
            }
            .validate(),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            PostChain {
                bloom: BloomConfig {
                    levels: 0,
                    ..BloomConfig::default()
                },
                ..PostChain::default()
            }
            .validate(),
            Err(PostError::Invalid(_))
        ));
        let hdr = vec![[0.5, 0.5, 0.5]; 16];
        assert!(matches!(
            run_chain(&hdr, 4, 4, &PostChain::default(), &lights()).map(|_| ()),
            Ok(())
        ));
        assert!(matches!(
            run_chain(&hdr[..15], 4, 4, &PostChain::default(), &lights()),
            Err(PostError::Invalid(_))
        ));
        // PostError display names the failure without panicking.
        assert!(format!("{}", PostError::Device("needs a device".into())).contains("device"));
        assert!(format!("{}", PostError::Validation("scope hit".into())).contains("validation"));
        assert!(format!(
            "{}",
            PostError::BudgetExceeded {
                requested: 9,
                allowed: 4
            }
        )
        .contains('9'));
    }

    // ------------------------------------------------------------------
    // `v1` parity: jitter equals the R4 sequence, the glam projection
    // hook touches the same terms, ACES equals the `shadow_post`
    // Narkowicz path, and the TAAU path equals the test-only temporal
    // oracle on identical inputs. (The V1 receipt Bevy `ACESFitted` is a
    // different formulation and is not pinned here.)
    // ------------------------------------------------------------------

    #[cfg(feature = "v1")]
    #[test]
    fn jitter_and_aces_match_r4_r1_references() {
        for frame in [0u64, 1, 7, 123] {
            // Single owner: the R4 sequence is defined once, here. Pixel
            // anchors are pinned against textbook fractions in
            // `v1_lighting::tests::taa_jitter_matches_halton_values`; this
            // pins the NDC/projection composition on top of them.
            let px = jitter_offset(frame);
            let ndc = jitter_ndc(frame, 1280, 720);
            assert_eq!(
                ndc,
                [px[0] * 2.0 / 1280.0, px[1] * 2.0 / 720.0],
                "frame {frame} NDC is offset * 2 / res"
            );
            let proj = jitter_projection_glam(glam::Mat4::IDENTITY, ndc);
            assert_eq!(
                (proj.z_axis.x, proj.z_axis.y),
                (ndc[0], ndc[1]),
                "frame {frame} projection carries the NDC jitter"
            );
            assert_eq!(proj.x_axis, glam::Mat4::IDENTITY.x_axis);
            assert_eq!(proj.y_axis, glam::Mat4::IDENTITY.y_axis);
            assert_eq!(proj.w_axis, glam::Mat4::IDENTITY.w_axis);
        }
        for x in [0.0, 0.25, 1.0, 4.0] {
            assert_eq!(
                aces_fitted(x),
                crate::shadow_post::aces_fitted(x),
                "aces fit matches the shadow_post path at {x}"
            );
        }
        let src: Vec<[f32; 3]> = (0..64).map(|i| [i as f32 * 0.01, 0.2, 0.05]).collect();
        let ours = taa_upsample(&src, 8, 8, 16, 16).expect("taau runs");
        let theirs = crate::v1_temporal::upscale_bilinear(&src, 8, 8, 16, 16).expect("oracle runs");
        assert_eq!(ours, theirs, "TAAU equals the test-only oracle exactly");
    }

    #[cfg(feature = "v1")]
    #[test]
    fn postlights_reads_rl1_rl2_receipts() {
        // The taps constructor quotes the exact RL1/RL2 pass ms: the chain
        // reads the passes, never guesses them.
        let sun =
            crate::shadow_cluster::SunShadowPass::build(crate::shadow_cluster::SunShadowConfig {
                near_m: 0.5,
                far_m: 60.0,
                half_extents: [4.0, 8.0, 16.0],
                biases: [0.002, 0.004, 0.008],
                size: 128,
            })
            .expect("sun builds");
        let cluster = crate::shadow_cluster::ClusterPass::build(
            crate::shadow_cluster::ClusterConfig {
                width_px: 64,
                height_px: 64,
                tile_px: crate::shadow_cluster::CLUSTER_TILE_PX,
                depth_slices: 4,
                near_m: 0.5,
                far_m: 60.0,
                max_lights: crate::shadow_cluster::MAX_TORCH_LIGHTS,
                max_per_cluster: crate::shadow_cluster::MAX_LIGHTS_PER_CLUSTER,
            },
            &[],
        )
        .expect("cluster builds");
        let lights = crate::shadow_cluster::FrameLightsReceipt {
            sun: sun.receipt(1.25),
            cluster: cluster.receipt(),
        };
        let ibl = crate::ibl::IblReceipt {
            cube_texels: 1,
            lut_texels: 1,
            sh_coeffs: crate::ibl::IBL_SH_COEFFS as u32,
            ms: 2.0,
        };
        let taps = PostLights::from_receipts(&lights, &ibl);
        assert_eq!(
            (taps.sun_ms, taps.cluster_ms, taps.ibl_ms),
            (1.25, cluster.last_ms, 2.0)
        );
    }

    // ------------------------------------------------------------------
    // GPU descriptors behind `window`: pure format/extent checks plus a
    // real history allocation that skips without an adapter (RA1 pattern).
    // ------------------------------------------------------------------

    #[cfg(feature = "window")]
    #[test]
    fn post_descriptors_are_sampler_readable() {
        let hist = gpu::history_desc(128, 72);
        assert_eq!(hist.format, wgpu::TextureFormat::Rgba32Float);
        assert_eq!((hist.size.width, hist.size.height), (128, 72));
        assert!(hist.usage.contains(wgpu::TextureUsages::RENDER_ATTACHMENT));
        assert!(hist.usage.contains(wgpu::TextureUsages::TEXTURE_BINDING));
        let target = gpu::post_target_desc(128, 72);
        assert_eq!(target.format, wgpu::TextureFormat::Rgba16Float);
        assert!(!gpu::TAA_RESOLVE_WGSL.is_empty());
        assert!(!gpu::POST_CHAIN_WGSL.is_empty());
        assert!(matches!(
            gpu::check_history_dims(0, 0),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            gpu::check_history_dims(4096, 64),
            Err(PostError::BudgetExceeded { .. })
        ));
        gpu::check_history_dims(64, 64).expect("valid dims pass");
    }

    #[cfg(feature = "window")]
    #[test]
    fn history_allocates_on_llvmpipe() {
        crate::test_gpu::hold_gpu_slot();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter =
            match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })) {
                Ok(a) => a,
                Err(_) => {
                    eprintln!("SKIPPED post gpu test: no wgpu adapter on this machine");
                    return;
                }
            };
        let (device, _queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("rp1-post-history"),
                ..Default::default()
            }))
            .expect("device requests succeed when the adapter exists");
        let tex = gpu::allocate_history(&device, 64, 64).expect("history allocates");
        assert_eq!(tex.width(), 64);
        assert!(matches!(
            gpu::allocate_history(&device, 4096, 64),
            Err(PostError::BudgetExceeded { .. })
        ));
    }

    #[test]
    fn jittered_resolve_is_plain_at_zero_and_shifts_otherwise() {
        // Zero jitter is exactly the plain resolve; a real Halton offset
        // shifts sub-pixel sampling on the bar edge.
        let scene = GhostingScene::moving_bar().expect("scene builds");
        let params = TaaParams::default();
        let plain = taa_resolve(
            &scene.current,
            &scene.prev,
            &scene.motion,
            scene.width,
            scene.height,
            &params,
        )
        .expect("plain resolves");
        let zero = taa_resolve_jittered(
            &scene.current,
            &scene.prev,
            &scene.motion,
            scene.width,
            scene.height,
            &params,
            [0.0, 0.0],
        )
        .expect("zero jitter resolves");
        assert_eq!(plain, zero, "zero jitter is the plain path");
        let j = jitter_offset(1);
        assert!(j != [0.0, 0.0], "frame 1 jitters off the origin");
        let shifted = taa_resolve_jittered(
            &scene.current,
            &scene.prev,
            &scene.motion,
            scene.width,
            scene.height,
            &params,
            j,
        )
        .expect("jittered resolves");
        assert_ne!(shifted, plain, "jitter shifts the bar edge sampling");
        let again = taa_resolve_jittered(
            &scene.current,
            &scene.prev,
            &scene.motion,
            scene.width,
            scene.height,
            &params,
            j,
        )
        .expect("jittered resolves again");
        assert_eq!(shifted, again, "jittered pixels bit-identical twice");
        assert!(matches!(
            taa_resolve_jittered(
                &scene.current,
                &scene.prev,
                &scene.motion,
                scene.width,
                scene.height,
                &params,
                [f32::NAN, 0.0]
            ),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            taa_resolve_jittered(
                &scene.current,
                &scene.prev,
                &scene.motion,
                scene.width,
                scene.height,
                &params,
                [2.0, 0.0]
            ),
            Err(PostError::Invalid(_))
        ));
    }

    #[test]
    fn rp1_graph_declares_six_passes_in_order() {
        // The RA2 declaration names the six RP1 stages in pipeline order;
        // the compiled order pins the same sequence and the CPU executor
        // records one draw per pass.
        let chain = PostChain::default();
        let graph = build_rp1_graph(16, 16, 32, 32, &chain).expect("graph builds");
        assert_eq!(graph.pass_count(), 6);
        assert_eq!(graph.pass(0).map(|p| p.name.as_str()), Some(RP1_PASS_TAA));
        let mut compiled = compile_rp1_for_present(16, 16, 32, 32, &chain).expect("compiles");
        let names: Vec<&str> = compiled
            .order()
            .iter()
            .filter_map(|id| compiled.pass(*id).map(|p| p.name.as_str()))
            .collect();
        assert_eq!(names, rp1_pass_names(), "compiled order is the chain order");
        let receipt = compiled.execute(&crate::graph::FrameInfo {
            frame_index: 7,
            width_px: 32,
            height_px: 32,
        });
        assert_eq!(receipt.draws, 6, "one draw per RP1 pass");
        assert_eq!(receipt.per_pass_cpu_ms.len(), 6);
        assert_eq!(receipt.frame_index, 7);
    }

    #[test]
    fn rp1_graph_rejects_bad_sizes_and_counts_bytes() {
        let chain = PostChain::default();
        assert!(matches!(
            build_rp1_graph(0, 16, 16, 16, &chain),
            Err(PostError::Invalid(_))
        ));
        build_rp1_graph(16, 16, 16, 16, &chain).expect("square present is fine");
        assert!(matches!(
            build_rp1_graph(4096, 16, 16, 16, &chain),
            Err(PostError::BudgetExceeded { .. })
        ));
        assert!(matches!(
            build_rp1_graph(
                16,
                16,
                16,
                16,
                &PostChain {
                    bloom: BloomConfig {
                        levels: 0,
                        ..BloomConfig::default()
                    },
                    ..PostChain::default()
                }
            ),
            Err(PostError::Invalid(_))
        ));
        let small = rp1_transient_bytes(16, 16, 16, 16, 1).expect("bytes count");
        let deep = rp1_transient_bytes(16, 16, 16, 16, 4).expect("bytes count");
        assert!(
            small > 0 && deep > small,
            "more bloom levels need more bytes"
        );
        assert!(matches!(
            rp1_transient_bytes(16, 16, 16, 16, 0),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            rp1_transient_bytes(4096, 16, 16, 16, 4),
            Err(PostError::BudgetExceeded { .. })
        ));
    }

    #[cfg(feature = "window")]
    fn test_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        crate::test_gpu::hold_gpu_slot();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter =
            match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })) {
                Ok(a) => a,
                Err(_) => {
                    eprintln!("SKIPPED post gpu test: no wgpu adapter on this machine");
                    return None;
                }
            };
        match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("rp1-test"),
            ..Default::default()
        })) {
            Ok((d, q)) => Some((d, q)),
            Err(e) => panic!("test device requests succeed when the adapter exists: {e}"),
        }
    }

    #[cfg(feature = "window")]
    #[test]
    fn gpu_taa_kills_ghost_and_matches_cpu() {
        // Device TAA over the moving-bar scene: reprojection plus jitter
        // plus box clamp on the device, parity against the CPU jittered
        // oracle, trail under the posted gate, 0 validation errors.
        let (device, queue) = match test_device() {
            Some(d) => d,
            None => return,
        };
        let sub = gpu::Rp1Submitter::new(&device);
        let scene = GhostingScene::moving_bar().expect("scene builds");
        let params = TaaParams::default();
        let jit = jitter_offset(7);
        let cpu = taa_resolve_jittered(
            &scene.current,
            &scene.prev,
            &scene.motion,
            scene.width,
            scene.height,
            &params,
            jit,
        )
        .expect("cpu jittered resolves");
        let g = sub
            .resolve(
                &device,
                &queue,
                &scene.current,
                &scene.prev,
                &scene.motion,
                scene.width,
                scene.height,
                &params,
                jit,
            )
            .expect("gpu resolves");
        assert_eq!(g.validation_errors, 0, "gpu resolve is validation clean");
        let diff = gpu::max_abs_diff(&cpu, &g.pixels);
        let trail = GhostingScene::trail_luma(&g.pixels, scene.width, scene.trail);
        eprintln!(
            "RP1 gpu ghost: trail {trail:.4} (gate {GHOST_MAX_TRAIL}), parity {diff:.6} (tol {})",
            gpu::GPU_PARITY_TOL_TAA
        );
        assert!(
            diff <= gpu::GPU_PARITY_TOL_TAA,
            "gpu/cpu parity {diff} exceeds {}",
            gpu::GPU_PARITY_TOL_TAA
        );
        assert!(
            trail <= GHOST_MAX_TRAIL,
            "gpu trail {trail} exceeds posted gate {GHOST_MAX_TRAIL}"
        );
        // Failure case first: motion/frame mismatch fails before any submit.
        let wrong = MotionField::zero(8, 8).expect("zero builds");
        assert!(matches!(
            sub.resolve(
                &device,
                &queue,
                &scene.current,
                &scene.prev,
                &wrong,
                scene.width,
                scene.height,
                &params,
                jit
            ),
            Err(PostError::Invalid(_))
        ));
    }

    #[cfg(feature = "window")]
    #[test]
    fn gpu_shimmer_settles_and_matches_cpu() {
        // Alternating checkerboard through the device resolve with the
        // per-frame Halton jitter: the settled clip variance stays under
        // the posted gate and every GPU frame matches its CPU twin.
        let (device, queue) = match test_device() {
            Some(d) => d,
            None => return,
        };
        let sub = gpu::Rp1Submitter::new(&device);
        let scene = ShimmerScene::checkerboard().expect("scene builds");
        let motion = MotionField::zero(scene.width, scene.height).expect("zero builds");
        let params = TaaParams::default();
        let mut history = vec![[0.5, 0.5, 0.5]; scene.width as usize * scene.height as usize];
        let mut clip = Vec::new();
        let mut worst_diff = 0f32;
        for k in 0..12 {
            let current = &scene.frames[k % 2];
            let jit = jitter_offset(k as u64);
            let cpu = taa_resolve_jittered(
                current,
                &history,
                &motion,
                scene.width,
                scene.height,
                &params,
                jit,
            )
            .expect("cpu resolves");
            let g = sub
                .resolve(
                    &device,
                    &queue,
                    current,
                    &history,
                    &motion,
                    scene.width,
                    scene.height,
                    &params,
                    jit,
                )
                .expect("gpu resolves");
            assert_eq!(g.validation_errors, 0);
            worst_diff = worst_diff.max(gpu::max_abs_diff(&cpu, &g.pixels));
            clip.push(g.pixels.clone());
            history = g.pixels;
        }
        let settled = ShimmerScene::temporal_variance(&clip[8..]);
        eprintln!(
            "RP1 gpu shimmer: settled {settled:.6} (gate {SHIMMER_MAX_VAR}), worst parity {worst_diff:.6}"
        );
        assert!(
            worst_diff <= gpu::GPU_PARITY_TOL_TAA,
            "gpu/cpu parity {worst_diff} exceeds {}",
            gpu::GPU_PARITY_TOL_TAA
        );
        assert!(
            settled <= SHIMMER_MAX_VAR,
            "gpu settled variance {settled} exceeds posted gate {SHIMMER_MAX_VAR}"
        );
    }

    #[cfg(feature = "window")]
    #[test]
    fn gpu_full_frame_matches_cpu_chain() {
        // Whole RP1 frame on the device in graph order (TAA at 16x16,
        // TAAU to 32x32, bloom plus exposure plus ACES plus grade): pixels
        // match the CPU oracle within tolerance, every receipt slot is
        // filled, 0 validation errors.
        let (device, queue) = match test_device() {
            Some(d) => d,
            None => return,
        };
        let sub = gpu::Rp1Submitter::new(&device);
        let scene = GhostingScene::moving_bar().expect("scene builds");
        let taa = TaaParams::default();
        let chain = PostChain::default();
        let jit = [0.0, 0.0];
        let g = sub
            .run_frame(
                &device,
                &queue,
                &scene.current,
                &scene.prev,
                &scene.motion,
                scene.width,
                scene.height,
                32,
                32,
                &taa,
                &chain,
                &lights(),
                jit,
            )
            .expect("gpu frame runs");
        assert_eq!(g.validation_errors, 0, "gpu frame is validation clean");
        let cpu = run_frame(
            &scene.current,
            &scene.prev,
            &scene.motion,
            scene.width,
            scene.height,
            32,
            32,
            &taa,
            &chain,
            &lights(),
        )
        .expect("cpu frame runs");
        assert_eq!((g.width, g.height, g.ldr.len()), (32, 32, 1024));
        // Bloom-shape sweep at 16x16: same-size (TAAU identity), bloom-off
        // (base path only), single/two-level chains and a full-flood
        // stress every corner of the composite against the CPU oracle.
        let shapes: [(&str, PostChain); 5] = [
            ("same-size", PostChain::default()),
            (
                "bloom-off",
                PostChain {
                    bloom: BloomConfig {
                        intensity: 0.0,
                        ..BloomConfig::default()
                    },
                    ..PostChain::default()
                },
            ),
            (
                "flood",
                PostChain {
                    bloom: BloomConfig {
                        threshold: 0.0,
                        knee: 0.0,
                        intensity: 1.0,
                        ..BloomConfig::default()
                    },
                    ..PostChain::default()
                },
            ),
            (
                "one-level",
                PostChain {
                    bloom: BloomConfig {
                        levels: 1,
                        ..BloomConfig::default()
                    },
                    ..PostChain::default()
                },
            ),
            (
                "two-level",
                PostChain {
                    bloom: BloomConfig {
                        levels: 2,
                        ..BloomConfig::default()
                    },
                    ..PostChain::default()
                },
            ),
        ];
        for (name, cfg) in shapes {
            let gf = sub
                .run_frame(
                    &device,
                    &queue,
                    &scene.current,
                    &scene.prev,
                    &scene.motion,
                    scene.width,
                    scene.height,
                    16,
                    16,
                    &taa,
                    &cfg,
                    &lights(),
                    jit,
                )
                .expect("shape gpu frame runs");
            let cf = run_frame(
                &scene.current,
                &scene.prev,
                &scene.motion,
                scene.width,
                scene.height,
                16,
                16,
                &taa,
                &cfg,
                &lights(),
            )
            .expect("shape cpu frame runs");
            let diff = gpu::max_abs_diff(&cf.ldr, &gf.ldr);
            eprintln!("RP1 gpu frame shape {name}: parity {diff:.6}");
            assert_eq!(gf.validation_errors, 0);
            assert!(
                diff <= gpu::GPU_PARITY_TOL_POST,
                "shape {name} parity {diff} exceeds {}",
                gpu::GPU_PARITY_TOL_POST
            );
        }
        eprintln!(
            "RP1 gpu frame: parity {:.6} (tol {}), submit {:.2}ms (budget {}ms, evidence only)",
            g.parity_max_diff,
            gpu::GPU_PARITY_TOL_POST,
            g.submit_ms,
            gpu::GPU_SUBMIT_BUDGET_MS
        );
        assert!(
            g.parity_max_diff <= gpu::GPU_PARITY_TOL_POST,
            "gpu/cpu frame parity {} exceeds {}",
            g.parity_max_diff,
            gpu::GPU_PARITY_TOL_POST
        );
        assert_eq!(gpu::max_abs_diff(&cpu.ldr, &g.ldr), g.parity_max_diff);
        assert_eq!(
            (g.receipt.taa.pixels, g.receipt.upsample.pixels),
            (256, 1024)
        );
        assert_eq!(g.receipt.taa.pass, "taa");
        assert_eq!(g.receipt.upsample.pass, "taau");
        assert_eq!(g.receipt.bloom.pass, "bloom");
        assert!((g.receipt.lights_ms() - 3.75).abs() < 1e-9);
        // Timing is evidence only (PF2 owns budgets on the reference GPU):
        // debug llvmpipe under pool contention is noisy by construction.
        // Jittered device frame matches the jittered CPU pipeline too.
        let jit2 = jitter_offset(3);
        let g2 = sub
            .run_frame(
                &device,
                &queue,
                &scene.current,
                &scene.prev,
                &scene.motion,
                scene.width,
                scene.height,
                16,
                16,
                &taa,
                &chain,
                &lights(),
                jit2,
            )
            .expect("jittered gpu frame runs");
        let resolved = taa_resolve_jittered(
            &scene.current,
            &scene.prev,
            &scene.motion,
            16,
            16,
            &taa,
            jit2,
        )
        .expect("cpu jittered resolves");
        let up = taa_upsample(&resolved, 16, 16, 16, 16).expect("cpu upscales");
        let oracle = run_chain(&up, 16, 16, &chain, &lights()).expect("cpu chain runs");
        let diff2 = gpu::max_abs_diff(&oracle.ldr, &g2.ldr);
        eprintln!("RP1 gpu jittered frame: parity {diff2:.6}");
        assert!(
            diff2 <= gpu::GPU_PARITY_TOL_POST,
            "jittered gpu/cpu parity {diff2} exceeds {}",
            gpu::GPU_PARITY_TOL_POST
        );
    }

    #[test]
    fn rp5_scale_seam_drives_presented_size() {
        // RP5 production wiring: the post/TAAU seam feeds the live
        // ScaleController into `compile_taa_off_for_scale` so the
        // SMAA-first graph drives the presented size. Under load the
        // hysteresis floors the scale, the floor picks the render size,
        // and the compiled graph is SMAA-first with the RP1 TAAU plus
        // chain after it; clearing the load recovers full rate.
        use crate::aa_scale::{
            taa_off_pass_names, ScaleController, ScalePolicy, SmaaParams, TARGET_60FPS_MS,
        };
        let policy = ScalePolicy {
            over_frames: 2,
            under_frames: 4,
            ..ScalePolicy::default()
        };
        let mut ctl = ScaleController::new(policy).expect("policy builds");
        let chain = PostChain::default();
        // Sustained overload floors the scale through the per-present
        // tick: every present feeds PF2 frame ms through the hysteresis
        // and recompiles the SMAA-first graph at the new scale.
        for _ in 0..40 {
            tick_scale_for_present(&mut ctl, TARGET_60FPS_MS + 9.0, 1920, 1080, &chain)
                .expect("tick compiles");
        }
        assert_eq!(ctl.scale(), policy.min_scale, "load floors the scale");
        // Scale-hold figure: pixel-proportional cost at the floor tail
        // holds 60 fps (fixed 4ms plus 20ms at full rate, quadratic).
        let tail_ms = 4.0 + 20.0 * f64::from(ctl.scale() * ctl.scale());
        eprintln!(
            "RP5 scale under load: scale {:.2}, down {}, up {}, tail {tail_ms:.2}ms (target {TARGET_60FPS_MS})",
            ctl.scale(),
            ctl.steps_down,
            ctl.steps_up
        );
        assert!(
            tail_ms <= TARGET_60FPS_MS,
            "tail cost {tail_ms:.2}ms must hold 60 fps"
        );
        let (compiled, rw, rh) =
            compile_taa_off_for_scale(&ctl, 1920, 1080, &chain).expect("floor compiles");
        assert_eq!((rw, rh), (960, 540));
        assert_eq!(compiled.order().len(), 6);
        assert!(
            compiled.order_names_match(&taa_off_pass_names()),
            "taa-off leads with smaa through the graph seam"
        );
        // CPU oracle at the controller scale runs bit-identical twice and
        // lands at the presented size (small present keeps the debug
        // suite fast; the 1080p floor compile above posts the numbers).
        let smaa = SmaaParams::default();
        let hdr: Vec<[f32; 3]> = (0..160usize * 90usize)
            .map(|i| [i as f32 * 0.00001 + 0.2, 0.3, 0.15])
            .collect();
        let (a, b) = (
            run_taa_off_frame_for_scale(&hdr, &ctl, 320, 180, &smaa, &chain, &lights())
                .expect("scale frame runs"),
            run_taa_off_frame_for_scale(&hdr, &ctl, 320, 180, &smaa, &chain, &lights())
                .expect("scale frame runs again"),
        );
        assert_eq!(a.ldr, b.ldr, "scale frame pixels bit-identical twice");
        assert_eq!((a.width, a.height), (320, 180));
        assert_eq!(a.receipt.smaa.pixels, 160 * 90);
        // Clearing the load climbs back to full rate with the full-size graph.
        for _ in 0..120 {
            tick_scale_for_present(&mut ctl, 8.0, 1920, 1080, &chain).expect("tick compiles");
        }
        assert_eq!(ctl.scale(), 1.0, "cleared load recovers full rate");
        let (full, frw, frh) =
            compile_taa_off_for_scale(&ctl, 1920, 1080, &chain).expect("full compiles");
        assert_eq!((frw, frh), (1920, 1080));
        assert!(full.order_names_match(&taa_off_pass_names()));
        eprintln!("RP5 scale graph: floor {rw}x{rh}, full {frw}x{frh}");
    }

    #[test]
    fn rp5_scale_seam_fails_loudly() {
        // Failure edges: zero and over-bound present sizes, non-finite
        // frame ms, and a non-SMAA-first order all fail loudly (never a
        // silent wrong-size present); error kinds survive the aa_scale
        // mapping.
        use crate::aa_scale::{ScaleController, ScalePolicy, SmaaParams};
        let mut ctl = ScaleController::new(ScalePolicy::default()).expect("policy builds");
        let chain = PostChain::default();
        assert!(matches!(
            compile_taa_off_for_scale(&ctl, 0, 1080, &chain),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            compile_taa_off_for_scale(&ctl, 4096, 1080, &chain),
            Err(PostError::BudgetExceeded { .. })
        ));
        assert!(matches!(
            tick_scale_for_present(&mut ctl, f64::NAN, 1920, 1080, &chain),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            tick_scale_for_present(&mut ctl, -1.0, 1920, 1080, &chain),
            Err(PostError::Invalid(_))
        ));
        // The graph seam itself: RP1 order is not SMAA-first.
        let rp1 = compile_rp1_for_present(16, 16, 32, 32, &chain).expect("rp1 compiles");
        assert!(!rp1.order_names_match(&crate::aa_scale::taa_off_pass_names()));
        assert!(rp1.order_names_match(&rp1_pass_names()));
        // Undersized hdr for the controller render size fails loudly.
        let smaa = SmaaParams::default();
        let short = vec![[0.5, 0.5, 0.5]; 16];
        assert!(matches!(
            run_taa_off_frame_for_scale(&short, &ctl, 32, 32, &smaa, &chain, &lights()),
            Err(PostError::Invalid(_))
        ));
        // Error-kind mapping keeps BudgetExceeded across the seam.
        let mapped = PostError::from(crate::aa_scale::AaScaleError::BudgetExceeded {
            requested: 4096,
            allowed: 2048,
        });
        assert!(matches!(
            mapped,
            PostError::BudgetExceeded {
                requested: 4096,
                allowed: 2048
            }
        ));
    }

    #[test]
    fn post_frame_budget_accessors_are_exact() {
        // Exactness default (no wall clock): the posted full-post bar is
        // 1.5 ms at 1080p on boosted release clocks (PF2 enforces); synthetic
        // receipts pin the flag both ways. Wall-clock enforcement lives in
        // post_chain_wall_budget (--perf only, load-dependent).
        const POST_FRAME_BUDGET_MS: f64 = 1.5;
        let flat = |ms: f64| FramePostReceipt {
            taa: PassReceipt {
                pass: "taa",
                pixels: 256,
                ms,
            },
            upsample: PassReceipt {
                pass: "taau",
                pixels: 1024,
                ms,
            },
            bloom: PassReceipt {
                pass: "bloom",
                pixels: 1024,
                ms,
            },
            exposure: PassReceipt {
                pass: "exposure",
                pixels: 1024,
                ms,
            },
            aces: PassReceipt {
                pass: "aces",
                pixels: 1024,
                ms,
            },
            grade: PassReceipt {
                pass: "grade",
                pixels: 1024,
                ms,
            },
            sun_ms: 1.25,
            cluster_ms: 0.5,
            ibl_ms: 2.0,
        };
        assert!((flat(0.1).total_ms() - 0.6).abs() < 1e-12);
        assert!(flat(0.1).total_ms() <= POST_FRAME_BUDGET_MS);
        assert!(flat(9.9).total_ms() > POST_FRAME_BUDGET_MS);
        assert!((flat(0.1).lights_ms() - 3.75).abs() < 1e-9);
    }

    #[test]
    #[ignore = "timing: post chain wall-clock needs release --perf on boosted clocks (fails under parallel load)"]
    fn post_chain_wall_budget() {
        // Seeded budget regression: the same 16x16 chain the default suite
        // proves exact must clear the 1.5 ms bar on boosted release clocks.
        // Perf harness line keeps --perf medians readable.
        const POST_FRAME_BUDGET_MS: f64 = 1.5;
        let chain = PostChain::default();
        let hdr = vec![[0.35, 0.3, 0.25]; 16 * 16];
        let out = run_chain(&hdr, 16, 16, &chain, &lights()).expect("chain runs");
        eprintln!(
            "post_chain_wall_budget: measured {:.3} ms vs {:.1} ms budget",
            out.receipt.total_ms(),
            POST_FRAME_BUDGET_MS
        );
        assert!(
            out.receipt.total_ms() <= POST_FRAME_BUDGET_MS,
            "seeded chain over budget: {:.3} ms vs {:.1} ms",
            out.receipt.total_ms(),
            POST_FRAME_BUDGET_MS
        );
    }

    // ------------------------------------------------------------------
    // Auto-exposure: histogram, trim, adaptation, fixtures, cost.
    // ------------------------------------------------------------------

    #[test]
    fn auto_histogram_bins_edges_and_validation() {
        // A flat key field lands in one bin with every texel counted.
        let flat = vec![[AUTO_KEY_LUMA, AUTO_KEY_LUMA, AUTO_KEY_LUMA]; 16];
        let hist = histogram_log_luma(&flat, 4, 4).expect("histograms");
        assert_eq!(hist.texels, 16.0);
        let filled: Vec<usize> = hist
            .bins
            .iter()
            .enumerate()
            .filter(|(_, count)| **count > 0.0)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(filled.len(), 1, "flat field fills one bin");
        assert_eq!(hist.bins[filled[0]], 16.0);
        // Determinism: the same frame bins identically twice.
        let again = histogram_log_luma(&flat, 4, 4).expect("histograms again");
        assert_eq!(hist, again);
        // Non-positive luma parks in bin 0 (deep shadow, never NaN).
        let black = vec![[0.0, 0.0, 0.0]; 4];
        let parked = histogram_log_luma(&black, 2, 2).expect("black histograms");
        assert_eq!(parked.bins[0], 4.0);
        // Validation matches the chain inputs.
        assert!(matches!(
            histogram_log_luma(&flat[..15], 4, 4),
            Err(PostError::Invalid(_))
        ));
        let mut nan = flat.clone();
        nan[3] = [f32::NAN, 0.0, 0.0];
        assert!(matches!(
            histogram_log_luma(&nan, 4, 4),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            histogram_log_luma(&flat, 0, 4),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            histogram_log_luma(&flat, 4096, 4),
            Err(PostError::BudgetExceeded { .. })
        ));
    }

    #[test]
    fn auto_trim_and_target_math() {
        // Four grey texels one stop apart: the full-band mean is the middle
        // (bin centers quantize, so the gate is a fifth of a stop).
        let hdr = vec![
            [1.0, 1.0, 1.0],
            [2.0, 2.0, 2.0],
            [4.0, 4.0, 4.0],
            [8.0, 8.0, 8.0],
        ];
        let hist = histogram_log_luma(&hdr, 2, 2).expect("histograms");
        assert_eq!(hist.texels, 4.0);
        let full = trimmed_log_mean(&hist, 0.0, 1.0).expect("means");
        assert!(
            (full - 1.5).abs() <= 0.2,
            "full band means the middle stops, got {full}"
        );
        // Narrow bands read below/above the full mean: trim is order-correct.
        let dark_half = trimmed_log_mean(&hist, 0.0, 0.5).expect("dark means");
        let bright_half = trimmed_log_mean(&hist, 0.5, 1.0).expect("bright means");
        assert!(
            dark_half < full && full < bright_half,
            "bands order {dark_half} < {full} < {bright_half}"
        );
        // Torch-grid discipline: sparse highlights barely move the mean.
        let scene = ExposureScene::day_sweep_torch().expect("scene builds");
        let dark_hist =
            histogram_log_luma(&scene.dark, scene.width, scene.height).expect("dark histograms");
        let torch_hist =
            histogram_log_luma(&scene.torch, scene.width, scene.height).expect("torch histograms");
        let dark_mean =
            trimmed_log_mean(&dark_hist, AUTO_TRIM_LO, AUTO_TRIM_HI).expect("dark means");
        let torch_mean =
            trimmed_log_mean(&torch_hist, AUTO_TRIM_LO, AUTO_TRIM_HI).expect("torch means");
        assert!(
            (torch_mean - dark_mean).abs() < AUTO_FLICKER_MAX_EV,
            "trim keeps torches out: dark {dark_mean} vs torch {torch_mean}"
        );
        // Key math: the mean of key needs no correction, exactly.
        let key_log = AUTO_KEY_LUMA.log2();
        assert_eq!(
            target_ev_for_mean(key_log, AUTO_KEY_LUMA).expect("targets"),
            0.0
        );
        // A mean one stop brighter costs exactly one stop (1e-6 for libm).
        let darker = target_ev_for_mean(key_log + 1.0, AUTO_KEY_LUMA).expect("targets");
        assert!(
            (darker - -1.0).abs() < 1e-6,
            "one stop brighter costs one stop, got {darker}"
        );
    }

    #[test]
    fn auto_exposure_config_validates() {
        AutoExposureConfig::default()
            .validate()
            .expect("default validates");
        assert_eq!(AutoExposure::default().ev(), 0.0);
        AutoExposure::new(AutoExposureConfig::default()).expect("default builds");
        for cfg in [
            AutoExposureConfig {
                trim_lo: -0.1,
                ..AutoExposureConfig::default()
            },
            AutoExposureConfig {
                trim_hi: 1.5,
                ..AutoExposureConfig::default()
            },
            AutoExposureConfig {
                trim_lo: 0.9,
                trim_hi: 0.1,
                ..AutoExposureConfig::default()
            },
            AutoExposureConfig {
                key_luma: 0.0,
                ..AutoExposureConfig::default()
            },
            AutoExposureConfig {
                adapt_ev_per_s: 0.0,
                ..AutoExposureConfig::default()
            },
            AutoExposureConfig {
                min_ev: 2.0,
                max_ev: 1.0,
                ..AutoExposureConfig::default()
            },
            AutoExposureConfig {
                min_ev: -9.0,
                ..AutoExposureConfig::default()
            },
            AutoExposureConfig {
                start_ev: 9.0,
                ..AutoExposureConfig::default()
            },
        ] {
            assert!(
                matches!(AutoExposure::new(cfg), Err(PostError::Invalid(_))),
                "bad config {cfg:?} must fail"
            );
        }
    }

    #[test]
    fn auto_exposure_converges_and_holds() {
        let scene = ExposureScene::day_sweep_torch().expect("scene builds");
        let bright = &scene.sweep[4];
        let hist = histogram_log_luma(bright, scene.width, scene.height).expect("histograms");
        let mean = trimmed_log_mean(&hist, AUTO_TRIM_LO, AUTO_TRIM_HI).expect("means");
        let target = target_ev_for_mean(mean, AUTO_KEY_LUMA).expect("targets");
        assert!(
            target < 0.0,
            "bright sweep wants darker exposure, got {target}"
        );
        // Slew dynamics: 3 stops/s closes the gap, then lands on target.
        let mut state = AutoExposure::default();
        let first = state.tick(bright, 16, 16, 1.0).expect("ticks");
        assert!(
            (-AUTO_ADAPT_EV_PER_S..0.0).contains(&first),
            "first second slews down at most the rate, got {first}"
        );
        for _ in 0..4 {
            state.tick(bright, 16, 16, 1.0).expect("ticks");
        }
        assert!(
            (state.ev() - target).abs() < 1e-5,
            "slew lands on target: got {} want {target}",
            state.ev()
        );
        // Holding: a zero dt never moves, whatever the frame.
        let before = state.ev();
        state.tick(&scene.sweep[0], 16, 16, 0.0).expect("holds");
        assert_eq!(state.ev(), before, "zero dt holds the state");
        // Twin states track bit-identically (replays and captures agree).
        let mut a = AutoExposure::default();
        let mut b = AutoExposure::default();
        for k in 0..6 {
            let frame = &scene.sweep[k % scene.sweep.len()];
            let ea = a.tick(frame, 16, 16, 1.0 / 60.0).expect("ticks");
            let eb = b.tick(frame, 16, 16, 1.0 / 60.0).expect("ticks");
            assert_eq!(ea, eb, "twin states agree on frame {k}");
        }
        // Multiplier matches the stops (1e-6 for the libm path).
        assert!((a.mult() - 2f32.powf(a.ev())).abs() < 1e-6);
    }

    #[test]
    fn auto_exposure_beats_fixed_on_day_sweep() {
        // The accept metric: fixed stops clip the bright sweep end, the
        // converged adapted state cuts the clipped share by 15% or more.
        let scene = ExposureScene::day_sweep_torch().expect("scene builds");
        let bright = &scene.sweep[4];
        let fixed = PostChain::default();
        let fixed_out =
            run_chain(bright, scene.width, scene.height, &fixed, &lights()).expect("chain runs");
        let fixed_clip = clipped_share(&fixed_out.ldr);
        assert!(
            fixed_clip > 0.2,
            "bright sweep must actually clip under fixed exposure, got {fixed_clip}"
        );
        let mut auto_state = AutoExposure::default();
        for _ in 0..3 {
            auto_state
                .tick(bright, scene.width, scene.height, 1.0)
                .expect("ticks");
        }
        assert!(
            auto_state.ev() < 0.0,
            "bright sweep adapts down, got {}",
            auto_state.ev()
        );
        let adapted = fixed.with_exposure_ev(auto_state.ev()).expect("adapts");
        let auto_out = run_chain(bright, scene.width, scene.height, &adapted, &lights())
            .expect("adapted chain runs");
        let auto_clip = clipped_share(&auto_out.ldr);
        let improvement = (fixed_clip - auto_clip) / fixed_clip;
        eprintln!(
            "day sweep: fixed clipped {fixed_clip:.3}, adapted {auto_clip:.3} at {:.2} stops, improvement {improvement:.3} (gate {})",
            auto_state.ev(),
            AUTO_CLIP_IMPROVE_MIN
        );
        assert!(
            improvement >= AUTO_CLIP_IMPROVE_MIN,
            "adapted improvement {improvement} misses {AUTO_CLIP_IMPROVE_MIN}"
        );
    }

    #[test]
    fn auto_exposure_ignores_torch_grid_and_limits_flicker() {
        let scene = ExposureScene::day_sweep_torch().expect("scene builds");
        // Converge on the dark field, then strike the torches: the trimmed
        // mean barely moves, so one 60 Hz step barely follows.
        let mut state = AutoExposure::default();
        for _ in 0..3 {
            state
                .tick(&scene.dark, scene.width, scene.height, 1.0)
                .expect("ticks");
        }
        let dark_ev = state.ev();
        let step = state
            .tick(&scene.torch, scene.width, scene.height, 1.0 / 60.0)
            .expect("ticks");
        assert!(
            (step - dark_ev).abs() <= AUTO_ADAPT_EV_PER_S / 60.0 + 1e-6,
            "torch strike steps at most one slew quantum, got {}",
            (step - dark_ev).abs()
        );
        // Settle fully on the torch grid: steady state stays in the gate.
        for _ in 0..600 {
            state
                .tick(&scene.torch, scene.width, scene.height, 1.0 / 60.0)
                .expect("ticks");
        }
        let torch_ev = state.ev();
        eprintln!(
            "torch grid: dark {dark_ev:.4} stops, torch {torch_ev:.4} stops (gate {AUTO_FLICKER_MAX_EV})"
        );
        assert!(
            (torch_ev - dark_ev).abs() < AUTO_FLICKER_MAX_EV,
            "torches must not pump exposure"
        );
        // Blink war: alternating frames never step past the gate.
        let mut blink = AutoExposure::default();
        let mut worst = 0.0f32;
        let mut prev = blink.ev();
        for k in 0..120 {
            let frame = if k % 2 == 0 {
                &scene.dark
            } else {
                &scene.torch
            };
            blink
                .tick(frame, scene.width, scene.height, 1.0 / 60.0)
                .expect("ticks");
            worst = worst.max((blink.ev() - prev).abs());
            prev = blink.ev();
        }
        eprintln!("torch blink: worst step {worst:.4} stops (gate {AUTO_FLICKER_MAX_EV})");
        assert!(
            worst <= AUTO_FLICKER_MAX_EV,
            "blink step {worst} exceeds the flicker gate"
        );
    }

    #[test]
    fn auto_exposure_costs_no_heap() {
        // The per-frame adapt path (histogram plus tick) never touches the
        // heap: stack bins plus scalar walks, counted by the test allocator.
        let scene = ExposureScene::day_sweep_torch().expect("scene builds");
        let (hist, hist_heap) = engine_core::alloc_probe::measure(|| {
            histogram_log_luma(&scene.sweep[2], scene.width, scene.height)
        });
        hist.expect("histograms");
        assert_eq!(hist_heap.calls, 0, "histogram touches no heap");
        let mut state = AutoExposure::default();
        let (_, tick_heap) = engine_core::alloc_probe::measure(|| {
            state.tick(&scene.sweep[2], scene.width, scene.height, 1.0 / 60.0)
        });
        assert_eq!(tick_heap.calls, 0, "tick touches no heap");
    }

    #[test]
    fn auto_frame_serves_match_clip() {
        // Match-loop integration: 12 frames cycling the sweep at 60 Hz
        // through the auto frame path. Stops rerun bit-identically, steps
        // respect the flicker gate, every receipt slot is filled, and the
        // frame path equals the chain path ticked from the same buffer.
        let scene = ExposureScene::day_sweep_torch().expect("scene builds");
        let motion = MotionField::zero(scene.width, scene.height).expect("zero builds");
        let taa = TaaParams::default();
        let chain = PostChain::default();
        let run_clip = || {
            let mut auto_state = AutoExposure::default();
            let mut evs = Vec::with_capacity(12);
            let mut ldrs = Vec::with_capacity(12);
            for k in 0..12 {
                let frame = &scene.sweep[k % scene.sweep.len()];
                let out = run_frame_auto(
                    frame,
                    frame,
                    &motion,
                    scene.width,
                    scene.height,
                    scene.width,
                    scene.height,
                    &taa,
                    &chain,
                    &mut auto_state,
                    1.0 / 60.0,
                    &lights(),
                )
                .expect("frame runs");
                evs.push(auto_state.ev());
                ldrs.push(out.ldr);
            }
            (evs, ldrs)
        };
        let (evs_a, ldrs_a) = run_clip();
        let (evs_b, ldrs_b) = run_clip();
        assert_eq!(evs_a, evs_b, "clip stops bit-identical twice");
        assert_eq!(ldrs_a, ldrs_b, "clip pixels bit-identical twice");
        let mut worst = 0.0f32;
        for w in evs_a.windows(2) {
            worst = worst.max((w[1] - w[0]).abs());
        }
        eprintln!("match clip: worst step {worst:.4} stops (gate {AUTO_FLICKER_MAX_EV})");
        assert!(
            worst <= AUTO_FLICKER_MAX_EV,
            "clip step {worst} exceeds the flicker gate"
        );
        assert!(
            evs_a.iter().all(|e| (-8.0..=8.0).contains(e)),
            "clip stops stay in range"
        );
        // Frame path equals the chain path ticked from the same buffer.
        let mut s1 = AutoExposure::default();
        let mut s2 = AutoExposure::default();
        let frame = &scene.sweep[4];
        let f = run_frame_auto(
            frame,
            frame,
            &motion,
            16,
            16,
            16,
            16,
            &taa,
            &chain,
            &mut s1,
            1.0 / 60.0,
            &lights(),
        )
        .expect("frame runs");
        let c = run_chain_auto(frame, 16, 16, &chain, &mut s2, 1.0 / 60.0, &lights())
            .expect("chain runs");
        assert_eq!(
            s1.ev(),
            s2.ev(),
            "frame and chain tick from the same buffer"
        );
        assert_eq!(f.ldr, c.ldr, "frame and chain agree pixel for pixel");
        let r = &f.receipt;
        assert_eq!(
            (r.taa.pixels, r.upsample.pixels, r.exposure.pixels),
            (256, 256, 256)
        );
        assert!((r.lights_ms() - 3.75).abs() < 1e-9);
        assert!(r.exposure.ms >= 0.0);
    }

    #[test]
    fn auto_exposure_fails_loudly() {
        let scene = ExposureScene::day_sweep_torch().expect("scene builds");
        let mut state = AutoExposure::default();
        // Bad frame data never adapts.
        let mut bad = scene.sweep[2].clone();
        bad[7] = [f32::NAN, 0.0, 0.0];
        assert!(matches!(
            state.tick(&bad, 16, 16, 1.0 / 60.0),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            state.tick(&scene.sweep[2][..255], 16, 16, 1.0 / 60.0),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            state.tick(&scene.sweep[2], 0, 16, 1.0 / 60.0),
            Err(PostError::Invalid(_))
        ));
        // Bad dts never advance the state.
        let held = state.ev();
        for dt in [f32::NAN, -1.0, f32::INFINITY] {
            assert!(matches!(
                state.tick(&scene.sweep[2], 16, 16, dt),
                Err(PostError::Invalid(_))
            ));
        }
        assert_eq!(state.ev(), held, "failed ticks hold the state");
        // Out-of-range stops never enter the chain.
        assert!(matches!(
            PostChain::default().with_exposure_ev(9.0),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            PostChain::default().with_exposure_ev(f32::NAN),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            run_chain_auto(
                &scene.sweep[2],
                16,
                16,
                &PostChain::default(),
                &mut state,
                -0.5,
                &lights()
            ),
            Err(PostError::Invalid(_))
        ));
        // Empty clips read zero, never NaN.
        assert_eq!(clipped_share(&[]), 0.0);
        // Empty histograms and empty trims fail, never divide by zero.
        assert!(matches!(
            trimmed_log_mean(
                &ExposureHistogram {
                    bins: [0.0f32; AUTO_HIST_BINS],
                    texels: 0.0
                },
                0.1,
                0.95
            ),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            target_ev_for_mean(f32::NAN, 0.18),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            target_ev_for_mean(-2.0, 0.0),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            AutoExposure::new(AutoExposureConfig {
                trim_lo: 0.9,
                trim_hi: 0.1,
                ..AutoExposureConfig::default()
            }),
            Err(PostError::Invalid(_))
        ));
    }

    #[test]
    #[ignore = "timing: auto-exposure overhead needs release --perf on boosted clocks (fails under parallel load)"]
    fn auto_exposure_wall_overhead() {
        // Kill-gate evidence: histogram plus adapt must cost under 5% of
        // the fixed chain on a 64x64 ramp (medians of 21 reps per leg).
        let side = 64usize;
        let total = side * side;
        let hdr: Vec<[f32; 3]> = (0..total)
            .map(|i| {
                let luma = 0.05 + i as f32 / (total - 1) as f32 * 3.0;
                [luma, luma, luma]
            })
            .collect();
        let chain = PostChain::default();
        let mut fixed_ms = Vec::with_capacity(21);
        for _ in 0..21 {
            let started = Instant::now();
            std::hint::black_box(run_chain(&hdr, 64, 64, &chain, &lights()).expect("chain runs"));
            fixed_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        let mut state = AutoExposure::default();
        let mut auto_ms = Vec::with_capacity(21);
        for _ in 0..21 {
            let started = Instant::now();
            std::hint::black_box(
                run_chain_auto(&hdr, 64, 64, &chain, &mut state, 1.0 / 60.0, &lights())
                    .expect("auto chain runs"),
            );
            auto_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        fixed_ms.sort_by(|a, b| a.total_cmp(b));
        auto_ms.sort_by(|a, b| a.total_cmp(b));
        let (fixed_med, auto_med) = (fixed_ms[10], auto_ms[10]);
        let ratio = auto_med / fixed_med.max(1e-9);
        eprintln!(
            "auto-exposure overhead 64x64: fixed {fixed_med:.3}ms auto {auto_med:.3}ms ratio {ratio:.3} (kill above 1.05)"
        );
        assert!(
            ratio <= 1.05,
            "auto chain {auto_med:.3}ms costs over 5% above fixed {fixed_med:.3}ms"
        );
    }

    #[cfg(feature = "window")]
    #[test]
    fn gpu_auto_chain_matches_cpu_chain() {
        // Adapted exposure rides the existing exposure uniform: converge
        // the CPU state on the bright sweep frame, run the adapted chain
        // on the device and against the CPU oracle, parity within the
        // posted tolerance.
        let (device, queue) = match test_device() {
            Some(d) => d,
            None => return,
        };
        let sub = gpu::Rp1Submitter::new(&device);
        let scene = ExposureScene::day_sweep_torch().expect("scene builds");
        let mut auto_state = AutoExposure::default();
        for _ in 0..3 {
            auto_state
                .tick(&scene.sweep[4], scene.width, scene.height, 1.0)
                .expect("ticks");
        }
        let chain = PostChain::default()
            .with_exposure_ev(auto_state.ev())
            .expect("adapts");
        let motion = MotionField::zero(scene.width, scene.height).expect("zero builds");
        let taa = TaaParams::default();
        let g = sub
            .run_frame(
                &device,
                &queue,
                &scene.sweep[4],
                &scene.sweep[3],
                &motion,
                scene.width,
                scene.height,
                32,
                32,
                &taa,
                &chain,
                &lights(),
                [0.0, 0.0],
            )
            .expect("gpu frame runs");
        let cpu = run_frame(
            &scene.sweep[4],
            &scene.sweep[3],
            &motion,
            scene.width,
            scene.height,
            32,
            32,
            &taa,
            &chain,
            &lights(),
        )
        .expect("cpu frame runs");
        eprintln!(
            "auto gpu frame: parity {:.6} (tol {}), adapted stops {:.3}",
            g.parity_max_diff,
            gpu::GPU_PARITY_TOL_POST,
            auto_state.ev()
        );
        assert_eq!(g.validation_errors, 0, "gpu frame is validation clean");
        assert!(
            g.parity_max_diff <= gpu::GPU_PARITY_TOL_POST,
            "gpu/cpu frame parity {} exceeds {}",
            g.parity_max_diff,
            gpu::GPU_PARITY_TOL_POST
        );
        assert_eq!(gpu::max_abs_diff(&cpu.ldr, &g.ldr), g.parity_max_diff);
        assert!(
            auto_state.ev() < 0.0,
            "bright sweep adapts down, got {}",
            auto_state.ev()
        );
    }
}
