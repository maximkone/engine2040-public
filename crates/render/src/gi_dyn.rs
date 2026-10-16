//! gi_dyn: SDFGI-cascade CPU reference tables (spike, flag-gated, never default-on).
//!
//! Shape idea only from Godot SDFGI cascades (MIT, Godot Engine; no donor
//! code copied, analytic SDF + trilinear tables reimplemented). Owner is the
//! render lane; the consumer is the half-res [`crate::gtao::LookFrame`] trace
//! (this spike's test plus a future window submit). Units: meters for SDF,
//! pixels for sizes, ms in receipts, percent for leak. Determinism: fixed
//! analytic scene, row-major orders, `sqrt` only, no wall clock in state
//! (one `Instant` around the measured loop for the receipt only). Errors are
//! typed [`GiDynError`]; the disabled flag refuses loudly.
//!
//! UNLOCKS: E38 (window forward submit of these reference tables; caller is
//! the future `window`/`present` frame submit, which calls
//! [`SdfCascade::trace_half_res`] with `enabled = true` into a caller-owned
//! half-res buffer, then upscales beside the GTAO path).
//!
//! Spike only: production stays baked (`gi.rs` lookup-only); this module runs
//! only when the caller passes `enabled = true` (default
//! [`GI_DYN_ENABLED_DEFAULT`] is `false`).

#![forbid(unsafe_code)]

use std::fmt;
use std::time::Instant;

use crate::gtao::LookFrame;

/// Default gate: the dynamic path is never on unless the caller opts in.
pub const GI_DYN_ENABLED_DEFAULT: bool = false;
/// Posted half-res budget in ms (E38b: at most 1.2ms half-res).
pub const GI_DYN_BUDGET_MS: f64 = 1.2;
/// Posted leak ceiling in percent (E38b: leak under 5pct).
pub const GI_DYN_LEAK_MAX_PCT: f64 = 5.0;
/// Cascade nodes on x (covers 8m at 0.25m steps).
pub const CASCADE_NX: usize = 32;
/// Cascade nodes on y (covers 4m at 0.25m steps).
pub const CASCADE_NY: usize = 16;
/// Cascade nodes on z (covers 8m at 0.25m steps).
pub const CASCADE_NZ: usize = 32;
/// Cascade origin in meters (probe 0,0,0 corner).
pub const CASCADE_MIN: [f32; 3] = [-4.0, 0.0, -4.0];
/// Cascade node spacing in meters.
pub const CASCADE_STEP: [f32; 3] = [0.25, 0.25, 0.25];
/// Conservative bias in meters (quarter diagonal): erodes free space so thin
/// walls never leak through the trilinear table.
pub const CASCADE_BIAS: f32 = 0.06;
/// Probe height in meters for the half-res trace plane.
pub const TRACE_HEIGHT_M: f32 = 1.0;

/// Dynamic-GI spike failures: disabled flag vs bad input.
#[derive(Clone, PartialEq, Debug)]
pub enum GiDynError {
    /// Caller did not opt in; pass `enabled = true` for the spike.
    Disabled,
    /// Caller input is unusable; reason carried.
    Invalid(String),
}

impl fmt::Display for GiDynError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disabled => write!(f, "gi_dyn is disabled (spike flag off)"),
            Self::Invalid(e) => write!(f, "gi_dyn input: {e}"),
        }
    }
}

impl std::error::Error for GiDynError {}

/// Named index/grid conversions: the only conversion sites in this module
/// (call sites use these, never bare `as`, per docs/RUST-QUALITY.md).
/// Table sizes stay far below 2^16 so index-to-`f32` is exact; sample
/// scalars are clamped to `[0, n-1]` before flooring so they fit; texel
/// counts stay far below 2^32 so count-to-`f64` is exact. Fallbacks saturate
/// at the max (never wrap, never panic).
/// Grid index to `f32` scalar (exact for table sizes).
fn idx_f32(i: usize) -> f32 {
    match u16::try_from(i) {
        Ok(v) => f32::from(v),
        Err(_) => f32::from(u16::MAX),
    }
}

/// Floor a clamped grid scalar to a node index: the axis holds at most 32
/// nodes, so the bisection walk is exact and cheap (no float-to-int cast).
/// Returns the largest `k` in `[0, n-1]` with `idx_f32(k) <= v`.
fn floor_idx(v: f32, n: usize) -> usize {
    let mut lo = 0usize;
    let mut hi = n;
    while lo + 1 < hi {
        let mid = lo + (hi - lo) / 2;
        if idx_f32(mid) <= v {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

/// Texel count to `f64` percent math (exact for half-res counts).
fn count_f64(n: usize) -> f64 {
    match u32::try_from(n) {
        Ok(v) => f64::from(v),
        Err(_) => f64::from(u32::MAX),
    }
}

/// Baked SDF cascade: one conservative distance table over the arena slice.
pub struct SdfCascade {
    /// Nodes on x.
    pub nx: usize,
    /// Nodes on y.
    pub ny: usize,
    /// Nodes on z.
    pub nz: usize,
    /// Grid origin in meters.
    pub min: [f32; 3],
    /// Node spacing in meters.
    pub step: [f32; 3],
    /// Signed distances in `(iz*ny+iy)*nx+ix` order, meters.
    pub sdf: Vec<f32>,
}

/// Half-res trace receipt: leak percent plus the measured ms.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GiDynReceipt {
    /// Percent of oracle-inside texels the table calls outside (0..100).
    pub leak_pct: f64,
    /// Measured trace ms on this host (receipt only, never hashed).
    pub ms: f64,
    /// Half-res texels traced.
    pub texels: usize,
    /// Oracle-inside texels (leak denominator).
    pub occluded: usize,
}

/// Box SDF (analytic scene piece, meters).
fn sd_box(p: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> f32 {
    let cx = (lo[0] + hi[0]) * 0.5;
    let cy = (lo[1] + hi[1]) * 0.5;
    let cz = (lo[2] + hi[2]) * 0.5;
    let hx = (hi[0] - lo[0]) * 0.5;
    let hy = (hi[1] - lo[1]) * 0.5;
    let hz = (hi[2] - lo[2]) * 0.5;
    let dx = (p[0] - cx).abs() - hx;
    let dy = (p[1] - cy).abs() - hy;
    let dz = (p[2] - cz).abs() - hz;
    let ox = dx.max(0.0);
    let oy = dy.max(0.0);
    let oz = dz.max(0.0);
    (ox * ox + oy * oy + oz * oz).sqrt() + dx.max(dy.max(dz)).min(0.0)
}

/// Analytic oracle SDF in meters: ground plane plus one thick wall.
fn analytic_sdf(p: [f32; 3]) -> f32 {
    let plane = p[1];
    let wall = sd_box(p, [-4.0, 0.0, -0.5], [4.0, 3.0, 0.5]);
    plane.min(wall)
}

impl SdfCascade {
    /// Bake the fixed spike cascade (deterministic, row-major).
    pub fn bake_spike() -> Self {
        let (nx, ny, nz) = (CASCADE_NX, CASCADE_NY, CASCADE_NZ);
        let mut sdf = Vec::with_capacity(nx * ny * nz);
        for iz in 0..nz {
            for iy in 0..ny {
                for ix in 0..nx {
                    sdf.push(analytic_sdf([
                        CASCADE_MIN[0] + idx_f32(ix) * CASCADE_STEP[0],
                        CASCADE_MIN[1] + idx_f32(iy) * CASCADE_STEP[1],
                        CASCADE_MIN[2] + idx_f32(iz) * CASCADE_STEP[2],
                    ]));
                }
            }
        }
        Self {
            nx,
            ny,
            nz,
            min: CASCADE_MIN,
            step: CASCADE_STEP,
            sdf,
        }
    }

    /// Node count.
    pub fn node_count(&self) -> usize {
        self.sdf.len()
    }

    /// Trilinear SDF in meters at `pos` (clamped, never panics).
    pub fn sample(&self, pos: [f32; 3]) -> f32 {
        if self.sdf.is_empty() || self.nx == 0 || self.ny == 0 || self.nz == 0 {
            return 1.0;
        }
        let fx = ((pos[0] - self.min[0]) / self.step[0].max(1e-9)).clamp(0.0, idx_f32(self.nx - 1));
        let fy = ((pos[1] - self.min[1]) / self.step[1].max(1e-9)).clamp(0.0, idx_f32(self.ny - 1));
        let fz = ((pos[2] - self.min[2]) / self.step[2].max(1e-9)).clamp(0.0, idx_f32(self.nz - 1));
        let (x0, y0, z0) = (
            floor_idx(fx, self.nx),
            floor_idx(fy, self.ny),
            floor_idx(fz, self.nz),
        );
        let (x1, y1, z1) = (
            (x0 + 1).min(self.nx - 1),
            (y0 + 1).min(self.ny - 1),
            (z0 + 1).min(self.nz - 1),
        );
        let (tx, ty, tz) = (fx.fract(), fy.fract(), fz.fract());
        let at = |x: usize, y: usize, z: usize| self.sdf[(z * self.ny + y) * self.nx + x];
        let x00 = at(x0, y0, z0) + (at(x1, y0, z0) - at(x0, y0, z0)) * tx;
        let x10 = at(x0, y1, z0) + (at(x1, y1, z0) - at(x0, y1, z0)) * tx;
        let x01 = at(x0, y0, z1) + (at(x1, y0, z1) - at(x0, y0, z1)) * tx;
        let x11 = at(x0, y1, z1) + (at(x1, y1, z1) - at(x0, y1, z1)) * tx;
        let z0v = x00 + (x10 - x00) * ty;
        let z1v = x01 + (x11 - x01) * ty;
        z0v + (z1v - z0v) * tz
    }

    /// Trace the half-res texels of `frame` (spike only when `enabled`).
    ///
    /// Each half texel maps to the probe plane at [`TRACE_HEIGHT_M`]; `out`
    /// receives clamped occlusion (`0` = inside wall, `1` = free) row-major.
    /// Buffers are caller-owned and reused: no allocation here.
    pub fn trace_half_res(
        &self,
        frame: &LookFrame,
        enabled: bool,
        out: &mut [f32],
    ) -> Result<GiDynReceipt, GiDynError> {
        if !enabled {
            return Err(GiDynError::Disabled);
        }
        frame
            .validate()
            .map_err(|e| GiDynError::Invalid(e.to_string()))?;
        let half = frame.half_size();
        let w = usize::try_from(half[0])
            .map_err(|_| GiDynError::Invalid("gi_dyn half width over usize".into()))?;
        let h = usize::try_from(half[1])
            .map_err(|_| GiDynError::Invalid("gi_dyn half height over usize".into()))?;
        if w == 0 || h == 0 {
            return Err(GiDynError::Invalid("gi_dyn half size is zero".into()));
        }
        if out.len() < w * h {
            return Err(GiDynError::Invalid(format!(
                "gi_dyn out holds {} but half needs {}",
                out.len(),
                w * h
            )));
        }
        if self.sdf.is_empty() {
            return Err(GiDynError::Invalid("gi_dyn cascade is empty".into()));
        }
        let size_x = self.step[0] * idx_f32(self.nx - 1);
        let size_z = self.step[2] * idx_f32(self.nz - 1);
        let t0 = Instant::now();
        let mut inside = 0usize;
        let mut leaked = 0usize;
        for y in 0..h {
            for x in 0..w {
                let wx = self.min[0] + ((idx_f32(x) + 0.5) / idx_f32(w)) * size_x;
                let wz = self.min[2] + ((idx_f32(y) + 0.5) / idx_f32(h)) * size_z;
                let p = [wx, TRACE_HEIGHT_M, wz];
                let oracle = analytic_sdf(p);
                let table = self.sample(p) - CASCADE_BIAS;
                if oracle < -0.02 {
                    inside += 1;
                    if table > 0.0 {
                        leaked += 1;
                    }
                }
                let occ = (table / 1.0).clamp(0.0, 1.0);
                out[y * w + x] = occ;
            }
        }
        let ms = t0.elapsed().as_secs_f64() * 1000.0;
        let leak_pct = if inside == 0 {
            0.0
        } else {
            count_f64(leaked) * 100.0 / count_f64(inside)
        };
        Ok(GiDynReceipt {
            leak_pct,
            ms,
            texels: w * h,
            occluded: inside,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_192() -> LookFrame {
        LookFrame {
            frame_index: 0,
            viewport_px: [192, 108],
            exposure_ev100: 10.0,
            sun_dir: [0.3, -0.8, 0.2],
        }
    }

    #[test]
    fn gi_dyn_spike_leak_and_ms() {
        let cascade = SdfCascade::bake_spike();
        assert_eq!(cascade.node_count(), CASCADE_NX * CASCADE_NY * CASCADE_NZ);
        let frame = frame_192();
        let (w, h) = (96usize, 54usize);
        let mut out = vec![0.0f32; w * h];
        let receipt = cascade
            .trace_half_res(&frame, true, &mut out)
            .expect("spike trace runs when enabled");
        println!(
            "gi_dyn leak_pct={:.3} ms={:.4} texels={} occluded={}",
            receipt.leak_pct, receipt.ms, receipt.texels, receipt.occluded
        );
        assert!(
            receipt.occluded > 0,
            "fixture must hide texels behind the wall"
        );
        assert!(
            receipt.leak_pct < GI_DYN_LEAK_MAX_PCT,
            "leak {} >= {}",
            receipt.leak_pct,
            GI_DYN_LEAK_MAX_PCT
        );
        assert!(
            receipt.ms < GI_DYN_BUDGET_MS,
            "ms {} >= {}",
            receipt.ms,
            GI_DYN_BUDGET_MS
        );
        assert_eq!(out.len(), w * h);
    }

    #[test]
    fn gi_dyn_disabled_refuses() {
        let cascade = SdfCascade::bake_spike();
        let frame = frame_192();
        let mut out = vec![0.0f32; 96 * 54];
        let err = cascade
            .trace_half_res(&frame, GI_DYN_ENABLED_DEFAULT, &mut out)
            .expect_err("default flag stays off");
        assert_eq!(err, GiDynError::Disabled);
    }

    #[test]
    fn gi_dyn_bad_frame_refuses() {
        let cascade = SdfCascade::bake_spike();
        let bad = LookFrame {
            viewport_px: [0, 108],
            ..frame_192()
        };
        let mut out = vec![0.0f32; 96 * 54];
        let err = cascade
            .trace_half_res(&bad, true, &mut out)
            .expect_err("zero viewport must fail");
        assert!(matches!(err, GiDynError::Invalid(_)));
    }
}
