use super::*;

// ---------------------------------------------------------------------------
// Temporal upsampling: engine-owned TAAU present path (no FSR code).
// ---------------------------------------------------------------------------

/// Engine-owned temporal upsample: clamp-to-edge bilinear from the render
/// size to the presentation size. Same-size input is the bit-exact
/// identity; solid colors survive unchanged. Same complete inputs give
/// bit-identical outputs.
pub fn taa_upsample(
    src: &[[f32; 3]],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Result<Vec<[f32; 3]>, PostError> {
    for (dim, name) in [
        (src_w, "src_w"),
        (src_h, "src_h"),
        (dst_w, "dst_w"),
        (dst_h, "dst_h"),
    ] {
        if dim == 0 {
            return Err(PostError::Invalid(format!("taau {name} is zero")));
        }
        if dim > MAX_POST_DIM {
            return Err(PostError::BudgetExceeded {
                requested: dim,
                allowed: MAX_POST_DIM,
            });
        }
    }
    if src.len() != src_w as usize * src_h as usize {
        return Err(PostError::Invalid(format!(
            "taau src pixels {} differ from {src_w}x{src_h}",
            src.len()
        )));
    }
    for (i, p) in src.iter().enumerate() {
        if !p.iter().all(|c| c.is_finite()) {
            return Err(PostError::Invalid(format!(
                "taau src texel {i} is not finite"
            )));
        }
    }
    let sx = src_w as f32 / dst_w as f32;
    let sy = src_h as f32 / dst_h as f32;
    let mut out = Vec::with_capacity(dst_w as usize * dst_h as usize);
    for y in 0..dst_h as usize {
        for x in 0..dst_w as usize {
            let fx = (x as f32 + 0.5) * sx - 0.5;
            let fy = (y as f32 + 0.5) * sy - 0.5;
            out.push(sample_bilinear(src, src_w, src_h, fx, fy));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Spatial upscale: FSR1-inspired EASU plus RCAS, engine-owned.
// ---------------------------------------------------------------------------

/// Lowest RCAS sharpness (the identity).
pub const FSR_SHARPEN_MIN: f32 = 0.0;
/// Highest RCAS sharpness.
pub const FSR_SHARPEN_MAX: f32 = 1.0;
/// Default RCAS sharpness for the combined path.
pub const FSR_DEFAULT_SHARPNESS: f32 = 0.4;
/// Posted shimmer gate for the spatial path (mirrors the fallback gate).
pub const FSR_SHIMMER_GATE: f32 = 0.06;
/// Smallest accepted MTF50 lift over bilinear at matched shimmer.
pub const FSR_MTF50_LIFT_MIN: f32 = 0.10;

/// Dynamic-resolution constants for the EASU pass (FSR1 `FsrEasuCon` shape).
///
/// PORT(GPUOpen-Effects/FidelityFX-FSR@a21ffb8:ffx-fsr/ffx_fsr1.h): MIT, shape
/// only reimplemented, zero HLSL bytes pasted. Own CPU plus WGSL, presentation
/// only, never in tick. The bilinear TAAU stays frozen as the default.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FsrEasuCon {
    /// Source width in pixels.
    pub src_w: u32,
    /// Source height in pixels.
    pub src_h: u32,
    /// Destination width in pixels.
    pub dst_w: u32,
    /// Destination height in pixels.
    pub dst_h: u32,
    /// Scale per axis (`src / dst`).
    pub scale: [f32; 2],
    /// Offset per axis (`0.5 * scale - 0.5`).
    pub offset: [f32; 2],
    /// Texel size per axis (`1 / src`).
    pub texel: [f32; 2],
}

// Small exact conversions without numeric casts (dims stay under u16).
pub(super) fn f32_from_dim(v: u32) -> Result<f32, PostError> {
    let small =
        u16::try_from(v).map_err(|_| PostError::Invalid("fsr dim does not fit u16".into()))?;
    Ok(f32::from(small))
}

pub(super) fn f32_from_tap(v: i64) -> Result<f32, PostError> {
    let small = i16::try_from(v).map_err(|_| PostError::Invalid("fsr tap out of range".into()))?;
    Ok(f32::from(small))
}

pub(super) fn usize_from_dim(v: u32) -> Result<usize, PostError> {
    usize::try_from(v).map_err(|_| PostError::Invalid("fsr dim does not fit usize".into()))
}

pub(super) fn fsr_capacity(w: u32, h: u32) -> Result<usize, PostError> {
    let total = u64::from(w) * u64::from(h);
    usize::try_from(total).map_err(|_| PostError::Invalid("fsr size does not fit".into()))
}

/// Build the EASU constants for a source to destination size pair.
pub fn fsr_easu_con(
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Result<FsrEasuCon, PostError> {
    check_dims(src_w, src_h)?;
    check_dims(dst_w, dst_h)?;
    let src_w_f = f32_from_dim(src_w)?;
    let src_h_f = f32_from_dim(src_h)?;
    let dst_w_f = f32_from_dim(dst_w)?;
    let dst_h_f = f32_from_dim(dst_h)?;
    let sx = src_w_f / dst_w_f;
    let sy = src_h_f / dst_h_f;
    Ok(FsrEasuCon {
        src_w,
        src_h,
        dst_w,
        dst_h,
        scale: [sx, sy],
        offset: [0.5 * sx - 0.5, 0.5 * sy - 0.5],
        texel: [1.0 / src_w_f, 1.0 / src_h_f],
    })
}

// Lanczos-2 kernel value at `x` (zero past `|x| >= 2`, one at zero).
pub(crate) fn fsr_lanczos2(x: f32) -> f32 {
    let ax = x.abs();
    if ax >= 2.0 {
        return 0.0;
    }
    if ax < 1e-6 {
        return 1.0;
    }
    let pi_ax = std::f32::consts::PI * ax;
    let sinc1 = pi_ax.sin() / pi_ax;
    let half = pi_ax * 0.5;
    sinc1 * (half.sin() / half)
}

pub(crate) fn fsr_texel_clamped(
    src: &[[f32; 3]],
    w: u32,
    h: u32,
    x: i64,
    y: i64,
) -> Result<[f32; 3], PostError> {
    let w64 = i64::from(w);
    let h64 = i64::from(h);
    let cx = x.clamp(0, w64 - 1);
    let cy = y.clamp(0, h64 - 1);
    let idx64 = cy * w64 + cx;
    let idx = usize::try_from(idx64)
        .map_err(|_| PostError::Invalid("fsr texel index out of range".into()))?;
    src.get(idx)
        .copied()
        .ok_or_else(|| PostError::Invalid("fsr texel index out of range".into()))
}

pub(super) fn fsr_luma_clamped(
    src: &[[f32; 3]],
    w: u32,
    h: u32,
    x: i64,
    y: i64,
) -> Result<f32, PostError> {
    Ok(luminance(fsr_texel_clamped(src, w, h, x, y)?))
}

// Edge frame at one source texel: gradient axes plus stretch.
pub(super) struct FsrDir {
    nx: f32,
    ny: f32,
    ex: f32,
    ey: f32,
    stretch: f32,
}

pub(super) fn fsr_dir_at(
    src: &[[f32; 3]],
    w: u32,
    h: u32,
    ix: i64,
    iy: i64,
) -> Result<FsrDir, PostError> {
    let one: i64 = 1;
    let l_px = fsr_luma_clamped(src, w, h, ix + one, iy)?;
    let l_nx = fsr_luma_clamped(src, w, h, ix - one, iy)?;
    let l_py = fsr_luma_clamped(src, w, h, ix, iy + one)?;
    let l_ny = fsr_luma_clamped(src, w, h, ix, iy - one)?;
    let l_pp = fsr_luma_clamped(src, w, h, ix + one, iy + one)?;
    let l_nn = fsr_luma_clamped(src, w, h, ix - one, iy - one)?;
    let l_pn = fsr_luma_clamped(src, w, h, ix + one, iy - one)?;
    let l_np = fsr_luma_clamped(src, w, h, ix - one, iy + one)?;
    let gx = (l_px - l_nx).abs();
    let gy = (l_py - l_ny).abs();
    let gd1 = (l_pp - l_nn).abs();
    let gd2 = (l_pn - l_np).abs();
    let gx_all = gx + (gd1 + gd2) * 0.5;
    let gy_all = gy + (gd1 + gd2) * 0.5;
    let len = (gx_all * gx_all + gy_all * gy_all).sqrt();
    if !len.is_finite() || len < 1e-6 {
        return Ok(FsrDir {
            nx: 1.0,
            ny: 0.0,
            ex: 0.0,
            ey: 1.0,
            stretch: 1.0,
        });
    }
    let nx = gx_all / len;
    let ny = gy_all / len;
    let aniso = (len * 2.0).clamp(0.0, 1.0);
    Ok(FsrDir {
        nx,
        ny,
        ex: -ny,
        ey: nx,
        stretch: 1.0 + aniso,
    })
}

pub(super) fn fsr_easu_pixel(
    src: &[[f32; 3]],
    w: u32,
    h: u32,
    con: &FsrEasuCon,
    x: u32,
    y: u32,
) -> Result<[f32; 3], PostError> {
    let x_f = f32_from_dim(x)?;
    let y_f = f32_from_dim(y)?;
    let fx = (x_f + 0.5) * con.scale[0] + con.offset[0];
    let fy = (y_f + 0.5) * con.scale[1] + con.offset[1];
    // Integer base with exact math (no float to int cast).
    let x64 = i64::from(x);
    let y64 = i64::from(y);
    let src_w64 = i64::from(con.src_w);
    let src_h64 = i64::from(con.src_h);
    let dst_w64 = i64::from(con.dst_w);
    let dst_h64 = i64::from(con.dst_h);
    let num_x = (2 * x64 + 1) * src_w64 - dst_w64;
    let num_y = (2 * y64 + 1) * src_h64 - dst_h64;
    let ix = num_x.div_euclid(2 * dst_w64);
    let iy = num_y.div_euclid(2 * dst_h64);
    let dir = fsr_dir_at(src, w, h, ix, iy)?;
    let ix_f = f32_from_tap(ix)?;
    let iy_f = f32_from_tap(iy)?;
    // 4x4 directional Lanczos-2 taps around the base.
    let mut acc = [0.0f32; 3];
    let mut wsum = 0.0f32;
    for dy in [-1i8, 0i8, 1i8, 2i8] {
        for dx in [-1i8, 0i8, 1i8, 2i8] {
            let tap = fsr_texel_clamped(src, w, h, ix + i64::from(dx), iy + i64::from(dy))?;
            let off_x = ix_f + f32::from(dx) - fx;
            let off_y = iy_f + f32::from(dy) - fy;
            let along = off_x * dir.ex + off_y * dir.ey;
            let across = off_x * dir.nx + off_y * dir.ny;
            let wgt = fsr_lanczos2(along / dir.stretch) * fsr_lanczos2(across * dir.stretch);
            if wgt.is_finite() {
                acc[0] += tap[0] * wgt;
                acc[1] += tap[1] * wgt;
                acc[2] += tap[2] * wgt;
                wsum += wgt;
            }
        }
    }
    if !wsum.is_finite() || wsum <= 0.0 {
        return Ok(sample_bilinear(src, w, h, fx, fy));
    }
    let mut pix = [acc[0] / wsum, acc[1] / wsum, acc[2] / wsum];
    // 2x2 min/max ringing clamp around the sample point.
    let one: i64 = 1;
    let c00 = fsr_texel_clamped(src, w, h, ix, iy)?;
    let c10 = fsr_texel_clamped(src, w, h, ix + one, iy)?;
    let c01 = fsr_texel_clamped(src, w, h, ix, iy + one)?;
    let c11 = fsr_texel_clamped(src, w, h, ix + one, iy + one)?;
    for c in 0..3 {
        let lo = c00[c].min(c10[c]).min(c01[c]).min(c11[c]);
        let hi = c00[c].max(c10[c]).max(c01[c]).max(c11[c]);
        pix[c] = pix[c].clamp(lo, hi);
    }
    Ok(pix)
}

/// Spatial upscale with directional Lanczos-2 taps plus a 2x2 ringing clamp
/// (EASU shape). Same complete inputs give bit-identical outputs. The frozen
/// bilinear TAAU stays the default; this path sits beside it.
pub fn fsr_easu_upsample(
    src: &[[f32; 3]],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Result<Vec<[f32; 3]>, PostError> {
    check_image(src, src_w, src_h, "fsr easu src")?;
    check_dims(dst_w, dst_h)?;
    let con = fsr_easu_con(src_w, src_h, dst_w, dst_h)?;
    let mut out = Vec::with_capacity(fsr_capacity(dst_w, dst_h)?);
    for y in 0..dst_h {
        for x in 0..dst_w {
            out.push(fsr_easu_pixel(src, src_w, src_h, &con, x, y)?);
        }
    }
    Ok(out)
}

/// Contrast-adaptive sharpen with 4-neighbour lobe limiters (RCAS shape).
///
/// `sharpness` rides `0..=1`; 0 is the identity. Output clamps per channel
/// to the neighbour min/max, so no new halo escapes the local lobe. Same
/// complete inputs give bit-identical outputs.
pub fn fsr_rcas_sharpen(
    img: &[[f32; 3]],
    w: u32,
    h: u32,
    sharpness: f32,
) -> Result<Vec<[f32; 3]>, PostError> {
    if !sharpness.is_finite() || !(FSR_SHARPEN_MIN..=FSR_SHARPEN_MAX).contains(&sharpness) {
        return Err(PostError::Invalid("fsr sharpness outside 0..=1".into()));
    }
    check_image(img, w, h, "fsr rcas src")?;
    if sharpness <= 0.0 {
        return Ok(img.to_vec());
    }
    let mut out = Vec::with_capacity(fsr_capacity(w, h)?);
    let one: i64 = 1;
    for y in 0..h {
        for x in 0..w {
            let x64 = i64::from(x);
            let y64 = i64::from(y);
            let e = fsr_texel_clamped(img, w, h, x64, y64)?;
            let b = fsr_texel_clamped(img, w, h, x64, y64 - one)?;
            let d = fsr_texel_clamped(img, w, h, x64 - one, y64)?;
            let f = fsr_texel_clamped(img, w, h, x64 + one, y64)?;
            let hh = fsr_texel_clamped(img, w, h, x64, y64 + one)?;
            let mut pix = [0.0f32; 3];
            for c in 0..3 {
                let mn = b[c].min(d[c]).min(f[c]).min(hh[c]);
                let mx = b[c].max(d[c]).max(f[c]).max(hh[c]);
                let blur = (b[c] + d[c] + f[c] + hh[c]) * 0.25;
                pix[c] = (e[c] + (e[c] - blur) * sharpness).clamp(mn, mx);
            }
            out.push(pix);
        }
    }
    Ok(out)
}

/// Full spatial path: EASU upscale then RCAS sharpen (FSR1 shape).
/// Presentation only, never in tick. Same complete inputs give bit-identical
/// outputs.
pub fn fsr_upscale(
    src: &[[f32; 3]],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
    sharpness: f32,
) -> Result<Vec<[f32; 3]>, PostError> {
    let easu = fsr_easu_upsample(src, src_w, src_h, dst_w, dst_h)?;
    fsr_rcas_sharpen(&easu, dst_w, dst_h, sharpness)
}

/// Pinned FC3 spatial path: 720p EASU-shape upscale plus the RCAS finish.
/// Spatial alternative to [`taa_upsample`]; TAAU stays frozen. Flat fields
/// bit-exact; presentation only. Caller: FC3 governor via `aa_scale`.
pub fn spatial_720p_to_1080p(src: &[[f32; 3]], sharpness: f32) -> Result<Vec<[f32; 3]>, PostError> {
    let (up, _) = upscale_easu::upscale_720p_to_1080p(src)?;
    fsr_rcas_sharpen(&up, upscale_easu::DST_W, upscale_easu::DST_H, sharpness)
}

/// Build a vertical step edge at low res for the MTF50 probe (dark left,
/// bright right, edge at half width). Deterministic, no wall clock.
pub fn fsr_edge_lowres(w: u32, h: u32) -> Result<Vec<[f32; 3]>, PostError> {
    check_dims(w, h)?;
    let mut out = Vec::with_capacity(fsr_capacity(w, h)?);
    let half = w / 2;
    for _ in 0..h {
        for x in 0..w {
            if x < half {
                out.push([0.1, 0.1, 0.1]);
            } else {
                out.push([0.9, 0.9, 0.9]);
            }
        }
    }
    Ok(out)
}

/// 10-90 percent edge width in output pixels, averaged over middle rows.
/// Flat rows fail loudly (no edge to measure).
pub fn fsr_edge_width_10_90(up: &[[f32; 3]], w: u32, h: u32) -> Result<f32, PostError> {
    check_image(up, w, h, "fsr edge up")?;
    if h < 4 {
        return Err(PostError::Invalid("fsr edge needs 4 rows".into()));
    }
    let w_us = usize_from_dim(w)?;
    let h_us = usize_from_dim(h)?;
    let y0_us = h_us / 4;
    let y1_us = h_us - h_us / 4;
    let mut sum = 0.0f32;
    let mut rows = 0u32;
    for y_us in y0_us..y1_us {
        let mut mn = f32::INFINITY;
        let mut mx = f32::NEG_INFINITY;
        for x_us in 0..w_us {
            let l = luminance(up[y_us * w_us + x_us]);
            mn = mn.min(l);
            mx = mx.max(l);
        }
        let span = mx - mn;
        if !span.is_finite() || span < 1e-6 {
            return Err(PostError::Invalid("fsr edge row is flat".into()));
        }
        let lo = mn + span * 0.1;
        let hi = mn + span * 0.9;
        let mut x10: Option<usize> = None;
        let mut x90: Option<usize> = None;
        for x_us in 0..w_us {
            let l = luminance(up[y_us * w_us + x_us]);
            if x10.is_none() && l >= lo {
                x10 = Some(x_us);
            }
            if l >= hi {
                x90 = Some(x_us);
                break;
            }
        }
        let (a_us, b_us) = match (x10, x90) {
            (Some(a), Some(b)) => (a, b),
            _ => return Err(PostError::Invalid("fsr edge never rises".into())),
        };
        let a32 = u32::try_from(a_us)
            .map_err(|_| PostError::Invalid("fsr edge x does not fit".into()))?;
        let b32 = u32::try_from(b_us)
            .map_err(|_| PostError::Invalid("fsr edge x does not fit".into()))?;
        sum += f32_from_dim(b32)? - f32_from_dim(a32)?;
        rows += 1;
    }
    if rows == 0 {
        return Err(PostError::Invalid("fsr edge has no rows".into()));
    }
    Ok(sum / f32_from_dim(rows)?)
}

/// MTF50 proxy from a 10-90 edge width (`0.5 / width`, floored at 0.5 px).
pub fn fsr_mtf50(edge_width_px: f32) -> Result<f32, PostError> {
    if !edge_width_px.is_finite() || edge_width_px < 0.0 {
        return Err(PostError::Invalid("fsr edge width not finite".into()));
    }
    Ok(0.5 / edge_width_px.max(0.5))
}

/// MTF50 lift of a candidate width over a baseline width.
pub fn fsr_mtf50_lift(base_width_px: f32, cand_width_px: f32) -> Result<f32, PostError> {
    let base = fsr_mtf50(base_width_px)?;
    let cand = fsr_mtf50(cand_width_px)?;
    if !base.is_finite() || base <= 0.0 {
        return Err(PostError::Invalid("fsr base mtf not positive".into()));
    }
    Ok((cand - base) / base)
}

/// Shimmer scene: a static high-frequency checkerboard sampled with
/// alternating sub-pixel offsets. Frame `k` shifts the sampling grid by
/// `(k % 2) * 0.5` px, so raw frames flicker while the TAA resolve must
/// converge to a stable mean.
#[derive(Clone, PartialEq, Debug)]
pub struct ShimmerScene {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Alternating raw frames (even/odd sampling offsets).
    pub frames: Vec<Vec<[f32; 3]>>,
}

impl ShimmerScene {
    /// Build the 16x16 checkerboard pair (2px cells, dark 0.1 / lit 0.9).
    pub fn checkerboard() -> Result<Self, PostError> {
        let (width, height) = (16u32, 16u32);
        let cell = |x: u32, y: u32| {
            if (x / 2 + y / 2).is_multiple_of(2) {
                [0.1, 0.1, 0.1]
            } else {
                [0.9, 0.9, 0.9]
            }
        };
        let mut even = Vec::with_capacity(256);
        let mut odd = Vec::with_capacity(256);
        for y in 0..height {
            for x in 0..width {
                even.push(cell(x, y));
                // Odd frames sample half a cell diagonally off: every other
                // texel flips, which is the aliasing shimmer under motion.
                odd.push(cell(x.wrapping_add(1) % width, y.wrapping_add(1) % height));
            }
        }
        check_image(&even, width, height, "shimmer even")?;
        check_image(&odd, width, height, "shimmer odd")?;
        Ok(Self {
            width,
            height,
            frames: vec![even, odd],
        })
    }

    /// Per-pixel temporal variance across `clip` frames (mean of channel
    /// variances, then the max over pixels). Must stay under
    /// [`SHIMMER_MAX_VAR`].
    pub fn temporal_variance(clip: &[Vec<[f32; 3]>]) -> f32 {
        if clip.is_empty() {
            return 0.0;
        }
        let n = clip.len() as f32;
        let len = clip[0].len();
        let mut max = 0.0f32;
        for i in 0..len {
            let mut mean = [0.0f32; 3];
            for frame in clip {
                for (m, p) in mean.iter_mut().zip(frame[i].iter()) {
                    *m += *p;
                }
            }
            for m in &mut mean {
                *m /= n;
            }
            let mut var = 0.0f32;
            for frame in clip {
                for c in 0..3 {
                    let d = frame[i][c] - mean[c];
                    var += d * d;
                }
            }
            max = max.max(var / (3.0 * n));
        }
        max
    }
}
