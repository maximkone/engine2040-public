//! post_dof: the finisher beat's depth of field (SW173), half resolution.
//!
//! While `FinisherBeat` holds the strike, the frame focuses on the impact
//! point: the circle of confusion (CoC) of a pixel grows with how far its view
//! depth sits from the focus depth, so the crowd and the berm fall out of
//! focus and the hit lands. Off at `Potato` and off outside the beat, so the
//! arena pays nothing for a look it never shows.
//!
//! Two half-res dispatches, the shape every cheap DOF uses: (1) `coc` writes
//! one radius per half texel from the depth, (2) `gather` blends a fixed disc
//! of taps whose own CoC reaches the sample distance (scatter-as-gather, so a
//! sharp foreground never smears onto a blurred background). The CPU twin
//! below is the oracle the WGSL answers to, and its tests are the row's
//! accept numbers.
//!
//! Owner: render. Units: metres for view depth and focus, pixels for sizes
//! and radii. View only, never hashed. Determinism: row-major integer orders,
//! one fixed tap table, f32 only, no clock, no map, no thread.

use crate::post::PostError;
use crate::quality::Quality;

/// Full-to-half resolution divisor: the pass runs at `ceil(w/2) x ceil(h/2)`
/// except on the top tiers, where the gather still runs at half but reads the
/// full-res depth.
pub const DOF_HALF_DIV: u32 = 2;

/// Largest CoC radius the gather reads, in half-res pixels: a bigger radius
/// costs taps for pixels the eye already reads as fully soft.
pub const DOF_MAX_COC_PX: f32 = 8.0;

/// CoC radius the preset asks for at 1 m behind the focus, before the clamp.
pub const DOF_APERTURE_PX: f32 = 4.0;

/// Taps in the gather disc: one centre plus two rings of six.
pub const DOF_TAPS: usize = 13;

/// Depth range the configuration accepts for a focus or a sample, in metres.
pub const DOF_MAX_DEPTH_M: f32 = 1000.0;

/// Depth of field configuration: frame size, focus depth and aperture.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct DofConfig {
    /// Full-res frame width in pixels.
    pub width_px: u32,
    /// Full-res frame height in pixels.
    pub height_px: u32,
    /// View depth of the focus plane in metres (the impact point's depth).
    pub focus_m: f32,
    /// CoC radius at one metre behind the focus, in half-res pixels.
    pub aperture_px: f32,
    /// Radius clamp, in half-res pixels.
    pub max_coc_px: f32,
}

impl DofConfig {
    /// The preset: 4 px of CoC at a metre, clamped at [`DOF_MAX_COC_PX`].
    #[must_use]
    pub const fn focused(width_px: u32, height_px: u32, focus_m: f32) -> Self {
        Self {
            width_px,
            height_px,
            focus_m,
            aperture_px: DOF_APERTURE_PX,
            max_coc_px: DOF_MAX_COC_PX,
        }
    }

    /// Validate sizes, focus and aperture before any allocation.
    pub fn validate(&self) -> Result<(), PostError> {
        if self.width_px == 0 || self.height_px == 0 {
            return Err(PostError::Invalid(format!(
                "dof frame {}x{} is zero",
                self.width_px, self.height_px
            )));
        }
        if self.width_px > crate::post::MAX_POST_DIM || self.height_px > crate::post::MAX_POST_DIM {
            return Err(PostError::Invalid(format!(
                "dof frame {}x{} exceeds {}",
                self.width_px,
                self.height_px,
                crate::post::MAX_POST_DIM
            )));
        }
        if !self.focus_m.is_finite() || self.focus_m <= 0.0 || self.focus_m > DOF_MAX_DEPTH_M {
            return Err(PostError::Invalid(format!(
                "dof focus {} outside (0, {DOF_MAX_DEPTH_M}]",
                self.focus_m
            )));
        }
        if !self.aperture_px.is_finite() || self.aperture_px <= 0.0 {
            return Err(PostError::Invalid(format!(
                "dof aperture {} is not positive finite",
                self.aperture_px
            )));
        }
        if !self.max_coc_px.is_finite() || self.max_coc_px <= 0.0 {
            return Err(PostError::Invalid(format!(
                "dof max CoC {} is not positive finite",
                self.max_coc_px
            )));
        }
        Ok(())
    }

    /// Half-res width in pixels (`ceil(w/2)`).
    #[must_use]
    pub fn half_width(&self) -> u32 {
        self.width_px.div_ceil(DOF_HALF_DIV)
    }

    /// Half-res height in pixels (`ceil(h/2)`).
    #[must_use]
    pub fn half_height(&self) -> u32 {
        self.height_px.div_ceil(DOF_HALF_DIV)
    }

    /// Texels one half-res dispatch writes.
    #[must_use]
    pub fn half_texels(&self) -> u64 {
        u64::from(self.half_width()) * u64::from(self.half_height())
    }

    /// Circle of confusion at `depth_m`, in half-res pixels.
    ///
    /// Thin-lens shape: `aperture * |depth - focus| / depth`, clamped to
    /// `max_coc_px`. A pixel on the focus plane is exactly `0.0`, and the
    /// radius saturates with distance instead of growing without bound.
    #[must_use]
    pub fn coc_px(&self, depth_m: f32) -> f32 {
        if !depth_m.is_finite() || depth_m <= 0.0 {
            return self.max_coc_px;
        }
        let coc = self.aperture_px * (depth_m - self.focus_m).abs() / depth_m;
        coc.min(self.max_coc_px)
    }
}

/// True when the pass runs: only while the finisher beat holds, and never on
/// `Potato` (the row's "off at Potato"). Pure, so the caller reads one rule.
#[must_use]
pub const fn dof_runs(beat_holds: bool, tier: Quality) -> bool {
    beat_holds && !matches!(tier, Quality::Potato)
}

/// The two dispatches the pass records: CoC texels and gather texels, both at
/// the half target.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct DofPassPlan {
    /// Half-res width in pixels.
    pub width_half: u32,
    /// Half-res height in pixels.
    pub height_half: u32,
    /// Workgroups of 8x8 for the CoC dispatch.
    pub coc_groups: [u32; 2],
    /// Workgroups of 8x8 for the gather dispatch.
    pub gather_groups: [u32; 2],
}

/// Plan the pass for a validated config: both dispatches cover the half
/// target in 8x8 workgroups, so the gather reads what the CoC dispatch wrote.
pub fn dof_pass_plan(config: &DofConfig) -> Result<DofPassPlan, PostError> {
    config.validate()?;
    let groups = |edge: u32| edge.div_ceil(8).max(1);
    let (w, h) = (config.half_width(), config.half_height());
    Ok(DofPassPlan {
        width_half: w,
        height_half: h,
        coc_groups: [groups(w), groups(h)],
        gather_groups: [groups(w), groups(h)],
    })
}

/// The half-res gather result: RGB in the same units the source colour is in.
#[derive(Clone, PartialEq, Debug)]
pub struct DofImage {
    /// Half-res width in pixels.
    pub width_half: u32,
    /// Half-res height in pixels.
    pub height_half: u32,
    /// Row-major RGB, three lanes per texel.
    pub rgb: Vec<f32>,
}

impl DofImage {
    /// One texel's RGB.
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a validated frame is under 2048x2048, so the index fits usize"
    )]
    pub fn texel(&self, x: u32, y: u32) -> Result<[f32; 3], PostError> {
        if x >= self.width_half || y >= self.height_half {
            return Err(PostError::Invalid(format!(
                "dof texel {x},{y} outside {}x{}",
                self.width_half, self.height_half
            )));
        }
        let i = ((y * self.width_half + x) * 3) as usize;
        Ok([self.rgb[i], self.rgb[i + 1], self.rgb[i + 2]])
    }
}

/// Tap offsets on the gather disc, in units of the CoC radius: one centre plus
/// six taps at 0.5 and six at 1.0. Fixed order, so twins agree to the bit.
const DOF_TAP: [(f32, f32); DOF_TAPS] = [
    (0.0, 0.0),
    (0.5, 0.0),
    (0.25, 0.433_012_7),
    (-0.25, 0.433_012_7),
    (-0.5, 0.0),
    (-0.25, -0.433_012_7),
    (0.25, -0.433_012_7),
    (1.0, 0.0),
    (0.5, 0.866_025_4),
    (-0.5, 0.866_025_4),
    (-1.0, 0.0),
    (-0.5, -0.866_025_4),
    (0.5, -0.866_025_4),
];

/// Half-res CoC plus gather over a full-res HDR frame and depth buffer.
///
/// `colour` is full-res RGBA (four lanes, sRGB-free working values) and
/// `depth` full-res view depth in metres. Each half texel reads the full-res
/// pixel at twice its coordinates; a texel whose CoC is under half a pixel
/// copies its colour exactly, so the focus plane is untouched.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a validated frame is under 2048x2048 and a tap is floored inside 0..=half-1"
)]
pub fn dof_half_res(
    colour: &[f32],
    depth: &[f32],
    config: &DofConfig,
) -> Result<DofImage, PostError> {
    config.validate()?;
    let full = (config.width_px as usize) * (config.height_px as usize);
    if colour.len() != full * 4 {
        return Err(PostError::Invalid(format!(
            "dof colour length {} mismatches {}x{} RGBA",
            colour.len(),
            config.width_px,
            config.height_px
        )));
    }
    if depth.len() != full {
        return Err(PostError::Invalid(format!(
            "dof depth length {} mismatches {}x{}",
            depth.len(),
            config.width_px,
            config.height_px
        )));
    }
    let (hw, hh) = (config.half_width(), config.half_height());
    let mut rgb = vec![0.0f32; (hw * hh * 3) as usize];
    let w = config.width_px as usize;
    for hy in 0..hh {
        for hx in 0..hw {
            let cx = (hx * DOF_HALF_DIV).min(config.width_px - 1) as usize;
            let cy = (hy * DOF_HALF_DIV).min(config.height_px - 1) as usize;
            let centre = &colour[(cy * w + cx) * 4..(cy * w + cx) * 4 + 3];
            let radius = config.coc_px(depth[cy * w + cx]);
            let out = (hy * hw + hx) as usize * 3;
            if radius < 0.5 {
                rgb[out..out + 3].copy_from_slice(centre);
                continue;
            }
            let mut acc = [0.0f32; 3];
            let mut weight = 0.0f32;
            for (dx, dy) in DOF_TAP {
                let sx = (hx as f32 + dx * radius).round();
                let sy = (hy as f32 + dy * radius).round();
                if sx < 0.0 || sy < 0.0 {
                    continue;
                }
                let (sx, sy) = (sx as u32, sy as u32);
                if sx >= hw || sy >= hh {
                    continue;
                }
                let fx = (sx * DOF_HALF_DIV).min(config.width_px - 1) as usize;
                let fy = (sy * DOF_HALF_DIV).min(config.height_px - 1) as usize;
                // Scatter-as-gather: a neighbour joins only when its own CoC
                // reaches this sample, so a sharp edge stays sharp.
                let reach = config.coc_px(depth[fy * w + fx]);
                let dist = ((dx * radius).powi(2) + (dy * radius).powi(2)).sqrt();
                if reach < dist {
                    continue;
                }
                let c = &colour[(fy * w + fx) * 4..(fy * w + fx) * 4 + 3];
                acc[0] += c[0];
                acc[1] += c[1];
                acc[2] += c[2];
                weight += 1.0;
            }
            if weight < 0.5 {
                rgb[out..out + 3].copy_from_slice(centre);
            } else {
                rgb[out] = acc[0] / weight;
                rgb[out + 1] = acc[1] / weight;
                rgb[out + 2] = acc[2] / weight;
            }
        }
    }
    Ok(DofImage {
        width_half: hw,
        height_half: hh,
        rgb,
    })
}

#[cfg(test)]
#[path = "post_dof_tests.rs"]
mod tests;
