//! shadow_post as its callers see it: desat/luma for the look path,
//! texel snap for `sun_shadow`, FNV receipts for `backend`, shot clamps
//! for headless captures, and CSM validation for the render lane.
//!
//! Why this file exists: `crates/render/src/shadow_post.rs` is hot
//! (7 changes in 30 days) with no `#[test]` in it and no
//! `tests/it/shadow_post.rs` module. `shadow_aces.rs` covers splits,
//! ACES fit and presets; this module covers the caller-visible edges it
//! does not: desat/luma, snap passthrough, FNV basis, 0-size shots,
//! validation boundaries, single-cascade behavior and tonemap saturation.

use engine_render::shadow_post as sp;
use sp::{AcesConfig, CsmConfig, Look};

const EPS: f32 = 1e-4;

fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() <= EPS
}

#[test]
fn desat_identity_full_gray_and_clamped_amt() {
    let c = [0.9, 0.3, 0.1];
    assert_eq!(sp::desat3(c, 0.0), c);
    let gray = sp::desat3(c, 1.0);
    let l = sp::luma_srgb(c);
    assert!(approx(gray[0], l) && approx(gray[1], l) && approx(gray[2], l));
    // Amount clamps to 0..=1, so out-of-range matches the edges.
    assert_eq!(sp::desat3(c, -1.0), sp::desat3(c, 0.0));
    assert_eq!(sp::desat3(c, 2.0), sp::desat3(c, 1.0));
}

#[test]
fn luma_weights_match_srgb_primaries() {
    assert!(approx(sp::luma_srgb([0.0, 0.0, 0.0]), 0.0));
    assert!(approx(sp::luma_srgb([1.0, 1.0, 1.0]), 1.0));
    assert!(approx(sp::luma_srgb([1.0, 0.0, 0.0]), 0.2126));
    assert!(approx(sp::luma_srgb([0.0, 1.0, 0.0]), 0.7152));
    assert!(approx(sp::luma_srgb([0.0, 0.0, 1.0]), 0.0722));
}

#[test]
fn snap_passes_through_bad_inputs_and_snaps_otherwise() {
    // `sun_shadow` feeds finite dots with a positive texel; bad inputs pass through.
    assert_eq!(sp::snap_texel(2.5, 0.0), 2.5);
    assert_eq!(sp::snap_texel(2.5, -0.5), 2.5);
    assert_eq!(sp::snap_texel(1.0, f32::INFINITY), 1.0);
    assert!(sp::snap_texel(f32::NAN, 0.5).is_nan());
    assert!(sp::snap_texel(f32::INFINITY, 0.5).is_infinite());
    // Normal path floors to the texel grid.
    assert_eq!(sp::snap_texel(0.9, 0.5), 0.5);
    assert_eq!(sp::snap_texel(1.0, 0.5), 1.0);
}

#[test]
fn fnv_basis_deterministic_and_avalanches() {
    // Owner `backend` prints this hash in receipts; empty is the offset basis.
    assert_eq!(sp::fnv1a64(&[]), engine_core::hash_bytes(&[]));
    assert_eq!(sp::fnv1a64(b"a"), sp::fnv1a64(b"a"));
    assert_ne!(sp::fnv1a64(b"a"), sp::fnv1a64(b"b"));
    assert_ne!(sp::fnv1a64(b"a"), sp::fnv1a64(b"aa"));
}

#[test]
fn shot_clamps_zero_size_and_stays_deterministic() {
    let look = Look::sunset();
    let zero = sp::shot_rgb8(&look, 0, 0);
    let one = sp::shot_rgb8(&look, 1, 1);
    assert_eq!(zero.len(), 3);
    assert_eq!(zero, one);
    let small = sp::shot_rgb8(&look, 4, 3);
    assert_eq!(small.len(), 4 * 3 * 3);
    assert_eq!(small, sp::shot_rgb8(&look, 4, 3));
    assert_eq!(sp::shot_hash(&look, 4, 3), sp::shot_hash(&look, 4, 3));
    // Different looks still diverge at a different size than `shadow_aces` uses.
    assert_ne!(
        sp::shot_hash(&look, 5, 7),
        sp::shot_hash(&Look::noir(), 5, 7)
    );
}

#[test]
fn csm_validate_rejects_boundary_breaks() {
    let mut c = CsmConfig::default();
    assert!(c.validate());
    c.bias = 0.05;
    assert!(!c.validate(), "bias 0.05 is over the <0.05 cap");
    c = CsmConfig::default();
    c.normal_bias_m = 1.0;
    assert!(!c.validate(), "normal bias 1.0 is over the <1.0 cap");
    c = CsmConfig::default();
    c.far_m = c.near_m;
    assert!(!c.validate(), "far must exceed near");
    c = CsmConfig::default();
    c.near_m = 0.0;
    assert!(!c.validate(), "near must be positive");
    c = CsmConfig::default();
    c.cascades = 5;
    assert!(!c.validate(), "at most 4 cascades");
    c = CsmConfig::default();
    c.map_size = 511;
    assert!(!c.validate(), "map size below 512 refuses");
    c.map_size = 512;
    assert!(c.validate(), "map size 512 is the floor");
    c = CsmConfig::default();
    c.blend_frac = 0.51;
    assert!(!c.validate(), "blend over 0.5 refuses");
    c.blend_frac = 0.5;
    assert!(c.validate(), "blend 0.5 is the ceiling");
}

#[test]
fn single_cascade_collapses_and_zero_clamps_to_one() {
    let one = sp::cascade_splits(1.0, 9.0, 1, 0.5);
    assert_eq!(one, [9.0, 9.0, 9.0, 9.0]);
    assert_eq!(sp::cascade_splits(1.0, 9.0, 0, 0.5), one, "n clamps 0 to 1");
    let four = sp::cascade_splits(1.0, 9.0, 4, 0.5);
    assert!(four[0] > 1.0 && four[0] < four[1] && four[1] < four[2] && four[3] == 9.0);
    assert_eq!(sp::cascade_select(5.0, one, 1), 0);
    assert_eq!(sp::cascade_fade(5.0, one, 1, 0.1), (0, 0.0));
}

#[test]
fn tonemap_black_white_and_srgb_edges() {
    let cfg = AcesConfig::default();
    assert_eq!(sp::tonemap_aces([0.0, 0.0, 0.0], &cfg), [0.0, 0.0, 0.0]);
    // White point maps to itself through the shared fit.
    let white = sp::tonemap_aces([cfg.white, cfg.white, cfg.white], &cfg);
    assert!(approx(white[0], 1.0) && approx(white[1], 1.0) && approx(white[2], 1.0));
    // Large inputs saturate inside 0..=1 rather than blowing past it.
    let hot = sp::tonemap_aces([10.0, 10.0, 10.0], &cfg);
    assert!(hot.iter().all(|v| (0.0..=1.0).contains(v)));
    assert_eq!(sp::srgb_u8(-0.5), 0);
    assert_eq!(sp::srgb_u8(1.5), 255);
}
