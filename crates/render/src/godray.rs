//! godray: one quarter-res sun-shaft pass (SW91).
//!
//! Owner: render lane. Presentation only, never hashed. Units: texels for the
//! mask, 0..1 for shares, stops-free linear light for the added colour.
//!
//! Shape: bright-pass the frame into a `ceil(w/4) x ceil(h/4)` mask, then sum
//! `samples` taps from each pixel toward the sun's screen position. The
//! `sum_along_ray` below is the CPU parity oracle of `shaders/godray.wgsl`;
//! both walk the same loop so a frame can be checked headless.
//!
//! Donor (shape only, zero bytes pasted): GPU Gems 3 ch. 13, Listing 13-1
//! (`deltaTexCoord *= 1.0/NUM_SAMPLES * Density`, `illuminationDecay *= Decay`,
//! `sample *= illuminationDecay * Weight`, `return color * Exposure`).
//! PORT(NVIDIA GPU Gems 3, ch. 13 "Volumetric Light Scattering as a Post-Process").
//!
//! Errors: [`GodrayError`] names the bad field loudly; nothing panics.

use std::error::Error;
use std::fmt;

/// Mask divisor: the pass runs at a quarter of the frame on each axis.
pub const QUARTER: u32 = 4;

/// Taps on the potato tier: the pass still reads one texel per tap.
pub const SAMPLES_POTATO: u32 = 8;

/// Taps on every other tier.
pub const SAMPLES_REST: u32 = 16;

/// Luma above which a pixel feeds the mask (the emissive region the sun cuts).
pub const MASK_THRESHOLD: f32 = 0.75;

/// One quarter-res shaft pass: the mask grid plus the radial sum's constants.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GodrayConfig {
    /// Mask width in texels (`ceil(frame_w / QUARTER)`).
    pub quarter_w: u32,
    /// Mask height in texels (`ceil(frame_h / QUARTER)`).
    pub quarter_h: u32,
    /// Taps walked from the pixel toward the sun.
    pub samples: u32,
    /// Per-tap falloff multiplier, `0..1`.
    pub decay: f32,
    /// Per-tap weight, `>= 0`.
    pub weight: f32,
    /// Final gain on the summed colour, `>= 0`.
    pub exposure: f32,
    /// Step scale between taps, `0..=1`: 1 walks the full pixel-to-sun ray.
    pub density: f32,
    /// Luma a texel must pass to feed the mask, `0..1`.
    pub mask_threshold: f32,
    /// Sun position in mask texels, as the frame's sun lands on screen.
    pub sun_px: [f32; 2],
}

impl Default for GodrayConfig {
    fn default() -> Self {
        Self {
            quarter_w: 0,
            quarter_h: 0,
            samples: SAMPLES_POTATO,
            decay: 0.96,
            weight: 0.5,
            exposure: 1.0,
            density: 1.0,
            mask_threshold: MASK_THRESHOLD,
            sun_px: [0.0, 0.0],
        }
    }
}

impl GodrayConfig {
    /// Pass off: no mask, no taps, every pixel adds nothing.
    #[must_use]
    pub fn off() -> Self {
        Self {
            samples: 0,
            ..Self::default()
        }
    }

    /// Grid for a frame at `w x h` and `samples` taps per pixel.
    #[must_use]
    pub fn for_frame(w: u32, h: u32, samples: u32) -> Self {
        let (quarter_w, quarter_h) = quarter_dims(w, h);
        Self {
            quarter_w,
            quarter_h,
            samples,
            ..Self::default()
        }
    }

    /// True when the pass draws nothing: a zero tap count is the off switch.
    #[must_use]
    pub fn is_off(&self) -> bool {
        self.samples == 0
    }

    /// Validate every field before a frame runs.
    pub fn validate(&self) -> Result<(), GodrayError> {
        if self.quarter_w == 0 || self.quarter_h == 0 {
            return Err(GodrayError::BadSize {
                w: self.quarter_w,
                h: self.quarter_h,
            });
        }
        for (field, value) in [
            ("decay", self.decay),
            ("weight", self.weight),
            ("exposure", self.exposure),
            ("density", self.density),
            ("mask_threshold", self.mask_threshold),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(GodrayError::BadNumber { field, value });
            }
        }
        if self.decay > 1.0 || self.density > 1.0 || self.mask_threshold > 1.0 {
            return Err(GodrayError::BadNumber {
                field: "share over one",
                value: self.decay.max(self.density).max(self.mask_threshold),
            });
        }
        Ok(())
    }
}

/// Mask grid for a frame: quarter size on each axis, rounded up so the last
/// frame column and row keep a texel.
#[must_use]
pub fn quarter_dims(w: u32, h: u32) -> (u32, u32) {
    (w.div_ceil(QUARTER), h.div_ceil(QUARTER))
}

/// Share of a luma that feeds the mask: 0 at or under `threshold`, 1 at white,
/// linear between. A NaN or negative luma feeds nothing (fail closed).
#[must_use]
pub fn mask_share(luma: f32, threshold: f32) -> f32 {
    if !luma.is_finite() || luma <= 0.0 {
        return 0.0;
    }
    let t = threshold.clamp(0.0, 1.0);
    let span = 1.0 - t;
    if span <= 0.0 {
        return if luma >= 1.0 { 1.0 } else { 0.0 };
    }
    ((luma - t) / span).clamp(0.0, 1.0)
}

/// True when the sun's clip-space position lands on screen: in front of the
/// eye (`w > 0`) and inside `0..=1` on both axes. Anything else skips the pass.
#[must_use]
pub fn sun_on_screen(sun_clip: [f32; 4]) -> bool {
    if !sun_clip.iter().all(|c| c.is_finite()) {
        return false;
    }
    let w = sun_clip[3];
    if w <= 0.0 {
        return false;
    }
    let u = sun_clip[0] / w * 0.5 + 0.5;
    let v = 0.5 - sun_clip[1] / w * 0.5;
    (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)
}

/// Sun position in mask texels from its clip-space point; `None` off screen.
#[must_use]
pub fn sun_mask_texel(sun_clip: [f32; 4], mask_w: u32, mask_h: u32) -> Option<[i32; 2]> {
    if !sun_on_screen(sun_clip) || mask_w == 0 || mask_h == 0 {
        return None;
    }
    let w = sun_clip[3];
    let u = sun_clip[0] / w * 0.5 + 0.5;
    let v = 0.5 - sun_clip[1] / w * 0.5;
    let x = texel_of(u, mask_w)?;
    let y = texel_of(v, mask_h)?;
    Some([x, y])
}

/// One mask texel from a `0..=1` coordinate: no wrapping, clamped to the edge.
fn texel_of(uv: f32, edge: u32) -> Option<i32> {
    let Ok(edge_u16) = u16::try_from(edge) else {
        return None;
    };
    if edge_u16 == 0 || !uv.is_finite() {
        return None;
    }
    let scaled = (uv * f32::from(edge_u16)).floor();
    let last = i32::from(edge_u16) - 1;
    Some(clamp_i32(scaled, last))
}

/// Rounded coordinate in `0..=max`, clamped before the one narrowing cast.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the value is clamped to 0..=65535 before the cast"
)]
fn clamp_i32(v: f32, max: i32) -> i32 {
    if !v.is_finite() {
        return 0;
    }
    let bound = f32::from(u16::try_from(max.clamp(0, 65535)).unwrap_or(0));
    v.round().clamp(0.0, bound) as i32
}

/// An integer coordinate as `f32` for the ray walk.
#[expect(
    clippy::cast_precision_loss,
    reason = "mask coordinates stay far inside f32's exact integer range"
)]
fn to_f32(v: i32) -> f32 {
    v as f32
}

/// One mask tap: out-of-range texels read empty, so a ray leaves the frame
/// without wrapping.
fn tap(mask: &[f32], w: u32, h: u32, x: i32, y: i32) -> f32 {
    let (Ok(xi), Ok(yi), Ok(wi)) = (u32::try_from(x), u32::try_from(y), Ok::<u32, ()>(w)) else {
        return 0.0;
    };
    if xi >= w || yi >= h {
        return 0.0;
    }
    let Some(row) = wi.checked_mul(yi) else {
        return 0.0;
    };
    let Some(at) = row.checked_add(xi) else {
        return 0.0;
    };
    mask.get(usize::try_from(at).unwrap_or(usize::MAX))
        .copied()
        .filter(|v| v.is_finite())
        .unwrap_or(0.0)
}

/// Sum the shaft along one pixel's ray toward the sun: the CPU oracle of
/// `shaders/godray.wgsl` `ray_sum`. A pixel with no bright texel on its ray
/// adds exactly zero.
#[must_use]
pub fn sum_along_ray(
    mask: &[f32],
    w: u32,
    h: u32,
    sun_px: [i32; 2],
    px: [i32; 2],
    cfg: &GodrayConfig,
) -> f32 {
    if cfg.samples == 0 || mask.is_empty() {
        return 0.0;
    }
    let count = i32::try_from(cfg.samples).unwrap_or(1).max(1);
    let divisor = to_f32(count);
    let step = [
        (to_f32(px[0]) - to_f32(sun_px[0])) / divisor * cfg.density,
        (to_f32(px[1]) - to_f32(sun_px[1])) / divisor * cfg.density,
    ];
    let mut decay = 1.0f32;
    let mut sum = 0.0f32;
    let mut qx = to_f32(px[0]);
    let mut qy = to_f32(px[1]);
    for _ in 0..count {
        qx -= step[0];
        qy -= step[1];
        let x = clamp_i32(qx, i32::try_from(w).unwrap_or(0));
        let y = clamp_i32(qy, i32::try_from(h).unwrap_or(0));
        // The mask already holds bright-pass shares; the ray sums them as-is.
        let share = tap(mask, w, h, x, y);
        sum += share * decay * cfg.weight;
        decay *= cfg.decay;
    }
    sum * cfg.exposure
}

/// Quarter-res bright mask of a fogged HDR frame: each texel carries the
/// largest bright-pass share of its 4x4 block, so one emissive pixel still
/// feeds the shaft. Cost: one pass over the frame and the mask.
#[must_use]
pub fn bright_mask(frame: &[[f32; 3]], w: u32, h: u32, cfg: &GodrayConfig) -> Vec<f32> {
    let (mask_w, mask_h) = quarter_dims(w, h);
    let mut mask =
        vec![0.0f32; usize::try_from(u64::from(mask_w) * u64::from(mask_h)).unwrap_or(0)];
    for y in 0..h {
        for x in 0..w {
            let Some(px) = frame.get(
                usize::try_from(u64::from(y) * u64::from(w) + u64::from(x)).unwrap_or(usize::MAX),
            ) else {
                continue;
            };
            let luma = 0.2126 * px[0] + 0.7152 * px[1] + 0.0722 * px[2];
            let share = mask_share(luma, cfg.mask_threshold);
            if share <= 0.0 {
                continue;
            }
            let (mx, my) = (x / QUARTER, y / QUARTER);
            let at =
                usize::try_from(u64::from(my) * u64::from(mask_w) + u64::from(mx)).unwrap_or(0);
            if let Some(slot) = mask.get_mut(at) {
                *slot = slot.max(share);
            }
        }
    }
    mask
}

/// Add the shafts over a fogged HDR frame, before exposure and the tone map.
/// Off returns the frame untouched, so a chain with godray off keeps its exact
/// bytes; on adds one radial sum per pixel to every channel.
#[must_use]
pub fn composite(frame: Vec<[f32; 3]>, w: u32, h: u32, cfg: &GodrayConfig) -> Vec<[f32; 3]> {
    if cfg.is_off() || w == 0 || h == 0 {
        return frame;
    }
    let mask = bright_mask(&frame, w, h, cfg);
    let (mask_w, mask_h) = quarter_dims(w, h);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the sun's mask texel is clamped to the mask edge first"
    )]
    let sun = [
        clamp_i32(cfg.sun_px[0], i32::try_from(mask_w).unwrap_or(0)),
        clamp_i32(cfg.sun_px[1], i32::try_from(mask_h).unwrap_or(0)),
    ];
    let mut out = Vec::with_capacity(frame.len());
    for (i, px) in frame.iter().enumerate() {
        let at = u64::try_from(i).unwrap_or(0);
        let x = u32::try_from(at % u64::from(w)).unwrap_or(0);
        let y = u32::try_from(at / u64::from(w)).unwrap_or(0);
        let px_mask = [
            i32::try_from(x / QUARTER).unwrap_or(0),
            i32::try_from(y / QUARTER).unwrap_or(0),
        ];
        let add = sum_along_ray(&mask, mask_w, mask_h, sun, px_mask, cfg);
        out.push([px[0] + add, px[1] + add, px[2] + add]);
    }
    out
}

/// What one frame runs: the mask grid, the tap count and whether the pass
/// draws at all (the sun is on screen and the config is on).#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GodrayPlan {
    /// Mask width in texels.
    pub mask_w: u32,
    /// Mask height in texels.
    pub mask_h: u32,
    /// Taps summed per pixel.
    pub samples: u32,
    /// True when the pass records this frame.
    pub runs: bool,
}

impl GodrayPlan {
    /// Plan for a frame: skip the pass when the config is off or the sun is
    /// behind the camera or off screen.
    pub fn for_frame(
        w: u32,
        h: u32,
        cfg: &GodrayConfig,
        sun_clip: [f32; 4],
    ) -> Result<Self, GodrayError> {
        let (mask_w, mask_h) = quarter_dims(w, h);
        let runs = !cfg.is_off() && sun_on_screen(sun_clip) && mask_w > 0 && mask_h > 0;
        Ok(Self {
            mask_w,
            mask_h,
            samples: cfg.samples,
            runs,
        })
    }

    /// Mask texels the pass reads: a sixteenth of the frame's texel count.
    #[must_use]
    pub fn mask_texels(&self) -> u64 {
        u64::from(self.mask_w) * u64::from(self.mask_h)
    }
}

/// Shaft failures; every variant names the bad input.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum GodrayError {
    /// The mask grid is empty.
    BadSize {
        /// Mask width in texels.
        w: u32,
        /// Mask height in texels.
        h: u32,
    },
    /// A constant is negative, over its bound, or not finite.
    BadNumber {
        /// Field name.
        field: &'static str,
        /// Rejected value.
        value: f32,
    },
}

impl fmt::Display for GodrayError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadSize { w, h } => write!(f, "godray mask {w}x{h} is empty"),
            Self::BadNumber { field, value } => {
                write!(f, "godray {field} is unusable: {value}")
            }
        }
    }
}

impl Error for GodrayError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn godray_quarter_mask_is_a_sixteenth_of_the_texels() {
        let (w, h) = quarter_dims(1280, 720);
        assert_eq!((w, h), (320, 180), "a quarter on each axis");
        let plan = GodrayPlan::for_frame(
            1280,
            720,
            &GodrayConfig::for_frame(1280, 720, 8),
            [0.0, 0.0, 0.5, 1.0],
        )
        .expect("plan builds");
        assert_eq!(plan.mask_texels(), 57_600, "320x180 texels");
        assert_eq!(
            plan.mask_texels() * 16,
            1280 * 720,
            "one sixteenth of the frame"
        );
        assert!(plan.runs, "an on-screen sun runs the pass");
        assert_eq!(plan.samples, 8, "potato taps");
        // An odd frame edge still keeps its last texel.
        assert_eq!(quarter_dims(1279, 719), (320, 180));
    }

    #[test]
    fn godray_occluded_pixel_adds_nothing() {
        let cfg = GodrayConfig::for_frame(64, 64, 8);
        let mask = vec![0.0f32; 16 * 16];
        assert_eq!(
            sum_along_ray(&mask, 16, 16, [8, 8], [1, 1], &cfg),
            0.0,
            "a dark ray adds exactly zero"
        );
        // An unoccluded ray adds exposure times the weight sum.
        let lit = vec![1.0f32; 16 * 16];
        let got = sum_along_ray(&lit, 16, 16, [8, 8], [1, 1], &cfg);
        let mut expect = 0.0f32;
        let mut decay = 1.0f32;
        for _ in 0..cfg.samples {
            expect += decay * cfg.weight;
            decay *= cfg.decay;
        }
        assert!(
            (got - expect * cfg.exposure).abs() < 1e-4,
            "lit ray {got} vs {expect}"
        );
        // And the off switch adds nothing whatever the mask holds.
        assert_eq!(
            sum_along_ray(&lit, 16, 16, [8, 8], [1, 1], &GodrayConfig::off()),
            0.0
        );
    }

    #[test]
    fn godray_sun_behind_the_camera_skips_the_pass() {
        let cfg = GodrayConfig::for_frame(1280, 720, 8);
        let behind = [0.0, 0.0, 0.5, -1.0];
        assert!(!sun_on_screen(behind), "w <= 0 is behind the eye");
        assert_eq!(sun_mask_texel(behind, 320, 180), None);
        let plan = GodrayPlan::for_frame(1280, 720, &cfg, behind).expect("plan builds");
        assert!(!plan.runs, "no pass for a sun behind the camera");
        // Off screen on x also skips.
        let side = [9.0, 0.0, 0.5, 1.0];
        assert!(!sun_on_screen(side));
        assert!(!GodrayPlan::for_frame(1, 1, &cfg, side).expect("plan").runs);
        // A centred sun lands mid-mask.
        assert_eq!(
            sun_mask_texel([0.0, 0.0, 0.5, 1.0], 320, 180),
            Some([160, 90])
        );
    }
}
