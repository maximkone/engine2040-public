//! clip_hold: hero motion-hold clip preset as data.
//!
//! Owner: render lane. A fixed 120-frame slow push-in over the twin
//! establishing view proves temporal hold without boiling. Pass code is
//! frozen: every frame resolves through the engine temporal resolve
//! ([`crate::post::taa_resolve`]) plus the temporal-upsample seam
//! (`taa_upsample` identity at full rate), so this file only tunes the
//! clip preset as data plus the CPU oracle that scores it. Std-only.
//! Consumer: the twin establishing view through `present::look`, then
//! reference captures.
//! Units: meters for dolly distance, pixels for sizes and drift, luma for
//! ghost/shimmer. Determinism: fixed 64x36 row-major oracle, integer bar
//! steps, same clip gives bit-identical bytes twice; wall clock never
//! enters pixels. Errors are typed [`HoldError`].
//!
//! Cost: this clip adds no pass. Per-pass GPU budget it rides, enforced on
//! boosted release clocks: TAA resolve plus temporal upsample inside the
//! 1.5 ms full-post bar (`aa_scale::TAA_OFF_BUDGET_MS`), bloom/exposure/ACES/grade
//! inside the same bar; the oracle runs on the CPU only. A prettier clip
//! that blows the frame budget fails.

use std::fmt;

use crate::post::{self, MotionField, TaaParams};

/// Frames in the fixed hold clip.
pub const HOLD_FRAMES: u64 = 120;
/// Oracle frame width in pixels.
pub const HOLD_W: u32 = 64;
/// Oracle frame height in pixels.
pub const HOLD_H: u32 = 36;
/// Dolly distance at frame 0 in meters.
pub const HOLD_START_DIST_M: f32 = 14.0;
/// Dolly distance at the last frame in meters.
pub const HOLD_END_DIST_M: f32 = 11.0;
/// Mean screen drift per frame in pixels (uniform push, under the temporal
/// velocity-reject threshold so history is kept and must hold).
pub const HOLD_DRIFT_PX_PER_FRAME: f32 = 0.1;
/// Hero bar width in pixels.
pub const HOLD_BAR_W_PX: u32 = 6;
/// Bar left edge at frame 0 in pixels.
pub const HOLD_BAR_X0_PX: u32 = 8;
/// Frames per integer bar step (slow push quantized to whole texels).
pub const HOLD_STEP_EVERY: u64 = 10;
/// Dark sand tone (linear) of the oracle backdrop.
pub const HOLD_SAND_DARK: [f32; 3] = [0.30, 0.24, 0.16];
/// Light sand tone (linear) of the oracle backdrop.
pub const HOLD_SAND_LIGHT: [f32; 3] = [0.42, 0.35, 0.26];
/// Hero cloth tone (linear): low contrast against sand on purpose so the
/// gate measures boiling, not edge contrast. High-contrast edges stay
/// covered by the temporal ghost test.
pub const HOLD_HERO: [f32; 3] = [0.50, 0.47, 0.43];
/// Last frames scored for shimmer.
pub const HOLD_SETTLE_TAIL: usize = 12;
/// Last frames scored for shimmer, as a frame count. Tripled with
/// [`HOLD_SETTLE_TAIL`] and [`HOLD_TAIL_FRAMES_F32`]: the assert below
/// breaks the build if the three drift apart.
pub const HOLD_TAIL_FRAMES_U64: u64 = 12;
/// Last frames scored for shimmer, as f32 for variance normalization.
pub const HOLD_TAIL_FRAMES_F32: f32 = 12.0;
/// The tail triple stays one value; update all three together.
const _: [(); 12] = [(); HOLD_SETTLE_TAIL];

/// Motion-hold clip failures. Bad clip input fails loudly; device faults
/// never arise here because the oracle is the std-only temporal resolve.
#[derive(Clone, PartialEq, Debug)]
pub enum HoldError {
    /// Clip input is unusable; reason carried.
    Invalid(String),
    /// The temporal resolve refused the frame; message carried.
    Resolve(String),
}

impl fmt::Display for HoldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "hold clip input: {e}"),
            Self::Resolve(e) => write!(f, "hold clip resolve: {e}"),
        }
    }
}

impl std::error::Error for HoldError {}

impl From<post::PostError> for HoldError {
    fn from(e: post::PostError) -> Self {
        Self::Resolve(e.to_string())
    }
}

/// Push-in camera for one clip frame: dolly distance plus the Halton
/// jitter the device applies (recorded here, applied on the device).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct HoldCamera {
    /// Clip frame index.
    pub frame: u64,
    /// Dolly distance in meters.
    pub distance_m: f32,
    /// Halton jitter in pixels from the shared temporal sequence.
    pub jitter_px: [f32; 2],
}

/// Temporal-hold receipt for one full clip run.
#[derive(Clone, PartialEq, Debug)]
pub struct HoldReceipt {
    /// Frames resolved (always [`HOLD_FRAMES`]).
    pub frames: u64,
    /// Worst mean-abs luma gap between final resolved and current frame.
    pub ghost_trail: f32,
    /// Worst per-texel temporal variance over the settled tail.
    pub shimmer_var: f32,
}

impl HoldReceipt {
    /// Deterministic little-endian bytes: frame count, ghost trail,
    /// shimmer variance. Same clip twice gives bit-identical bytes.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(24);
        out.extend_from_slice(&self.frames.to_le_bytes());
        out.extend_from_slice(&self.ghost_trail.to_le_bytes());
        out.extend_from_slice(&self.shimmer_var.to_le_bytes());
        out.extend_from_slice(&self.frames.to_le_bytes());
        out
    }
}

/// Push-in camera for `frame`: dolly lerps from start to end distance
/// across the clip, jitter reuses the temporal Halton sequence (imported,
/// never re-derived). Frames past the clip fail loudly.
pub fn hold_camera(frame: u64) -> Result<HoldCamera, HoldError> {
    if frame >= HOLD_FRAMES {
        return Err(HoldError::Invalid(format!(
            "hold camera frame {frame} past {HOLD_FRAMES} frames"
        )));
    }
    let span = HOLD_START_DIST_M - HOLD_END_DIST_M;
    let total = HOLD_FRAMES - 1;
    // Integer ratio in u64, one float multiply at the end. The clip is
    // 120 frames, so both ends fit u16 with room to spare.
    let num = total - frame;
    let den_u16 = u16::try_from(total)
        .map_err(|_| HoldError::Invalid(format!("hold clip length {total} past u16")))?;
    let den = f32::from(den_u16);
    let num_u16 = u16::try_from(num)
        .map_err(|_| HoldError::Invalid(format!("hold frame {frame} past u16")))?;
    let ratio = f32::from(num_u16) / den;
    Ok(HoldCamera {
        frame,
        distance_m: HOLD_END_DIST_M + span * ratio,
        jitter_px: post::jitter_offset(frame),
    })
}

/// Uniform push motion for the oracle: every texel drifts
/// [`HOLD_DRIFT_PX_PER_FRAME`] right per frame.
pub fn hold_motion() -> Result<MotionField, HoldError> {
    hold_motion_sized(HOLD_W, HOLD_H)
}

/// Uniform push motion at an explicit size (validated before allocation).
pub fn hold_motion_sized(width: u32, height: u32) -> Result<MotionField, HoldError> {
    check_dims(width, height)?;
    let count = count_texels(width, height)?;
    Ok(MotionField {
        width,
        height,
        flow: vec![[HOLD_DRIFT_PX_PER_FRAME, 0.0]; count],
    })
}

/// Oracle frame at `frame`: sand bands plus the hero bar stepped every
/// [`HOLD_STEP_EVERY`] frames. Fixed 64x36 size.
pub fn hold_frame(frame: u64) -> Result<Vec<[f32; 3]>, HoldError> {
    hold_frame_sized(frame, HOLD_W, HOLD_H)
}

/// Oracle frame at `frame` and an explicit size. Bad frames and sizes
/// fail loudly before any allocation.
pub fn hold_frame_sized(frame: u64, width: u32, height: u32) -> Result<Vec<[f32; 3]>, HoldError> {
    if frame >= HOLD_FRAMES {
        return Err(HoldError::Invalid(format!(
            "hold frame {frame} past {HOLD_FRAMES} frames"
        )));
    }
    check_dims(width, height)?;
    let count = count_texels(width, height)?;
    let step = frame / HOLD_STEP_EVERY;
    let step32 = u32::try_from(step)
        .map_err(|_| HoldError::Invalid(format!("hold step {step} past u32")))?;
    let bar_x = HOLD_BAR_X0_PX
        .checked_add(step32)
        .ok_or_else(|| HoldError::Invalid(format!("hold bar {step} past u32")))?;
    let mut out = Vec::with_capacity(count);
    for y in 0..height {
        let band = if y < height / 2 {
            HOLD_SAND_DARK
        } else {
            HOLD_SAND_LIGHT
        };
        for x in 0..width {
            let in_bar = x >= bar_x && x < bar_x.saturating_add(HOLD_BAR_W_PX);
            out.push(if in_bar { HOLD_HERO } else { band });
        }
    }
    debug_assert_eq!(out.len(), count);
    Ok(out)
}

/// Run the full 120-frame clip through the frozen temporal resolve with the
/// push motion field, chaining history frame to frame from mid-grey.
/// Returns the ghost/shimmer receipt; same clip twice is bit-identical.
pub fn run_hold_clip() -> Result<HoldReceipt, HoldError> {
    let motion = hold_motion()?;
    let params = TaaParams::default();
    params.validate()?;
    let count = count_texels(HOLD_W, HOLD_H)?;
    let mut history = vec![[0.5, 0.5, 0.5]; count];
    let mut tail: Vec<Vec<[f32; 3]>> = Vec::with_capacity(HOLD_SETTLE_TAIL);
    let mut current = hold_frame(0)?;
    let mut resolved = current.clone();
    for frame in 0..HOLD_FRAMES {
        current = hold_frame(frame)?;
        resolved = post::taa_resolve(&current, &history, &motion, HOLD_W, HOLD_H, &params)?;
        history.clone_from(&resolved);
        if HOLD_FRAMES - frame <= HOLD_TAIL_FRAMES_U64 {
            tail.push(resolved.clone());
        }
    }
    let ghost_trail = max_abs_luma_gap(&resolved, &current)?;
    let shimmer_var = tail_temporal_variance(&tail)?;
    if !ghost_trail.is_finite() || !shimmer_var.is_finite() {
        return Err(HoldError::Invalid(format!(
            "hold metrics not finite: ghost {ghost_trail} shimmer {shimmer_var}"
        )));
    }
    Ok(HoldReceipt {
        frames: HOLD_FRAMES,
        ghost_trail,
        shimmer_var,
    })
}

fn check_dims(width: u32, height: u32) -> Result<(), HoldError> {
    if width == 0 || height == 0 {
        return Err(HoldError::Invalid(format!(
            "hold frame size {width}x{height} is zero"
        )));
    }
    if width > post::MAX_POST_DIM || height > post::MAX_POST_DIM {
        return Err(HoldError::Invalid(format!(
            "hold frame size {width}x{height} past {}",
            post::MAX_POST_DIM
        )));
    }
    Ok(())
}

fn count_texels(width: u32, height: u32) -> Result<usize, HoldError> {
    let cells = width
        .checked_mul(height)
        .ok_or_else(|| HoldError::Invalid(format!("hold size {width}x{height} overflows")))?;
    usize::try_from(cells)
        .map_err(|_| HoldError::Invalid(format!("hold size {width}x{height} past usize")))
}

fn luma(p: [f32; 3]) -> f32 {
    (p[0] + p[1] + p[2]) / 3.0
}

fn max_abs_luma_gap(a: &[[f32; 3]], b: &[[f32; 3]]) -> Result<f32, HoldError> {
    if a.len() != b.len() {
        return Err(HoldError::Invalid(format!(
            "hold gap lengths {} vs {}",
            a.len(),
            b.len()
        )));
    }
    let mut worst = 0.0_f32;
    for i in 0..a.len() {
        let gap = (luma(a[i]) - luma(b[i])).abs();
        if gap > worst {
            worst = gap;
        }
    }
    Ok(worst)
}

fn tail_temporal_variance(tail: &[Vec<[f32; 3]>]) -> Result<f32, HoldError> {
    if tail.len() != HOLD_SETTLE_TAIL {
        return Err(HoldError::Invalid(format!(
            "hold tail {} frames, want {HOLD_SETTLE_TAIL}",
            tail.len()
        )));
    }
    let count = tail[0].len();
    for f in tail.iter() {
        if f.len() != count {
            return Err(HoldError::Invalid("hold tail ragged".into()));
        }
    }
    let frames_f32 = HOLD_TAIL_FRAMES_F32;
    let mut worst = 0.0_f32;
    for i in 0..count {
        let mut mean = 0.0_f32;
        for f in tail.iter() {
            mean += luma(f[i]);
        }
        mean /= frames_f32;
        let mut var = 0.0_f32;
        for f in tail.iter() {
            let d = luma(f[i]) - mean;
            var += d * d;
        }
        var /= frames_f32;
        if var > worst {
            worst = var;
        }
    }
    Ok(worst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_clip_resolves_without_boiling() {
        let first = run_hold_clip().expect("clip runs");
        assert_eq!(first.frames, HOLD_FRAMES);
        eprintln!(
            "hold: ghost {g:.4} (gate {gate}), shimmer {s:.6} (gate {sgate})",
            g = first.ghost_trail,
            gate = post::GHOST_MAX_TRAIL,
            s = first.shimmer_var,
            sgate = post::SHIMMER_MAX_VAR
        );
        assert!(
            first.ghost_trail <= post::GHOST_MAX_TRAIL,
            "ghost {} over {}",
            first.ghost_trail,
            post::GHOST_MAX_TRAIL
        );
        assert!(
            first.shimmer_var <= post::SHIMMER_MAX_VAR,
            "shimmer {} over {}",
            first.shimmer_var,
            post::SHIMMER_MAX_VAR
        );
        let second = run_hold_clip().expect("clip runs again");
        assert_eq!(first, second, "clip bit-identical twice");
        assert_eq!(first.to_bytes(), second.to_bytes(), "bytes x2");
    }

    #[test]
    fn push_in_moves_monotonically_with_bounded_jitter() {
        let near = hold_camera(0).expect("first frame reads");
        let far = hold_camera(HOLD_FRAMES - 1).expect("last frame reads");
        assert_eq!(near.distance_m, HOLD_START_DIST_M);
        assert_eq!(far.distance_m, HOLD_END_DIST_M);
        let mut prev = near.distance_m;
        for frame in 1..HOLD_FRAMES {
            let cam = hold_camera(frame).expect("frame reads");
            assert!(cam.distance_m < prev, "dolly pushes in at {frame}");
            assert!(
                cam.jitter_px[0].abs() <= 0.5 && cam.jitter_px[1].abs() <= 0.5,
                "jitter bounded at {frame}"
            );
            prev = cam.distance_m;
        }
    }

    #[test]
    fn bad_hold_inputs_fail_loudly() {
        assert!(hold_camera(HOLD_FRAMES).is_err(), "frame past end refused");
        assert!(hold_camera(u64::MAX).is_err(), "huge frame refused");
        assert!(hold_frame(HOLD_FRAMES).is_err(), "frame past end refused");
        assert!(
            hold_frame_sized(0, 0, HOLD_H).is_err(),
            "zero width refused"
        );
        assert!(
            hold_frame_sized(0, HOLD_W, 0).is_err(),
            "zero height refused"
        );
        assert!(
            hold_frame_sized(0, post::MAX_POST_DIM + 1, HOLD_H).is_err(),
            "oversize refused"
        );
        assert!(hold_motion_sized(0, 8).is_err(), "zero motion refused");
    }
}
