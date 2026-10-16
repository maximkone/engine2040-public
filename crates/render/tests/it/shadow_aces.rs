//! shadow_aces: CSM-stable + ACES shots, deterministic.
use engine_render::shadow_post as sp;
use sp::{AcesConfig, CsmConfig, Look};
const EPS: f32 = 1e-4;
fn approx(a: f32, b: f32) -> bool {
    (a - b).abs() <= EPS
}

#[test]
fn splits_uniform_vs_log_vectors() {
    assert_eq!(sp::cascade_splits(1.0, 9.0, 2, 0.0), [5.0, 9.0, 9.0, 9.0]);
    assert_eq!(sp::cascade_splits(1.0, 9.0, 2, 1.0), [3.0, 9.0, 9.0, 9.0]);
}
#[test]
fn splits_monotonic_endpoints_deterministic() {
    let c = CsmConfig::default();
    assert!(c.validate());
    let (a, b) = (c.splits(), c.splits());
    assert_eq!(a, b);
    assert!(a[0] > c.near_m && a[2] == c.far_m);
    assert!(a[0] < a[1] && a[1] < a[2]);
}
#[test]
fn snap_stable_under_texel_jitter() {
    assert_eq!(sp::snap_texel(1.1, 0.5), sp::snap_texel(1.4, 0.5));
    assert!(approx(
        sp::snap_texel(1.6, 0.5) - sp::snap_texel(1.4, 0.5),
        0.5
    ));
    assert_eq!(sp::snap_texel(3.0, 0.0), 3.0);
}
#[test]
fn select_and_fade_follow_splits() {
    let s = sp::cascade_splits(1.0, 9.0, 2, 0.0);
    assert_eq!(
        (
            sp::cascade_select(4.0, s, 2),
            sp::cascade_select(5.0, s, 2),
            sp::cascade_select(5.1, s, 2),
            sp::cascade_select(99.0, s, 2)
        ),
        (0, 0, 1, 1)
    );
    let (i, f) = sp::cascade_fade(4.99, s, 2, 0.1);
    assert_eq!(i, 0);
    assert!(f > 0.0 && f <= 1.0);
    assert_eq!(sp::cascade_fade(1.0, s, 2, 0.1), (0, 0.0));
    assert_eq!(sp::cascade_fade(9.0, s, 2, 0.0), (1, 0.0));
}
#[test]
fn csm_validate_rejects_bad() {
    let mut c = CsmConfig::default();
    assert!(c.validate());
    c.cascades = 0;
    assert!(!c.validate());
    c.cascades = 3;
    c.map_size = 1000;
    assert!(!c.validate());
    c.map_size = 2048;
    c.lambda = 2.0;
    assert!(!c.validate());
}
#[test]
fn aces_known_fit() {
    assert_eq!(sp::aces_fitted(0.0), 0.0);
    assert_eq!(sp::aces_fitted(-5.0), 0.0);
    assert!(approx(sp::aces_fitted(1.0), 0.8038));
    assert!(sp::aces_fitted(0.5) < sp::aces_fitted(1.0));
    assert_eq!(sp::aces_fitted(100.0), 1.0);
}
#[test]
fn exposure_and_srgb_ramps() {
    assert_eq!(
        (sp::ev_mult(0.0), sp::ev_mult(1.0), sp::ev_mult(-1.0)),
        (1.0, 2.0, 0.5)
    );
    assert_eq!(sp::linear_to_srgb(0.0), 0.0);
    // 1.055f32-0.055f32 is 0.99999994, not 1.0 — approximate by construction.
    assert!(approx(sp::linear_to_srgb(1.0), 1.0));
    assert_eq!((sp::srgb_u8(0.0), sp::srgb_u8(1.0)), (0, 255));
    let lo = sp::tonemap_aces(
        [0.5, 0.5, 0.5],
        &AcesConfig {
            exposure_ev: 0.0,
            white: 11.2,
        },
    );
    let hi = sp::tonemap_aces(
        [0.5, 0.5, 0.5],
        &AcesConfig {
            exposure_ev: 1.0,
            white: 11.2,
        },
    );
    assert!(hi[0] > lo[0] && hi[1] > lo[1] && hi[2] > lo[2]);
}
#[test]
fn presets_sunset_warm_noir_gray() {
    let (s, n) = (Look::sunset(), Look::noir());
    assert_eq!(
        (
            Look::preset("Sunset").unwrap(),
            Look::preset("NOIR").unwrap()
        ),
        (s, n)
    );
    assert!(Look::preset("dawn").is_none());
    assert_ne!(sp::config_hash(&s), sp::config_hash(&n));
    assert_eq!(sp::config_hash(&s), sp::config_hash(&Look::sunset()));
    let (ps, pn) = (sp::shot_rgb8(&s, 16, 16), sp::shot_rgb8(&n, 16, 16));
    assert_eq!((ps.len(), pn.len()), (16 * 16 * 3, 16 * 16 * 3));
    assert_ne!(ps, pn);
    let mean = |p: &Vec<u8>, ch: usize| -> f32 {
        p.iter().skip(ch).step_by(3).map(|v| *v as f32).sum::<f32>() / (p.len() / 3) as f32
    };
    assert!(mean(&ps, 0) > mean(&ps, 2) + 10.0, "sunset must be warm");
    let gray = pn
        .chunks_exact(3)
        .map(|c| {
            (c[0] as i16 - c[1] as i16)
                .abs()
                .max((c[1] as i16 - c[2] as i16).abs())
        })
        .max()
        .unwrap();
    assert!(gray <= 6, "noir must be desaturated, max spread={}", gray);
}
#[test]
fn shots_csm_stable_deterministic() {
    let s = Look::sunset();
    assert_eq!(sp::shot_rgb8(&s, 12, 8), sp::shot_rgb8(&s, 12, 8));
    assert_eq!(sp::shot_hash(&s, 12, 8), sp::shot_hash(&s, 12, 8));
    assert_ne!(
        sp::shot_hash(&s, 12, 8),
        sp::shot_hash(&Look::noir(), 12, 8)
    );
}
