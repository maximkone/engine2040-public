//! upscale_easu: pinned 720p to 1080p spatial upscale (FC3 perf path).
//!
//! Owner: render lane; consumer `super::spatial_720p_to_1080p` (Caller:
//! `crates/render/src/post.rs`). Units: pixels for sizes, ms in receipt.
//! Determinism: integer base math, fixed 12-tap order, same inputs give bit-identical
//! pixels; wall clock feeds the receipt only. Errors reuse `super::PostError`.
//!
//! Edge-directed 12-tap lobe (4x3 Lanczos-2 window in a local gradient frame)
//! with gradient reject plus a 2x2 clamp; caller runs RCAS finish.
//! PORT(pmndrs/upscaler@96184a6812bc5fd5e5fd7898257788a1663105a9:src/shaders/easu.ts):
//! edge-directed 12-tap shape plus per-texel gradient reject; reimplemented, zero donor bytes.

use super::{fsr_lanczos2, fsr_texel_clamped, luminance, PostError};
use std::time::Instant;

/// Pinned source width: 1280 (720p internal).
pub const SRC_W: u32 = 1280;
/// Pinned source height: 720.
pub const SRC_H: u32 = 720;
/// Pinned dest width: 1920 (1080p output).
pub const DST_W: u32 = 1920;
/// Pinned dest height: 1080.
pub const DST_H: u32 = 1080;
const REJECT_GRAD: f32 = 0.05; // gradient above which across-edge taps reject
const REJECT_LUMA: f32 = 0.35; // tap luma distance above which a tap rejects

/// Upscale receipt: wall-clock ms (evidence only, never hashed).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct UpscaleReceipt(pub f64);

fn f32_from_int(v: i64) -> Result<f32, PostError> {
    i16::try_from(v)
        .map(f32::from)
        .map_err(|_| PostError::Invalid("upscale int out of range".into()))
}

fn easu_pixel(src: &[[f32; 3]], x: u32, y: u32) -> Result<[f32; 3], PostError> {
    // Integer base for the pinned 2/3 scale (FSR-path derivation, no cast).
    let x64 = i64::from(x);
    let y64 = i64::from(y);
    let dw64 = i64::from(DST_W);
    let dh64 = i64::from(DST_H);
    let ix = ((2 * x64 + 1) * i64::from(SRC_W) - i64::from(DST_W)).div_euclid(2 * dw64);
    let iy = ((2 * y64 + 1) * i64::from(SRC_H) - i64::from(DST_H)).div_euclid(2 * dh64);
    // Local gradient frame at the base texel; flat fields stay isotropic.
    let gx = luminance(fsr_texel_clamped(src, SRC_W, SRC_H, ix + 1, iy)?)
        - luminance(fsr_texel_clamped(src, SRC_W, SRC_H, ix - 1, iy)?);
    let gy = luminance(fsr_texel_clamped(src, SRC_W, SRC_H, ix, iy + 1)?)
        - luminance(fsr_texel_clamped(src, SRC_W, SRC_H, ix, iy - 1)?);
    let len = (gx * gx + gy * gy).sqrt();
    let (nx, ny, stretch) = if !len.is_finite() || len < 1e-6 {
        (1.0, 0.0, 1.0)
    } else {
        (gx / len, gy / len, 1.0 + (len * 2.0).clamp(0.0, 1.0))
    };
    let center = luminance(fsr_texel_clamped(src, SRC_W, SRC_H, ix, iy)?);
    let sx = f32_from_int(i64::from(SRC_W))? / f32_from_int(i64::from(DST_W))?;
    let sy = f32_from_int(i64::from(SRC_H))? / f32_from_int(i64::from(DST_H))?;
    let fx = (f32_from_int(x64)? + 0.5) * sx - 0.5;
    let fy = (f32_from_int(y64)? + 0.5) * sy - 0.5;
    let ix_f = f32_from_int(ix)?;
    let iy_f = f32_from_int(iy)?;
    let mut acc = [0.0f32; 3];
    let mut wsum = 0.0f32;
    for dy in [-1i64, 0, 1] {
        for dx in [-1i64, 0, 1, 2] {
            let tap = fsr_texel_clamped(src, SRC_W, SRC_H, ix + dx, iy + dy)?;
            if len > REJECT_GRAD && (luminance(tap) - center).abs() > REJECT_LUMA {
                continue;
            }
            let ox = ix_f + f32_from_int(dx)? - fx;
            let oy = iy_f + f32_from_int(dy)? - fy;
            let wgt = fsr_lanczos2((ox * -ny + oy * nx) / stretch)
                * fsr_lanczos2((ox * nx + oy * ny) * stretch);
            if wgt.is_finite() {
                acc[0] += tap[0] * wgt;
                acc[1] += tap[1] * wgt;
                acc[2] += tap[2] * wgt;
                wsum += wgt;
            }
        }
    }
    if !wsum.is_finite() || wsum <= 0.0 {
        return Err(PostError::Invalid("upscale weights collapsed".into()));
    }
    // 2x2 clamp kills ringing; flat fields clamp to the exact value.
    let mut pix = [acc[0] / wsum, acc[1] / wsum, acc[2] / wsum];
    let c00 = fsr_texel_clamped(src, SRC_W, SRC_H, ix, iy)?;
    let c10 = fsr_texel_clamped(src, SRC_W, SRC_H, ix + 1, iy)?;
    let c01 = fsr_texel_clamped(src, SRC_W, SRC_H, ix, iy + 1)?;
    let c11 = fsr_texel_clamped(src, SRC_W, SRC_H, ix + 1, iy + 1)?;
    for c in 0..3 {
        let lo = c00[c].min(c10[c]).min(c01[c]).min(c11[c]);
        let hi = c00[c].max(c10[c]).max(c01[c]).max(c11[c]);
        pix[c] = pix[c].clamp(lo, hi);
    }
    Ok(pix)
}

/// Pinned 720p to 1080p spatial upscale: validates size plus finite texels;
/// edge-directed 12-tap lobe per output texel. Flat fields stay bit-exact.
pub fn upscale_720p_to_1080p(
    src: &[[f32; 3]],
) -> Result<(Vec<[f32; 3]>, UpscaleReceipt), PostError> {
    let sw = usize::try_from(SRC_W).map_err(|_| PostError::Invalid("upscale width".into()))?;
    let sh = usize::try_from(SRC_H).map_err(|_| PostError::Invalid("upscale height".into()))?;
    let want = sw
        .checked_mul(sh)
        .ok_or_else(|| PostError::Invalid("upscale size".into()))?;
    if src.len() != want {
        return Err(PostError::Invalid(format!(
            "upscale src pixels {} differ from {SRC_W}x{SRC_H} ({want})",
            src.len()
        )));
    }
    for (i, p) in src.iter().enumerate() {
        if !p.iter().all(|c| c.is_finite()) {
            return Err(PostError::Invalid(format!(
                "upscale src texel {i} is not finite"
            )));
        }
    }
    let dw = usize::try_from(DST_W).map_err(|_| PostError::Invalid("upscale width".into()))?;
    let dh = usize::try_from(DST_H).map_err(|_| PostError::Invalid("upscale height".into()))?;
    let cap = dw
        .checked_mul(dh)
        .ok_or_else(|| PostError::Invalid("upscale size".into()))?;
    let started = Instant::now();
    let mut out = Vec::with_capacity(cap);
    for y in 0..DST_H {
        for x in 0..DST_W {
            out.push(easu_pixel(src, x, y)?);
        }
    }
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok((out, UpscaleReceipt(ms)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upscale_720p_to_1080p_matches_dims_and_rerun_identical() {
        let flat = vec![[0.4, 0.4, 0.4]; 1280 * 720];
        let (out, receipt) = upscale_720p_to_1080p(&flat).expect("720p upscales");
        assert_eq!(out.len(), 1920 * 1080, "output dims match 1080p");
        assert!(
            out.iter().all(|p| *p == [0.4, 0.4, 0.4]),
            "flat field bit-exact"
        );
        let (again, _) = upscale_720p_to_1080p(&flat).expect("720p reruns");
        assert_eq!(out, again, "rerun pixels identical");
        assert!(receipt.0.is_finite(), "receipt posts ms");
    }

    #[test]
    fn upscale_720p_rejects_bad_inputs() {
        assert!(upscale_720p_to_1080p(&[[0.4, 0.4, 0.4]; 10]).is_err());
        let mut bad = vec![[0.4, 0.4, 0.4]; 1280 * 720];
        bad[7] = [f32::NAN, 0.0, 0.0];
        assert!(upscale_720p_to_1080p(&bad).is_err());
    }
}
