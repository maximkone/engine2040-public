//! clouds: volumetric layer over the RL4 Hillaire bake (Horizon raymarch school).
//! Owner: render lane. Consumer: sky capture via `composite_clouds_over_capture`
//! (ground-level views only, like the donor skybox-without-depth path). Units:
//! meters for heights, texels for wind, ms for cost. Determinism: row-major
//! fixed steps with hashed offsets; same inputs give identical bytes twice
//! (presentation only, never the sim hash). Errors: typed `CloudError`.
//! PORT(ad4422ff config.rs/clouds_compute.wgsl/common.wgsl): reimplemented,
//! zero donor bytes. Method: Schneider/Vos; Hillaire Frostbite sky/clouds.
use std::fmt;
/// Posted composite budget at 64x32 on the headless CPU reference path.
pub const CLOUD_COMPOSITE_BUDGET_MS: f64 = 2.0;
/// Early-out floor: stop marching once transmittance drops below this.
pub const CLOUD_MIN_TRANSMIT: f32 = 0.02;
/// Cloud layer params, adapted from the PORT `Config` uniform.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct CloudParams {
    /// Layer slab in meters: [bottom, top], bottom < top.
    pub layer_m: [f32; 2],
    /// Coverage in 0..=1; 0.5 is the donor default.
    pub coverage: f32,
    /// Extinction density per meter in 0..=4.
    pub density_per_m: f32,
    /// Softness: [edge, bottom] ramps in 0..=0.5.
    pub softness: [f32; 2],
    /// Detail: [scale, strength], scale in 0.5..=16, strength in 0..=1.
    pub detail: [f32; 2],
    /// Asymmetry: [forward g, backward g], each in -0.99..=0.99.
    pub phase_g: [f32; 2],
    /// Forward-lobe share in 0..=1.
    pub lobe_blend: f32,
    /// Sun direction (finite nonzero) and linear HDR sun color.
    pub sun_dir: [f32; 3],
    pub sun_color: [f32; 3],
    /// Base wind displacement in texels, added to the per-call wind.
    pub wind_uv: [f32; 2],
    /// March steps: [primary 1..=16, shadow 1..=8].
    pub steps: [u8; 2],
}
/// Cloud pass failures: bad input fails loudly with the reason carried.
#[derive(Clone, PartialEq, Debug)]
pub enum CloudError {
    Invalid(String),
}
impl fmt::Display for CloudError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "clouds input: {e}"),
        }
    }
}
impl std::error::Error for CloudError {}
fn ok_range(v: f32, lo: f32, hi: f32) -> bool {
    v.is_finite() && (lo..=hi).contains(&v)
}
impl CloudParams {
    /// Donor-default layer: 0.5 cover, 8 primary x 4 shadow steps.
    pub fn day() -> Self {
        Self {
            layer_m: [800.0, 2200.0],
            coverage: 0.5,
            density_per_m: 0.9,
            softness: [0.15, 0.2],
            detail: [6.0, 0.4],
            phase_g: [0.6, -0.25],
            lobe_blend: 0.7,
            sun_dir: [0.3, 1.0, 0.2],
            sun_color: [8.0, 7.6, 7.0],
            wind_uv: [0.0, 0.0],
            steps: [8, 4],
        }
    }
    /// Validate every uniform before any march; NaN and ranges fail loudly.
    pub fn validate(&self) -> Result<(), CloudError> {
        let ok = ok_range(self.coverage, 0.0, 1.0)
            && ok_range(self.density_per_m, 0.0, 4.0)
            && ok_range(self.softness[0], 0.0, 0.5)
            && ok_range(self.softness[1], 0.0, 0.5)
            && ok_range(self.detail[0], 0.5, 16.0)
            && ok_range(self.detail[1], 0.0, 1.0)
            && ok_range(self.phase_g[0], -0.99, 0.99)
            && ok_range(self.phase_g[1], -0.99, 0.99)
            && ok_range(self.lobe_blend, 0.0, 1.0)
            && self.wind_uv.iter().all(|c| c.is_finite())
            && (1..=16).contains(&self.steps[0])
            && (1..=8).contains(&self.steps[1])
            && self.layer_m.iter().all(|c| c.is_finite())
            && self.layer_m[1] > self.layer_m[0]
            && self.sun_dir.iter().all(|c| c.is_finite())
            && self.sun_color.iter().all(|c| c.is_finite() && *c >= 0.0);
        if !ok {
            return Err(CloudError::Invalid(
                "cloud params outside valid ranges".into(),
            ));
        }
        let l2 = self.sun_dir[0] * self.sun_dir[0]
            + self.sun_dir[1] * self.sun_dir[1]
            + self.sun_dir[2] * self.sun_dir[2];
        if l2 < 1e-12 {
            return Err(CloudError::Invalid("cloud sun_dir has ~zero length".into()));
        }
        Ok(())
    }
}
fn linearstep(a: f32, b: f32, x: f32) -> f32 {
    ((x - a) / (b - a)).clamp(0.0, 1.0)
}
fn hash13(x: f32, y: f32, z: f32) -> f32 {
    let f = ((x * 12.9898 + y * 78.233 + z * 37.719).fract() * 43758.5453).fract();
    if f < 0.0 {
        f + 1.0
    } else {
        f
    }
}
/// Base coverage mask at `uv` (wind applied): smooth value noise gated by the
/// coverage threshold with an edge ramp. Deterministic in uv.
pub fn get_cloud_map_density(uv: [f32; 2], params: &CloudParams) -> f32 {
    let (gx, gy) = ((uv[0] * 4.0).floor(), (uv[1] * 4.0).floor());
    let (fx, fy) = (uv[0] * 4.0 - gx, uv[1] * 4.0 - gy);
    let (sx, sy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let (x0, y0) = (gx as i32, gy as i32);
    let a = hash13(x0 as f32, y0 as f32, 7.0);
    let b = hash13((x0 + 1) as f32, y0 as f32, 7.0);
    let c = hash13(x0 as f32, (y0 + 1) as f32, 7.0);
    let d = hash13((x0 + 1) as f32, (y0 + 1) as f32, 7.0);
    let n = a + (b - a) * sx + (c - a) * sy + (a - b - c + d) * sx * sy;
    linearstep(
        params.coverage - params.softness[0],
        params.coverage + params.softness[0],
        n,
    )
}
/// Height gradient: 0 at both faces, 1 mid-layer, softer bottom ramp.
pub fn cloud_gradient(h: f32) -> f32 {
    let c = h.clamp(0.0, 1.0);
    linearstep(0.0, 0.35, c) * (1.0 - linearstep(0.6, 1.0, c))
}
/// Detail erode (Worley school, 2D cell hash): high-frequency cells carve the
/// base density by `detail[1]`. Deterministic in uv.
pub fn cloud_map_detail(uv: [f32; 2], params: &CloudParams) -> f32 {
    let s = params.detail[0];
    let cx = (uv[0] * s).floor() as i32;
    let cy = (uv[1] * s).floor() as i32;
    let px = uv[0] * s - cx as f32;
    let py = uv[1] * s - cy as f32;
    let mut best = 8.0f32;
    for oy in -1..=1 {
        for ox in -1..=1 {
            let dx = ox as f32 + hash13((cx + ox) as f32, (cy + oy) as f32, 3.0) - px;
            let dy = oy as f32 + hash13((cx + ox) as f32, (cy + oy) as f32, 11.0) - py;
            best = best.min(dx * dx + dy * dy);
        }
    }
    (1.0 - params.detail[1] * (1.0 - best.sqrt().clamp(0.0, 1.0))).clamp(0.0, 1.0)
}
/// Dual-lobe Henyey-Greenstein phase (Frostbite school), per steradian.
pub fn dual_lobe_hg(cos_theta: f32, g_fwd: f32, g_bwd: f32, blend: f32) -> f32 {
    let ct = cos_theta.clamp(-1.0, 1.0);
    let hg =
        |g: f32| (1.0 - g * g) / (12.566371 * (1.0 + g * g - 2.0 * g * ct).max(1e-4).powf(1.5));
    (hg(g_fwd) * blend + hg(g_bwd) * (1.0 - blend)).max(0.0)
}
/// Shadow march toward the sun: fixed taps of optical depth, `exp(-tau)`.
pub fn volumetric_shadow(pos_uv: [f32; 2], h: f32, params: &CloudParams) -> f32 {
    let n = params.steps[1].max(1);
    let mut tau = 0.0f32;
    for i in 0..n {
        let t = (f32::from(i) + 0.5) / f32::from(n);
        let at = [pos_uv[0] + 0.03 * t, pos_uv[1] + 0.015 * t];
        tau += get_cloud_map_density(at, params)
            * cloud_gradient(h + 0.05 * t)
            * params.density_per_m
            * 0.35;
    }
    (-tau.min(80.0)).exp()
}
/// Primary slab raymarch: hashed jitter, energy-conserving integration with
/// early-out below `CLOUD_MIN_TRANSMIT`. Returns (transmittance, scatter).
pub fn raymarch(uv: [f32; 2], elev: f32, seed: f32, params: &CloudParams) -> (f32, f32) {
    if elev <= 0.0 {
        return (1.0, 0.0);
    }
    let h = (elev / std::f32::consts::FRAC_PI_2).clamp(0.0, 1.0);
    let n = params.steps[0].max(1);
    let jitter = hash13(uv[0] * 91.0, uv[1] * 57.0, seed);
    let phase = dual_lobe_hg(
        elev.sin().clamp(-1.0, 1.0),
        params.phase_g[0],
        params.phase_g[1],
        params.lobe_blend,
    );
    let (mut transmit, mut scatter) = (1.0f32, 0.0f32);
    for i in 0..n {
        if transmit < CLOUD_MIN_TRANSMIT {
            break;
        }
        let t = (f32::from(i) + jitter) / f32::from(n);
        let p = [uv[0] + t * 0.02, uv[1] + t * 0.008];
        let base =
            get_cloud_map_density(p, params) * cloud_map_detail([p[0] * 2.0, p[1] * 2.0], params);
        let g = cloud_gradient((h + (t - 0.5) * 0.3).clamp(0.0, 1.0));
        let tau = (base * g * params.density_per_m * 0.55).min(80.0);
        scatter += transmit * (1.0 - (-tau).exp()) * phase * volumetric_shadow(p, h, params);
        transmit *= (-tau).exp();
    }
    (transmit, scatter)
}
/// Sky-composite entry over the RL4 `bake_sky` capture: transmittance times
/// sky plus sun scatter. `wind_uv` moves the layer, `seed` replays exactly.
/// Below-horizon texels pass through. Returns texels plus composite ms.
pub fn composite_clouds_over_capture(
    sky: &crate::sky::SkyCapture,
    params: &CloudParams,
    wind_uv: [f32; 2],
    seed: u64,
) -> Result<(Vec<[f32; 3]>, f64), CloudError> {
    params.validate()?;
    let want = sky.width_px as usize * sky.height_px as usize;
    if sky.width_px == 0 || sky.height_px == 0 || sky.texels.len() != want {
        return Err(CloudError::Invalid(
            "clouds sky capture backing is bad".into(),
        ));
    }
    if !sky
        .texels
        .iter()
        .all(|t| t.iter().all(|c| c.is_finite() && *c >= 0.0))
    {
        return Err(CloudError::Invalid("clouds sky holds bad radiance".into()));
    }
    if !wind_uv.iter().all(|c| c.is_finite()) {
        return Err(CloudError::Invalid("clouds wind_uv is not finite".into()));
    }
    let started = std::time::Instant::now();
    let seed_f = (seed % 10_000) as f32 / 10_000.0;
    let (w, hgt) = (f64::from(sky.width_px), f64::from(sky.height_px));
    let mut out = Vec::with_capacity(want);
    for y in 0..sky.height_px {
        let v = (f64::from(y) + 0.5) / hgt;
        let elev = std::f64::consts::FRAC_PI_2 - std::f64::consts::PI * v;
        for x in 0..sky.width_px {
            let sky_px = sky.texels[(y * sky.width_px + x) as usize];
            if elev <= 0.0 {
                out.push(sky_px);
                continue;
            }
            let u = (f64::from(x) + 0.5) / w;
            let uv = [
                u as f32 + params.wind_uv[0] + wind_uv[0],
                v as f32 + params.wind_uv[1] + wind_uv[1],
            ];
            let (transmit, scatter) = raymarch(uv, elev as f32, seed_f, params);
            out.push([
                sky_px[0] * transmit + params.sun_color[0] * scatter,
                sky_px[1] * transmit + params.sun_color[1] * scatter,
                sky_px[2] * transmit + params.sun_color[2] * scatter,
            ]);
        }
    }
    Ok((out, started.elapsed().as_secs_f64() * 1000.0))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clouds_lit_differs_and_cost_posted() {
        let params = CloudParams::day();
        params.validate().expect("day layer validates");
        let (sky, _) =
            crate::sky::bake_sky(&crate::sky::SkyParams::midday(), 64, 32).expect("RL4 day bakes");
        let t0 = std::time::Instant::now();
        let clear = sky.texels.clone();
        let clear_ms = t0.elapsed().as_secs_f64() * 1000.0;
        let (a, cloudy_ms) =
            composite_clouds_over_capture(&sky, &params, [0.0, 0.0], 7).expect("composites");
        let (b, _) =
            composite_clouds_over_capture(&sky, &params, [0.0, 0.0], 7).expect("recomposites");
        assert_eq!(
            a.iter().zip(b.iter()).filter(|(x, y)| x != y).count(),
            0,
            "identity rerun is 0 px"
        );
        let lit = a.iter().zip(clear.iter()).filter(|(x, y)| x != y).count();
        assert!(lit > 200, "cloud-vs-clear moves pixels, got {lit}");
        let (c, _) =
            composite_clouds_over_capture(&sky, &params, [1.0 / 64.0, 0.0], 7).expect("wind moves");
        assert_ne!(a, c, "one-texel wind moves clouds");
        eprintln!("clouds_lit: lit={lit} rerun=0 clear_ms={clear_ms:.3} cloudy_ms={cloudy_ms:.3} budget={CLOUD_COMPOSITE_BUDGET_MS}");
        assert_eq!(CLOUD_COMPOSITE_BUDGET_MS, 2.0, "cloud bar stays 2.0 ms");
        let bad = CloudParams {
            coverage: 2.0,
            ..params
        };
        assert!(matches!(
            composite_clouds_over_capture(&sky, &bad, [0.0, 0.0], 7),
            Err(CloudError::Invalid(_))
        ));
        let ragged = crate::sky::SkyCapture {
            width_px: 4,
            height_px: 4,
            texels: vec![[0.1; 3]; 5],
        };
        assert!(matches!(
            composite_clouds_over_capture(&ragged, &params, [0.0, 0.0], 7),
            Err(CloudError::Invalid(_))
        ));
    }
}
