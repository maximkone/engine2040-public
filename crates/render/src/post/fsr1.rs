//! fsr1: the AMD FSR 1.0 EASU edge-adaptive upscale on the CPU, step for step.
//!
//! Owner: render lane. Consumer: the parity test of the live GPU upscale pass
//! (`present/gpu_present/look_seam/upscale_gpu.rs`), which holds the same kernel in WGSL and must
//! land on these pixels. Units: linear-or-display f32 triples in `0..=1`, pixel sizes. Determinism:
//! float math in a fixed order and the two bit-trick reciprocals of the donor, so the same input
//! gives the same pixels twice. Errors: [`PostError`] for a bad size or a non-finite texel.
//! Presentation only, never hashed.
//!
//! PORT(GPUOpen-Effects/FidelityFX-FSR@a21ffb8:ffx-fsr/ffx_fsr1.h FsrEasuF): MIT. The 12-tap
//! window, the gradient direction from the four bilinear weights, the anisotropic kernel and the
//! 2x2 dering clamp are the donor's; written here as scalar Rust with no pasted bytes. The donor
//! gathers channel by channel; this port reads whole texels, which are the same twelve values.
//!
//! The older [`fsr_easu_upsample`](super::fsr_easu_upsample) is a Lanczos-2 kernel in the same
//! spirit and stays as it was; this module is the real EASU the live frame runs.

use super::{check_dims, check_image, fsr_easu_con, PostError};

/// Tap window around the base texel `f` (column, row), in the donor's letter order
/// `b c e f g h i j k l n o`.
///
/// ```text
///     b c
///   e f g h
///   i j k l
///     n o
/// ```
const TAPS: [(i16, i16); 12] = [
    (0, -1),
    (1, -1),
    (-1, 0),
    (0, 0),
    (1, 0),
    (2, 0),
    (-1, 1),
    (0, 1),
    (1, 1),
    (2, 1),
    (0, 2),
    (1, 2),
];

/// Slot of each letter inside [`TAPS`].
const B: usize = 0;
const C: usize = 1;
const E: usize = 2;
const F: usize = 3;
const G: usize = 4;
const H: usize = 5;
const I: usize = 6;
const J: usize = 7;
const K: usize = 8;
const L: usize = 9;
const N: usize = 10;
const O: usize = 11;

/// Smallest squared gradient length below which the direction falls back to horizontal.
const DIR_EPS: f32 = 1.0 / 32768.0;
/// Slope of the kernel window with edge strength: `(1/4 - 0.04) - 0.5`.
const LOB_SLOPE: f32 = -0.29;

/// The donor's low-precision reciprocal: an integer subtract on the float bits.
pub(crate) fn rcp_lo(a: f32) -> f32 {
    f32::from_bits(0x7ef0_7ebb_u32.wrapping_sub(a.to_bits()))
}

/// The donor's low-precision reciprocal square root.
pub(crate) fn rsq_lo(a: f32) -> f32 {
    f32::from_bits(0x5f34_7d74_u32.wrapping_sub(a.to_bits() >> 1))
}

fn sat(x: f32) -> f32 {
    x.clamp(0.0, 1.0)
}

/// The donor's cheap luma times two: `b/2 + (r/2 + g)`.
fn luma2(c: [f32; 3]) -> f32 {
    c[2] * 0.5 + (c[0] * 0.5 + c[1])
}

/// Floor of a source position. The position sits inside a 2048 px image, so the cast is exact.
#[expect(
    clippy::cast_possible_truncation,
    reason = "floor of a position inside a 2048 px image"
)]
fn floor_i64(v: f32) -> i64 {
    v.floor() as i64
}

/// One texel with clamp-to-edge addressing.
fn load(src: &[[f32; 3]], w: usize, h: usize, x: i64, y: i64) -> [f32; 3] {
    let cx = usize::try_from(x.max(0))
        .unwrap_or(0)
        .min(w.saturating_sub(1));
    let cy = usize::try_from(y.max(0))
        .unwrap_or(0)
        .min(h.saturating_sub(1));
    src.get(cy * w + cx).copied().unwrap_or([0.0; 3])
}

/// Add one bilinear corner's gradient and edge length (`FsrEasuSetF`): `la..le` are the luma of
/// the plus-shaped neighbourhood `a / b c d / e` around the corner's texel `c`.
fn set(dir: &mut [f32; 2], len: &mut f32, w: f32, l: [f32; 5]) {
    let [la, lb, lc, ld, le] = l;
    let len_x = (ld - lc).abs().max((lc - lb).abs());
    let dir_x = ld - lb;
    dir[0] += dir_x * w;
    let mut lx = sat(dir_x.abs() * rcp_lo(len_x));
    lx *= lx;
    *len += lx * w;
    let len_y = (le - lc).abs().max((lc - la).abs());
    let dir_y = le - la;
    dir[1] += dir_y * w;
    let mut ly = sat(dir_y.abs() * rcp_lo(len_y));
    ly *= ly;
    *len += ly * w;
}

/// The kernel shape of one output pixel: rotation, anisotropy and window.
struct Kernel {
    dir: [f32; 2],
    len2: [f32; 2],
    lob: f32,
    clp: f32,
}

/// Direction and window of the kernel from the luma of the twelve taps and the sub-texel
/// position `pp` (`0..1` per axis).
fn kernel(luma: &[f32; 12], pp: [f32; 2]) -> Kernel {
    let mut dir = [0.0f32; 2];
    let mut len = 0.0f32;
    let (px, py) = (pp[0], pp[1]);
    set(
        &mut dir,
        &mut len,
        (1.0 - px) * (1.0 - py),
        [luma[B], luma[E], luma[F], luma[G], luma[J]],
    );
    set(
        &mut dir,
        &mut len,
        px * (1.0 - py),
        [luma[C], luma[F], luma[G], luma[H], luma[K]],
    );
    set(
        &mut dir,
        &mut len,
        (1.0 - px) * py,
        [luma[F], luma[I], luma[J], luma[K], luma[N]],
    );
    set(
        &mut dir,
        &mut len,
        px * py,
        [luma[G], luma[J], luma[K], luma[L], luma[O]],
    );
    // Normalize with the approximate rsqrt; a near-zero gradient points along x.
    let mut dir_r = dir[0] * dir[0] + dir[1] * dir[1];
    let zro = dir_r < DIR_EPS;
    dir_r = rsq_lo(dir_r);
    if zro {
        dir_r = 1.0;
        dir[0] = 1.0;
    }
    dir = [dir[0] * dir_r, dir[1] * dir_r];
    // {0..2} to {0..1}, shaped with a square.
    len *= 0.5;
    len *= len;
    // Stretch the kernel from 1.0 on an axis to sqrt(2) on a diagonal.
    let stretch = (dir[0] * dir[0] + dir[1] * dir[1]) * rcp_lo(dir[0].abs().max(dir[1].abs()));
    let len2 = [1.0 + (stretch - 1.0) * len, 1.0 + (-0.5) * len];
    // The window shifts from sqrt(2) to just past 2 with the edge amount.
    let lob = 0.5 + LOB_SLOPE * len;
    Kernel {
        dir,
        len2,
        lob,
        clp: rcp_lo(lob),
    }
}

/// Weight of one tap at offset `off` from the output position (`FsrEasuTapF`).
fn tap_weight(k: &Kernel, off: [f32; 2]) -> f32 {
    let vx = off[0] * k.dir[0] + off[1] * k.dir[1];
    let vy = off[0] * (-k.dir[1]) + off[1] * k.dir[0];
    let vx = vx * k.len2[0];
    let vy = vy * k.len2[1];
    let d2 = (vx * vx + vy * vy).min(k.clp);
    let mut wb = 0.4 * d2 - 1.0;
    let mut wa = k.lob * d2 - 1.0;
    wb *= wb;
    wa *= wa;
    wb = 1.5625 * wb - 0.5625;
    wb * wa
}

/// One output pixel at integer position `(x, y)`.
fn easu_pixel(
    src: &[[f32; 3]],
    w: usize,
    h: usize,
    scale: [f32; 2],
    offset: [f32; 2],
    x: u16,
    y: u16,
) -> [f32; 3] {
    // Output pixel to a source position; `fp` is the base texel `f`.
    let ppx = f32::from(x) * scale[0] + offset[0];
    let ppy = f32::from(y) * scale[1] + offset[1];
    let (fpx, fpy) = (ppx.floor(), ppy.floor());
    let pp = [ppx - fpx, ppy - fpy];
    let (bx, by) = (floor_i64(fpx), floor_i64(fpy));
    let mut texel = [[0.0f32; 3]; 12];
    let mut luma = [0.0f32; 12];
    for (slot, (dx, dy)) in TAPS.iter().enumerate() {
        texel[slot] = load(src, w, h, bx + i64::from(*dx), by + i64::from(*dy));
        luma[slot] = luma2(texel[slot]);
    }
    let k = kernel(&luma, pp);
    let mut acc = [0.0f32; 3];
    let mut acc_w = 0.0f32;
    for (slot, (dx, dy)) in TAPS.iter().enumerate() {
        let off = [f32::from(*dx) - pp[0], f32::from(*dy) - pp[1]];
        let wgt = tap_weight(&k, off);
        for (a, t) in acc.iter_mut().zip(texel[slot]) {
            *a += t * wgt;
        }
        acc_w += wgt;
    }
    // Dering against the four nearest texels `f g j k`.
    let near = [texel[F], texel[G], texel[J], texel[K]];
    let inv = 1.0 / acc_w;
    std::array::from_fn(|c| {
        let lo = near[0][c].min(near[1][c]).min(near[2][c]).min(near[3][c]);
        let hi = near[0][c].max(near[1][c]).max(near[2][c]).max(near[3][c]);
        hi.min(lo.max(acc[c] * inv))
    })
}

/// FSR 1.0 EASU: upscale `src` (`src_w` x `src_h`) to `dst_w` x `dst_h`.
///
/// Same complete inputs give bit-identical outputs. A flat field comes back exact (the dering
/// clamp pins it).
///
/// # Errors
///
/// [`PostError::Invalid`] for a zero or oversize dimension, a pixel count that differs from the
/// size, or a non-finite texel.
pub fn fsr1_easu(
    src: &[[f32; 3]],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Result<Vec<[f32; 3]>, PostError> {
    check_image(src, src_w, src_h, "fsr1 easu src")?;
    check_dims(dst_w, dst_h)?;
    let con = fsr_easu_con(src_w, src_h, dst_w, dst_h)?;
    let w = usize::try_from(src_w).map_err(|_| PostError::Invalid("fsr1 width".into()))?;
    let h = usize::try_from(src_h).map_err(|_| PostError::Invalid("fsr1 height".into()))?;
    let mut out = Vec::with_capacity(
        usize::try_from(u64::from(dst_w) * u64::from(dst_h))
            .map_err(|_| PostError::Invalid("fsr1 size".into()))?,
    );
    for y in 0..dst_h {
        let yy = u16::try_from(y).map_err(|_| PostError::Invalid("fsr1 row".into()))?;
        for x in 0..dst_w {
            let xx = u16::try_from(x).map_err(|_| PostError::Invalid("fsr1 column".into()))?;
            out.push(easu_pixel(src, w, h, con.scale, con.offset, xx, yy));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::post::{
        fsr_edge_lowres, fsr_edge_width_10_90, fsr_mtf50_lift, taa_upsample, FSR_MTF50_LIFT_MIN,
    };

    /// A deterministic image with ramps, diagonal edges, a checker patch and grain.
    fn fixture(w: u32, h: u32) -> Vec<[f32; 3]> {
        let mut state = 0x2040_u32;
        let mut grain = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            f32::from(u16::try_from(state >> 16).unwrap_or(0)) / 65535.0
        };
        let mut out = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let u = f32::from(u16::try_from(x).unwrap_or(0))
                    / f32::from(u16::try_from(w).unwrap_or(1));
                let v = f32::from(u16::try_from(y).unwrap_or(0))
                    / f32::from(u16::try_from(h).unwrap_or(1));
                let ramp = 0.15 + 0.5 * u;
                let diag = if u + v * 0.7 > 0.9 { 0.35 } else { 0.0 };
                let check = if x < w / 4 && y < h / 4 && (x / 2 + y / 2) % 2 == 0 {
                    0.4
                } else {
                    0.0
                };
                let g = grain() * 0.04;
                out.push([
                    ramp + diag + g,
                    0.2 + 0.4 * v + check,
                    0.1 + 0.3 * (1.0 - u) + diag * 0.5,
                ]);
            }
        }
        out
    }

    #[test]
    fn approximate_reciprocals_stay_inside_the_donors_error() {
        for a in [0.01_f32, 0.25, 0.5, 1.0, 2.0, 3.7, 100.0] {
            let rel = (rcp_lo(a) * a - 1.0).abs();
            assert!(rel < 0.13, "rcp_lo({a}) off by {rel}");
            let rel = (rsq_lo(a) * a.sqrt() - 1.0).abs();
            assert!(rel < 0.05, "rsq_lo({a}) off by {rel}");
        }
    }

    #[test]
    fn flat_fields_come_back_exact() {
        for color in [
            [0.4, 0.4, 0.4],
            [0.9, 0.1, 0.5],
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
        ] {
            let src = vec![color; 40 * 24];
            let out = fsr1_easu(&src, 40, 24, 60, 36).expect("flat upscales");
            assert_eq!(out.len(), 60 * 36);
            assert!(
                out.iter().all(|p| *p == color),
                "flat {color:?} stays exact"
            );
        }
    }

    #[test]
    fn same_size_keeps_a_smooth_ramp() {
        let w = 32u32;
        let src: Vec<[f32; 3]> = (0..16)
            .flat_map(|_| {
                (0..w).map(|x| {
                    let v = f32::from(u16::try_from(x).unwrap_or(0)) / 31.0;
                    [v, v * 0.5, 1.0 - v]
                })
            })
            .collect();
        let out = fsr1_easu(&src, w, 16, w, 16).expect("same size");
        let worst = src
            .iter()
            .zip(&out)
            .flat_map(|(a, b)| (0..3).map(move |c| (a[c] - b[c]).abs()))
            .fold(0.0f32, f32::max);
        assert!(worst < 2e-3, "a smooth ramp moved by {worst}");
    }

    #[test]
    fn output_never_leaves_the_input_range() {
        let src = fixture(48, 27);
        let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
        for p in &src {
            for c in 0..3 {
                lo[c] = lo[c].min(p[c]);
                hi[c] = hi[c].max(p[c]);
            }
        }
        let out = fsr1_easu(&src, 48, 27, 72, 40).expect("upscales");
        for p in &out {
            for c in 0..3 {
                assert!(p[c] >= lo[c] && p[c] <= hi[c], "ring escaped: {p:?}");
            }
        }
    }

    #[test]
    fn same_inputs_give_the_same_pixels_twice() {
        let src = fixture(40, 24);
        let a = fsr1_easu(&src, 40, 24, 60, 36).expect("first");
        let b = fsr1_easu(&src, 40, 24, 60, 36).expect("second");
        assert_eq!(a, b);
    }

    #[test]
    fn a_step_edge_is_sharper_than_bilinear() {
        let src = fsr_edge_lowres(64, 32).expect("edge");
        let bilinear = taa_upsample(&src, 64, 32, 96, 48).expect("bilinear");
        let easu = fsr1_easu(&src, 64, 32, 96, 48).expect("easu");
        let wb = fsr_edge_width_10_90(&bilinear, 96, 48).expect("bilinear width");
        let we = fsr_edge_width_10_90(&easu, 96, 48).expect("easu width");
        let lift = fsr_mtf50_lift(wb, we).expect("lift");
        assert!(
            lift >= FSR_MTF50_LIFT_MIN,
            "edge width {wb} -> {we}, lift {lift} under {FSR_MTF50_LIFT_MIN}"
        );
    }

    #[test]
    fn bad_inputs_are_typed_errors() {
        assert!(fsr1_easu(&[[0.0; 3]; 10], 4, 4, 8, 8).is_err());
        assert!(fsr1_easu(&[[0.0; 3]; 16], 4, 4, 0, 8).is_err());
        let mut bad = vec![[0.5; 3]; 16];
        bad[3] = [f32::NAN, 0.0, 0.0];
        assert!(fsr1_easu(&bad, 4, 4, 8, 8).is_err());
    }
}
