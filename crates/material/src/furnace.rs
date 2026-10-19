//! furnace: the white-furnace test over the lobe math in [`crate::twin`].
//!
//! Test-only (`#[cfg(test)] mod furnace;`). The twin evaluates the same
//! formulas as the WGSL in `compile` for one white channel (white base color,
//! one light) and integrates the outgoing radiance over the upper hemisphere
//! for a viewer at a given angle: under a uniform white environment a surface
//! that keeps energy returns at most 1. The bar is 1.02 on a 5 by 5 roughness
//! by view-angle grid (roughness 0.1 to 1, 0 to 85 degrees) for the base BRDF
//! and for each lobe added over it (coat, sheen, thin transmission); the
//! anisotropic lobe gets 1.03, because its albedo is read from the isotropic
//! table at the same alpha product. The smooth part of the radiance (diffuse,
//! sheen) is integrated over light directions; each specular peak is
//! integrated over half vectors with GGX importance, so a peak narrower than a
//! light-space cell is still resolved. Wrapped subsurface is a pseudo-energy
//! trick (it lights from below the horizon on purpose) and is not judged here.
//! Math in f64; this module never runs in a tick.

use std::f64::consts::PI;

use crate::twin::furnace_math::{albedo, grid_max, integrate_light_space, lit_terms, Config};

#[test]
fn white_furnace_holds_for_the_base_and_every_lobe() {
    let skin = |curvature, channel| Config {
        skin: Some((curvature, channel)),
        ..Config::BASE
    };
    let cases: [(&str, Config, f64); 12] = [
        ("dielectric", Config::BASE, 1.02),
        (
            "metal",
            Config {
                metallic: 1.0,
                ..Config::BASE
            },
            1.02,
        ),
        (
            "coat",
            Config {
                coat: Some((1.0, 0.3, 1.5)),
                ..Config::BASE
            },
            1.02,
        ),
        (
            "wet coat",
            Config {
                coat: Some((1.0, 0.35, 1.33)),
                ..Config::BASE
            },
            1.02,
        ),
        (
            "sheen",
            Config {
                sheen: Some((1.0, 0.5)),
                ..Config::BASE
            },
            1.02,
        ),
        (
            "anisotropy",
            Config {
                aniso: Some((0.6, 0.0)),
                metallic: 1.0,
                ..Config::BASE
            },
            1.03,
        ),
        (
            "transmission",
            Config {
                transmission: 1.0,
                ..Config::BASE
            },
            1.02,
        ),
        ("skin red, curvature 0.5", skin(0.5, 0), 1.02),
        ("skin green, curvature 0.5", skin(0.5, 1), 1.02),
        ("skin blue, curvature 0.5", skin(0.5, 2), 1.02),
        ("skin red, flat", skin(0.0, 0), 1.02),
        ("skin red, curvature 1", skin(1.0, 0), 1.02),
    ];
    for (name, cfg, bar) in cases {
        let worst = grid_max(cfg);
        assert!(
            worst <= bar,
            "{name}: outgoing albedo {worst:.4} over {bar}"
        );
    }
}

#[test]
fn a_rough_metal_keeps_most_of_its_light() {
    // Multiple-scatter compensation: single-scatter GGX at roughness 1 and
    // normal view returns 0.31; the compensated lobe lands near 1.
    let metal = Config {
        metallic: 1.0,
        ..Config::BASE
    };
    let rough = albedo(metal, 1.0, 0.0);
    assert!((0.85..=1.02).contains(&rough), "{rough:.4}");
}

#[test]
fn a_white_dielectric_is_not_brighter_than_its_light_and_not_dark() {
    let dielectric = albedo(Config::BASE, 0.6, 45.0);
    assert!((0.85..=1.0).contains(&dielectric), "{dielectric:.4}");
}

/// The whole radiance integrated over light directions only: the old way,
/// right wherever the specular peak is wider than a light-space cell.
fn albedo_light_space(cfg: Config, perceptual: f64, view_deg: f64) -> f64 {
    let n = [0.0, 0.0, 1.0];
    let tv = view_deg.to_radians();
    let v = [tv.sin(), 0.0, tv.cos()];
    integrate_light_space(PI / 2.0, |l| {
        let t = lit_terms(cfg, perceptual, n, v, l);
        t.broad + t.spec + t.coat
    })
}

#[test]
fn half_vector_sampling_agrees_with_light_space_quadrature_where_both_resolve() {
    let metal = Config {
        metallic: 1.0,
        ..Config::BASE
    };
    let cases = [
        Config::BASE,
        metal,
        Config {
            coat: Some((1.0, 0.35, 1.5)),
            sheen: Some((1.0, 0.6)),
            ..Config::BASE
        },
        Config {
            aniso: Some((0.6, 0.25)),
            ..metal
        },
    ];
    for cfg in cases {
        for roughness in [0.5, 0.75] {
            for angle in [0.0, 45.0, 70.0] {
                let sampled = albedo(cfg, roughness, angle);
                let gridded = albedo_light_space(cfg, roughness, angle);
                assert!(
                    (sampled - gridded).abs() < 0.01,
                    "roughness {roughness} angle {angle}: half-vector {sampled:.4} vs light-space {gridded:.4}"
                );
            }
        }
    }
}

#[test]
fn a_metal_keeps_its_light_at_low_roughness() {
    // The peak is sharper than a light-space cell here, and the anisotropic
    // lobe once lost it to an underflow guard in the WGSL (albedo 0.0006 at
    // roughness 0.1).
    let metal = Config {
        metallic: 1.0,
        ..Config::BASE
    };
    let brushed = Config {
        aniso: Some((0.6, 0.0)),
        ..metal
    };
    for (name, cfg) in [("metal", metal), ("brushed metal", brushed)] {
        for roughness in [0.1, 0.3] {
            for angle in [0.0, 45.0, 70.0] {
                let kept = albedo(cfg, roughness, angle);
                assert!(
                    (0.9..=1.03).contains(&kept),
                    "{name} roughness {roughness} angle {angle}: albedo {kept:.4}"
                );
            }
        }
    }
}

#[test]
fn anisotropy_stays_in_the_bar_for_every_strength_and_rotation() {
    // The albedo tables are isotropic, so the lobe's furnace depends on how
    // stretched it is and on which way the view lies. Without the derate this
    // sweep reaches 1.042.
    for strength in [0.3, 0.6, 0.9] {
        for rotation in [0.0, 0.25] {
            let cfg = Config {
                aniso: Some((strength, rotation)),
                metallic: 1.0,
                ..Config::BASE
            };
            let worst = grid_max(cfg);
            assert!(
                worst <= 1.03,
                "strength {strength} rotation {rotation}: outgoing albedo {worst:.4}"
            );
        }
    }
}
