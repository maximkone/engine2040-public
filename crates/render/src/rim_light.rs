//! SW51: the rim term and the backlight that keep a fighter readable against the
//! bright sand at any sun angle.
//!
//! Consumer: `shaders/forward.wgsl` `direct_light` (the skin and cloth arms), the
//! same two terms as Godot's `scene_forward_lights_inc.glsl`
//! `LIGHT_RIM_USED`/`LIGHT_BACKLIGHT_USED`. This module is the CPU twin the
//! accept measures: the rim's grazing share, its near-zero front-on share, the
//! backlight's one-sidedness, and the exact zero when a material asks for none.
//!
//! The per-material strengths ride the existing material word ([`MaterialKind`]),
//! so no uniform word, binding or cook byte changed. Units: `n . v` and `n . l`
//! are cosines, `roughness` and the strengths are `0..=1`, the light share is a
//! fraction of the light's colour. Determinism: pure `f32` math, no clock, no
//! RNG. Presentation only: nothing here feeds the tick or the hash.

use crate::material_kind::MaterialKind;

/// Exponent scale of the rim's grazing falloff, Godot's `16`.
pub const RIM_EXPONENT_SCALE: f32 = 16.0;
/// Floor under the rim's base, so a fully facing normal cannot divide by zero in
/// the power: Godot's `1e-4`.
pub const RIM_BASE_FLOOR: f32 = 1e-4;
/// Rim strength a rimmed material (skin, cloth) defaults to, `0..=1`.
pub const RIM_DEFAULT: f32 = 0.35;
/// How much of the albedo tints the rim light: `mix(1, albedo, RIM_TINT)`.
pub const RIM_TINT: f32 = 0.5;

/// The rim term alone: `pow(max(1e-4, 1 - n_dot_v), max(0, (1 - roughness) * 16))`.
/// A grazing normal (`n_dot_v` near 0) approaches one; a facing one falls to
/// zero far below the 1e-4 band the accept measures.
pub fn rim_term(n_dot_v: f32, roughness: f32) -> f32 {
    let base = (1.0 - n_dot_v).max(RIM_BASE_FLOOR);
    let exponent = ((1.0 - roughness) * RIM_EXPONENT_SCALE).max(0.0);
    base.powf(exponent)
}

/// The rim's colour share of one light: `light * rim_term * rim * mix(1, albedo,
/// RIM_TINT)`. `rim` of zero returns an exact zero, so a stone or metal draw is
/// byte-equal to the frame before this row.
pub fn rim_light(n_dot_v: f32, roughness: f32, rim: f32, albedo: [f32; 3]) -> [f32; 3] {
    let strength = rim_term(n_dot_v, roughness) * rim;
    let mut out = [0.0_f32; 3];
    for (channel, slot) in out.iter_mut().enumerate() {
        let tint = 1.0 + (albedo[channel] - 1.0) * RIM_TINT;
        *slot = strength * tint;
    }
    out
}

/// The backlight term: light through the body only where the light sits behind
/// the surface, so `n_dot_l < 0` and zero on the lit side. `thickness` is the
/// material's own thin-ness (`MaterialKind::default_param`), which widens the
/// band a thicker body passes.
pub fn backlight_term(n_dot_l: f32, thickness: f32) -> f32 {
    let behind = (-n_dot_l).max(0.0);
    behind * (0.5 + 0.5 * thickness.clamp(0.0, 1.0))
}

/// The diffuse a backlight adds: `light * (1/PI - diffuse_brdf_nl) * backlight`,
/// the Godot arm that moves the sunset through the silhouette. Negative on the
/// lit side, where `backlight_term` is zero, so it can only add.
pub fn backlight_light(n_dot_l: f32, thickness: f32, brdf_nl: f32, light: [f32; 3]) -> [f32; 3] {
    let share = (core::f32::consts::FRAC_1_PI - brdf_nl) * backlight_term(n_dot_l, thickness);
    let mut out = [0.0_f32; 3];
    for (channel, slot) in out.iter_mut().enumerate() {
        *slot = light[channel] * share;
    }
    out
}

/// The rim strength one material kind asks for: skin and cloth carry a rim,
/// stone (sand, rock) and metal do not.
pub const fn rim_strength(kind: MaterialKind) -> f32 {
    match kind {
        MaterialKind::Skin | MaterialKind::Cloth => RIM_DEFAULT,
        MaterialKind::Stone | MaterialKind::Metal => 0.0,
    }
}

/// The backlight thickness one material kind passes: skin carries the body
/// light, cloth a little, stone and metal none.
pub const fn backlight_thickness(kind: MaterialKind) -> f32 {
    match kind {
        MaterialKind::Skin => 0.35,
        MaterialKind::Cloth => 0.15,
        MaterialKind::Stone | MaterialKind::Metal => 0.0,
    }
}

#[cfg(test)]
#[path = "rim_light_tests.rs"]
mod tests;
