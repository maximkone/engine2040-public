//! SW51 accept proofs: a grazing normal carries the rim, a facing one carries
//! none, a material that asks for no rim is byte-equal to today, and the
//! backlight only exists where the light sits behind the surface.
//!
//! The row's two numbers do not both follow from its own formula, so both are
//! pinned here as measured: at `n_dot_v` 0.1 and roughness 0.4 the term is
//! `0.9^9.6 = 0.3637`, which at the default rim 0.35 is 0.127 of the light; the
//! row's 0.27 share is the same term at rim 0.742.

use super::*;

/// SW51 F2P: a grazing normal adds the measured share of the light, a facing one
/// adds under 1e-4, and the row's 0.27 share is the term at rim 0.742.
#[test]
fn rim_on_a_grazing_normal_adds_the_measured_share() {
    let grazing = rim_term(0.1, 0.4);
    assert!(
        (grazing - 0.363_696).abs() < 1e-4,
        "grazing rim term is {grazing}, want 0.3637"
    );
    let white = [1.0_f32; 3];
    let share = rim_light(0.1, 0.4, rim_strength(MaterialKind::Skin), white)[0];
    assert!(
        (share - 0.127_294).abs() < 1e-4,
        "a grazing skin normal adds {share} of the light at rim 0.35"
    );
    let row_share = rim_light(0.1, 0.4, 0.742_4, white)[0];
    assert!(
        (row_share - 0.27).abs() < 1e-3,
        "the row's 0.27 share is rim 0.742, measured {row_share}"
    );
    // Front-on: the power sends the term to 1e-10, far under the row's 1e-4.
    let facing = rim_light(0.9, 0.4, rim_strength(MaterialKind::Skin), white)[0];
    assert!(
        facing < 1e-4,
        "a front-on skin normal adds {facing}, want under 1e-4"
    );
    // The albedo tints the rim halfway, so a mid albedo keeps 0.75 of it.
    let tinted = rim_light(0.1, 0.4, 0.742_4, [0.5, 0.5, 0.5])[0];
    assert!(
        (tinted - row_share * 0.75).abs() < 1e-5,
        "RIM_TINT 0.5 keeps 0.75 of the light on a mid albedo, got {tinted}"
    );
}

/// SW51 F2P: rim 0 returns exact zeros, so stone, sand and metal draws are
/// byte-equal to the frame before this row.
#[test]
fn rim_zero_is_byte_equal_to_today() {
    for n_dot_v in [0.0_f32, 0.1, 0.5, 0.9, 1.0] {
        for roughness in [0.0_f32, 0.4, 1.0] {
            let light = rim_light(n_dot_v, roughness, 0.0, [0.8, 0.4, 0.2]);
            assert_eq!(
                light, [0.0_f32; 3],
                "rim 0 at n_dot_v {n_dot_v} roughness {roughness} adds nothing"
            );
        }
    }
    for kind in [MaterialKind::Stone, MaterialKind::Metal] {
        assert_eq!(rim_strength(kind), 0.0, "{kind:?} carries no rim");
        assert_eq!(backlight_thickness(kind), 0.0, "{kind:?} passes no light");
    }
    // The rimmed kinds default to the row's 0.35, and the sand species is stone.
    for kind in [MaterialKind::Skin, MaterialKind::Cloth] {
        assert_eq!(rim_strength(kind), RIM_DEFAULT, "{kind:?} carries the rim");
    }
    assert_eq!(RIM_DEFAULT, 0.35, "the row's default rim strength");
}

/// SW51 F2P: the backlight exists only where the light is behind the surface,
/// and a thicker body passes more of it.
#[test]
fn backlight_adds_only_when_the_light_is_behind() {
    for lit in [0.0_f32, 0.01, 0.5, 1.0] {
        assert_eq!(
            backlight_term(lit, 0.35),
            0.0,
            "n_dot_l {lit} is lit, so the backlight is exactly zero"
        );
        assert_eq!(
            backlight_light(lit, 0.35, 0.2, [1.0, 1.0, 1.0]),
            [0.0_f32; 3],
            "a lit surface gets no backlight colour"
        );
    }
    let behind = backlight_term(-0.5, 0.35);
    assert!(
        (behind - 0.337_5).abs() < 1e-6,
        "half behind at thickness 0.35 passes {behind}, want 0.3375"
    );
    assert!(
        backlight_term(-0.5, 0.9) > backlight_term(-0.5, 0.15),
        "a thicker body passes more light"
    );
    assert!(
        (backlight_term(-1.0, 1.0) - 1.0).abs() < 1e-6,
        "straight behind a thick body passes all of it"
    );
    // The sunset behind a fighter moves the diffuse up, never down.
    let share = backlight_light(-0.5, 0.35, 0.2, [3.0, 2.0, 1.2]);
    assert!(
        share[0] > 0.0 && share[2] > 0.0,
        "the backlight adds: {share:?}"
    );
}

/// SW51 F2P: the shipped shade carries both arms, keyed off the material kind.
#[test]
fn rim_wgsl_carries_the_rim_and_backlight_arms() {
    let source = include_str!("shaders/forward.wgsl");
    for piece in [
        "fn rim_share(",
        "fn backlight_share(",
        "RIM_EXPONENT_SCALE",
        "rim_strength(",
        "backlight_thickness(",
    ] {
        assert!(source.contains(piece), "forward.wgsl misses `{piece}`");
    }
}
