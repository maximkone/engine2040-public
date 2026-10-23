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
    /// The history belongs to the scene being drawn. False on a first use,
    /// a resize, a dyn-res step or a camera cut (see
    /// [`crate::temporal::TemporalRegistry`]): the resolve then follows the
    /// current frame and reads no history.
    pub history_valid: bool,
}

impl Default for TaaParams {
    fn default() -> Self {
        Self {
            history_blend: DEFAULT_HISTORY_BLEND,
            use_clamp: true,
            motion_threshold_px: 32.0,
            history_valid: true,
        }
    }
}

impl TaaParams {
    /// These parameters with the history flag set from the temporal registry.
    #[must_use]
    pub fn with_history(self, history_valid: bool) -> Self {
        Self {
            history_valid,
            ..self
        }
    }

    /// History weight actually applied: zero while the history is invalid.
    #[must_use]
    pub fn effective_blend(&self) -> f32 {
        if self.history_valid {
            self.history_blend
        } else {
            0.0
        }
    }

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

/// YCoCg working space for the neighbourhood clamp, byte-identical in
/// convention to `shaders/taa_present.wgsl`: `Y = 0.25r + 0.5g + 0.25b`,
/// `Co = 0.5(r - b)`, `Cg = 0.5g - 0.25(r + b)`. Luma and both chroma axes
/// get their own box, so clipping cannot walk one channel of a saturated
/// edge away from the neighbourhood's hue the way a per-channel RGB clamp
/// does.
pub(super) fn to_ycocg(rgb: [f32; 3]) -> [f32; 3] {
    [
        0.25 * rgb[0] + 0.5 * rgb[1] + 0.25 * rgb[2],
        0.5 * rgb[0] - 0.5 * rgb[2],
        -0.25 * rgb[0] + 0.5 * rgb[1] - 0.25 * rgb[2],
    ]
}

/// Inverse of [`to_ycocg`]: `r = Y + Co - Cg`, `g = Y + Cg`, `b = Y - Co - Cg`.
pub(super) fn from_ycocg(ycocg: [f32; 3]) -> [f32; 3] {
    let [y, co, cg] = ycocg;
    [y + co - cg, y + cg, y - co - cg]
}

/// 3x3 neighborhood min/max around `(x, y)` in the current frame, in YCoCg
/// (see [`to_ycocg`]). The returned `lo`/`hi` are YCoCg bounds, not RGB.
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
            let p = to_ycocg(current[sy * width as usize + sx]);
            for c in 0..3 {
                lo[c] = lo[c].min(p[c]);
                hi[c] = hi[c].max(p[c]);
            }
        }
    }
    (lo, hi)
}

/// Clamp a history sample to YCoCg bounds, then convert back.
///
/// A sample already inside the box is returned untouched: the YCoCg round
/// trip is not bit exact, and a no-op clamp must stay the exact identity so
/// a static scene resolves to its own pixels.
pub(super) fn clamp_history(sample: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> [f32; 3] {
    let ycocg = to_ycocg(sample);
    let clamped = [
        ycocg[0].clamp(lo[0], hi[0]),
        ycocg[1].clamp(lo[1], hi[1]),
        ycocg[2].clamp(lo[2], hi[2]),
    ];
    if clamped == ycocg {
        return sample;
    }
    from_ycocg(clamped)
}

/// TAA resolve: reproject history along the motion field, box-clamp it to
/// the current 3x3 neighborhood in YCoCg (Karis-style ghosting guard), then
/// blend. Same complete inputs give bit-identical outputs.
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
                params.effective_blend()
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
                params.effective_blend()
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

// ---------------------------------------------------------------------------
// Registration tests (ENG20): a real history must land where the frame moved;
// a frozen one must miss the bar, and a cut or resize must drop the history.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::temporal::{HistoryDesc, TemporalRegistry};

    /// Frame width of the registration scene.
    const W: u32 = 32;
    /// Frame height of the registration scene.
    const H: u32 = 8;
    /// Pixels the bright block pans left per frame.
    const PAN_PX: u32 = 4;
    /// Block width in pixels.
    const BLOCK_W: u32 = 20;
    /// Block left edge at frame 0.
    const BLOCK_START: u32 = 8;
    /// Frames the pan loop runs: the block reaches the left border.
    const FRAMES: u32 = 3;
    /// Bright block value (HDR).
    const BRIGHT: f32 = 0.8;
    /// Dark field value (HDR).
    const DARK: f32 = 0.02;
    /// The resolved edge may sit at most this far from the current frame's
    /// edge. The row's bar: one pixel.
    const EDGE_BAR_PX: f32 = 1.0;

    /// Frame `k` of the pan: a bright block sliding left by [`PAN_PX`] a
    /// frame over a dark field.
    fn block_frame(k: u32) -> Vec<[f32; 3]> {
        let start = BLOCK_START as i32 - (PAN_PX * k) as i32;
        let mut frame = vec![[DARK; 3]; (W * H) as usize];
        for y in 0..H {
            for x in 0..W {
                let inside = (x as i32) >= start && (x as i32) < start + BLOCK_W as i32;
                if inside {
                    frame[(y * W + x) as usize] = [BRIGHT; 3];
                }
            }
        }
        frame
    }

    /// The real flow: current minus history is `-PAN_PX` on every pixel.
    fn pan_motion() -> MotionField {
        MotionField::build(W, H, vec![[-(PAN_PX as f32), 0.0]; (W * H) as usize])
            .expect("pan motion builds")
    }

    /// The frozen history: the field is zero, so history samples its own
    /// texel instead of the one the frame moved from.
    fn frozen_motion() -> MotionField {
        MotionField::zero(W, H).expect("zero motion builds")
    }

    /// Continuous x of the block's right edge on one row: the sub-pixel
    /// crossing of the bright/dark midpoint, interpolated between the last
    /// bright pixel and its successor.
    fn right_edge_px(frame: &[[f32; 3]], row: u32) -> f32 {
        let mid = (BRIGHT + DARK) * 0.5;
        let at = |x: u32| luminance(frame[(row * W + x) as usize]);
        let mut last = 0u32;
        for x in 0..W {
            if at(x) >= mid {
                last = x;
            }
        }
        if last + 1 >= W {
            return last as f32;
        }
        let (a, b) = (at(last), at(last + 1));
        if (a - b).abs() <= 1e-6 {
            return last as f32;
        }
        last as f32 + (a - mid) / (a - b)
    }

    /// Max luma in the band the pan vacated at frame `k`.
    fn vacated_luma(frame: &[[f32; 3]], k: u32) -> f32 {
        let start = BLOCK_START as i32 - (PAN_PX * k) as i32;
        let lo = (start + BLOCK_W as i32) as u32;
        GhostingScene::trail_luma(frame, W, (lo, lo + PAN_PX))
    }

    /// Largest per-pixel luma error between a resolved frame and the frame
    /// the scene actually holds.
    fn max_luma_error(resolved: &[[f32; 3]], current: &[[f32; 3]]) -> f32 {
        resolved
            .iter()
            .zip(current)
            .map(|(a, b)| (luminance(*a) - luminance(*b)).abs())
            .fold(0.0f32, f32::max)
    }

    /// Run the TAA loop over the pan: the first frame starts with no history
    /// (a first use), each later frame resolves against the previous result.
    fn run_pan(motion: &MotionField, params: TaaParams) -> Vec<Vec<[f32; 3]>> {
        let mut history = block_frame(0);
        let mut out = Vec::new();
        for k in 0..FRAMES {
            let current = block_frame(k);
            let frame_params = if k == 0 {
                params.with_history(false)
            } else {
                params
            };
            let resolved = taa_resolve(&current, &history, motion, W, H, &frame_params)
                .expect("pan resolve runs");
            history = resolved.clone();
            out.push(resolved);
        }
        out
    }

    #[test]
    fn taa_registration_tracks_a_panning_edge_and_a_frozen_history_ghosts() {
        // Same pan, same frames; only the flow field differs. A registered
        // history reproduces each current frame, so the edge lands exactly
        // where the frame moved it. A frozen history samples its own texel
        // and leaves the old frame's pixels on screen instead.
        let registered = run_pan(&pan_motion(), TaaParams::default());
        for (k, frame) in registered.iter().enumerate() {
            let current = block_frame(k as u32);
            assert_eq!(
                *frame, current,
                "frame {k}: registered history must reproduce the current frame"
            );
            let truth = right_edge_px(&current, H / 2);
            let got = right_edge_px(frame, H / 2);
            assert!(
                (got - truth).abs() <= EDGE_BAR_PX,
                "frame {k}: edge at {got:.3} px, truth {truth:.3} px, bar {EDGE_BAR_PX} px"
            );
            let trail = vacated_luma(frame, k as u32);
            assert!(
                trail <= GHOST_MAX_TRAIL,
                "frame {k}: registered history leaves vacated luma {trail:.3} over the gate"
            );
        }
        // Frozen history without the clamp: the leak is raw, so the resolved
        // edge sits behind the bar and the vacated band stays lit.
        let frozen = run_pan(
            &frozen_motion(),
            TaaParams {
                use_clamp: false,
                ..TaaParams::default()
            },
        );
        let last = (FRAMES - 1) as usize;
        let truth = right_edge_px(&block_frame(FRAMES - 1), H / 2);
        let ghost_edge = right_edge_px(&frozen[last], H / 2);
        let lag = ghost_edge - truth;
        assert!(
            lag > EDGE_BAR_PX,
            "frozen history: edge at {ghost_edge:.3} px lags {lag:.3} px past the bar \
             {EDGE_BAR_PX} px (truth {truth:.3} px)"
        );
        let trail = vacated_luma(&frozen[last], FRAMES - 1);
        assert!(
            trail > GHOST_MAX_TRAIL,
            "frozen history: vacated luma {trail:.3} must exceed the ghost gate \
             {GHOST_MAX_TRAIL}"
        );
        // Frozen history with the default clamp: the box clamp trims the
        // trail to one column at the old edge, and that column still misses
        // the gate by the full block contrast.
        let clamped = run_pan(&frozen_motion(), TaaParams::default());
        let error = max_luma_error(&clamped[last], &block_frame(FRAMES - 1));
        assert!(
            error > GHOST_MAX_TRAIL,
            "frozen history under the clamp: worst pixel misses the current frame by \
             {error:.3}, over the ghost gate {GHOST_MAX_TRAIL}"
        );
        assert_ne!(
            registered[last], clamped[last],
            "the flow field must change the resolve, or this test cannot fail"
        );
    }

    #[test]
    fn taa_cut_resets_the_history_so_no_old_shot_smears() {
        // A camera cut flips the registry flag; the resolve must then follow
        // the new frame even with the old shot's pixels handed in.
        let mut reg = TemporalRegistry::new();
        let desc = HistoryDesc {
            width: W,
            height: H,
            bytes_per_texel: 8,
        };
        let first = reg.take("taa", desc).expect("first take");
        assert!(!first.history_valid, "a first take has no history");
        reg.export("taa").expect("export");
        reg.end_frame().expect("frame end");
        assert!(
            reg.history_valid("taa"),
            "the export makes the history valid"
        );
        reg.camera_cut();
        let cut = reg.take("taa", desc).expect("take after the cut");
        assert!(
            !cut.history_valid,
            "a camera cut leaves the old pixels unregistered"
        );
        let current = block_frame(1);
        let stale = block_frame(0);
        let frozen = frozen_motion();
        let params = TaaParams::default().with_history(cut.history_valid);
        let out = taa_resolve(&current, &stale, &frozen, W, H, &params).expect("cut resolve runs");
        assert_eq!(out, current, "the first frame after a cut keeps no history");
        // The same pixels smear when the flag still says the history is valid.
        let smeared = taa_resolve(&current, &stale, &frozen, W, H, &TaaParams::default())
            .expect("valid resolve runs");
        assert_ne!(
            smeared, current,
            "stale history with a valid flag must smear, or this test cannot fail"
        );
    }

    /// The clamp box is built and applied in YCoCg, not RGB. A red/green
    /// saturated edge separates the two: a per-channel RGB clamp lets a white
    /// history sample through as (1, 1, 0), while YCoCg clamps luma to the
    /// edge's 0.5 ceiling and both chroma axes to zero, giving (0.5, 0.5, 0.5).
    #[test]
    fn taa_neighbourhood_clamp_runs_in_ycocg_at_a_saturated_edge() {
        // Working-space constants, the same convention as
        // `shaders/taa_present.wgsl`.
        assert_eq!(to_ycocg([1.0, 0.0, 0.0]), [0.25, 0.5, -0.25]);
        assert_eq!(to_ycocg([0.0, 1.0, 0.0]), [0.5, 0.0, 0.5]);
        let grey = [0.3, 0.8, 0.1];
        let back = from_ycocg(to_ycocg(grey));
        for c in 0..3 {
            assert!(
                (back[c] - grey[c]).abs() < 1e-6,
                "conversion round trip {back:?} drifts from {grey:?}"
            );
        }

        let (w, h) = (3u32, 3u32);
        let red = [1.0, 0.0, 0.0];
        let green = [0.0, 1.0, 0.0];
        let current: Vec<[f32; 3]> = (0..9)
            .map(|i| if i % 2 == 0 { red } else { green })
            .collect();
        let history = vec![[1.0, 1.0, 1.0]; 9];
        let motion = MotionField::zero(w, h).expect("zero motion builds");
        let params = TaaParams {
            history_blend: 1.0,
            ..TaaParams::default()
        };
        let out = taa_resolve(&current, &history, &motion, w, h, &params).expect("resolve runs");

        // The box the sample is clamped to is the YCoCg one.
        let (lo, hi) = neighborhood_minmax(&current, w, h, 1, 1);
        assert_eq!(lo, [0.25, 0.0, -0.25], "lo is not the YCoCg box");
        assert_eq!(hi, [0.5, 0.5, 0.5], "hi is not the YCoCg box");

        // What the old per-channel RGB box would have returned, computed here
        // so the difference is not just a hardcoded claim.
        let mut rgb_lo = [f32::INFINITY; 3];
        let mut rgb_hi = [f32::NEG_INFINITY; 3];
        for p in &current {
            for c in 0..3 {
                rgb_lo[c] = rgb_lo[c].min(p[c]);
                rgb_hi[c] = rgb_hi[c].max(p[c]);
            }
        }
        let rgb_clamped = [
            1.0f32.clamp(rgb_lo[0], rgb_hi[0]),
            1.0f32.clamp(rgb_lo[1], rgb_hi[1]),
            1.0f32.clamp(rgb_lo[2], rgb_hi[2]),
        ];
        assert_eq!(rgb_clamped, [1.0, 1.0, 0.0], "RGB box expectation");

        // Full history weight and zero motion: the resolve is the clamped
        // sample, so every pixel shows the space the clamp ran in.
        let ycocg_clamp = [0.5, 0.5, 0.5];
        for (i, p) in out.iter().enumerate() {
            for c in 0..3 {
                assert!(
                    (p[c] - ycocg_clamp[c]).abs() < 1e-6,
                    "pixel {i} is {p:?}, not the YCoCg clamp {ycocg_clamp:?}"
                );
            }
        }
        assert_ne!(
            out[4], rgb_clamped,
            "the clamp returned the RGB box value {rgb_clamped:?}: it is not YCoCg"
        );

        // Without the clamp the white history comes through, so the clamp is
        // what moved this pixel.
        let raw = taa_resolve(
            &current,
            &history,
            &motion,
            w,
            h,
            &TaaParams {
                use_clamp: false,
                ..params
            },
        )
        .expect("unclamped resolve runs");
        assert_eq!(raw[4], [1.0, 1.0, 1.0], "no clamp keeps the raw sample");

        // A hot red sample separates the two halves of the clamp: in YCoCg it
        // keeps its luma and lands on the chroma box edge as (1.25, 0.25,
        // 0.25), while per-channel clamping against the same numbers gives the
        // flat (0.5, 0, 0). The white sample above reads the same either way,
        // so this one catches a YCoCg min/max with an RGB clamp.
        let hot = vec![[2.0, 0.0, 0.0]; 9];
        let hot_out =
            taa_resolve(&current, &hot, &motion, w, h, &params).expect("hot history resolves");
        let ycocg_hot = [1.25, 0.25, 0.25];
        for (i, p) in hot_out.iter().enumerate() {
            for c in 0..3 {
                assert!(
                    (p[c] - ycocg_hot[c]).abs() < 1e-6,
                    "pixel {i} is {p:?}, not the YCoCg clamp of the hot sample {ycocg_hot:?}"
                );
            }
        }
        assert_ne!(
            hot_out[4],
            [0.5, 0.0, 0.0],
            "the hot sample was clamped per channel: the clamp is not YCoCg"
        );
    }

    #[test]
    fn taa_resize_resets_the_history() {
        // A new size is a new texture: its old-size pixels do not map onto
        // the new frame, so the first resolve after it must read nothing.
        let mut reg = TemporalRegistry::new();
        let desc = HistoryDesc {
            width: W,
            height: H,
            bytes_per_texel: 8,
        };
        reg.take("taa", desc).expect("take");
        reg.export("taa").expect("export");
        reg.end_frame().expect("frame end");
        assert!(reg.history_valid("taa"));
        let smaller = HistoryDesc {
            width: W / 2,
            height: H / 2,
            ..desc
        };
        let resized = reg.take("taa", smaller).expect("take at the new size");
        assert!(resized.recreated, "a new size is a new texture");
        assert!(
            !resized.history_valid,
            "a resized history does not map onto the new pixels"
        );
        let side = (W / 2) * (H / 2);
        let current = vec![[0.3, 0.4, 0.5]; side as usize];
        let stale = vec![[9.0, 9.0, 9.0]; side as usize];
        let motion = MotionField::zero(W / 2, H / 2).expect("zero motion builds");
        let params = TaaParams::default().with_history(resized.history_valid);
        let out = taa_resolve(&current, &stale, &motion, W / 2, H / 2, &params)
            .expect("resize resolve runs");
        assert_eq!(
            out, current,
            "no old-size content on the resized first frame"
        );
    }
}
