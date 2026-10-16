//! gi_probe: dynamic GI probe stub over the arena slice (RL-RT slice 1).
//!
//! Owner: render lane. Caller: `crate::gi::GiRuntime::irradiance_mixed`,
//! which blends this live sample beside the baked SH lookup. Units: meters
//! for grid bounds, linear RGB triples. Determinism: fixed probe order
//! with `+ - * /` and `sqrt` only; same sun gives bit-identical probes
//! twice (pixels stay presentation, never the sim hash). Errors are typed
//! [`GiProbeError`]; the grid allocates once in `new` and `update` reuses
//! it, so per-frame work allocates nothing.
//! UNLOCKS: window forward submit of this grid (caller feeds `sample`
//! beside the baked `irradiance`; weight 0 keeps today's pixels exactly).

#![forbid(unsafe_code)]

use std::fmt;

use crate::rt_shadows::{
    idx_f32, sand_radiance, sand_shadow_factor, sand_texel_pos, unit_sun, RtShadowError,
    RT_AMBIENT_FLAT, RT_FRAME_H, RT_FRAME_TEXELS, RT_FRAME_W,
};

/// Probes on x over `-8..+8` m.
pub const PROBE_NX: usize = 4;
/// Probes on y over `0..2` m (ankle vs head height).
pub const PROBE_NY: usize = 2;
/// Probes on z over `-8..+8` m.
pub const PROBE_NZ: usize = 4;
/// Probe count: the one grid allocation holds 32 triples (384 bytes).
pub const PROBE_COUNT: usize = PROBE_NX * PROBE_NY * PROBE_NZ;
/// Grid minimum corner in meters.
pub const PROBE_MIN: [f32; 3] = [-8.0, 0.0, -8.0];
/// Grid maximum corner in meters.
pub const PROBE_MAX: [f32; 3] = [8.0, 2.0, 8.0];
/// Probe spacing in meters, matching `(MAX - MIN) / (N - 1)` per axis.
pub const PROBE_STEP: [f32; 3] = [16.0 / 3.0, 2.0, 16.0 / 3.0];
/// Sky share at ankle height: low probes still see sky past the fighters.
pub const PROBE_SKY_LO: f32 = 0.35;
/// Sky-share climb to head height: higher probes see more sky, less sand.
pub const PROBE_SKY_CLIMB: f32 = 0.55;
/// Direct-sun share of the unit sun's climb: the stub spreads one sun tap
/// per probe instead of tracing visibility (visibility rides RT shadows).
pub const PROBE_SUN_SHARE: f32 = 0.6;
/// Input color ceiling in linear units (HDR headroom, finite only).
pub const PROBE_COLOR_MAX: f32 = 8.0;

/// Dynamic-GI stub failures: bad sun, bad colors or bad buffers.
#[derive(Clone, PartialEq, Debug)]
pub enum GiProbeError {
    /// Input is unusable; reason carried.
    Invalid(String),
}

impl fmt::Display for GiProbeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "gi probe input: {e}"),
        }
    }
}

impl std::error::Error for GiProbeError {}

impl From<RtShadowError> for GiProbeError {
    fn from(e: RtShadowError) -> Self {
        Self::Invalid(e.to_string())
    }
}

fn invalid(what: &str) -> GiProbeError {
    GiProbeError::Invalid(what.into())
}

fn check_color(name: &str, v: [f32; 3]) -> Result<(), GiProbeError> {
    if !v.iter().all(|c| c.is_finite()) {
        return Err(invalid(&format!("gi probe {name} is not finite")));
    }
    if v.iter().any(|c| *c < 0.0 || *c > PROBE_COLOR_MAX) {
        return Err(invalid(&format!(
            "gi probe {name} leaves 0..={PROBE_COLOR_MAX}"
        )));
    }
    Ok(())
}

/// One dynamic probe grid: RGB irradiance triples in
/// `(iz*NY + iy)*NX + ix` order, refreshed per frame by [`update`].
/// `min`/`step` are fixed by [`new`]; only `probes` mutates per frame.
///
/// [`update`]: ProbeGrid::update
pub struct ProbeGrid {
    nx: usize,
    ny: usize,
    nz: usize,
    min: [f32; 3],
    step: [f32; 3],
    probes: Vec<[f32; 3]>,
}

impl ProbeGrid {
    /// Allocate the one grid, seeded with the flat ambient floor so the
    /// first frame is valid before any [`update`](ProbeGrid::update).
    pub fn new() -> Self {
        let mut probes = Vec::with_capacity(PROBE_COUNT);
        for _ in 0..PROBE_COUNT {
            probes.push(RT_AMBIENT_FLAT);
        }
        Self {
            nx: PROBE_NX,
            ny: PROBE_NY,
            nz: PROBE_NZ,
            min: PROBE_MIN,
            step: PROBE_STEP,
            probes,
        }
    }

    /// Probe count (always [`PROBE_COUNT`]).
    pub fn probe_count(&self) -> usize {
        self.probes.len()
    }

    /// Refresh every probe from the live rig: sky/ground mix by probe
    /// height plus one sun tap. Same sun gives bit-identical probes twice;
    /// writes in place, so the buffer and capacity never move per frame.
    pub fn update(
        &mut self,
        sun_dir: [f32; 3],
        sun_color: [f32; 3],
        sky_color: [f32; 3],
        ground_color: [f32; 3],
    ) -> Result<(), GiProbeError> {
        let sun = unit_sun(sun_dir)?;
        check_color("sun_color", sun_color)?;
        check_color("sky_color", sky_color)?;
        check_color("ground_color", ground_color)?;
        if self.probes.len() != PROBE_COUNT {
            return Err(invalid("gi probe grid lost its 32 triples"));
        }
        let sun_w = sun[1].max(0.0) * PROBE_SUN_SHARE;
        for iz in 0..self.nz {
            for iy in 0..self.ny {
                // Two heights only, so the share is exact at both floors.
                let h01 = idx_f32(iy) / idx_f32(self.ny - 1);
                let sky_w = PROBE_SKY_LO + PROBE_SKY_CLIMB * h01;
                let gnd_w = 1.0 - sky_w;
                for ix in 0..self.nx {
                    self.probes[(iz * self.ny + iy) * self.nx + ix] = [
                        sky_color[0] * sky_w + ground_color[0] * gnd_w + sun_color[0] * sun_w,
                        sky_color[1] * sky_w + ground_color[1] * gnd_w + sun_color[1] * sun_w,
                        sky_color[2] * sky_w + ground_color[2] * gnd_w + sun_color[2] * sun_w,
                    ];
                }
            }
        }
        Ok(())
    }

    /// Trilinear irradiance at `pos` in meters (clamped into the grid, never
    /// panics). Sampling an exact node returns its triple bit-identically.
    pub fn sample_trilerp(&self, pos: [f32; 3]) -> Result<[f32; 3], GiProbeError> {
        if !pos.iter().all(|c| c.is_finite()) {
            return Err(invalid("gi probe sample pos is not finite"));
        }
        if self.probes.len() != PROBE_COUNT {
            return Err(invalid("gi probe grid lost its 32 triples"));
        }
        let dims = [self.nx, self.ny, self.nz];
        let mut ks = [0usize; 3];
        let mut k1s = [0usize; 3];
        let mut ts = [0.0f32; 3];
        for a in 0..3 {
            let f = ((pos[a] - self.min[a]) / self.step[a]).clamp(0.0, idx_f32(dims[a] - 1));
            let mut k = 0usize;
            while k + 1 < dims[a] && idx_f32(k + 1) <= f {
                k += 1;
            }
            k1s[a] = if k + 1 < dims[a] { k + 1 } else { k };
            ks[a] = k;
            ts[a] = (f - idx_f32(k)).clamp(0.0, 1.0);
        }
        let at = |x: usize, y: usize, z: usize| self.probes[(z * self.ny + y) * self.nx + x];
        let mut o = [0.0f32; 3];
        for c in 0..3 {
            let x00 = at(ks[0], ks[1], ks[2])[c]
                + (at(k1s[0], ks[1], ks[2])[c] - at(ks[0], ks[1], ks[2])[c]) * ts[0];
            let x10 = at(ks[0], k1s[1], ks[2])[c]
                + (at(k1s[0], k1s[1], ks[2])[c] - at(ks[0], k1s[1], ks[2])[c]) * ts[0];
            let x01 = at(ks[0], ks[1], k1s[2])[c]
                + (at(k1s[0], ks[1], k1s[2])[c] - at(ks[0], ks[1], k1s[2])[c]) * ts[0];
            let x11 = at(ks[0], k1s[1], k1s[2])[c]
                + (at(k1s[0], k1s[1], k1s[2])[c] - at(ks[0], k1s[1], k1s[2])[c]) * ts[0];
            let z0v = x00 + (x10 - x00) * ts[1];
            let z1v = x01 + (x11 - x01) * ts[1];
            o[c] = z0v + (z1v - z0v) * ts[2];
        }
        Ok(o)
    }
}

impl Default for ProbeGrid {
    fn default() -> Self {
        Self::new()
    }
}

/// Ambient pick: the flat floor when the stub stays off (today's pixels),
/// the live probe sample when on. The one branch the EG8 gate flips.
pub fn probe_ambient(
    grid: &ProbeGrid,
    point: [f32; 3],
    gi_on: bool,
) -> Result<[f32; 3], GiProbeError> {
    if !point.iter().all(|c| c.is_finite()) {
        return Err(invalid("gi probe point is not finite"));
    }
    if gi_on {
        grid.sample_trilerp(point)
    } else {
        Ok(RT_AMBIENT_FLAT)
    }
}

/// Render the GI frame: per texel a ray-traced shadow sample (forced lit
/// when `shadow_on` is false) plus the picked ambient through the shared
/// sand equation, so GI-off matches the RT frame triple for triple. `out`
/// is caller-owned with exactly `RT_FRAME_TEXELS` triples; returns the
/// texel count. Zero heap allocation on the success path.
pub fn render_gi_frame(
    grid: &ProbeGrid,
    sun_dir: [f32; 3],
    gi_on: bool,
    shadow_on: bool,
    out: &mut [[f32; 3]],
) -> Result<usize, GiProbeError> {
    if out.len() != RT_FRAME_TEXELS {
        return Err(invalid("gi probe frame buffer is not 32x32"));
    }
    unit_sun(sun_dir)?;
    for iy in 0..RT_FRAME_H {
        for ix in 0..RT_FRAME_W {
            let point = sand_texel_pos(ix, iy)?;
            let shadow = if shadow_on {
                sand_shadow_factor(point, sun_dir)?
            } else {
                1.0
            };
            let ambient = probe_ambient(grid, point, gi_on)?;
            out[iy * RT_FRAME_W + ix] = sand_radiance(sun_dir, shadow, ambient)?;
        }
    }
    Ok(RT_FRAME_TEXELS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena_day::ARENA_DAY_SUN_DIR;
    use crate::rt_shadows::{count_changed, render_sand_frame, RT_AMBIENT_FLAT, RT_EPSILON};

    const SUN_C: [f32; 3] = [1.0, 0.95, 0.88];
    const SKY_C: [f32; 3] = [0.35, 0.45, 0.60];
    const GND_C: [f32; 3] = [0.42, 0.32, 0.20];

    fn triples_equal(a: [f32; 3], b: [f32; 3]) -> bool {
        a[0].to_bits() == b[0].to_bits()
            && a[1].to_bits() == b[1].to_bits()
            && a[2].to_bits() == b[2].to_bits()
    }

    fn day_grid() -> ProbeGrid {
        let mut grid = ProbeGrid::new();
        grid.update(ARENA_DAY_SUN_DIR, SUN_C, SKY_C, GND_C)
            .expect("day rig updates the grid");
        grid
    }

    fn render_gi(grid: &ProbeGrid, gi_on: bool) -> Vec<[f32; 3]> {
        let mut out = vec![[0.0f32; 3]; RT_FRAME_TEXELS];
        render_gi_frame(grid, ARENA_DAY_SUN_DIR, gi_on, true, &mut out).expect("gi frame renders");
        out
    }

    #[test]
    fn gi_off_matches_rt_frame_bitwise() {
        let grid = ProbeGrid::new();
        let gi_off = render_gi(&grid, false);
        let mut rt = vec![[0.0f32; 3]; RT_FRAME_TEXELS];
        render_sand_frame(ARENA_DAY_SUN_DIR, true, RT_AMBIENT_FLAT, &mut rt)
            .expect("rt frame renders");
        assert_eq!(gi_off.len(), rt.len(), "same frame shape");
        assert!(
            gi_off
                .iter()
                .zip(rt.iter())
                .all(|(g, r)| triples_equal(*g, *r)),
            "GI-off runs the same equation, not a second truth"
        );
    }

    #[test]
    fn gi_on_changes_pixels() {
        let grid = day_grid();
        let on = render_gi(&grid, true);
        let off = render_gi(&grid, false);
        let changed = count_changed(&on, &off, RT_EPSILON).expect("gate runs");
        // The live floor lifts every texel far past epsilon: demand most of
        // the frame, so a dead update (flat grid) fails instead of passing.
        assert!(
            changed >= RT_FRAME_TEXELS * 3 / 4,
            "live probes move {changed} texels, want most of the frame"
        );
    }

    #[test]
    fn probe_follows_sun() {
        let mut grid = day_grid();
        let center = [0.0, 1.0, 0.0];
        let noon = grid.sample_trilerp(center).expect("noon samples");
        grid.update([0.2, 0.25, 0.9], SUN_C, SKY_C, GND_C)
            .expect("low sun updates the grid");
        let dusk = grid.sample_trilerp(center).expect("dusk samples");
        assert!(
            !triples_equal(noon, dusk),
            "the grid tracks the sun, it is not a bake"
        );
        assert!(
            dusk.iter().all(|c| c.is_finite() && *c >= 0.0),
            "low sun stays finite non-negative"
        );
        let mut again = ProbeGrid::new();
        again
            .update([0.2, 0.25, 0.9], SUN_C, SKY_C, GND_C)
            .expect("repeat updates the grid");
        let repeat = again.sample_trilerp(center).expect("repeat samples");
        assert!(
            triples_equal(dusk, repeat),
            "same sun gives bit-identical probes twice"
        );
    }

    #[test]
    fn grid_reuses_buffer() {
        let mut grid = ProbeGrid::new();
        assert_eq!(
            (grid.probe_count(), grid.probes.capacity()),
            (PROBE_COUNT, PROBE_COUNT),
            "one exact allocation at construction"
        );
        grid.update(ARENA_DAY_SUN_DIR, SUN_C, SKY_C, GND_C)
            .expect("first update runs");
        grid.update([0.2, 0.25, 0.9], SUN_C, SKY_C, GND_C)
            .expect("second update runs");
        assert_eq!(
            (grid.probe_count(), grid.probes.capacity()),
            (PROBE_COUNT, PROBE_COUNT),
            "per-frame updates never regrow the grid"
        );
    }

    #[test]
    fn rejects_bad_inputs() {
        let mut grid = ProbeGrid::new();
        assert!(
            grid.update([0.0, 0.0, 0.0], SUN_C, SKY_C, GND_C).is_err(),
            "zero sun fails"
        );
        assert!(
            grid.update(ARENA_DAY_SUN_DIR, [-1.0, 0.0, 0.0], SKY_C, GND_C)
                .is_err(),
            "negative sun color fails"
        );
        assert!(
            grid.update(ARENA_DAY_SUN_DIR, SUN_C, [99.0, 0.0, 0.0], GND_C)
                .is_err(),
            "HDR past the ceiling fails"
        );
        assert!(
            grid.sample_trilerp([f32::NAN, 0.0, 0.0]).is_err(),
            "NaN sample pos fails"
        );
        assert!(
            probe_ambient(&grid, [f32::INFINITY, 0.0, 0.0], true).is_err(),
            "infinite point fails the ambient pick"
        );
        let mut short = vec![[0.0f32; 3]; 8];
        assert!(
            render_gi_frame(&grid, ARENA_DAY_SUN_DIR, true, true, &mut short).is_err(),
            "short frame buffer fails"
        );
    }

    #[test]
    fn bridge_weight_zero_keeps_baked_pixels() {
        use crate::gi::{
            BakeScene, DirLight, Lightmap, ProbeGrid as BakedGrid, SkyAmbient, SurfaceSample,
        };
        let scene = BakeScene {
            dirs: &[DirLight {
                dir: [0.0, 1.0, 0.0],
                color: [1.0, 1.0, 1.0],
            }],
            points: &[],
            sky: SkyAmbient {
                sky: [0.4, 0.5, 0.7],
                ground: [0.2, 0.15, 0.1],
            },
        };
        let sample = SurfaceSample {
            pos: [0.0, 0.0, 0.0],
            normal: [0.0, 1.0, 0.0],
            albedo: [0.5, 0.5, 0.5],
            occlusion: 1.0,
        };
        let map = Lightmap::bake(1, 1, &[sample], &scene);
        let probes = BakedGrid::bake(1, 1, 1, [0.0, 0.0, 0.0], [1.0, 1.0, 1.0], &scene);
        let runtime = crate::gi::GiRuntime {
            map: &map,
            probes: &probes,
        };
        let grid = day_grid();
        let pos = [0.0, 1.0, 0.0];
        let dir = [0.0, 1.0, 0.0];
        let baked = runtime.irradiance(pos, dir);
        let kept = runtime
            .irradiance_mixed(&grid, pos, dir, 0.0)
            .expect("weight zero blends");
        assert!(
            triples_equal(baked, kept),
            "weight zero keeps today's baked pixels exactly"
        );
        let live = grid.sample_trilerp(pos).expect("probe samples");
        let full = runtime
            .irradiance_mixed(&grid, pos, dir, 1.0)
            .expect("weight one blends");
        assert!(
            triples_equal(live, full),
            "weight one hands the frame to the live grid"
        );
        assert!(
            runtime.irradiance_mixed(&grid, pos, dir, 2.0).is_err(),
            "weight past one fails loudly"
        );
    }
}
