use super::*;

// ---------------------------------------------------------------------------
// TAA resolve: bilinear reproject plus Karis-style neighborhood clamp.
// ---------------------------------------------------------------------------

/// TAA resolve parameters.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TaaParams {
    /// History weight in `0..=1` (0 follows current, 1 freezes history).
    pub history_blend: f32,
    /// Clamp the reprojected history sample to the current 3x3 min/max
    /// (Karis-style box clamp against ghosting).
    pub use_clamp: bool,
    /// Drop history entirely past this motion length in pixels (velocity
    /// rejection for fast movers). Nonnegative and finite.
    pub motion_threshold_px: f32,
}

impl Default for TaaParams {
    fn default() -> Self {
        Self {
            history_blend: DEFAULT_HISTORY_BLEND,
            use_clamp: true,
            motion_threshold_px: 32.0,
        }
    }
}

impl TaaParams {
    /// Validate ranges before the resolve runs.
    pub fn validate(&self) -> Result<(), PostError> {
        if !self.history_blend.is_finite() || !(0.0..=1.0).contains(&self.history_blend) {
            return Err(PostError::Invalid(format!(
                "taa blend {} outside 0..=1",
                self.history_blend
            )));
        }
        if !self.motion_threshold_px.is_finite() || self.motion_threshold_px < 0.0 {
            return Err(PostError::Invalid(format!(
                "taa motion threshold {} is not finite non-negative",
                self.motion_threshold_px
            )));
        }
        Ok(())
    }
}

/// Bilinear history sample at continuous `(u, v)` where texel `(x, y)` is
/// centered on the integer lattice: integer positions reproduce the stored
/// texel exactly, so static scenes resolve bit-identically.
pub(super) fn sample_bilinear(
    history: &[[f32; 3]],
    width: u32,
    height: u32,
    u: f32,
    v: f32,
) -> [f32; 3] {
    let w = width as usize;
    let h = height as usize;
    let x = u.clamp(0.0, width as f32 - 1.0);
    let y = v.clamp(0.0, height as f32 - 1.0);
    let x0 = (x.floor() as usize).min(w - 1);
    let y0 = (y.floor() as usize).min(h - 1);
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let tx = (x - x0 as f32).clamp(0.0, 1.0);
    let ty = (y - y0 as f32).clamp(0.0, 1.0);
    let a = history[y0 * w + x0];
    let b = history[y0 * w + x1];
    let c = history[y1 * w + x0];
    let d = history[y1 * w + x1];
    let mix = |p: [f32; 3], q: [f32; 3], t: f32| {
        [
            p[0] + (q[0] - p[0]) * t,
            p[1] + (q[1] - p[1]) * t,
            p[2] + (q[2] - p[2]) * t,
        ]
    };
    mix(mix(a, b, tx), mix(c, d, tx), ty)
}

/// 3x3 neighborhood min/max around `(x, y)` in the current frame.
pub(super) fn neighborhood_minmax(
    current: &[[f32; 3]],
    width: u32,
    height: u32,
    x: u32,
    y: u32,
) -> ([f32; 3], [f32; 3]) {
    let w = width as i32;
    let h = height as i32;
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    for dy in -1..=1 {
        for dx in -1..=1 {
            let sx = (x as i32 + dx).clamp(0, w - 1) as usize;
            let sy = (y as i32 + dy).clamp(0, h - 1) as usize;
            let p = current[sy * width as usize + sx];
            for c in 0..3 {
                lo[c] = lo[c].min(p[c]);
                hi[c] = hi[c].max(p[c]);
            }
        }
    }
    (lo, hi)
}

pub(super) fn clamp_history(sample: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> [f32; 3] {
    [
        sample[0].clamp(lo[0], hi[0]),
        sample[1].clamp(lo[1], hi[1]),
        sample[2].clamp(lo[2], hi[2]),
    ]
}

/// TAA resolve: reproject history along the motion field, box-clamp it to
/// the current 3x3 neighborhood (Karis-style ghosting guard), then blend.
/// Same complete inputs give bit-identical outputs.
pub fn taa_resolve(
    current: &[[f32; 3]],
    history: &[[f32; 3]],
    motion: &MotionField,
    width: u32,
    height: u32,
    params: &TaaParams,
) -> Result<Vec<[f32; 3]>, PostError> {
    params.validate()?;
    check_image(current, width, height, "taa current")?;
    check_image(history, width, height, "taa history")?;
    if motion.width != width || motion.height != height {
        return Err(PostError::Invalid(format!(
            "taa motion {}x{} differs from frame {width}x{height}",
            motion.width, motion.height
        )));
    }
    let mut out = Vec::with_capacity(width as usize * height as usize);
    for y in 0..height {
        for x in 0..width {
            let [mx, my] = motion.at(x, y);
            let speed = (mx * mx + my * my).sqrt();
            // Velocity rejection: fast movers follow the current frame.
            let blend = if speed > params.motion_threshold_px {
                0.0
            } else {
                params.history_blend
            };
            let sample = sample_bilinear(history, width, height, x as f32 - mx, y as f32 - my);
            let kept = if params.use_clamp {
                let (lo, hi) = neighborhood_minmax(current, width, height, x, y);
                clamp_history(sample, lo, hi)
            } else {
                sample
            };
            let c = current[y as usize * width as usize + x as usize];
            out.push([
                c[0] + (kept[0] - c[0]) * blend,
                c[1] + (kept[1] - c[1]) * blend,
                c[2] + (kept[2] - c[2]) * blend,
            ]);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Jittered resolve: the GPU reprojection adds the current-frame jitter
// offset to the history sample position, so the device pass consumes the
// Halton sequence instead of ignoring it. Zero jitter is exactly
// [`taa_resolve`]; nonzero jitter shifts sub-pixel sampling.
// ---------------------------------------------------------------------------

/// TAA resolve with an explicit current-frame jitter offset in pixels.
/// Sample position is `(x - mx + jx, y - my + jy)`; everything else matches
/// [`taa_resolve`]. Same complete inputs give bit-identical outputs.
pub fn taa_resolve_jittered(
    current: &[[f32; 3]],
    history: &[[f32; 3]],
    motion: &MotionField,
    width: u32,
    height: u32,
    params: &TaaParams,
    jitter_px: [f32; 2],
) -> Result<Vec<[f32; 3]>, PostError> {
    if !jitter_px.iter().all(|c| c.is_finite()) {
        return Err(PostError::Invalid("taa jitter is not finite".into()));
    }
    if jitter_px[0].abs() > 1.0 || jitter_px[1].abs() > 1.0 {
        return Err(PostError::Invalid(format!(
            "taa jitter {jitter_px:?} outside +-1px"
        )));
    }
    params.validate()?;
    check_image(current, width, height, "taa current")?;
    check_image(history, width, height, "taa history")?;
    if motion.width != width || motion.height != height {
        return Err(PostError::Invalid(format!(
            "taa motion {}x{} differs from frame {width}x{height}",
            motion.width, motion.height
        )));
    }
    if jitter_px == [0.0, 0.0] {
        return taa_resolve(current, history, motion, width, height, params);
    }
    let mut out = Vec::with_capacity(width as usize * height as usize);
    for y in 0..height {
        for x in 0..width {
            let [mx, my] = motion.at(x, y);
            let speed = (mx * mx + my * my).sqrt();
            let blend = if speed > params.motion_threshold_px {
                0.0
            } else {
                params.history_blend
            };
            let sample = sample_bilinear(
                history,
                width,
                height,
                x as f32 - mx + jitter_px[0],
                y as f32 - my + jitter_px[1],
            );
            let kept = if params.use_clamp {
                let (lo, hi) = neighborhood_minmax(current, width, height, x, y);
                clamp_history(sample, lo, hi)
            } else {
                sample
            };
            let c = current[y as usize * width as usize + x as usize];
            out.push([
                c[0] + (kept[0] - c[0]) * blend,
                c[1] + (kept[1] - c[1]) * blend,
                c[2] + (kept[2] - c[2]) * blend,
            ]);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Test scenes: ghosting (moving bar) and shimmer (static checkerboard).
// ---------------------------------------------------------------------------

/// Ghosting scene: a bright bar slides over a dark field. Frame A holds
/// the bar at `bar_x`, frame B at `bar_x + bar_speed`; motion is
/// `[bar_speed, 0]` on bar pixels, zero elsewhere. The trail region (old
/// bar pixels now dark) is where naive history leaks light.
#[derive(Clone, PartialEq, Debug)]
pub struct GhostingScene {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Previous frame (bar at `bar_x`).
    pub prev: Vec<[f32; 3]>,
    /// Current frame (bar moved by `bar_speed`).
    pub current: Vec<[f32; 3]>,
    /// Motion field for the resolve.
    pub motion: MotionField,
    /// Trail x-range `[lo, hi)` in the current frame (must stay dark).
    /// Excludes the 1px boundary layer touching the new bar edge: any 3x3
    /// clamp legitimately sees the bar there, so ghosting is measured on
    /// the interior vacated pixels, the standard practice.
    pub trail: (u32, u32),
}

impl GhostingScene {
    /// Build the two-frame slide: 16x16 field, bar 4 wide, speed 4 px.
    /// The measured trail is the interior `[bar_x, bar_x + bar_speed - 1)`.
    pub fn moving_bar() -> Result<Self, PostError> {
        let (width, height) = (16u32, 16u32);
        let (bar_x, bar_w, bar_speed) = (4u32, 4u32, 4u32);
        let dark = [0.05, 0.05, 0.05];
        let bright = [4.0, 4.0, 4.0];
        let mut prev = vec![dark; 256];
        let mut current = vec![dark; 256];
        let mut flow = vec![[0.0, 0.0]; 256];
        for y in 0..height {
            for x in 0..width {
                let i = y as usize * width as usize + x as usize;
                if x >= bar_x && x < bar_x + bar_w {
                    prev[i] = bright;
                }
                if x >= bar_x + bar_speed && x < bar_x + bar_speed + bar_w {
                    current[i] = bright;
                    flow[i] = [bar_speed as f32, 0.0];
                }
            }
        }
        Ok(Self {
            width,
            height,
            prev,
            current,
            motion: MotionField::build(width, height, flow)?,
            trail: (bar_x, bar_x + bar_speed - 1),
        })
    }

    /// Ghost metric: max luma inside the trail region (old bar pixels that
    /// are dark in both frames). Must stay under [`GHOST_MAX_TRAIL`].
    pub fn trail_luma(resolved: &[[f32; 3]], width: u32, trail: (u32, u32)) -> f32 {
        let h = resolved.len() as u32 / width;
        let mut max = 0.0f32;
        for y in 0..h {
            for x in trail.0..trail.1 {
                let p = resolved[y as usize * width as usize + x as usize];
                max = max.max(luminance(p));
            }
        }
        max
    }
}
