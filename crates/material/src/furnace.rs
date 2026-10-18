//! furnace: a CPU twin of the generated lighting and the white-furnace test.
//!
//! Test-only. The twin evaluates the same formulas as the WGSL in `compile`
//! for one white channel (white base color, one light) and integrates the
//! outgoing radiance over the upper hemisphere for a viewer at a given angle:
//! under a uniform white environment a surface that keeps energy returns at
//! most 1. The bar is 1.02 on a 5 by 5 roughness by view-angle grid
//! (roughness 0.1 to 1, 0 to 85 degrees) for the base BRDF and for each lobe
//! added over it (coat, sheen, thin transmission); the anisotropic lobe gets
//! 1.03, because its albedo is read from the isotropic table at the same
//! alpha product. The smooth part of the radiance (diffuse, sheen) is
//! integrated over light directions; each specular peak is integrated over
//! half vectors with GGX importance, so a peak narrower than a light-space
//! cell is still resolved. Wrapped subsurface is a pseudo-energy trick (it
//! lights from below the horizon on purpose) and is not judged here. Math in
//! f64; this module never runs in a tick.

use std::f64::consts::PI;

use crate::compile::ANISO_ENERGY_KEEP;
use crate::dfg::{dfg, sheen_albedo, SHEEN_ROUGHNESS_MIN};
use crate::skin::terminator;

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
    f0 + (f90 - f0) * (1.0 - vh).powi(5)
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
    // Filament's a2^3 / d2^2 as 1 / (PI a2 s^2): the same value without the f32 underflow
    // at low roughness (see the WGSL `d_ggx_anisotropic`).
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

/// One lobe configuration over a white base.
#[derive(Clone, Copy)]
struct Config {
    metallic: f64,
    /// Coat weight, perceptual roughness, ior.
    coat: Option<(f64, f64, f64)>,
    /// Sheen weight and roughness (white color).
    sheen: Option<(f64, f64)>,
    /// Anisotropy strength and rotation in turns.
    aniso: Option<(f64, f64)>,
    transmission: f64,
    /// Skin terminator LUT at full weight: curvature and the color channel read.
    skin: Option<(f64, usize)>,
}

impl Config {
    const BASE: Self = Self {
        metallic: 0.0,
        coat: None,
        sheen: None,
        aniso: None,
        transmission: 0.0,
        skin: None,
    };
}

fn table(nv: f64, p: f64) -> (f64, f64) {
    let [a, b] = dfg(nv as f32, p as f32);
    (f64::from(a), f64::from(b))
}

/// Anisotropic roughness pair for an isotropic `alpha` and a strength, as the WGSL builds it.
fn aniso_alphas(alpha: f64, strength: f64) -> [f64; 2] {
    [
        (alpha * (1.0 + strength)).min(1.0),
        (alpha * (1.0 - strength)).max(0.002),
    ]
}

/// Base roughness as the shader clamps it.
fn base_alpha(perceptual: f64) -> f64 {
    perceptual.clamp(0.089, 1.0).powi(2)
}

/// The three parts of the outgoing radiance for one light direction, split by how fast they
/// change with it: `broad` (diffuse and sheen) is smooth, `spec` and `coat` are the
/// specular peaks of the base and of the coat. Their sum is the WGSL `lit` for a white base
/// and light color 1.
#[derive(Clone, Copy)]
struct Terms {
    broad: f64,
    spec: f64,
    coat: f64,
}

fn lit_terms(cfg: Config, perceptual: f64, n: V3, v: V3, l: V3) -> Terms {
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
            perceptual.clamp(0.089, 1.0),
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
        f64::from(terminator(dot(n, l) as f32, curvature as f32)[channel])
    });
    // Sheen and coat scale what is under them by a factor of the view angle alone, then add
    // their own term: out = coat_keep * (sheen_keep * (diffuse + spec) * nl + sheen) + coat.
    let (sheen_keep, sheen) = match cfg.sheen {
        Some((w, r)) => {
            let sr = r.clamp(f64::from(SHEEN_ROUGHNESS_MIN), 1.0);
            let e = f64::from(sheen_albedo(nv as f32, sr as f32));
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
            let cr = r.clamp(0.089, 1.0);
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

/// Integral of `f(l)` over the light directions with polar angle up to `span` (pi / 2 for the
/// upper hemisphere, pi for the whole sphere) by quadrature in light space (theta, phi).
/// Right for terms that vary slowly with the light direction.
fn integrate_light_space(span: f64, f: impl Fn(V3) -> f64) -> f64 {
    const PHI: usize = 240;
    let theta_cells = (span / (PI / 2.0) * 120.0).round() as usize;
    let mut sum = 0.0;
    for i in 0..theta_cells {
        let theta = (i as f64 + 0.5) / theta_cells as f64 * span;
        for j in 0..PHI {
            let phi = (j as f64 + 0.5) / PHI as f64 * 2.0 * PI;
            let l = [
                theta.sin() * phi.cos(),
                theta.sin() * phi.sin(),
                theta.cos(),
            ];
            // `f` already holds the cosine; the measure is sin(theta).
            sum += f(l) * theta.sin();
        }
    }
    sum * (span / theta_cells as f64) * (2.0 * PI / PHI as f64)
}

/// Integral over the upper hemisphere of `f(l)` for a specular peak, sampled in half-vector
/// space: the half vector follows GGX at `alpha` (inverse CDF in `u`, a grid in `phi`), the
/// light is the view reflected about it, and the weight is `4 (v.h) / (D(h) n.h)`. A peak
/// narrower than a light-space cell is still resolved, so roughness 0.1 is judged like 1.
fn integrate_half_vector(alpha: f64, v: V3, f: impl Fn(V3) -> f64) -> f64 {
    const NU: usize = 512;
    const NP: usize = 192;
    let mut sum = 0.0;
    for i in 0..NU {
        let u = (i as f64 + 0.5) / NU as f64;
        let tan2 = alpha * alpha * u / (1.0 - u);
        let ct = 1.0 / (1.0 + tan2).sqrt();
        let st = (1.0 - ct * ct).sqrt();
        for j in 0..NP {
            let phi = (j as f64 + 0.5) / NP as f64 * 2.0 * PI;
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
    sum / (NU * NP) as f64
}

/// Outgoing albedo for a viewer `view_deg` from the normal: the integral of the lit radiance
/// over the upper hemisphere. The smooth part goes by light-space quadrature, each specular
/// peak by half-vector sampling at its own roughness.
fn albedo(cfg: Config, perceptual: f64, view_deg: f64) -> f64 {
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

fn grid_max(cfg: Config) -> f64 {
    let mut worst: f64 = 0.0;
    for roughness in ROUGHNESS_GRID {
        for angle in ANGLE_GRID {
            worst = worst.max(albedo(cfg, roughness, angle));
        }
    }
    worst
}

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

/// The whole radiance integrated over light directions only: the old way, right wherever the
/// specular peak is wider than a light-space cell.
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
    // The peak is sharper than a light-space cell here, and the anisotropic lobe once lost it
    // to an underflow guard in the WGSL (albedo 0.0006 at roughness 0.1).
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
    // The albedo tables are isotropic, so the lobe's furnace depends on how stretched it is and
    // on which way the view lies. Without the derate this sweep reaches 1.042.
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
