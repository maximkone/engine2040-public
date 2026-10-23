//! v1_kind: the CPU twin of the LF8 kind branches in `forward.wgsl`
//! `direct_light`, kept beside `v1::shade_pbr` so the software receipt and the
//! behavior tests run the same lobe math the shader runs. Stone is the old
//! expression order; cloth, skin and metal swap the specular and, for skin,
//! add transmitted scatter. Shapes are Filament's; see `material_kind`.

use std::f32::consts::PI;

use crate::material_kind::MaterialKind;
use crate::v1::{
    add3, d_ggx, diffuse_color, dot3, f_ab, fd_burley, fresnel, mul3, mul3v, norm3,
    perceptual_to_roughness, reflectance_f0, saturate, specular_light, v_smith_ggx_correlated,
    ShadePoint,
};

const SKIN_SUBSURFACE_POWER: f32 = 12.0;
const SKIN_SUBSURFACE_TINT: [f32; 3] = [1.0, 0.55, 0.45];
const LOOK_ANISO_MIN_ROUGHNESS: f32 = 0.089 * 0.089;

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

/// LF8 metal: anisotropic GGX NDF (Burley 2012 via Filament
/// `D_GGX_Anisotropic`).
fn d_ggx_anisotropic(at: f32, ab: f32, toh: f32, boh: f32, noh: f32) -> f32 {
    let a2 = at * ab;
    let d = [ab * toh, at * boh, a2 * noh];
    let d2 = dot3(d, d);
    let b2 = a2 / d2.max(1e-12);
    a2 * b2 * b2 * (1.0 / PI)
}

/// LF8 metal: anisotropic Smith-GGX correlated visibility (Heitz 2014 via
/// Filament `V_SmithGGXCorrelated_Anisotropic`).
fn v_smith_ggx_correlated_anisotropic(
    at: f32,
    ab: f32,
    tv: [f32; 2],
    tl: [f32; 2],
    nov: f32,
    nol: f32,
) -> f32 {
    let lv = [at * tv[0], ab * tv[1], nov];
    let ll = [at * tl[0], ab * tl[1], nol];
    let lambda_v = nol * dot3(lv, lv).sqrt();
    let lambda_l = nov * dot3(ll, ll).sqrt();
    0.5 / (lambda_v + lambda_l).max(0.0000077)
}

/// LF8 cloth: Charlie sheen NDF (Estevez and Kulla 2017 via Filament
/// `D_Charlie`).
fn d_charlie(roughness: f32, noh: f32) -> f32 {
    let inv_alpha = 1.0 / roughness.max(0.0001);
    let sin2h = (1.0 - noh * noh).max(0.0078125);
    (2.0 + inv_alpha) * sin2h.powf(inv_alpha * 0.5) / (2.0 * PI)
}

/// LF8 cloth: Neubelt visibility (Neubelt and Pettineo 2013 via Filament
/// `V_Neubelt`).
fn v_neubelt(nov: f32, nol: f32) -> f32 {
    1.0 / (4.0 * (nol + nov - nol * nov)).max(0.00001532)
}

/// The sun's reflection off one surface point, before exposure and tone map:
/// the CPU twin of `direct_light` with `sun_vis = 1` folded into `radiance`.
pub(crate) fn kind_lit(p: &ShadePoint) -> [f32; 3] {
    let roughness = perceptual_to_roughness(p.perceptual_roughness);
    let h = norm3(add3(p.light_dir, p.view));
    let ndot_l = saturate(dot3(p.normal, p.light_dir));
    let ndot_v = dot3(p.normal, p.view).max(0.0001);
    let ndot_h = saturate(dot3(p.normal, h));
    let ldot_h = saturate(dot3(p.light_dir, h));
    let diffuse = mul3(
        diffuse_color(p.base_color, p.metallic),
        fd_burley(roughness, ndot_v, ndot_l, ldot_h),
    );
    let f0 = reflectance_f0(p.base_color, p.metallic);
    let f = fresnel(f0, ldot_h);
    let fab = f_ab(p.perceptual_roughness, ndot_v);
    match p.kind {
        MaterialKind::Cloth => {
            // Charlie sheen specular at the dielectric 0.04; Burley diffuse.
            let d = d_charlie(roughness, ndot_h);
            let vv = v_neubelt(ndot_v, ndot_l);
            let spec = [0.04 * d * vv; 3];
            mul3v(mul3(add3(diffuse, spec), ndot_l), p.radiance)
        }
        MaterialKind::Skin => {
            // Wrap plus thickness transmittance on the diffuse lobe; the
            // specular stays today's isotropic GGX.
            let thickness = saturate(p.kind_param);
            let scatter_voh = saturate(dot3(p.view, mul3(p.light_dir, -1.0)));
            let forward_scatter = ((scatter_voh - 1.0) * SKIN_SUBSURFACE_POWER).exp2();
            let back_scatter = (ndot_l * thickness + (1.0 - thickness)) * 0.5;
            let subsurface =
                (back_scatter + (1.0 - back_scatter) * forward_scatter) * (1.0 - thickness);
            let scatter = mul3(
                mul3v(p.base_color, SKIN_SUBSURFACE_TINT),
                subsurface * (1.0 / PI),
            );
            let d = d_ggx(roughness, ndot_h);
            let vv = v_smith_ggx_correlated(roughness, ndot_v, ndot_l);
            let spec = specular_light(d, vv, f, f0, fab, 1.0);
            mul3v(add3(scatter, mul3(add3(diffuse, spec), ndot_l)), p.radiance)
        }
        MaterialKind::Metal => {
            // Anisotropic GGX over the mesh tangent frame: the tangent is
            // orthogonalized against the normal, bitangent = w * (n x t).
            let aniso = p.kind_param.clamp(0.0, 0.95);
            let at = (roughness * (1.0 + aniso)).max(LOOK_ANISO_MIN_ROUGHNESS);
            let ab = (roughness * (1.0 - aniso)).max(LOOK_ANISO_MIN_ROUGHNESS);
            let n = p.normal;
            let tangent = [p.tangent[0], p.tangent[1], p.tangent[2]];
            let tx = add3(tangent, mul3(n, -dot3(tangent, n)));
            let tl2 = dot3(tx, tx);
            let t = if tl2 >= 1e-12 {
                mul3(tx, 1.0 / tl2.sqrt())
            } else {
                let axis = if n[0].abs() > n[1].abs() {
                    [0.0, 1.0, 0.0]
                } else {
                    [1.0, 0.0, 0.0]
                };
                norm3(cross3(axis, n))
            };
            let w = if p.tangent[3] >= 0.0 { 1.0 } else { -1.0 };
            let b = mul3(cross3(n, t), w);
            let d = d_ggx_anisotropic(at, ab, dot3(t, h), dot3(b, h), ndot_h);
            let vv = v_smith_ggx_correlated_anisotropic(
                at,
                ab,
                [dot3(t, p.view), dot3(b, p.view)],
                [dot3(t, p.light_dir), dot3(b, p.light_dir)],
                ndot_v,
                ndot_l,
            );
            let spec = specular_light(d, vv, f, f0, fab, 1.0);
            mul3v(mul3(add3(diffuse, spec), ndot_l), p.radiance)
        }
        MaterialKind::Stone => {
            // The one BRDF every earlier frame drew, in its old order.
            let d = d_ggx(roughness, ndot_h);
            let vv = v_smith_ggx_correlated(roughness, ndot_v, ndot_l);
            let spec = specular_light(d, vv, f, f0, fab, 1.0);
            mul3(mul3v(add3(diffuse, spec), p.radiance), ndot_l)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_brdf_terms_stay_finite() {
        for perceptual in [0.0, 0.25, 1.0] {
            let roughness = perceptual_to_roughness(perceptual);
            for noh in [0.0, 0.5, 1.0] {
                let d = d_charlie(roughness, noh);
                // Charlie's pow underflows to zero at the grazing end for a
                // very low alpha; it must never read NaN or a negative.
                assert!(
                    d.is_finite() && d >= 0.0,
                    "d_charlie({roughness}, {noh}) = {d}"
                );
                if roughness >= 0.1 && noh <= 0.5 {
                    assert!(d > 0.0, "d_charlie({roughness}, {noh}) = {d}");
                }
            }
            let vv = v_neubelt(0.7, 0.6);
            assert!(vv.is_finite() && vv > 0.0);
            let d = d_ggx_anisotropic(0.2, 0.1, 0.3, 0.2, 0.8);
            assert!(d.is_finite() && d > 0.0);
            let v = v_smith_ggx_correlated_anisotropic(0.2, 0.1, [0.3, 0.2], [0.1, -0.4], 0.7, 0.6);
            assert!(v.is_finite() && v > 0.0);
        }
        // The guards keep the degenerate inputs off a zero denominator.
        assert!(d_ggx_anisotropic(0.0, 0.0, 0.0, 0.0, 0.0).is_finite());
        assert!(
            v_smith_ggx_correlated_anisotropic(0.0, 0.0, [0.0; 2], [0.0; 2], 0.0, 0.0).is_finite()
        );
    }
}
