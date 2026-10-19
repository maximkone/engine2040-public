//! twin: the CPU twin of the generated material shader at one surface point.
//!
//! The compiled WGSL ([`crate::compile`]) has no CPU oracle of its own: the
//! furnace test pins its energy (the white-furnace integral over the same
//! scalar lobes) and the GPU swatch test
//! (`crates/render/tests/material_swatch.rs`) compares a rendered pixel to
//! [`twin_lit_rgb`]. Both read the copy of the lobe math below, so the shader
//! and the twin cannot drift apart without one of the two failing.
//!
//! The scalar lobes (GGX, correlated Smith, Schlick, Burley, Charlie,
//! anisotropic GGX) and the light-space integrals are the f64 math the
//! furnace test integrates; `furnace.rs` keeps the tests and stays test-only
//! (`#[cfg(test)] mod furnace;`), so this module holds the one copy.
//!
//! Units: none for the lobes; linear radiance in, presented sRGB in `0..=1`
//! out of [`twin_lit_rgb`]. Determinism: pure math, no clock, no IO, no GPU,
//! no state; the same inputs give the same numbers on every host. Errors:
//! none.
//!
//! Consumers: `furnace.rs` (the white-furnace bars) and the render swatch
//! test through [`twin_lit_rgb`].

use std::f64::consts::PI;

use engine_core::checked::f32_from_f64;

use crate::channels::{twin_layers, Coverage, LayerResult};
use crate::compile::{
    lobe, pack_uniform, Structure, ANISOTROPY_FLOOR, ANISO_ENERGY_KEEP, MIN_HALF_NORMAL,
    PERCEPTUAL_ROUGHNESS_FLOOR,
};
use crate::dfg::{dfg, sheen_albedo, SHEEN_ROUGHNESS_MIN};
use crate::record::MaterialRecord;
use crate::skin::{terminator, transmittance};

/// A three-vector of f64: the twin's working type.
type V3 = [f64; 3];

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: V3) -> V3 {
    let n = dot(a, a).sqrt();
    [a[0] / n, a[1] / n, a[2] / n]
}

fn cross(a: V3, b: V3) -> V3 {
    let [ax, ay, az] = a;
    let [bx, by, bz] = b;
    [ay * bz - az * by, az * bx - ax * bz, ax * by - ay * bx]
}

fn add(a: V3, b: V3) -> V3 {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn mul(a: V3, b: V3) -> V3 {
    [a[0] * b[0], a[1] * b[1], a[2] * b[2]]
}

fn scale(a: V3, s: f64) -> V3 {
    [a[0] * s, a[1] * s, a[2] * s]
}

/// A toward `b` by `t`, built from this module's own add/scale.
fn mix3(a: V3, b: V3, t: f64) -> V3 {
    add(a, scale(sub(b, a), t))
}

fn min3(a: V3, b: V3) -> V3 {
    [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])]
}

fn max3(a: V3, b: V3) -> V3 {
    [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])]
}

/// Schlick Fresnel per channel, at the shader's clamped grazing term `f90`.
fn fresnel3(f0: V3, f90: f64, ldot_h: f64) -> V3 {
    let t = (1.0 - ldot_h).powi(5);
    add(f0, scale(sub([f90; 3], f0), t))
}

// ---------------------------------------------------------------------------
// The scalar lobes, as the generated BRDF chunk writes them.
// ---------------------------------------------------------------------------

fn d_ggx(alpha: f64, nh: f64) -> f64 {
    let one_minus = 1.0 - nh * nh;
    let a = nh * alpha;
    let k = alpha / (one_minus + a * a);
    k * k / PI
}

fn v_smith(alpha: f64, nv: f64, nl: f64) -> f64 {
    let a2 = alpha * alpha;
    let lv = nl * ((nv - a2 * nv) * nv + a2).sqrt();
    let ll = nv * ((nl - a2 * nl) * nl + a2).sqrt();
    0.5 / (lv + ll)
}

fn f_schlick(f0: f64, f90: f64, vh: f64) -> f64 {
    // Mirrors the WGSL: the power base takes the half-normal floor.
    f0 + (f90 - f0) * (1.0 - vh).max(f64::from(MIN_HALF_NORMAL)).powi(5)
}

fn fd_burley(alpha: f64, nv: f64, nl: f64, lh: f64) -> f64 {
    let f90 = 0.5 * alpha + 2.0 * alpha * lh * lh;
    let ls = 1.0 + (f90 - 1.0) * (1.0 - nl).powi(5);
    let vs = 1.0 + (f90 - 1.0) * (1.0 - nv).powi(5);
    ls * vs * (1.0 + (1.0 / 1.51 - 1.0) * alpha) / PI
}

fn d_charlie(r: f64, nh: f64) -> f64 {
    let inv_alpha = 1.0 / (r * r);
    let sin2h = (1.0 - nh * nh).max(0.0078125);
    (2.0 + inv_alpha) * sin2h.powf(inv_alpha * 0.5) / (2.0 * PI)
}

fn v_neubelt(nv: f64, nl: f64) -> f64 {
    1.0 / (4.0 * (nl + nv - nl * nv))
}

fn d_aniso(alphas: [f64; 2], th: f64, bh: f64, nh: f64) -> f64 {
    // The anisotropic GGX of the generated chunk as 1 / (PI a2 s^2): the same
    // value without the f32 underflow at low roughness.
    let [at, ab] = alphas;
    let s = th * th / (at * at) + bh * bh / (ab * ab) + nh * nh;
    1.0 / (PI * at * ab * s * s)
}

/// Anisotropic Smith visibility; `v` and `l` are `[t, b, n]` components.
fn v_aniso(alphas: [f64; 2], v: V3, l: V3) -> f64 {
    let [at, ab] = alphas;
    let lv = l[2] * (at * v[0]).hypot(ab * v[1]).hypot(v[2]);
    let ll = v[2] * (at * l[0]).hypot(ab * l[1]).hypot(l[2]);
    0.5 / (lv + ll)
}

/// The tangent frame the fragment builds from the normal and a rotation in
/// turns.
fn frame(n: V3, rotation: f64) -> (V3, V3) {
    let up = if n[1].abs() > 0.99 {
        [1.0, 0.0, 0.0]
    } else {
        [0.0, 1.0, 0.0]
    };
    let t0 = norm(cross(up, n));
    let b0 = cross(n, t0);
    let ang = rotation * 2.0 * PI;
    let t = [
        t0[0] * ang.cos() + b0[0] * ang.sin(),
        t0[1] * ang.cos() + b0[1] * ang.sin(),
        t0[2] * ang.cos() + b0[2] * ang.sin(),
    ];
    (t, cross(n, t))
}

/// The directional albedo table at view cosine `nv` and perceptual roughness
/// `p`, as the shader's `dfg_lookup` reads it.
fn table(nv: f64, p: f64) -> (f64, f64) {
    let [a, b] = dfg(f32_from_f64(nv), f32_from_f64(p));
    (f64::from(a), f64::from(b))
}

/// Anisotropic roughness pair for an isotropic `alpha` and a strength, as the
/// fragment builds it. The alpha floor is [`ANISOTROPY_FLOOR`], the same const
/// the generated WGSL names.
fn aniso_alphas(alpha: f64, strength: f64) -> [f64; 2] {
    [
        (alpha * (1.0 + strength)).min(1.0),
        (alpha * (1.0 - strength)).max(f64::from(ANISOTROPY_FLOOR)),
    ]
}

/// Base roughness as the shader clamps it: the microfacet alpha, off the one
/// [`PERCEPTUAL_ROUGHNESS_FLOOR`].
fn base_alpha(perceptual: f64) -> f64 {
    perceptual
        .clamp(f64::from(PERCEPTUAL_ROUGHNESS_FLOOR), 1.0)
        .powi(2)
}

/// The record's sheen roughness, clamped as the sheen block clamps it.
fn sheen_roughness(roughness: f64) -> f64 {
    roughness.clamp(f64::from(SHEEN_ROUGHNESS_MIN), 1.0)
}

/// The Charlie sheen lobe at one point: `D_Charlie(NoH) * V_Neubelt(NoV, NoL)`,
/// the scalar the generated fragment's sheen block multiplies by the tint and
/// the layer weight (`CHUNK_SHEEN` of [`crate::compile`]).
///
/// This is the CPU reference of the cloth lobe. `roughness` is the record's
/// sheen perceptual roughness, clamped to `SHEEN_ROUGHNESS_MIN..=1` exactly as
/// the fragment clamps `material.sheen2.x`; the three cosines are the
/// half-vector, the view and the light alignments. [`lit_point`] runs the same
/// two scalars for the tinted term.
///
/// Units: none (a BRDF value). Determinism: pure math, no state, no clock; the
/// same inputs give the same number on every host. Errors: none.
#[must_use]
pub fn sheen_lobe(roughness: f32, ndot_h: f32, ndot_v: f32, ndot_l: f32) -> f32 {
    let r = sheen_roughness(f64::from(roughness));
    let d = d_charlie(r, f64::from(ndot_h));
    f32_from_f64(d * v_neubelt(f64::from(ndot_v), f64::from(ndot_l)))
}

// ---------------------------------------------------------------------------
// The white-furnace integral: a white base under a uniform white environment.
// Test-only: `furnace.rs` is the only consumer, so the non-test build of this
// module carries the fragment twin alone.
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod furnace_math {
    use engine_core::checked::saturating_f64_to_u64;

    use super::*;

    /// A loop index as an f64. The loops here run over a few thousand steps,
    /// so the conversion is exact.
    fn index(i: usize) -> f64 {
        f64::from(u32::try_from(i).unwrap_or(u32::MAX))
    }

    /// A non-negative f64 as a loop bound, rounded to nearest.
    fn bound(v: f64) -> usize {
        usize::try_from(saturating_f64_to_u64(v.round())).unwrap_or(usize::MAX)
    }

    /// One lobe configuration over a white base.
    #[derive(Clone, Copy)]
    pub(crate) struct Config {
        pub(crate) metallic: f64,
        /// Coat weight, perceptual roughness, ior.
        pub(crate) coat: Option<(f64, f64, f64)>,
        /// Sheen weight and roughness (white color).
        pub(crate) sheen: Option<(f64, f64)>,
        /// Anisotropy strength and rotation in turns.
        pub(crate) aniso: Option<(f64, f64)>,
        pub(crate) transmission: f64,
        /// Skin terminator LUT at full weight: curvature and the color channel read.
        pub(crate) skin: Option<(f64, usize)>,
    }

    impl Config {
        /// A plain dielectric: no lobe over the base BRDF.
        pub(crate) const BASE: Self = Self {
            metallic: 0.0,
            coat: None,
            sheen: None,
            aniso: None,
            transmission: 0.0,
            skin: None,
        };
    }

    /// The three parts of the outgoing radiance for one light direction, split by
    /// how fast they change with it: `broad` (diffuse and sheen) is smooth, `spec`
    /// and `coat` are the specular peaks of the base and of the coat. Their sum is
    /// the WGSL `lit` for a white base and light color 1.
    #[derive(Clone, Copy)]
    pub(crate) struct Terms {
        pub(crate) broad: f64,
        pub(crate) spec: f64,
        pub(crate) coat: f64,
    }

    /// The terms of one light direction under a white base, as the fragment chain
    /// builds them.
    pub(crate) fn lit_terms(cfg: Config, perceptual: f64, n: V3, v: V3, l: V3) -> Terms {
        let h = norm([l[0] + v[0], l[1] + v[1], l[2] + v[2]]);
        let nl = dot(n, l).clamp(0.0, 1.0);
        let nv = dot(n, v).max(0.0001);
        let nh = dot(n, h).clamp(0.0, 1.0);
        let lh = dot(l, h).clamp(0.0, 1.0);
        let alpha = base_alpha(perceptual);
        let f0 = 0.04 * (1.0 - cfg.metallic) + cfg.metallic;
        // The shader sums three channels: white f0 times 3 times 50 * 0.33.
        let f90 = (f0 * 3.0 * 50.0 * 0.33).clamp(0.0, 1.0);
        let f = f_schlick(f0, f90, lh);
        let (d, vis, rough_e) = match cfg.aniso {
            Some((strength, rotation)) => {
                let (t, b) = frame(n, rotation);
                let [at, ab] = aniso_alphas(alpha, strength);
                (
                    d_aniso([at, ab], dot(t, h), dot(b, h), nh),
                    v_aniso(
                        [at, ab],
                        [dot(t, v), dot(b, v), nv],
                        [dot(t, l), dot(b, l), nl],
                    ) * f64::from(ANISO_ENERGY_KEEP),
                    (at * ab).sqrt().sqrt(),
                )
            }
            None => (
                d_ggx(alpha, nh),
                v_smith(alpha, nv, nl),
                perceptual.clamp(f64::from(PERCEPTUAL_ROUGHNESS_FLOOR), 1.0),
            ),
        };
        let (fa, fb) = table(nv, rough_e);
        let mut spec = d * vis * f;
        spec *= 1.0 + f0 * (1.0 / (fa + fb) - 1.0);
        let spec_albedo = (f0 * fa + f90 * fb).min(1.0);
        let mut diffuse = (1.0 - cfg.metallic) * (1.0 - spec_albedo) * fd_burley(alpha, nv, nl, lh);
        diffuse *= 1.0 - cfg.transmission;
        // The skin LUT replaces the diffuse cosine, and keeps light on the dark side.
        let diffuse_cos = cfg.skin.map_or(nl, |(curvature, channel)| {
            f64::from(terminator(f32_from_f64(dot(n, l)), f32_from_f64(curvature))[channel])
        });
        // Sheen and coat scale what is under them by a factor of the view angle
        // alone, then add their own term.
        let (sheen_keep, sheen) = match cfg.sheen {
            Some((w, r)) => {
                let sr = sheen_roughness(r);
                let e = f64::from(sheen_albedo(f32_from_f64(nv), f32_from_f64(sr)));
                (
                    (1.0 - w * e).max(0.0),
                    d_charlie(sr, nh) * v_neubelt(nv, nl) * w * nl,
                )
            }
            None => (1.0, 0.0),
        };
        let (coat_keep, coat) = match cfg.coat {
            Some((w, r, ior)) => {
                let cf0 = ((ior - 1.0) / (ior + 1.0)).powi(2);
                let fc = f_schlick(cf0, 1.0, lh) * w;
                let cr = r.clamp(f64::from(PERCEPTUAL_ROUGHNESS_FLOOR), 1.0);
                let (ca, cb) = table(nv, cr);
                let coat_view = (cf0 * ca + cb) * w;
                (
                    1.0 - coat_view,
                    d_ggx(cr * cr, nh) * v_smith(cr * cr, nv, nl) * fc * nl,
                )
            }
            None => (1.0, 0.0),
        };
        Terms {
            broad: coat_keep * (sheen_keep * diffuse * diffuse_cos + sheen),
            spec: coat_keep * sheen_keep * spec * nl,
            coat,
        }
    }

    /// Integral of `f(l)` over the light directions with polar angle up to `span`
    /// (pi / 2 for the upper hemisphere, pi for the whole sphere) by quadrature in
    /// light space (theta, phi). Right for terms that vary slowly with the light
    /// direction.
    pub(crate) fn integrate_light_space(span: f64, f: impl Fn(V3) -> f64) -> f64 {
        const PHI: usize = 240;
        let theta_cells = bound(span / (PI / 2.0) * 120.0);
        let mut sum = 0.0;
        for i in 0..theta_cells {
            let theta = (index(i) + 0.5) / index(theta_cells) * span;
            for j in 0..PHI {
                let phi = (index(j) + 0.5) / index(PHI) * 2.0 * PI;
                let l = [
                    theta.sin() * phi.cos(),
                    theta.sin() * phi.sin(),
                    theta.cos(),
                ];
                // `f` already holds the cosine; the measure is sin(theta).
                sum += f(l) * theta.sin();
            }
        }
        sum * (span / index(theta_cells)) * (2.0 * PI / index(PHI))
    }

    /// Integral over a specular peak, sampled in half-vector space: the half
    /// vector follows GGX at `alpha` (inverse CDF in `u`, a grid in `phi`), the
    /// light is the view reflected about it, and the weight is
    /// `4 (v.h) / (D(h) n.h)`. A peak narrower than a light-space cell is still
    /// resolved, so roughness 0.1 is judged like 1.
    fn integrate_half_vector(alpha: f64, v: V3, f: impl Fn(V3) -> f64) -> f64 {
        const NU: usize = 512;
        const NP: usize = 192;
        let mut sum = 0.0;
        for i in 0..NU {
            let u = (index(i) + 0.5) / index(NU);
            let tan2 = alpha * alpha * u / (1.0 - u);
            let ct = 1.0 / (1.0 + tan2).sqrt();
            let st = (1.0 - ct * ct).sqrt();
            for j in 0..NP {
                let phi = (index(j) + 0.5) / index(NP) * 2.0 * PI;
                let h = [st * phi.cos(), st * phi.sin(), ct];
                let vh = dot(v, h);
                if vh <= 0.0 {
                    continue;
                }
                let l = [
                    2.0 * vh * h[0] - v[0],
                    2.0 * vh * h[1] - v[1],
                    2.0 * vh * h[2] - v[2],
                ];
                if l[2] <= 0.0 {
                    continue;
                }
                sum += f(l) * 4.0 * vh / (d_ggx(alpha, ct) * ct);
            }
        }
        sum / index(NU * NP)
    }

    /// Outgoing albedo for a viewer `view_deg` from the normal: the integral of the
    /// lit radiance over the upper hemisphere. The smooth part goes by light-space
    /// quadrature, each specular peak by half-vector sampling at its own roughness.
    pub(crate) fn albedo(cfg: Config, perceptual: f64, view_deg: f64) -> f64 {
        let n = [0.0, 0.0, 1.0];
        let tv = view_deg.to_radians();
        let v = [tv.sin(), 0.0, tv.cos()];
        let terms = |l| lit_terms(cfg, perceptual, n, v, l);
        let alpha = base_alpha(perceptual);
        // An anisotropic peak is sampled with the wider of its two roughnesses.
        let spec_alpha = cfg.aniso.map_or(alpha, |(strength, _)| {
            let [at, ab] = aniso_alphas(alpha, strength);
            at.max(ab)
        });
        // Skin lights the dark side too: its diffuse is integrated over the whole sphere.
        let span = if cfg.skin.is_some() { PI } else { PI / 2.0 };
        let mut sum = integrate_light_space(span, |l| terms(l).broad)
            + integrate_half_vector(spec_alpha, v, |l| terms(l).spec);
        if let Some((_, r, _)) = cfg.coat {
            sum += integrate_half_vector(base_alpha(r), v, |l| terms(l).coat);
        }
        sum
    }

    const ROUGHNESS_GRID: [f64; 5] = [0.1, 0.3, 0.5, 0.75, 1.0];
    const ANGLE_GRID: [f64; 5] = [0.0, 20.0, 45.0, 70.0, 85.0];

    /// Worst outgoing albedo of `cfg` over the roughness by view-angle grid; the
    /// white-furnace bar.
    pub(crate) fn grid_max(cfg: Config) -> f64 {
        let mut worst: f64 = 0.0;
        for roughness in ROUGHNESS_GRID {
            for angle in ANGLE_GRID {
                worst = worst.max(albedo(cfg, roughness, angle));
            }
        }
        worst
    }
}

// ---------------------------------------------------------------------------
// The twin of the generated fragment at one point.
// ---------------------------------------------------------------------------

/// The geometric inputs of one shaded point: where the eye is, which point is
/// shaded, how it faces, and the curvature the shader's screen derivatives give
/// it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TwinCamera {
    /// Eye position, world units.
    pub pos: [f32; 3],
    /// The shaded surface point, world units.
    pub point: [f32; 3],
    /// Unit surface normal at `point`.
    pub normal: [f32; 3],
    /// `|dN| / |dP|` in the shader's curvature units: a sphere of radius `r`
    /// gives `CURVATURE_PER_INVERSE_METER / r` at every point, which is what
    /// the fragment's `fwidth` pair reduces to there.
    pub curvature: f32,
}

/// One directional light, as the material uniform carries it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TwinLight {
    /// Direction toward the light; normalized here, as the fragment does.
    pub dir_to_light: [f32; 3],
    /// Linear radiance: the `light_color` the host writes (`color *
    /// intensity`).
    pub radiance: [f32; 3],
}

/// The record's material streams, read out of the packed uniform so the twin
/// and the shader see the same numbers.
struct Surface {
    base: V3,
    metallic: f64,
    rough: f64,
    ior: f64,
    /// Coat weight, perceptual roughness, ior.
    coat: [f64; 3],
    /// Sheen color and weight.
    sheen: [f64; 4],
    /// Sheen perceptual roughness.
    sheen_r: f64,
    /// Anisotropy strength and rotation in turns.
    aniso: [f64; 2],
    /// Transmission attenuation color and weight.
    trans: [f64; 4],
    /// Transmission thickness.
    thickness: f64,
    /// Subsurface color and weight.
    sss: [f64; 4],
    sss_radius: V3,
    /// Emission color and scene luminance, after the layers.
    emissive: V3,
    exposure: f64,
}

/// Three floats of `u` from `at`, as f64.
fn three(u: &[f32], at: usize) -> V3 {
    [f64::from(u[at]), f64::from(u[at + 1]), f64::from(u[at + 2])]
}

/// The shader's streams for `record` at `coverage`, through the layer twin.
fn surface(u: &[f32], layer: &LayerResult) -> Surface {
    Surface {
        base: [
            f64::from(layer.base_color[0]),
            f64::from(layer.base_color[1]),
            f64::from(layer.base_color[2]),
        ],
        metallic: f64::from(u[3]),
        rough: f64::from(layer.roughness),
        ior: f64::from(u[5]),
        coat: [
            f64::from(layer.coat_weight),
            f64::from(layer.coat_roughness),
            f64::from(u[10]),
        ],
        sheen: [
            f64::from(u[12]),
            f64::from(u[13]),
            f64::from(u[14]),
            f64::from(u[15]),
        ],
        sheen_r: f64::from(u[16]),
        aniso: [f64::from(u[17]), f64::from(u[18])],
        trans: [
            f64::from(u[20]),
            f64::from(u[21]),
            f64::from(u[22]),
            f64::from(u[23]),
        ],
        thickness: f64::from(u[24]),
        sss: [
            f64::from(u[28]),
            f64::from(u[29]),
            f64::from(u[30]),
            f64::from(u[31]),
        ],
        sss_radius: three(u, 32),
        emissive: [
            f64::from(layer.emissive[0]),
            f64::from(layer.emissive[1]),
            f64::from(layer.emissive[2]),
        ],
        exposure: f64::from(u[40]),
    }
}

/// One shaded point: the four unit directions and the dot products the chain
/// reads off them.
struct Point {
    n: V3,
    v: V3,
    l: V3,
    h: V3,
    ndot_l: f64,
    ndot_v: f64,
    ndot_h: f64,
    ldot_h: f64,
}

impl Point {
    /// The point's geometry from the three directions, as the fragment reads
    /// them.
    fn new(n: V3, v: V3, l: V3) -> Self {
        let h = norm(add(l, v));
        Self {
            n,
            v,
            l,
            h,
            ndot_l: dot(n, l).clamp(0.0, 1.0),
            ndot_v: dot(n, v).max(0.0001),
            ndot_h: dot(n, h).clamp(0.0, 1.0),
            ldot_h: dot(l, h).clamp(0.0, 1.0),
        }
    }
}

/// The specular distribution, visibility and the albedo row of one point:
/// `(D, V, rough_e)`, in the branch the compiled structure names.
fn spec_terms(s: &Surface, anisotropic: bool, p: &Point, roughness: f64) -> (f64, f64, f64) {
    if anisotropic {
        let (t, b) = frame(p.n, s.aniso[1]);
        let [at, ab] = aniso_alphas(roughness, s.aniso[0]);
        (
            d_aniso([at, ab], dot(t, p.h), dot(b, p.h), p.ndot_h),
            v_aniso(
                [at, ab],
                [dot(t, p.v), dot(b, p.v), p.ndot_v],
                [dot(t, p.l), dot(b, p.l), p.ndot_l],
            ) * f64::from(ANISO_ENERGY_KEEP),
            (at * ab).sqrt().sqrt(),
        )
    } else {
        (
            d_ggx(roughness, p.ndot_h),
            v_smith(roughness, p.ndot_v, p.ndot_l),
            s.rough.clamp(f64::from(PERCEPTUAL_ROUGHNESS_FLOOR), 1.0),
        )
    }
}

/// The specular term of the chain: `D * V * F`, with the multiscatter
/// compensation the albedo row carries.
fn specular_term(d: f64, vis: f64, f: V3, f0: V3, fa: f64, fb: f64) -> V3 {
    let k = [1.0 + f0[0] * (1.0 / (fa + fb) - 1.0); 3];
    mul(scale(f, d * vis), k)
}

/// The skin lobe's extra term: the terminator LUT replacing the cosine, plus
/// the thin-slab light from behind.
fn skin_extra(s: &Surface, p: &Point, diffuse: V3, diffuse_albedo: V3, curvature: f64) -> V3 {
    let w = s.sss[3];
    let blurred: V3 = {
        let [r, g, b] = terminator(f32_from_f64(dot(p.n, p.l)), f32_from_f64(curvature));
        [f64::from(r), f64::from(g), f64::from(b)]
    };
    let mut extra = mul(diffuse, sub(mix3([p.ndot_l; 3], blurred, w), [p.ndot_l; 3]));
    let back = dot(p.v, scale(add(p.l, scale(p.n, 0.5)), -1.0))
        .clamp(0.0, 1.0)
        .powi(3);
    let spread = (s.sss_radius[0] + s.sss_radius[1] + s.sss_radius[2]) / 3.0;
    let thin: V3 = {
        let [r, g, b] = transmittance(f32_from_f64(spread * 250.0));
        [f64::from(r), f64::from(g), f64::from(b)]
    };
    let mix = w * back * (spread * 40.0).clamp(0.0, 1.0) / PI;
    extra = add(
        extra,
        scale(
            mul(mul(diffuse_albedo, [s.sss[0], s.sss[1], s.sss[2]]), thin),
            mix,
        ),
    );
    extra
}

/// One lit point: the chain the generated `fs` runs, without the tonemap, for
/// a light whose radiance is `radiance`.
fn lit_point(s: &Surface, structure: &Structure, p: &Point, curvature: f64, radiance: V3) -> V3 {
    let roughness = base_alpha(s.rough);
    let ior_f0 = ((s.ior - 1.0) / (s.ior + 1.0)).powi(2);
    let f0 = mix3([ior_f0; 3], s.base, s.metallic);
    let f90 = dot(f0, [16.5; 3]).clamp(0.0, 1.0);
    let f = fresnel3(f0, f90, p.ldot_h);
    let (spec_d, vis, rough_e) = spec_terms(s, structure.has_lobe(lobe::ANISOTROPY), p, roughness);
    let (fa, fb) = table(p.ndot_v, rough_e);
    let spec_albedo = min3(add(scale(f0, fa), [f90 * fb; 3]), [1.0; 3]);
    let mut diffuse_albedo = mul(scale(s.base, 1.0 - s.metallic), sub([1.0; 3], spec_albedo));
    if structure.has_lobe(lobe::TRANSMISSION) {
        diffuse_albedo = scale(diffuse_albedo, 1.0 - s.trans[3]);
    }
    let diffuse = scale(
        diffuse_albedo,
        fd_burley(roughness, p.ndot_v, p.ndot_l, p.ldot_h),
    );
    let spec = specular_term(spec_d, vis, f, f0, fa, fb);
    let mut lit = scale(add(diffuse, spec), p.ndot_l);
    if structure.has_lobe(lobe::SUBSURFACE) {
        lit = add(lit, skin_extra(s, p, diffuse, diffuse_albedo, curvature));
    }
    if structure.has_lobe(lobe::SHEEN) {
        let sr = sheen_roughness(s.sheen_r);
        let sheen_term = scale(
            [s.sheen[0], s.sheen[1], s.sheen[2]],
            d_charlie(sr, p.ndot_h) * v_neubelt(p.ndot_v, p.ndot_l) * s.sheen[3],
        );
        let peak = s.sheen[0].max(s.sheen[1]).max(s.sheen[2]);
        let keep = (1.0
            - s.sheen[3]
                * peak
                * f64::from(sheen_albedo(f32_from_f64(p.ndot_v), f32_from_f64(sr))))
        .max(0.0);
        lit = add(scale(lit, keep), scale(sheen_term, p.ndot_l));
    }
    if structure.has_lobe(lobe::COAT) {
        let (w, r, ior) = (s.coat[0], s.coat[1], s.coat[2]);
        let cf0 = ((ior - 1.0) / (ior + 1.0)).powi(2);
        let fc = f_schlick(cf0, 1.0, p.ldot_h) * w;
        let cr = base_alpha(r);
        let (ca, cb) = table(
            p.ndot_v,
            r.clamp(f64::from(PERCEPTUAL_ROUGHNESS_FLOOR), 1.0),
        );
        let coat_view = (cf0 * ca + cb) * w;
        let coat_spec = d_ggx(cr, p.ndot_h) * v_smith(cr, p.ndot_v, p.ndot_l) * fc * p.ndot_l;
        lit = add(scale(lit, 1.0 - coat_view), [coat_spec; 3]);
    }
    add(mul(lit, radiance), s.emissive)
}

/// The RRT/ODT fit of `aces_fitted`, per channel.
fn rrt_odt(v: f64) -> f64 {
    let a = v * (v + 0.0245786) - 0.000090537;
    let b = v * (0.983729 * v + 0.4329510) + 0.238081;
    a / b
}

/// `aces_fitted`: the ACES input and output matrices around the fit, clamped.
fn aces_fitted(c: V3) -> V3 {
    let r = 0.59719 * c[0] + 0.35458 * c[1] + 0.04823 * c[2];
    let g = 0.07600 * c[0] + 0.90834 * c[1] + 0.01566 * c[2];
    let b = 0.02840 * c[0] + 0.13383 * c[1] + 0.83777 * c[2];
    let fit = [rrt_odt(r), rrt_odt(g), rrt_odt(b)];
    [
        (1.60475 * fit[0] - 0.53108 * fit[1] - 0.07367 * fit[2]).clamp(0.0, 1.0),
        (-0.10208 * fit[0] + 1.10813 * fit[1] - 0.00605 * fit[2]).clamp(0.0, 1.0),
        (-0.00327 * fit[0] - 0.07276 * fit[1] + 1.07602 * fit[2]).clamp(0.0, 1.0),
    ]
}

/// One channel of `linear_to_srgb_term`: the two branches, selected on the
/// sRGB threshold.
fn srgb_term(v: f64) -> f64 {
    let lo = v * 12.92;
    let hi = 1.055 * v.powf(1.0 / 2.4) - 0.055;
    if v <= 0.0031308 {
        lo
    } else {
        hi
    }
}

/// The transmission alpha of the generated fragment: the attenuation color
/// raised to the thickness, and its luminance.
fn transmission_alpha(s: &Surface) -> f64 {
    let exponent = (s.thickness * 10.0).clamp(0.0, 10.0);
    let lum = 0.2126 * s.trans[0].max(0.0001).powf(exponent)
        + 0.7152 * s.trans[1].max(0.0001).powf(exponent)
        + 0.0722 * s.trans[2].max(0.0001).powf(exponent);
    (1.0 - s.trans[3] * lum).clamp(0.0, 1.0)
}

/// The pixel the forward pass presents for one point of `record`, in `0..=1`
/// sRGB: the fragment chain (`fs` of the generated WGSL) tonemapped, sRGB
/// encoded, and premultiplied by the transmission alpha the blended pass
/// applies over a black target. A record without transmission keeps alpha 1.
///
/// Same inputs give the same three numbers on every host: pure f64 math over
/// the record's packed uniform, the layer twin at `coverage`, and the two
/// directions. Never runs in a tick.
#[must_use]
pub fn twin_lit_rgb(
    record: &MaterialRecord,
    coverage: &Coverage,
    camera: &TwinCamera,
    light: &TwinLight,
) -> [f32; 3] {
    let structure = Structure::of(record);
    let u = pack_uniform(record, 1.0);
    let layer = twin_layers(record, coverage);
    let s = surface(&u, &layer);
    let n = norm([
        f64::from(camera.normal[0]),
        f64::from(camera.normal[1]),
        f64::from(camera.normal[2]),
    ]);
    let v = norm([
        f64::from(camera.pos[0] - camera.point[0]),
        f64::from(camera.pos[1] - camera.point[1]),
        f64::from(camera.pos[2] - camera.point[2]),
    ]);
    let l = norm([
        f64::from(light.dir_to_light[0]),
        f64::from(light.dir_to_light[1]),
        f64::from(light.dir_to_light[2]),
    ]);
    let radiance = [
        f64::from(light.radiance[0]),
        f64::from(light.radiance[1]),
        f64::from(light.radiance[2]),
    ];
    let point = Point::new(n, v, l);
    let lit = lit_point(
        &s,
        &structure,
        &point,
        f64::from(camera.curvature),
        radiance,
    );
    let exposed = max3(scale(lit, s.exposure), [0.0; 3]);
    let encoded = aces_fitted(exposed);
    let alpha = if structure.has_lobe(lobe::TRANSMISSION) {
        transmission_alpha(&s)
    } else {
        1.0
    };
    let mut out = [0.0f32; 3];
    for (slot, value) in out.iter_mut().zip(encoded) {
        *slot = f32_from_f64(srgb_term(value) * alpha);
    }
    out
}

// ---------------------------------------------------------------------------
// CPU reference of the cloth sheen lobe.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::{compile, lobe, material_row, Structure};
    use crate::set::MaterialSet;

    /// The shipped cloth's sheen perceptual roughness, inside the
    /// `SHEEN_ROUGHNESS_MIN..=1` band the fragment clamps to.
    const CLOTH_SHEEN_ROUGHNESS: f32 = 0.7;

    /// The nine angles of the sweep: three view angles by three light angles
    /// off the surface normal, from near head-on to a grazing 80 degrees.
    const SWEEP_DEG: [f64; 3] = [5.0, 45.0, 80.0];

    /// The row's F2P and first Accept clause: the CPU reference of the cloth
    /// Charlie sheen lobe matches the Filament formula within 1e-3 at nine
    /// angles. The formula below is written from the Filament source
    /// (`shaders/src/brdf.fs`): the caller hands the linear roughness, so the
    /// perceptual sheen roughness squares first, and the half-vector sine is
    /// floored at `2^-7`.
    #[test]
    fn charlie_sheen_matches_the_filament_formula_at_nine_angles() {
        assert_eq!(2f64.powi(-7), 0.0078125, "the 2^-7 clamp");
        let n: V3 = [0.0, 0.0, 1.0];
        let dir = |deg: f64| -> V3 {
            let (s, c) = (deg.to_radians().sin(), deg.to_radians().cos());
            [s, 0.0, c]
        };
        let mut worst = 0.0f64;
        let mut sampled = 0usize;
        for view_deg in SWEEP_DEG {
            for light_deg in SWEEP_DEG {
                let v = dir(view_deg);
                let l = dir(light_deg);
                let h = norm(add(l, v));
                let nh = dot(n, h).clamp(0.0, 1.0);
                let nv = dot(n, v).clamp(0.0, 1.0);
                let nl = dot(n, l).clamp(0.0, 1.0);
                // Filament D_Charlie at the linear roughness, times V_Neubelt.
                let inv_alpha = 1.0 / f64::from(CLOTH_SHEEN_ROUGHNESS).powi(2);
                let sin2h = (1.0 - nh * nh).max(0.0078125);
                let d = (2.0 + inv_alpha) * sin2h.powf(inv_alpha * 0.5) / (2.0 * PI);
                let vis = 1.0 / (4.0 * (nl + nv - nl * nv));
                let expected = d * vis;
                let got = f64::from(sheen_lobe(
                    CLOTH_SHEEN_ROUGHNESS,
                    f32_from_f64(nh),
                    f32_from_f64(nv),
                    f32_from_f64(nl),
                ));
                assert!(
                    (got - expected).abs() < 1e-3,
                    "view {view_deg} light {light_deg}: reference {got} vs Filament {expected}"
                );
                assert!(
                    got > 0.0,
                    "the lobe keeps light at view {view_deg} light {light_deg}"
                );
                worst = worst.max((got - expected).abs());
                sampled += 1;
            }
        }
        assert_eq!(sampled, 9, "nine angles");
        // The clamp is load-bearing: at NoH one the sine is zero and the
        // floored sine is what keeps D_Charlie positive. F2P: drop the clamp
        // and this is exactly zero.
        let head_on = f64::from(sheen_lobe(CLOTH_SHEEN_ROUGHNESS, 1.0, 1.0, 1.0));
        assert!(
            head_on > 0.0,
            "the 2^-7 clamp keeps D_Charlie positive at NoH 1"
        );
        eprintln!(
            "charlie sheen sweep: 9 angles, worst |reference - Filament| = {worst:e}, NoH 1 lobe {head_on:e}"
        );
    }

    /// The row's Diff: the shipped cloth material id carries the sheen lobe and
    /// the generated text holds the Charlie D, the Neubelt denominator and the
    /// `2^-7` clamp. F2P: drop the lobe from the record or the chunk and one of
    /// the three named pieces is gone.
    #[test]
    fn the_cloth_material_id_generates_the_charlie_sheen_lobe() {
        let set = MaterialSet::shipped();
        let cloth = set.get("cloth").cloned().expect("shipped cloth");
        let structure = Structure::of(&cloth);
        assert!(structure.has_lobe(lobe::SHEEN), "cloth carries sheen");
        let sheen = cloth.look.sheen.expect("cloth has sheen parameters");
        assert_eq!(
            sheen.roughness, CLOTH_SHEEN_ROUGHNESS,
            "cloth sheen roughness"
        );
        let text = compile(&cloth).wgsl;
        assert!(
            text.contains("fn d_charlie(roughness: f32, ndot_h: f32) -> f32 {"),
            "the Charlie D the row names"
        );
        assert!(
            text.contains("max(1.0 - cos2h, 0.0078125)"),
            "the 2^-7 clamp"
        );
        assert!(
            text.contains("4.0 * (ndot_l + ndot_v - ndot_l * ndot_v)"),
            "the Neubelt 4(NoL+NoV-NoL NoV) denominator"
        );
        // The material table row the forward pass bins cloth under keeps it.
        let id = set.wire_id("cloth").expect("cloth has a wire id");
        let row = material_row(id).expect("cloth has a table row");
        assert_eq!(row.lobes, structure.lobes, "cloth table row lobes");
        assert_eq!(row.lobes & lobe::SHEEN, lobe::SHEEN, "the row names sheen");
    }
}
