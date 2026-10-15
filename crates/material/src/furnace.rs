//! furnace: a CPU twin of the generated lighting and the white-furnace test.
//!
//! Test-only. The twin evaluates the same formulas as the WGSL in `compile`
//! for one white channel (white base color, one light) and integrates the
//! outgoing radiance over the upper hemisphere for a viewer at a given angle:
//! under a uniform white environment a surface that keeps energy returns at
//! most 1. The bar is 1.02 on a 5 by 5 roughness by view-angle grid
//! (roughness 0.3 to 1, 0 to 85 degrees) for the base BRDF and for each lobe
//! added over it (coat, sheen, thin transmission); the anisotropic lobe gets
//! 1.03, because its albedo is read from the isotropic table at the same
//! alpha product. Below roughness 0.3 the specular lobe is narrower than the
//! quadrature cell (the tables themselves are exact there, see `dfg`), and
//! wrapped subsurface is a pseudo-energy trick (it lights from below the
//! horizon on purpose); neither is judged here. Math in f64; this module
//! never runs in a tick.

use std::f64::consts::PI;

use crate::dfg::{dfg, sheen_albedo, SHEEN_ROUGHNESS_MIN};

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
    let [at, ab] = alphas;
    let a2 = at * ab;
    let d = [ab * th, at * bh, a2 * nh];
    let w2 = a2 / dot(d, d).max(0.00001);
    a2 * w2 * w2 / PI
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
}

impl Config {
    const BASE: Self = Self {
        metallic: 0.0,
        coat: None,
        sheen: None,
        aniso: None,
        transmission: 0.0,
    };
}

fn table(nv: f64, p: f64) -> (f64, f64) {
    let [a, b] = dfg(nv as f32, p as f32);
    (f64::from(a), f64::from(b))
}

/// Outgoing radiance factor for one light direction: the WGSL `lit` for a
/// white base and light color 1.
fn lit(cfg: Config, perceptual: f64, n: V3, v: V3, l: V3) -> f64 {
    let h = norm([l[0] + v[0], l[1] + v[1], l[2] + v[2]]);
    let nl = dot(n, l).clamp(0.0, 1.0);
    let nv = dot(n, v).max(0.0001);
    let nh = dot(n, h).clamp(0.0, 1.0);
    let lh = dot(l, h).clamp(0.0, 1.0);
    let alpha = perceptual.clamp(0.089, 1.0).powi(2);
    let f0 = 0.04 * (1.0 - cfg.metallic) + cfg.metallic;
    // The shader sums three channels: white f0 times 3 times 50 * 0.33.
    let f90 = (f0 * 3.0 * 50.0 * 0.33).clamp(0.0, 1.0);
    let f = f_schlick(f0, f90, lh);
    let (d, vis, rough_e) = match cfg.aniso {
        Some((strength, rotation)) => {
            let (t, b) = frame(n, rotation);
            let at = (alpha * (1.0 + strength)).min(1.0);
            let ab = (alpha * (1.0 - strength)).max(0.002);
            (
                d_aniso([at, ab], dot(t, h), dot(b, h), nh),
                v_aniso(
                    [at, ab],
                    [dot(t, v), dot(b, v), nv],
                    [dot(t, l), dot(b, l), nl],
                ),
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
    let mut out = (diffuse + spec) * nl;
    if let Some((w, r)) = cfg.sheen {
        let sr = r.clamp(f64::from(SHEEN_ROUGHNESS_MIN), 1.0);
        let term = d_charlie(sr, nh) * v_neubelt(nv, nl) * w;
        let e = f64::from(sheen_albedo(nv as f32, sr as f32));
        out = out * (1.0 - w * e).max(0.0) + term * nl;
    }
    if let Some((w, r, ior)) = cfg.coat {
        let cf0 = ((ior - 1.0) / (ior + 1.0)).powi(2);
        let fc = f_schlick(cf0, 1.0, lh) * w;
        let cr = r.clamp(0.089, 1.0);
        let (ca, cb) = table(nv, cr);
        let coat_view = (cf0 * ca + cb) * w;
        let coat_spec = d_ggx(cr * cr, nh) * v_smith(cr * cr, nv, nl) * fc * nl;
        out = out * (1.0 - coat_view) + coat_spec;
    }
    out
}

/// Outgoing albedo for a viewer `view_deg` from the normal: the integral of
/// `lit` over the upper hemisphere.
fn albedo(cfg: Config, perceptual: f64, view_deg: f64) -> f64 {
    let n = [0.0, 0.0, 1.0];
    let tv = view_deg.to_radians();
    let v = [tv.sin(), 0.0, tv.cos()];
    const THETA: usize = 120;
    const PHI: usize = 240;
    let mut sum = 0.0;
    for i in 0..THETA {
        let theta = (i as f64 + 0.5) / THETA as f64 * (PI / 2.0);
        for j in 0..PHI {
            let phi = (j as f64 + 0.5) / PHI as f64 * 2.0 * PI;
            let l = [
                theta.sin() * phi.cos(),
                theta.sin() * phi.sin(),
                theta.cos(),
            ];
            // `lit` already holds the cosine; the measure is sin(theta).
            sum += lit(cfg, perceptual, n, v, l) * theta.sin();
        }
    }
    sum * (PI / 2.0 / THETA as f64) * (2.0 * PI / PHI as f64)
}

const ROUGHNESS_GRID: [f64; 5] = [0.3, 0.45, 0.6, 0.8, 1.0];
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
    let cases: [(&str, Config, f64); 7] = [
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
