//! heat_shimmer: a half-res heat ripple over the brazier and the hot-sand
//! patches only (QA9), with a soft mask and a strength cap.
//!
//! Owner: render lane. Presentation only, never hashed. Units: texels for the
//! grid and the offsets, `0..1` for the mask share.
//!
//! Shape: a soft mask around a few heat sources (the brazier plus the hot-sand
//! patches), then a sinusoidal uv nudge inside that mask, sampled from the full
//! frame and written at half resolution. Off at the potato tier: an iGPU pays
//! nothing for a look term it cannot see at 720p.
//!
//! `apply` is the CPU parity oracle of `shaders/heat_shimmer.wgsl`; both walk
//! the same mask and the same phase, so a frame can be checked headless.
//! Determinism: the phase is the caller's frame number, never a wall clock.

use std::error::Error;
use std::fmt;

/// Mask grid divisor: half the frame on each axis.
pub const HALF: u32 = 2;

/// Offsets are clamped to this many texels whatever the strength asks for.
pub const CAP_TEXELS: f32 = 4.0;

/// Phase steps per frame: fast enough to read as heat, slow enough to be calm.
pub const PHASE_STEPS: f32 = 9.0;

/// Heat sources one frame may carry (the brazier plus three sand patches).
pub const SOURCES_MAX: usize = 4;

/// One heat source: a centre in half-res texels and a soft radius.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct HeatSource {
    /// Centre in half-res texels.
    pub center_px: [f32; 2],
    /// Soft radius in half-res texels; the share falls to 0 at this edge.
    pub radius_px: f32,
}

/// The shimmer pass for one frame: the half-res grid, the sources, the strength
/// and the frame phase.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ShimmerConfig {
    /// Half-res grid width in texels (`ceil(frame_w / HALF)`).
    pub half_w: u32,
    /// Half-res grid height in texels.
    pub half_h: u32,
    /// Offset scale at full mask, in texels.
    pub strength_px: f32,
    /// Hard clamp on the offset, in texels (`<= CAP_TEXELS`).
    pub cap_px: f32,
    /// Frame phase: the caller's frame number, never a clock.
    pub phase: u64,
    /// Heat sources; `None` slots are empty.
    pub sources: [Option<HeatSource>; SOURCES_MAX],
}

impl Default for ShimmerConfig {
    fn default() -> Self {
        Self {
            half_w: 0,
            half_h: 0,
            strength_px: 1.5,
            cap_px: 2.0,
            phase: 0,
            sources: [None; SOURCES_MAX],
        }
    }
}

impl ShimmerConfig {
    /// Pass off: no grid, no offset, every pixel untouched.
    #[must_use]
    pub fn off() -> Self {
        Self {
            sources: [None; SOURCES_MAX],
            strength_px: 0.0,
            ..Self::default()
        }
    }

    /// Grid for a frame at `w x h`, with no source yet.
    #[must_use]
    pub fn for_frame(w: u32, h: u32) -> Self {
        let (half_w, half_h) = half_dims(w, h);
        Self {
            half_w,
            half_h,
            ..Self::default()
        }
    }

    /// True when the pass draws nothing.
    #[must_use]
    pub fn is_off(&self) -> bool {
        self.strength_px <= 0.0 || self.source_count() == 0
    }

    /// Live sources this frame.
    #[must_use]
    pub fn source_count(&self) -> usize {
        self.sources.iter().flatten().count()
    }

    /// Validate every field before a frame runs.
    pub fn validate(&self) -> Result<(), ShimmerError> {
        if !self.is_off() && (self.half_w == 0 || self.half_h == 0) {
            return Err(ShimmerError::BadGrid {
                w: self.half_w,
                h: self.half_h,
            });
        }
        for (field, value) in [("strength_px", self.strength_px), ("cap_px", self.cap_px)] {
            if !value.is_finite() || value < 0.0 {
                return Err(ShimmerError::BadNumber { field, value });
            }
        }
        for slot in self.sources.iter().flatten() {
            if !slot.center_px.iter().all(|c| c.is_finite())
                || !slot.radius_px.is_finite()
                || slot.radius_px <= 0.0
            {
                return Err(ShimmerError::BadNumber {
                    field: "source radius",
                    value: slot.radius_px,
                });
            }
        }
        Ok(())
    }
}

/// Half-res grid for a frame: half on each axis, rounded up.
#[must_use]
pub fn half_dims(w: u32, h: u32) -> (u32, u32) {
    (w.div_ceil(HALF), h.div_ceil(HALF))
}

/// Soft mask share at one half-res texel: 1 at a source centre, 0 at or past
/// its radius, `smoothstep` between, and the strongest source wins. A NaN point
/// reads 0 (fail closed).
#[must_use]
pub fn mask_at(px: [f32; 2], cfg: &ShimmerConfig) -> f32 {
    if !px[0].is_finite() || !px[1].is_finite() {
        return 0.0;
    }
    let mut best = 0.0f32;
    for slot in cfg.sources.iter().flatten() {
        let dx = px[0] - slot.center_px[0];
        let dy = px[1] - slot.center_px[1];
        let d = (dx * dx + dy * dy).sqrt();
        if !d.is_finite() {
            continue;
        }
        let t = (1.0 - d / slot.radius_px).clamp(0.0, 1.0);
        best = best.max(t * t * (3.0 - 2.0 * t));
    }
    best
}

/// Offset in uv at one half-res texel: two out-of-phase sines scaled by the mask
/// and the strength, then clamped to the cap. No mask, no offset.
#[must_use]
pub fn shimmer_offset_uv(px: [f32; 2], mask: f32, cfg: &ShimmerConfig) -> [f32; 2] {
    if mask <= 0.0 || cfg.is_off() || cfg.half_w == 0 || cfg.half_h == 0 {
        return [0.0, 0.0];
    }
    let phase = phase_radians(cfg.phase);
    let scale = texel_uv(cfg);
    let strength = cfg.strength_px.min(cfg.cap_px.min(CAP_TEXELS));
    let wob_x = (px[0] * 0.35 + phase).sin();
    let wob_y = ((px[1] * 0.27 + phase * 1.31).cos()) * 0.7;
    let off = [wob_x * strength * mask, wob_y * strength * mask];
    [off[0] * scale[0], off[1] * scale[1]]
}

/// Frame phase in radians: the caller's frame number times the per-frame step.
fn phase_radians(frame: u64) -> f32 {
    let whole = f64::from(u32::try_from(frame % 1024).unwrap_or(0));
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the phase stays under 1024 * 9 radians and is used as f32"
    )]
    let phase = (whole * f64::from(PHASE_STEPS)) as f32;
    phase
}

/// One half-res texel as a uv step on the full frame.
fn texel_uv(cfg: &ShimmerConfig) -> [f32; 2] {
    let w = cfg.half_w.max(1);
    let h = cfg.half_h.max(1);
    #[expect(
        clippy::cast_precision_loss,
        reason = "half-res grid edges are frame sizes, exact in f32"
    )]
    let step = [0.5 / (w as f32), 0.5 / (h as f32)];
    step
}

/// Bilinear sample of the frame at `uv`, clamped to the edge.
fn sample(frame: &[[f32; 3]], w: u32, h: u32, uv: [f32; 2]) -> Option<[f32; 3]> {
    if w == 0 || h == 0 || !uv[0].is_finite() || !uv[1].is_finite() {
        return None;
    }
    #[expect(clippy::cast_precision_loss, reason = "frame edges are exact in f32")]
    let (fw, fh) = (w as f32, h as f32);
    let x = (uv[0].clamp(0.0, 1.0) * (fw - 1.0)).max(0.0);
    let y = (uv[1].clamp(0.0, 1.0) * (fh - 1.0)).max(0.0);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the coordinates are clamped to 0..edge before the floor"
    )]
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let at = |px: u32, py: u32| -> Option<[f32; 3]> {
        let i = u64::from(py) * u64::from(w) + u64::from(px);
        frame.get(usize::try_from(i).ok()?).copied()
    };
    let (c00, c10, c01, c11) = (at(x0, y0)?, at(x1, y0)?, at(x0, y1)?, at(x1, y1)?);
    let (tx, ty) = (x - x0_f32(x0), y - x0_f32(y0));
    let mut out = [0.0f32; 3];
    for c in 0..3 {
        let top = c00[c] + (c10[c] - c00[c]) * tx;
        let bot = c01[c] + (c11[c] - c01[c]) * tx;
        out[c] = top + (bot - top) * ty;
    }
    Some(out)
}

/// One texel index as `f32` for the bilinear weights.
fn x0_f32(px: u32) -> f32 {
    #[expect(
        clippy::cast_precision_loss,
        reason = "texel indices are frame sizes, exact in f32"
    )]
    let v = px as f32;
    v
}

/// Run the pass over a frame: every pixel outside the mask keeps its exact
/// bytes, every pixel inside resamples the frame at the nudged uv. Returns the
/// frame and the half-res texel count the receipt posts.
#[must_use]
pub fn apply(frame: Vec<[f32; 3]>, w: u32, h: u32, cfg: &ShimmerConfig) -> (Vec<[f32; 3]>, u64) {
    if cfg.is_off() || w == 0 || h == 0 {
        return (frame, 0);
    }
    let (half_w, half_h) = half_dims(w, h);
    let mut out = Vec::with_capacity(frame.len());
    for y in 0..h {
        for x in 0..w {
            let px = [x0_f32(x / HALF), x0_f32(y / HALF)];
            let mask = mask_at(px, cfg);
            let off = shimmer_offset_uv(px, mask, cfg);
            let idx =
                usize::try_from(u64::from(y) * u64::from(w) + u64::from(x)).unwrap_or(usize::MAX);
            let Some(base) = frame.get(idx).copied() else {
                continue;
            };
            if off[0] == 0.0 && off[1] == 0.0 {
                out.push(base);
                continue;
            }
            let uv = [
                x0_f32(x) / x0_f32(w.max(1)) + off[0],
                x0_f32(y) / x0_f32(h.max(1)) + off[1],
            ];
            out.push(sample(&frame, w, h, uv).unwrap_or(base));
        }
    }
    let texels = u64::from(half_w) * u64::from(half_h);
    (out, texels)
}

/// True when a tier runs the pass: off on the potato row, on everywhere else.
#[must_use]
pub const fn runs_for(q: crate::quality::Quality) -> bool {
    !matches!(q, crate::quality::Quality::Potato)
}

/// Shimmer failures; every variant names the bad input.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ShimmerError {
    /// The half-res grid is empty while the pass is on.
    BadGrid {
        /// Grid width in texels.
        w: u32,
        /// Grid height in texels.
        h: u32,
    },
    /// A constant is negative or not finite.
    BadNumber {
        /// Field name.
        field: &'static str,
        /// Rejected value.
        value: f32,
    },
}

impl fmt::Display for ShimmerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadGrid { w, h } => write!(f, "shimmer grid {w}x{h} is empty"),
            Self::BadNumber { field, value } => {
                write!(f, "shimmer {field} is unusable: {value}")
            }
        }
    }
}

impl Error for ShimmerError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality::Quality;

    fn brazier(cfg: &mut ShimmerConfig) {
        cfg.sources[0] = Some(HeatSource {
            center_px: [4.0, 4.0],
            radius_px: 3.0,
        });
    }

    #[test]
    fn shimmer_off_at_potato_and_on_elsewhere() {
        assert!(!runs_for(Quality::Potato), "the potato row pays nothing");
        for q in [
            Quality::Mobile,
            Quality::Web,
            Quality::Comfort,
            Quality::High,
        ] {
            assert!(runs_for(q), "{} runs the shimmer", q.name());
        }
        let mut cfg = ShimmerConfig::for_frame(16, 16);
        assert!(cfg.is_off(), "no source is off");
        brazier(&mut cfg);
        assert!(!cfg.is_off(), "one source turns it on");
        assert_eq!(cfg.source_count(), 1);
        cfg.validate().expect("the brazier config is valid");
    }

    #[test]
    fn shimmer_half_grid_is_a_quarter_of_the_texels() {
        let (w, h) = half_dims(1280, 720);
        assert_eq!((w, h), (640, 360), "half on each axis");
        assert_eq!(u64::from(w) * u64::from(h) * 4, 1280 * 720, "a quarter");
        assert_eq!(half_dims(1279, 719), (640, 360), "edges keep their texel");
    }

    #[test]
    fn shimmer_mask_only_touches_the_brazier() {
        let (w, h) = (64u32, 32u32);
        // A gradient frame, so a shifted sample reads differently from the base.
        let frame: Vec<[f32; 3]> = (0..w * h)
            .map(|i| {
                [
                    0.20 + f32::from(u16::try_from(i % 8).expect("fits")) * 0.01,
                    0.5,
                    0.75,
                ]
            })
            .collect();
        let mut cfg = ShimmerConfig::for_frame(w, h);
        brazier(&mut cfg);
        cfg.phase = 7;
        let (out, texels) = apply(frame.clone(), w, h, &cfg);
        assert_eq!(texels, 32 * 16, "half-res texels ride the receipt");
        let mut changed = 0;
        for (i, (a, b)) in out.iter().zip(frame.iter()).enumerate() {
            if a != b {
                changed += 1;
                let x = u32::try_from(i).expect("index fits") % w;
                let y = u32::try_from(i).expect("index fits") / w;
                assert!(x < 16 && y < 16, "only the brazier corner moves: {x},{y}");
            }
        }
        assert!(changed > 0, "the brazier moves");
        // An off config hands the frame back byte-identical with no texels.
        let (same, none) = apply(frame.clone(), w, h, &ShimmerConfig::off());
        assert_eq!(none, 0);
        assert_eq!(same, frame);
    }

    #[test]
    fn shimmer_strength_caps_the_offset() {
        let mut cfg = ShimmerConfig::for_frame(64, 64);
        brazier(&mut cfg);
        cfg.strength_px = 100.0;
        cfg.cap_px = 2.0;
        let off = shimmer_offset_uv([4.0, 4.0], 1.0, &cfg);
        // Cap 2 texels on a 32x32 half grid is 2 * 0.5 / 32 uv.
        let bar = 2.0 * 0.5 / 32.0;
        assert!(off[0].abs() <= bar + 1e-6, "x off {}", off[0]);
        assert!(off[1].abs() <= bar + 1e-6, "y off {}", off[1]);
        // Zero mask and zero strength both leave the uv alone.
        assert_eq!(shimmer_offset_uv([4.0, 4.0], 0.0, &cfg), [0.0, 0.0]);
        cfg.strength_px = 0.0;
        assert_eq!(shimmer_offset_uv([4.0, 4.0], 1.0, &cfg), [0.0, 0.0]);
    }
}
