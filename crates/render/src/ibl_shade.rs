//! ibl_shade: the CPU shade of the IBL bake and the RL47 multiple-scatter
//! factor the shader twins apply.
//!
//! Owner: render lane; `ibl.rs` re-exports every item, so callers keep the
//! `ibl::shade_ibl` path. Units: linear radiance; `occlusion`, `metallic`
//! and `roughness` are unitless `0..=1`. Determinism: pure f32, same inputs
//! give the same triple on every run. Twins: `ibl_specular` in
//! `shaders/forward.wgsl` and `ibl_sample` in `FORWARD_IBL_WGSL`.

use std::f32::consts::PI;

use super::{
    ambient_occluded, bake_has_light, brdf_sample, check_roughness, check_unit_dir, sample_cube,
    IblBake, IblError,
};

/// RL47: multiple-scatter gain for the split-sum specular lobe,
/// `1 + F0 * (1 / E - 1)` with `E` the lobe's directional albedo
/// (`scale + bias` for `F0 = 1`) — Filament 9.5, the same factor the direct
/// lobe already applies in `forward.wgsl` `specular_term`. Single-scatter
/// GGX loses the paths that bounce more than once: at roughness 1 it returns
/// 0.31 of the light; this puts the rest back. `E` floors at
/// [`super::spec_lobe::MS_ENERGY_FLOOR`] so a degenerate lobe gains at most 3x
/// and stays finite. Pure f32; the one formula is [`super::spec_lobe::ms_gain`],
/// whose device test runs the shipped WGSL against it.
#[must_use]
pub fn energy_compensation(f0: [f32; 3], lut: [f32; 2]) -> [f32; 3] {
    super::spec_lobe::ms_gain(f0, lut[0] + lut[1])
}

/// Forward-pass IBL shade with ambient-only occlusion: diffuse from the
/// irradiance SH scaled by `occlusion`, specular left open (pipeline rule 4:
/// AO scales ambient, never the final color; mirrors `eg12_ambient` in
/// `shaders/forward_maps.wgsl`). Owner: render lane; consumer: `shade_ibl`
/// (open 1.0) plus the live forward frame through the same rule. Units:
/// linear radiance, `occlusion` unitless `0..=1` (`1` open, `0` shut).
/// Determinism: same inputs give the same triple on every run; no
/// allocation, no clock. Errors: [`IblError::Invalid`] for a bad direction,
/// color, metallic, roughness or occlusion outside `0..=1`.
pub fn shade_ibl_occluded(
    bake: &IblBake,
    normal: [f32; 3],
    view_dir: [f32; 3],
    base_color: [f32; 3],
    metallic: f32,
    roughness: f32,
    occlusion: f32,
) -> Result<[f32; 3], IblError> {
    if !occlusion.is_finite() || !(0.0..=1.0).contains(&occlusion) {
        return Err(IblError::Invalid(format!(
            "ibl occlusion {occlusion} outside 0..=1"
        )));
    }
    let n = check_unit_dir(normal, "shade normal")?;
    let v = check_unit_dir(view_dir, "shade view direction")?;
    if !base_color.iter().all(|c| c.is_finite()) {
        return Err(IblError::Invalid("ibl base color is not finite".into()));
    }
    if base_color.iter().any(|c| *c < 0.0 || *c > 1.0) {
        return Err(IblError::Invalid(format!(
            "ibl base color {base_color:?} outside 0..=1"
        )));
    }
    if !metallic.is_finite() || !(0.0..=1.0).contains(&metallic) {
        return Err(IblError::Invalid(format!(
            "ibl metallic {metallic} outside 0..=1"
        )));
    }
    let r = check_roughness(roughness)?;
    if !bake_has_light(bake) {
        return Ok([0.0, 0.0, 0.0]);
    }
    let e = ambient_occluded(&bake.sh, n, occlusion)?;
    let diffuse = [
        (1.0 - metallic) * base_color[0] * e[0] / PI,
        (1.0 - metallic) * base_color[1] * e[1] / PI,
        (1.0 - metallic) * base_color[2] * e[2] / PI,
    ];
    let ndotv = (n[0] * v[0] + n[1] * v[1] + n[2] * v[2]).clamp(0.0, 1.0);
    let refl = [
        2.0 * ndotv * n[0] - v[0],
        2.0 * ndotv * n[1] - v[1],
        2.0 * ndotv * n[2] - v[2],
    ];
    let pre = sample_cube(&bake.cube, refl, r)?;
    let lut = brdf_sample(&bake.brdf, ndotv, r)?;
    let f0 = [
        0.04 + (base_color[0] - 0.04) * metallic,
        0.04 + (base_color[1] - 0.04) * metallic,
        0.04 + (base_color[2] - 0.04) * metallic,
    ];
    let comp = energy_compensation(f0, lut);
    let spec = [
        pre[0] * (f0[0] * lut[0] + lut[1]) * comp[0],
        pre[1] * (f0[1] * lut[0] + lut[1]) * comp[1],
        pre[2] * (f0[2] * lut[0] + lut[1]) * comp[2],
    ];
    Ok([
        (diffuse[0] + spec[0]).max(0.0),
        (diffuse[1] + spec[1]).max(0.0),
        (diffuse[2] + spec[2]).max(0.0),
    ])
}

/// Forward-pass IBL shade in linear space: diffuse from the irradiance SH
/// plus split-sum specular from the prefiltered cubemap and the BRDF LUT.
/// `base_color` is linear `0..=1`, `metallic`/`roughness` in `0..=1`.
pub fn shade_ibl(
    bake: &IblBake,
    normal: [f32; 3],
    view_dir: [f32; 3],
    base_color: [f32; 3],
    metallic: f32,
    roughness: f32,
) -> Result<[f32; 3], IblError> {
    shade_ibl_occluded(bake, normal, view_dir, base_color, metallic, roughness, 1.0)
}

/// Flat ambient floor the off path adds when the bake is off: the retired
/// `AMBIENT_FLOOR` triple from `forward.wgsl` (linear RGB), kept here as
/// the single owner now that the shader shades ambient from the bake.
/// Units: linear radiance factor. Determinism: same triple on every run.
/// This item sizes nothing and allocates nothing.
pub const IBL_OFF_FLOOR: [f32; 3] = [0.20, 0.18, 0.16];

/// On-vs-off gap for one material probe: the baked shade minus the flat
/// floor term the off path adds.
///
/// Owner: render lane; consumer: the on-vs-off gate plus the live forward
/// frame that samples the same bake. Units: linear RGB radiance. Threading:
/// pure with no IO, locks or clock; safe on any thread. Determinism: the
/// same bake and inputs give the same triple on every run; no allocation,
/// no clock. Errors: [`IblError::Invalid`] from `shade_ibl` for a bad
/// direction, color, metallic or roughness; no new validation here.
pub fn ibl_on_off_diff(
    bake: &IblBake,
    normal: [f32; 3],
    view_dir: [f32; 3],
    base_color: [f32; 3],
    metallic: f32,
    roughness: f32,
) -> Result<[f32; 3], IblError> {
    let on = shade_ibl(bake, normal, view_dir, base_color, metallic, roughness)?;
    let keep = 1.0 - metallic;
    Ok([
        on[0] - keep * base_color[0] * IBL_OFF_FLOOR[0],
        on[1] - keep * base_color[1] * IBL_OFF_FLOOR[1],
        on[2] - keep * base_color[2] * IBL_OFF_FLOOR[2],
    ])
}
