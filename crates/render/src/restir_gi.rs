//! restir_gi: half-res ReSTIR-GI reservoir oracle (CPU reference, flag-gated).
//! Owner render; Caller: spike test only (UNLOCKS RT6 denoise); shape only from kajiya@9fdec0f, zero donor bytes.
#![forbid(unsafe_code)]
use crate::rt_shadows::{sand_shadow_factor, unit_sun};
/// Default gate: never on unless the caller opts in (production stays baked).
pub const RESTIR_ENABLED_DEFAULT: bool = false;
/// Posted tier budget in ms (CPU-oracle ms is info-only for the device bar).
pub const RESTIR_BUDGET_MS: f64 = 2.5;
/// Leak ceiling in pct (tunnel corner cells must stay dark).
pub const RESTIR_LEAK_MAX_PCT: f64 = 5.0;
const GRID_W: usize = 64;
const GRID_H: usize = 64;
const TEXELS: usize = GRID_W * GRID_H;
const CACHE_CELLS: usize = 32;
const HEMI5: [[f32; 3]; 5] = [
    [0.0, 1.0, 0.0],
    [0.7, 0.7, 0.0],
    [-0.7, 0.7, 0.0],
    [0.0, 0.7, 0.7],
    [0.0, 0.7, -0.7],
];
/// Reservoir failures: disabled flag vs bad input.
#[derive(Clone, Debug)]
pub enum RestirError {
    Disabled,
    Invalid(String),
}
impl std::fmt::Display for RestirError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Disabled => f.write_str("restir_gi is disabled (spike flag off)"),
            Self::Invalid(e) => write!(f, "restir_gi input: {e}"),
        }
    }
}
impl std::error::Error for RestirError {}
/// Half-res sweep receipt: leak pct plus measured ms (ms never hashed).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct RestirReceipt {
    pub leak_pct: f64,
    pub ms: f64,
}
/// One-sample half-res reservoirs plus the recurrent cache (oracle state).
pub struct RestirGi {
    org: Vec<[f32; 3]>,
    rad: Vec<[f32; 3]>,
    nrm: Vec<[f32; 3]>,
    hit: Vec<f32>,
    cache: Vec<[f32; 3]>,
    draws: engine_core::CounterRng,
    step: u32,
}
impl RestirGi {
    /// Allocate the one grid plus cache (caller traces one sun step at a time).
    pub fn new() -> Self {
        Self {
            org: vec![[0.0; 3]; TEXELS],
            rad: vec![[0.0; 3]; TEXELS],
            nrm: vec![[0.0, 1.0, 0.0]; TEXELS],
            hit: vec![60.0; TEXELS],
            cache: vec![[0.2, 0.18, 0.16]; CACHE_CELLS],
            draws: engine_core::CounterRng::new(8256), // fixed draw seed: same sweep, same bytes
            step: 0,
        }
    }
    /// Deterministic little-endian bytes (same sweep gives identical bytes).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut o = Vec::with_capacity((TEXELS * 2 + CACHE_CELLS) * 12);
        let (a, b, d) = (&self.org, &self.rad, &self.cache);
        for t in a.iter().chain(b.iter()).chain(d.iter()) {
            for c in t {
                o.extend_from_slice(&c.to_le_bytes());
            }
        }
        o
    }
    /// Trace one sun step (spike only when `enabled`, zero bake calls).
    pub fn trace(
        &mut self,
        scene: &crate::gi::BakeScene<'_>,
        samples: &[crate::gi::SurfaceSample],
        sun_dir: [f32; 3],
        enabled: bool,
    ) -> Result<RestirReceipt, RestirError> {
        if !enabled {
            return Err(RestirError::Disabled);
        }
        let sun = unit_sun(sun_dir).map_err(|e| RestirError::Invalid(e.to_string()))?;
        if samples.len() != TEXELS {
            return Err(RestirError::Invalid("need 64x64 samples".into()));
        }
        let scol = scene.dirs.first().map_or([1.0, 0.95, 0.88], |d| d.color);
        let (g, k) = (scene.sky.ground, scene.sky.sky);
        let t0 = std::time::Instant::now();
        let mut ctr = u64::from(self.step).wrapping_mul(7919);
        let (mut inside, mut leaked): (f64, f64) = (0.0f64, 0.0f64);
        for i in 0..TEXELS {
            let s = samples[i];
            let occ = s.occlusion.clamp(0.0, 1.0);
            let h01 = (s.pos[1] / 2.0).clamp(0.0, 1.0);
            let wd = HEMI5[usize::try_from(self.draws.sample(ctr) % 5).unwrap_or(4)];
            ctr = ctr.wrapping_add(1);
            let sh = sand_shadow_factor(s.pos, sun_dir)
                .map_err(|e| RestirError::Invalid(e.to_string()))?;
            let ndl = s.normal[0] * sun[0] + s.normal[1] * sun[1] + s.normal[2] * sun[2];
            let direct = ndl.max(0.0) * sh;
            let cy = usize::from(s.pos[1] >= 1.0);
            let cell = ((i / GRID_W / 16) * 2 + cy) * 4 + (i % GRID_W) / 16;
            let w = 1.0f32
                / f32::from(u8::try_from(self.step.wrapping_add(1).min(16).max(1)).unwrap_or(16));
            for c in 0..3 {
                let a = (g[c] + (k[c] - g[c]) * h01) * 0.5 + self.cache[cell][c] * 0.5;
                let cand = s.albedo[c] * (scol[c] * direct + a) * occ;
                self.cache[cell][c] = self.cache[cell][c] * 0.85 + (scol[c] * direct + a) * 0.15;
                self.rad[i][c] = self.rad[i][c] + (cand - self.rad[i][c]) * w;
            }
            self.org[i] = s.pos;
            self.nrm[i] = wd;
            self.hit[i] = sh * 60.0 + (1.0 - sh) * 1.2;
            if i % GRID_W != 0 {
                let l = i - 1;
                let al = wd[0] * self.nrm[l][0] + wd[1] * self.nrm[l][1] + wd[2] * self.nrm[l][2];
                if al > 0.9 && (self.hit[i] - self.hit[l]).abs() < 0.5 {
                    let (nb, cur) = (self.rad[l], self.rad[i]);
                    for c in 0..3 {
                        self.rad[i][c] = (cur[c] + nb[c]) * 0.5;
                    }
                }
            }
            if occ < 0.5 {
                inside += 1.0f64;
                if (self.rad[i][0] + self.rad[i][1] + self.rad[i][2]) / 3.0 > 0.08 {
                    leaked += 1.0f64;
                }
            }
        }
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        self.step = self.step.wrapping_add(1);
        let leak_pct = leaked * 100.0f64 / inside.max(1.0f64);
        Ok(RestirReceipt { leak_pct, ms })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restir_sun_sweep_no_bake_updates_bounce_and_posts_ms() {
        let suns = [[-0.62, 0.55, 0.45], [0.1, 0.2, 0.7], [0.45, 0.1, 0.55]];
        let dirs = [crate::gi::DirLight {
            dir: suns[0],
            color: [1.0, 0.95, 0.88],
        }];
        let scene = crate::gi::BakeScene {
            dirs: &dirs,
            points: &[],
            sky: crate::gi::SkyAmbient {
                sky: [0.35, 0.45, 0.6],
                ground: [0.42, 0.32, 0.2],
            },
        };
        let mut samples = Vec::with_capacity(TEXELS);
        for i in 0..TEXELS {
            let (ix, iy) = (i % GRID_W, i / GRID_W);
            let corner = (ix < 8 || ix >= GRID_W - 8) && (iy < 8 || iy >= GRID_H - 8);
            let px = (crate::rt_shadows::idx_f32(ix) + 0.5) * 0.25 - 8.0;
            let pz = (crate::rt_shadows::idx_f32(iy) + 0.5) * 0.25 - 8.0;
            samples.push(crate::gi::SurfaceSample {
                pos: [px, 0.0, pz],
                normal: [0.0, 1.0, 0.0],
                albedo: [0.42, 0.32, 0.2],
                occlusion: if corner { 0.0 } else { 1.0 },
            });
        }
        let mut gi = RestirGi::new();
        let off = gi.trace(&scene, &samples, suns[0], RESTIR_ENABLED_DEFAULT);
        assert!(matches!(off, Err(RestirError::Disabled)));
        let mut first = Vec::new();
        for (kk, sun) in suns.iter().enumerate() {
            let swept = gi.trace(&scene, &samples, *sun, true);
            let r = swept.expect("enabled sweep traces");
            if kk == 0 {
                first = gi.rad.to_vec();
            }
            println!("restir sun {kk}: leak_pct={:.3} ms={:.4}", r.leak_pct, r.ms);
            assert!(r.ms < RESTIR_BUDGET_MS, "ms over budget");
            assert!(r.leak_pct < RESTIR_LEAK_MAX_PCT, "leak over ceiling");
        }
        let mut delta = 0.0f64;
        for (a, b) in first.iter().zip(gi.rad.iter()) {
            delta += f64::from((a[0] - b[0]).abs() + (a[1] - b[1]).abs() + (a[2] - b[2]).abs());
        }
        delta /= f64::from(u32::try_from(first.len()).unwrap_or(u32::MAX)) * 3.0;
        assert!(delta > 0.01, "sun sweep must move bounce, delta {delta}");
        let mut gi2 = RestirGi::new();
        for sun in suns {
            gi2.trace(&scene, &samples, sun, true)
                .expect("repeat sweep traces");
        }
        assert_eq!(gi.to_bytes(), gi2.to_bytes(), "bytes bit-identical x2");
    }
}
