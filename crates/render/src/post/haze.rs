//! haze: hot air over the sand refracts the frame by a few pixels (SW143).
//!
//! Owner: render lane. Presentation only, never hashed. Units: texels for the
//! shift, `0..1` for the depth of the G-buffer sample.
//!
//! Shape: five octaves of value noise (8/16/32/64/128 cells across the frame)
//! from two counter-scrolling uv sets, one screen tap per output texel, and a
//! depth mask `max(depth - MIN_DEPTH, 0)` so the air over the sand wobbles and
//! nothing near the lens does. The shift is capped at [`MAX_SHIFT_PX`] at
//! 1080p and scales with the frame height, and the whole pass is skipped when
//! the tier's strength is 0.
//!
//! `apply` is the CPU parity oracle of `shaders/haze.wgsl`; both walk the same
//! octaves and the same depth mask, so a frame can be checked headless.
//! Determinism: the phase is the caller's seconds, never a wall clock.
//!
//! PORT(godotshaders.com/shader/heat-haze-shader by arlez80, MIT): `value_noise`, `noise`,
//! `random` and `fragment` shapes, `max_shift` 2.5, `speed` 0.03 and `min_heathaze_dist` 0.99.
//! Zero donor bytes; the octave table and the depth mask are this crate's.

use std::fmt;

/// Largest shift at the reference height, texels (donor `max_shift` 2.5 px).
pub const MAX_SHIFT_PX: f32 = 2.5;

/// Frame height the shift cap is quoted at, texels (1080p).
pub const REFERENCE_H: f32 = 1080.0;

/// Shallowest depth the air counts as hot: anything nearer the lens is still
/// (donor `min_heathaze_dist` 0.99, in normalized device depth).
pub const MIN_DEPTH: f32 = 0.99;

/// Octave cell counts across the frame, coarse to fine.
pub const CELLS: [u32; 5] = [8, 16, 32, 64, 128];

/// Scroll of the two uv sets, screens per second, opposite ways (donor `speed` 0.03).
pub const SPEED: f32 = 0.03;

/// Weight of each octave, coarse to fine: the sum is 1, so the noise stays in `0..1`.
const OCTAVE_WEIGHT: [f32; 5] = [0.32, 0.26, 0.20, 0.14, 0.08];

/// A refusal from the haze pass: a frame or depth buffer that cannot be one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HazeError {
    /// A dimension is zero or over the post bound.
    Size,
    /// The depth buffer is not one sample per frame texel.
    Depth,
    /// The frame is not `w * h` texels.
    Frame,
    /// The time or the strength is not finite, or the strength is outside `0..=1`.
    Input,
}

impl fmt::Display for HazeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Size => "haze needs a non-zero frame inside the post bound",
            Self::Depth => "haze needs one depth sample per frame texel",
            Self::Frame => "haze needs w * h frame texels",
            Self::Input => "haze needs a finite time and a strength in 0..=1",
        };
        f.write_str(text)
    }
}

impl std::error::Error for HazeError {}

/// Whether the pass runs at all at this strength: a tier at 0 pays nothing.
pub fn runs(strength: f32) -> bool {
    strength > 0.0
}

/// The depth mask of one sample: 0 up to [`MIN_DEPTH`], then linear.
pub fn depth_mask(depth: f32) -> f32 {
    (depth - MIN_DEPTH).max(0.0)
}

/// The shift cap for a frame `height` texels tall: [`MAX_SHIFT_PX`] at
/// [`REFERENCE_H`], proportional elsewhere, never negative.
pub fn max_shift_for(height: u32) -> f32 {
    let h = f32::from(u16::try_from(height.min(u32::from(u16::MAX))).unwrap_or(0));
    MAX_SHIFT_PX * (h / REFERENCE_H)
}

/// A hashed corner value in `0..1`, from the cell and a salt.
fn corner(cell: [i32; 2], salt: u32) -> f32 {
    let x = u32::from_ne_bytes(cell[0].to_ne_bytes());
    let y = u32::from_ne_bytes(cell[1].to_ne_bytes());
    let mut h = x
        .wrapping_mul(0x9E37_79B1)
        .wrapping_add(y.wrapping_mul(0x85EB_CA6B))
        .wrapping_add(salt.wrapping_mul(0xC2B2_AE35));
    h ^= h >> 15;
    h = h.wrapping_mul(0x2545_F491);
    h ^= h >> 13;
    h = h.wrapping_mul(0x27D4_EB2D);
    h ^= h >> 16;
    f32::from(u16::try_from((h >> 8) & 0xffff).unwrap_or(0)) / 65535.0
}

/// The lattice cell of `v`, with the fraction inside it.
#[expect(
    clippy::cast_possible_truncation,
    reason = "a uv in -1..2 times at most 128 cells is a lattice index inside i32"
)]
fn lattice(v: f32) -> (i32, f32) {
    let base = v.floor();
    let cell = base as i32;
    (cell, v - base)
}

/// Smoothstep: 0 at 0, 1 at 1, flat at both ends.
fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

/// One octave of value noise over `uv`, `cells` cells across: bilinear over four
/// hashed corners with a smoothstep between them.
pub fn value_noise(uv: [f32; 2], cells: u32, salt: u32) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "a cell count is at most 128, exactly representable in f32"
    )]
    let n = cells as f32;
    let (cx, fx) = lattice(uv[0] * n);
    let (cy, fy) = lattice(uv[1] * n);
    let (sx, sy) = (smooth(fx), smooth(fy));
    let west = corner([cx, cy], salt) * (1.0 - sx) + corner([cx + 1, cy], salt) * sx;
    let east = corner([cx, cy + 1], salt) * (1.0 - sx) + corner([cx + 1, cy + 1], salt) * sx;
    west * (1.0 - sy) + east * sy
}

/// The five-octave noise at `uv` and `seconds`, from two counter-scrolling uv sets:
/// the coarse half drifts one way, the fine half the other, so the air never
/// reads as one sliding sheet.
/// The five-octave noise at `uv` and `seconds`, from two counter-scrolling uv sets: the first
/// drifts one way, the second the other, so the air never reads as one sliding sheet. Returns both
/// fields in `0..1`, x from the first set and y from the second.
pub fn octaves(uv: [f32; 2], seconds: f32) -> [f32; 2] {
    let a = [uv[0] + seconds * SPEED, uv[1] - seconds * SPEED * 0.5];
    let b = [uv[0] - seconds * SPEED * 0.5, uv[1] + seconds * SPEED];
    let mut sum = [0.0; 2];
    let mut salt = 1;
    for (index, cells) in CELLS.iter().enumerate() {
        let weight = OCTAVE_WEIGHT[index];
        sum[0] += weight * value_noise(a, *cells, salt);
        sum[1] += weight * value_noise(b, *cells, salt);
        salt = salt.wrapping_add(0x9E37);
    }
    [sum[0].clamp(0.0, 1.0), sum[1].clamp(0.0, 1.0)]
}

/// The haze shift of one texel, `[x, y]` in texels: each field's own noise, centred on 0.5 and
/// scaled by the depth mask and the frame's shift cap. Zero under [`MIN_DEPTH`], zero at strength
/// 0, and never past the cap on either axis.
pub fn shift_px(px: [u32; 2], w: u32, h: u32, depth: f32, seconds: f32, strength: f32) -> [f32; 2] {
    let mask = depth_mask(depth) * strength.clamp(0.0, 1.0);
    if mask <= 0.0 || w == 0 || h == 0 {
        return [0.0; 2];
    }
    let uw = f32::from(u16::try_from(w.min(u32::from(u16::MAX))).unwrap_or(1));
    let uh = f32::from(u16::try_from(h.min(u32::from(u16::MAX))).unwrap_or(1));
    let uv = [
        f32::from(u16::try_from(px[0].min(u32::from(u16::MAX))).unwrap_or(0)) / uw,
        f32::from(u16::try_from(px[1].min(u32::from(u16::MAX))).unwrap_or(0)) / uh,
    ];
    let field = octaves(uv, seconds);
    let cap = max_shift_for(h) * mask;
    [(field[0] - 0.5) * 2.0 * cap, (field[1] - 0.5) * 2.0 * cap]
}

/// Applies the haze to a frame: every texel samples the frame at its own shifted
/// uv, clamped at the edges, and the depth buffer decides which texels move.
/// Returns the frame and a receipt hash of the shifted pixels.
pub fn apply(
    frame: &[[f32; 3]],
    depth: &[f32],
    w: u32,
    h: u32,
    seconds: f32,
    strength: f32,
) -> Result<(Vec<[f32; 3]>, u64), HazeError> {
    if w == 0 || h == 0 {
        return Err(HazeError::Size);
    }
    let texels = usize::try_from(u64::from(w) * u64::from(h)).map_err(|_| HazeError::Size)?;
    if frame.len() != texels {
        return Err(HazeError::Frame);
    }
    if depth.len() != texels {
        return Err(HazeError::Depth);
    }
    if !seconds.is_finite() || !strength.is_finite() || !(0.0..=1.0).contains(&strength) {
        return Err(HazeError::Input);
    }
    if !runs(strength) || depth.iter().all(|d| depth_mask(*d) <= 0.0) {
        return Ok((frame.to_vec(), receipt(frame)));
    }
    let mut out = Vec::with_capacity(texels);
    for y in 0..h {
        for x in 0..w {
            let at = usize::try_from(u64::from(y) * u64::from(w) + u64::from(x))
                .map_err(|_| HazeError::Size)?;
            let shift = shift_px([x, y], w, h, depth[at], seconds, strength);
            let sx = (f32::from(u16::try_from(x).unwrap_or(0)) + shift[0])
                .clamp(0.0, f32::from(u16::try_from(w - 1).unwrap_or(0)));
            let sy = (f32::from(u16::try_from(y).unwrap_or(0)) + shift[1])
                .clamp(0.0, f32::from(u16::try_from(h - 1).unwrap_or(0)));
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "the uv is clamped into the frame, so the truncation is a texel index"
            )]
            let (sx, sy) = (sx.round() as usize, sy.round() as usize);
            let from = sy * usize::try_from(w).map_err(|_| HazeError::Size)? + sx;
            out.push(frame[from]);
        }
    }
    let hash = receipt(&out);
    Ok((out, hash))
}

/// A receipt hash of a frame: fnv-1a over the pixel bits, so two runs can be
/// compared without keeping the buffer.
pub fn receipt(frame: &[[f32; 3]]) -> u64 {
    let mut hasher = 0xCBF2_9CE4_8422_2325u64;
    for texel in frame {
        for channel in texel {
            for byte in channel.to_bits().to_le_bytes() {
                hasher ^= u64::from(byte);
                hasher = hasher.wrapping_mul(0x0000_0100_0000_01B3);
            }
        }
    }
    hasher
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame of `w * h` texels whose every channel is a distinct cheap value.
    fn frame(w: u32, h: u32) -> Vec<[f32; 3]> {
        let n = usize::try_from(u64::from(w) * u64::from(h)).expect("size");
        (0..n)
            .map(|i| {
                let v = f32::from(u16::try_from(i % 977).unwrap_or(0)) / 977.0;
                [v, 1.0 - v, v * 0.5]
            })
            .collect()
    }

    #[test]
    fn haze_never_touches_the_near_field() {
        let (w, h) = (160, 90);
        let src = frame(w, h);
        let near = vec![0.5; src.len()];
        let (out, hash) = apply(&src, &near, w, h, 4.25, 1.0).expect("haze");
        assert_eq!(out, src, "a pixel at depth 0.5 must not move one bit");
        assert_eq!(hash, receipt(&src));
        for px in [[0, 0], [7, 11], [80, 45], [159, 89]] {
            assert_eq!(
                shift_px(px, w, h, 0.5, 4.25, 1.0),
                [0.0, 0.0],
                "near field {px:?} must hold still"
            );
        }
        println!("near field: 0 of {} texels move at depth 0.5", src.len());
    }

    #[test]
    fn haze_shifts_a_far_pixel_under_the_cap() {
        let (w, h) = (320, 180);
        let cap = max_shift_for(h);
        assert!(
            (max_shift_for(1080) - MAX_SHIFT_PX).abs() < 1.0e-6,
            "the cap is quoted at 1080p: {}",
            max_shift_for(1080)
        );
        let mut widest = 0.0f32;
        for y in (0..h).step_by(7) {
            for x in (0..w).step_by(7) {
                let shift = shift_px([x, y], w, h, 1.0, 2.5, 1.0);
                let len = (shift[0] * shift[0] + shift[1] * shift[1]).sqrt();
                widest = widest.max(len);
                assert!(
                    shift[0].abs() <= cap + 1.0e-6 && shift[1].abs() <= cap + 1.0e-6,
                    "a far pixel at {x},{y} shifts {shift:?} past the {cap} texel cap"
                );
            }
        }
        println!("far field: widest shift {widest:.3} of the {cap:.3} texel cap at {h}p");
        assert!(widest > 0.01, "the hot air must move something: {widest}");
        let deep = vec![1.0; usize::try_from(u64::from(w) * u64::from(h)).expect("size")];
        let src = frame(w, h);
        let (out, _) = apply(&src, &deep, w, h, 2.5, 1.0).expect("haze");
        assert_ne!(out, src, "a far field must sample somewhere else");
    }

    #[test]
    fn haze_strength_zero_is_byte_identical() {
        let (w, h) = (64, 36);
        let src = frame(w, h);
        let deep = vec![1.0; src.len()];
        let (out, hash) = apply(&src, &deep, w, h, 9.0, 0.0).expect("haze");
        assert_eq!(out, src, "strength 0 must not move one bit");
        assert_eq!(hash, receipt(&src));
        assert!(!runs(0.0), "a tier at 0 skips the pass");
    }

    #[test]
    fn haze_is_the_same_twice_and_refuses_bad_input() {
        let (w, h) = (80, 45);
        let src = frame(w, h);
        let deep = vec![1.0; src.len()];
        let (one, hash_one) = apply(&src, &deep, w, h, 1.75, 0.5).expect("haze");
        let (two, hash_two) = apply(&src, &deep, w, h, 1.75, 0.5).expect("haze");
        assert_eq!(one, two, "same seconds, same pixels");
        assert_eq!(hash_one, hash_two);
        assert_eq!(
            apply(&src, &deep[1..], w, h, 1.0, 1.0),
            Err(HazeError::Depth)
        );
        assert_eq!(
            apply(&src[1..], &deep, w, h, 1.0, 1.0),
            Err(HazeError::Frame)
        );
        assert_eq!(
            apply(&src, &deep, w, h, f32::NAN, 1.0),
            Err(HazeError::Input)
        );
        assert_eq!(apply(&src, &deep, w, h, 1.0, 1.5), Err(HazeError::Input));
        assert_eq!(apply(&src, &deep, 0, h, 1.0, 1.0), Err(HazeError::Size));
    }
}
