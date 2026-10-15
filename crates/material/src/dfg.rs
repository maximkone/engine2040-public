//! dfg: baked directional-albedo tables behind the energy-conserving lobes.
//!
//! The generated shader needs two numbers per pixel that have no closed form:
//! how much light the base specular lobe returns for a view angle and a
//! roughness (to compensate multiple scattering and to take that share out
//! of the diffuse), and the same for the Charlie sheen lobe (to take it out
//! of what lies below). Both are integrals. This module holds them as small
//! tables (the shader reads them with a bilinear lookup) and its test
//! regenerates them from the integrals, so no number here is a guess. The
//! white-furnace twin reads the same arrays, so shader and twin share one
//! source. The specular table is exact for the D_GGX and correlated Smith
//! visibility the shader uses (half-vector importance sampling); the sheen
//! table is the l-space integral with the roughness clamped to
//! `SHEEN_ROUGHNESS_MIN..=1`.

/// Axis size of both tables.
pub const TABLE_N: usize = 8;
/// Lowest sheen roughness the shader and the sheen table accept.
pub const SHEEN_ROUGHNESS_MIN: f32 = 0.3;

/// Directional albedo split of the base specular lobe (GGX, correlated
/// Smith): `E = f0 * a + f90 * b` for the Schlick weight. Eight view steps
/// (square root of the cosine) by eight perceptual roughness steps, one row
/// per roughness. The `tables_match_their_integrals` test regenerates them.
pub const DFG_AB: [[f32; 2]; 64] = [
    [0.09709, 0.78955],
    [0.12363, 0.80594],
    [0.35187, 0.64184],
    [0.63818, 0.36046],
    [0.86112, 0.13842],
    [0.97159, 0.02832],
    [0.99864, 0.00134],
    [1.00000, 0.00000],
    [0.21351, 0.69581],
    [0.20935, 0.67558],
    [0.37048, 0.59190],
    [0.64096, 0.35082],
    [0.85985, 0.13748],
    [0.97022, 0.02864],
    [0.99804, 0.00142],
    [1.00000, 0.00000],
    [0.52650, 0.43338],
    [0.51737, 0.42127],
    [0.52503, 0.35806],
    [0.66177, 0.25427],
    [0.84052, 0.11967],
    [0.95119, 0.02973],
    [0.98638, 0.00219],
    [0.99239, 0.00000],
    [0.72406, 0.24691],
    [0.71934, 0.23964],
    [0.70374, 0.19972],
    [0.72358, 0.14225],
    [0.79722, 0.07735],
    [0.88436, 0.02530],
    [0.93543, 0.00312],
    [0.95642, 0.00002],
    [0.82949, 0.14258],
    [0.82358, 0.13735],
    [0.79267, 0.11096],
    [0.76168, 0.07679],
    [0.76222, 0.04274],
    [0.78808, 0.01640],
    [0.82339, 0.00299],
    [0.85471, 0.00004],
    [0.88079, 0.08764],
    [0.87014, 0.08346],
    [0.81289, 0.06444],
    [0.75401, 0.04262],
    [0.70910, 0.02316],
    [0.68166, 0.00931],
    [0.67210, 0.00209],
    [0.67773, 0.00005],
    [0.90306, 0.05869],
    [0.88539, 0.05499],
    [0.79563, 0.03997],
    [0.70803, 0.02485],
    [0.62932, 0.01285],
    [0.56393, 0.00511],
    [0.51279, 0.00125],
    [0.47575, 0.00004],
    [0.90990, 0.04290],
    [0.88357, 0.03936],
    [0.76275, 0.02650],
    [0.64251, 0.01526],
    [0.53488, 0.00739],
    [0.44351, 0.00283],
    [0.36809, 0.00071],
    [0.30682, 0.00003],
];

/// Charlie sheen directional albedo: eight view steps by eight roughness
/// steps over `SHEEN_ROUGHNESS_MIN..=1`, one row per roughness.
pub const SHEEN_E: [f32; 64] = [
    1.29041, 1.20090, 0.86894, 0.55479, 0.30283, 0.13017, 0.03534, 0.00141, 1.03066, 0.97292,
    0.75407, 0.53533, 0.34216, 0.18703, 0.07618, 0.01118, 0.87839, 0.83621, 0.67505, 0.50996,
    0.35696, 0.22339, 0.11370, 0.03125, 0.78096, 0.74753, 0.61959, 0.48722, 0.36146, 0.24645,
    0.14424, 0.05635, 0.71486, 0.68681, 0.57967, 0.46854, 0.36171, 0.26147, 0.16821, 0.08167,
    0.66803, 0.64353, 0.55023, 0.45362, 0.36033, 0.27161, 0.18689, 0.10474, 0.63372, 0.61168,
    0.52805, 0.44175, 0.35841, 0.27870, 0.20152, 0.12478, 0.60790, 0.58763, 0.51100, 0.43227,
    0.35644, 0.28382, 0.21307, 0.14177,
];

/// Bilinear lookup the shader mirrors: `view` is the cosine of the view
/// angle, `row` the roughness position in `0..=1`; `get` reads one cell.
fn bilinear(view: f32, row: f32, get: impl Fn(usize) -> [f32; 2]) -> [f32; 2] {
    let last = (TABLE_N - 1) as f32;
    let sx = view.clamp(0.0, 1.0).sqrt() * last;
    let sy = row.clamp(0.0, 1.0) * last;
    let x0 = sx.floor().min(last - 1.0);
    let y0 = sy.floor().min(last - 1.0);
    let (fx, fy) = (sx - x0, sy - y0);
    let (xi, yi) = (x0 as usize, y0 as usize);
    let at = |x: usize, y: usize| get(y * TABLE_N + x);
    let mut out = [0.0; 2];
    for (k, slot) in out.iter_mut().enumerate() {
        let a = at(xi, yi)[k] * (1.0 - fx) + at(xi + 1, yi)[k] * fx;
        let b = at(xi, yi + 1)[k] * (1.0 - fx) + at(xi + 1, yi + 1)[k] * fx;
        *slot = a * (1.0 - fy) + b * fy;
    }
    out
}

/// `(a, b)` of the specular lobe at view cosine `nv` and perceptual
/// roughness `p`.
#[must_use]
pub fn dfg(nv: f32, p: f32) -> [f32; 2] {
    bilinear(nv, p, |i| DFG_AB[i])
}

/// Sheen directional albedo at view cosine `nv` and sheen roughness `r`.
#[must_use]
pub fn sheen_albedo(nv: f32, r: f32) -> f32 {
    let row =
        (r.clamp(SHEEN_ROUGHNESS_MIN, 1.0) - SHEEN_ROUGHNESS_MIN) / (1.0 - SHEEN_ROUGHNESS_MIN);
    bilinear(nv, row, |i| [SHEEN_E[i], 0.0])[0]
}

/// The WGSL source of both tables and their lookups: private arrays plus
/// `dfg_lookup` and `sheen_albedo_lookup`, exactly [`dfg`] and
/// [`sheen_albedo`] in shader form.
#[must_use]
pub fn wgsl() -> String {
    let mut out = String::with_capacity(4096);
    out.push_str("var<private> DFG_AB: array<vec2<f32>, 64> = array<vec2<f32>, 64>(\n");
    for cell in DFG_AB {
        out.push_str(&format!("    vec2<f32>({:.5}, {:.5}),\n", cell[0], cell[1]));
    }
    out.push_str(");\nvar<private> SHEEN_E: array<f32, 64> = array<f32, 64>(\n");
    for value in SHEEN_E {
        out.push_str(&format!("    {value:.5},\n"));
    }
    out.push_str(");\n");
    out.push_str(LOOKUPS);
    out
}

const LOOKUPS: &str = r#"
fn dfg_lookup(nv: f32, p: f32) -> vec2<f32> {
    let sx = sqrt(clamp(nv, 0.0, 1.0)) * 7.0;
    let sy = clamp(p, 0.0, 1.0) * 7.0;
    let x0 = u32(min(floor(sx), 6.0));
    let y0 = u32(min(floor(sy), 6.0));
    let fx = sx - f32(x0);
    let fy = sy - f32(y0);
    let a = mix(DFG_AB[y0 * 8u + x0], DFG_AB[y0 * 8u + x0 + 1u], fx);
    let b = mix(DFG_AB[(y0 + 1u) * 8u + x0], DFG_AB[(y0 + 1u) * 8u + x0 + 1u], fx);
    return mix(a, b, fy);
}

fn sheen_albedo_lookup(nv: f32, r: f32) -> f32 {
    let sx = sqrt(clamp(nv, 0.0, 1.0)) * 7.0;
    let sy = clamp((clamp(r, 0.3, 1.0) - 0.3) / 0.7, 0.0, 1.0) * 7.0;
    let x0 = u32(min(floor(sx), 6.0));
    let y0 = u32(min(floor(sy), 6.0));
    let fx = sx - f32(x0);
    let fy = sy - f32(y0);
    let a = mix(SHEEN_E[y0 * 8u + x0], SHEEN_E[y0 * 8u + x0 + 1u], fx);
    let b = mix(SHEEN_E[(y0 + 1u) * 8u + x0], SHEEN_E[(y0 + 1u) * 8u + x0 + 1u], fx);
    return mix(a, b, fy);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    /// Specular `(a, b)` at view cosine `nv` and perceptual roughness `p`:
    /// the integral of D * V * cos over the lobe, split by the Schlick
    /// weight, sampled in half-vector space with GGX importance.
    fn spec_integral(nv: f64, p: f64) -> (f64, f64) {
        let alpha = p.max(0.089).powi(2);
        let a2 = alpha * alpha;
        let v = [(1.0 - nv * nv).max(0.0).sqrt(), 0.0, nv];
        const NU: usize = 512;
        const NP: usize = 96;
        let (mut a_sum, mut b_sum) = (0.0, 0.0);
        for i in 0..NU {
            let u = (i as f64 + 0.5) / NU as f64;
            let t2 = alpha * alpha * u / (1.0 - u);
            let ct = 1.0 / (1.0 + t2).sqrt();
            let st = (1.0 - ct * ct).sqrt();
            for j in 0..NP {
                let phi = (j as f64 + 0.5) / NP as f64 * 2.0 * PI;
                let h = [st * phi.cos(), st * phi.sin(), ct];
                let vh = v[0] * h[0] + v[1] * h[1] + v[2] * h[2];
                if vh <= 0.0 {
                    continue;
                }
                let nl = 2.0 * vh * h[2] - v[2];
                if nl <= 0.0 {
                    continue;
                }
                let lv = nl * ((nv - a2 * nv) * nv + a2).sqrt();
                let ll = nv * ((nl - a2 * nl) * nl + a2).sqrt();
                let vis = 0.5 / (lv + ll);
                let w = 4.0 * vh * vis * nl / ct;
                let f5 = (1.0 - vh).powi(5);
                a_sum += w * (1.0 - f5);
                b_sum += w * f5;
            }
        }
        let n = (NU * NP) as f64;
        (a_sum / n, b_sum / n)
    }

    fn charlie_integral(nv: f64, r: f64) -> f64 {
        let inv_alpha = 1.0 / (r * r);
        let v = [(1.0 - nv * nv).max(0.0).sqrt(), 0.0, nv];
        const T: usize = 160;
        const P: usize = 320;
        let mut sum = 0.0;
        for i in 0..T {
            let th = (i as f64 + 0.5) / T as f64 * (PI / 2.0);
            for j in 0..P {
                let ph = (j as f64 + 0.5) / P as f64 * 2.0 * PI;
                let l = [th.sin() * ph.cos(), th.sin() * ph.sin(), th.cos()];
                let h = [l[0] + v[0], l[1] + v[1], l[2] + v[2]];
                let hn = (h[0] * h[0] + h[1] * h[1] + h[2] * h[2]).sqrt();
                let nh = h[2] / hn;
                let sin2h = (1.0 - nh * nh).max(0.0078125);
                let d = (2.0 + inv_alpha) * sin2h.powf(inv_alpha * 0.5) / (2.0 * PI);
                let vis = 1.0 / (4.0 * (l[2] + nv - l[2] * nv));
                sum += d * vis * l[2] * th.sin();
            }
        }
        sum * (PI / 2.0 / T as f64) * (2.0 * PI / P as f64)
    }

    fn grid_nv(ix: usize) -> f64 {
        let s = ix as f64 / (TABLE_N - 1) as f64;
        (s * s).max(0.01)
    }

    #[test]
    fn tables_match_their_integrals() {
        for iy in 0..TABLE_N {
            let p = iy as f64 / (TABLE_N - 1) as f64;
            let r = f64::from(SHEEN_ROUGHNESS_MIN)
                + (1.0 - f64::from(SHEEN_ROUGHNESS_MIN)) * iy as f64 / (TABLE_N - 1) as f64;
            for ix in 0..TABLE_N {
                let nv = grid_nv(ix);
                let (a, b) = spec_integral(nv, p);
                let cell = DFG_AB[iy * TABLE_N + ix];
                assert!(
                    (a - f64::from(cell[0])).abs() < 2e-3,
                    "a({nv:.3},{p:.2}) {a:.5} vs {}",
                    cell[0]
                );
                assert!(
                    (b - f64::from(cell[1])).abs() < 2e-3,
                    "b({nv:.3},{p:.2}) {b:.5} vs {}",
                    cell[1]
                );
                let e = charlie_integral(nv, r);
                let sheen = f64::from(SHEEN_E[iy * TABLE_N + ix]);
                assert!(
                    (e - sheen).abs() < 3e-3,
                    "sheen({nv:.3},{r:.2}) {e:.5} vs {sheen}"
                );
            }
        }
    }

    #[test]
    fn lookups_hit_the_grid_points_and_stay_in_range() {
        let cell = DFG_AB[2 * TABLE_N + 3];
        let at = dfg((3.0f32 / 7.0).powi(2), 2.0 / 7.0);
        assert!((at[0] - cell[0]).abs() < 1e-4 && (at[1] - cell[1]).abs() < 1e-4);
        for nv in [0.0, 0.01, 0.3, 1.0, 2.0] {
            for p in [-1.0, 0.0, 0.5, 1.0, 3.0] {
                let [a, b] = dfg(nv, p);
                assert!(
                    (0.0..=1.0001).contains(&a) && (0.0..=1.0001).contains(&b),
                    "{nv} {p}"
                );
            }
        }
        assert!(sheen_albedo(0.5, 0.0) > 0.0);
    }

    #[test]
    fn the_shader_source_names_every_cell() {
        let text = wgsl();
        assert_eq!(text.matches("vec2<f32>(").count(), 64);
        assert!(text.contains("fn dfg_lookup"));
        assert!(text.contains("fn sheen_albedo_lookup"));
    }
}
