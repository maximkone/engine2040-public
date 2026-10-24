//! ssr: wet and polished stone reflects the arena, at half resolution (SW90).
//!
//! The reflection is a screen-space ray march over the depth the frame already
//! has: the reflecting pixel leaves along its reflected view ray, each step
//! reads one depth texel, and a sample that sits inside the thickness of the
//! scene geometry is the hit. Validity rides in the alpha channel
//! ([`resolve_blend`]), so a miss keeps the forward colour instead of rejecting
//! the pixel: the donor writes `vec4(colour, valid)` for that reason.
//!
//! Donors (ported, not copied): Godot
//! `servers/rendering/renderer_rd/shaders/effects/screen_space_reflection.glsl`
//! (the parametric-t march, `depth_tolerance`, validity in alpha, the Hi-Z
//! level step [`mip_offset`]) and three.js
//! `examples/jsm/shaders/SSRShader.js` (the fixed-step march whose step count is
//! `max(|xLen|,|yLen|)`, the thickness test and the distance fade).
//! [`shaders/ssr.wgsl`] mirrors this file: this one is the oracle the GPU pass
//! answers to, and its tests are the accept numbers.
//!
//! Owner: render. Units: pixels for sizes and screen positions, meters for
//! view depth, parametric `t` in `0..=1` along the clipped ray. View only, never
//! hashed. Determinism: row-major integer orders, one fixed step table
//! ([`SsrConfig::hit_t`]), f32 only, no map, no clock, no thread.

use crate::quality::Quality;
use crate::types::{LookError, MAX_LOOK_DIM};

/// Reflection level a tier runs: the `ssr` step of `quality::CUT_ORDER` finally
/// has a target to cut.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SsrLevel {
    /// No reflection pass (the `Potato` default: the cheapest row pays no
    /// second full-frame read).
    Off,
    /// `ceil(w/2) x ceil(h/2)`, 8 march taps, composited by the depth-aware
    /// bilateral upsample (`gtao::upsample_bilateral`).
    Half,
    /// Every frame pixel, 16 march taps.
    Full,
}

impl SsrLevel {
    /// Numeric level: 0, 1, 2. Used by pages and logs.
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        match self {
            Self::Off => 0,
            Self::Half => 1,
            Self::Full => 2,
        }
    }

    /// True when the pass runs at all.
    #[must_use]
    pub const fn enabled(self) -> bool {
        !matches!(self, Self::Off)
    }
}

/// The reflection level a tier runs: off on `Potato` (reflections are the
/// first thing its row cannot pay for), half on `Mobile`/`Web`/`Comfort`, and
/// full only on `High`. Wet stone on the mid rows still reflects, at a quarter
/// of the texels and half the taps.
#[must_use]
pub const fn level_for(tier: Quality) -> SsrLevel {
    match tier {
        Quality::Potato => SsrLevel::Off,
        Quality::Mobile | Quality::Web | Quality::Comfort => SsrLevel::Half,
        Quality::High => SsrLevel::Full,
    }
}

/// The march a tier runs at `width` x `height`, or `None` when its row leaves
/// the pass off. The half result is composited by
/// [`crate::gtao::upsample_bilateral`], which already weights by depth.
#[must_use]
pub const fn config_for(tier: Quality, width: u32, height: u32) -> Option<SsrConfig> {
    match level_for(tier) {
        SsrLevel::Off => None,
        SsrLevel::Half => Some(SsrConfig::half(width, height)),
        SsrLevel::Full => Some(SsrConfig::full(width, height)),
    }
}

/// Full-to-half resolution divisor: the half target is `ceil(w/2) x ceil(h/2)`.
pub const SSR_HALF_DIV: u32 = 2;

/// March steps on the half target (`Potato`, `Web`, `Mobile`): the row's 8 taps.
pub const SSR_STEPS_HALF: u32 = 8;

/// March steps on the full target: the row's 16 taps.
pub const SSR_STEPS_FULL: u32 = 16;

/// A pixel at or past this roughness mirrors nothing: wet stone is smooth, dry
/// stone has no reflection to march for (the row's 0.7 cut).
pub const SSR_ROUGHNESS_CUT: f32 = 0.7;

/// Thickness of the geometry a sample may sit inside and still count as a hit,
/// in meters (three.js `thickness` 0.018). Thin sheets (decals, cloth) stay
/// see-through rather than swallowing the ray.
pub const SSR_DEPTH_TOLERANCE_M: f32 = 0.018;

/// Default distance fade in parametric `t`: the reflection is gone at this
/// fraction of the ray, so a long march never smears the far side of the frame.
pub const SSR_DISTANCE_FADE: f32 = 0.8;

/// Default fade-in over this fraction of `t`, so the reflected image does not
/// stick to the reflecting surface (three.js curves the same way).
pub const SSR_CURVE_FADE_IN: f32 = 0.05;

/// Largest accepted step count; a tier row never asks for more.
pub const SSR_STEPS_MAX: u32 = 64;

/// Largest fade fraction accepted: 1.0 keeps the whole clipped ray.
pub const SSR_FADE_MAX: f32 = 1.0;

/// Reflection ray march configuration. Sizes are the full-res frame; the
/// target size and step count follow from whether the pass runs at half.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SsrConfig {
    /// Full-res frame width in pixels, `1..=MAX_LOOK_DIM`.
    pub width: u32,
    /// Full-res frame height in pixels, `1..=MAX_LOOK_DIM`.
    pub height: u32,
    /// March steps before the ray gives up (taps).
    pub steps: u32,
    /// Depth a sample may sit inside and still hit (m).
    pub depth_tolerance: f32,
    /// Parametric `t` where the reflection has faded out (`0..=1`).
    pub distance_fade: f32,
    /// Parametric `t` the reflection fades in over (`0..=1`).
    pub curve_fade_in: f32,
    /// Roughness at or past which a pixel mirrors nothing (`0..=1`).
    pub roughness_cut: f32,
}

impl SsrConfig {
    /// The full-res march: every frame pixel traces, [`SSR_STEPS_FULL`] taps.
    #[must_use]
    pub const fn full(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            steps: SSR_STEPS_FULL,
            depth_tolerance: SSR_DEPTH_TOLERANCE_M,
            distance_fade: SSR_DISTANCE_FADE,
            curve_fade_in: SSR_CURVE_FADE_IN,
            roughness_cut: SSR_ROUGHNESS_CUT,
        }
    }

    /// The half-res march the cheap tiers run: [`SSR_STEPS_HALF`] taps, and the
    /// target is `ceil(w/2) x ceil(h/2)`.
    #[must_use]
    pub const fn half(width: u32, height: u32) -> Self {
        Self {
            steps: SSR_STEPS_HALF,
            ..Self::full(width, height)
        }
    }

    /// Validate sizes and march constants before any allocation.
    pub fn validate(&self) -> Result<(), LookError> {
        if self.width == 0 || self.height == 0 {
            return Err(LookError::Invalid(format!(
                "ssr frame {}x{} is zero",
                self.width, self.height
            )));
        }
        if self.width > MAX_LOOK_DIM || self.height > MAX_LOOK_DIM {
            return Err(LookError::Invalid(format!(
                "ssr frame {}x{} exceeds {MAX_LOOK_DIM}",
                self.width, self.height
            )));
        }
        if self.steps == 0 || self.steps > SSR_STEPS_MAX {
            return Err(LookError::Invalid(format!(
                "ssr steps {} outside 1..={SSR_STEPS_MAX}",
                self.steps
            )));
        }
        if !self.depth_tolerance.is_finite() || self.depth_tolerance <= 0.0 {
            return Err(LookError::Invalid(format!(
                "ssr depth tolerance {} is not positive finite",
                self.depth_tolerance
            )));
        }
        if !self.distance_fade.is_finite()
            || self.distance_fade <= 0.0
            || self.distance_fade > SSR_FADE_MAX
        {
            return Err(LookError::Invalid(format!(
                "ssr distance fade {} outside (0, {SSR_FADE_MAX}]",
                self.distance_fade
            )));
        }
        if !self.curve_fade_in.is_finite()
            || self.curve_fade_in < 0.0
            || self.curve_fade_in > SSR_FADE_MAX
        {
            return Err(LookError::Invalid(format!(
                "ssr curve fade in {} outside 0..={SSR_FADE_MAX}",
                self.curve_fade_in
            )));
        }
        if !self.roughness_cut.is_finite() || self.roughness_cut <= 0.0 || self.roughness_cut > 1.0
        {
            return Err(LookError::Invalid(format!(
                "ssr roughness cut {} outside (0, 1]",
                self.roughness_cut
            )));
        }
        Ok(())
    }

    /// Half-res width in pixels (`ceil(w/2)`).
    #[must_use]
    pub fn half_width(&self) -> u32 {
        self.width.div_ceil(SSR_HALF_DIV)
    }

    /// Half-res height in pixels (`ceil(h/2)`).
    #[must_use]
    pub fn half_height(&self) -> u32 {
        self.height.div_ceil(SSR_HALF_DIV)
    }

    /// The target this config traces at: the half size when `half`, else the
    /// full frame.
    #[must_use]
    pub fn target(&self, half: bool) -> [u32; 2] {
        if half {
            [self.half_width(), self.half_height()]
        } else {
            [self.width, self.height]
        }
    }

    /// Texels the half target touches.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a validated frame edge is u32, which fits usize on every target"
    )]
    pub fn half_texels(&self) -> usize {
        self.half_width() as usize * self.half_height() as usize
    }

    /// Texels a full-res pass would touch.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        reason = "a validated frame edge is u32, which fits usize on every target"
    )]
    pub fn full_texels(&self) -> usize {
        self.width as usize * self.height as usize
    }

    /// Parametric position of march step `step`, depth-independent.
    ///
    /// The step advances `t` on its own, so a degenerate depth step (a depth
    /// that does not move, or an anchored sample) hits at the same `t` every
    /// run: twins of the same depth buffer agree to the bit. `step` past the
    /// last tap reads as the last tap, so an overrun never leaves `0..=1`.
    #[must_use]
    #[expect(
        clippy::cast_precision_loss,
        reason = "a step index and a tap count are far below 2^24, so f32 is exact"
    )]
    pub fn hit_t(&self, step: u32) -> f32 {
        let last = self.steps.saturating_sub(1);
        (step.min(last) + 1) as f32 / self.steps.max(1) as f32
    }

    /// True when this pixel traces a reflection at all: rough stone and pure
    /// dielectrics have none, so they cost no taps.
    #[must_use]
    pub fn pixel_runs(&self, roughness: f32, metalness: f32) -> bool {
        roughness.is_finite() && roughness < self.roughness_cut && metalness > 0.0
    }

    /// Alpha the reflection contributes at parametric `t` for `roughness`:
    /// `0.0` on a miss or a rough pixel, else the fade-in times the distance
    /// fade, both in `0..=1`. The donor writes this into alpha instead of
    /// rejecting the pixel.
    #[must_use]
    pub fn resolve_blend(&self, t: f32, roughness: f32) -> f32 {
        if !t.is_finite() || !self.pixel_runs(roughness, 1.0) {
            return 0.0;
        }
        let t = t.clamp(0.0, 1.0);
        let fade_in = if self.curve_fade_in <= 0.0 {
            1.0
        } else {
            (t / self.curve_fade_in).clamp(0.0, 1.0)
        };
        let fade_out = (1.0 - t / self.distance_fade).clamp(0.0, 1.0);
        (fade_in * fade_out).clamp(0.0, 1.0)
    }
}

/// Hi-Z level step for one march sample, Godot's `mip_offset = hit ? -1 : +1`:
/// a sample inside geometry coarsens the level (`-1` relative to the row's
/// numbering: the next step covers twice the screen), a sample in front of it
/// refines back toward level 0. Kept pure so the WGSL loop and the oracle's
/// test read one rule.
#[must_use]
pub const fn mip_offset(inside_geometry: bool) -> i32 {
    if inside_geometry {
        -1
    } else {
        1
    }
}

/// The level the next march sample reads, clamped to `0..=max_level`.
#[must_use]
#[expect(
    clippy::cast_possible_wrap,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "both sides are a Hi-Z level, clamped to 0..=max_level"
)]
pub fn next_mip_level(level: u32, inside_geometry: bool, max_level: u32) -> u32 {
    let next = level as i32 + mip_offset(inside_geometry);
    next.clamp(0, max_level.min(i32::MAX as u32) as i32) as u32
}

/// One screen-space reflection ray, in the reflecting pixel's frame.
///
/// `x_px`/`y_px` are the surface's screen position, `depth_m` its view depth;
/// the three direction lanes are how far the ray travels over parametric
/// `t = 1` (pixels for the screen lanes, meters for depth).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SsrRay {
    /// Surface pixel column.
    pub x_px: f32,
    /// Surface pixel row.
    pub y_px: f32,
    /// Surface view depth in meters.
    pub depth_m: f32,
    /// Columns traveled over the whole ray.
    pub dx_px: f32,
    /// Rows traveled over the whole ray (negative goes up the screen).
    pub dy_px: f32,
    /// Depth gained over the whole ray in meters.
    pub dz_m: f32,
}

/// What the march found. `hit == false` means "keep the forward colour": the
/// ray left the frame or ran out of taps in front of every surface.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SsrHit {
    /// True when a sample landed inside scene geometry.
    pub hit: bool,
    /// Parametric position of the hit ([`SsrConfig::hit_t`] of the last step).
    pub t: f32,
    /// Hit pixel column.
    pub x_px: u32,
    /// Hit pixel row.
    pub y_px: u32,
    /// Scene depth read at the hit (m); the clear depth on a miss.
    pub depth_m: f32,
    /// Steps the march walked, never more than `config.steps`.
    pub steps: u32,
}

impl SsrHit {
    /// The miss a ray that never lands returns, at the last sample it read.
    #[must_use]
    fn miss(t: f32, x_px: u32, y_px: u32, depth_m: f32, steps: u32) -> Self {
        Self {
            hit: false,
            t,
            x_px,
            y_px,
            depth_m,
            steps,
        }
    }
}

/// The parametric `t` where a ray leaves the reflected sub-rectangle, clipped
/// to `0..=1`: the row's `t_max`. A lane with no travel never ends the ray.
#[must_use]
#[expect(
    clippy::cast_precision_loss,
    reason = "a frame edge is far below 2^24, so f32 holds it exactly"
)]
pub fn t_max(ray: SsrRay, w: u32, h: u32) -> f32 {
    let mut t_max = 1.0f32;
    let hi_x = w.saturating_sub(1) as f32;
    let hi_y = h.saturating_sub(1) as f32;
    for (start, dir, hi) in [(ray.x_px, ray.dx_px, hi_x), (ray.y_px, ray.dy_px, hi_y)] {
        if dir > 0.0 {
            t_max = t_max.min(((hi - start) / dir).max(0.0));
        } else if dir < 0.0 {
            t_max = t_max.min((start / -dir).max(0.0));
        }
    }
    t_max.clamp(0.0, 1.0)
}

/// March one reflected ray over `depth` (full-res, row-major, meters) and
/// return the first sample inside geometry, or a miss that names where the ray
/// ran out. Never walks more than `config.steps` samples and never leaves the
/// frame: `t_max` stops it at the screen edge.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "a sample is floored inside 0..=w-1 and 0..=h-1 before it becomes an index"
)]
pub fn march(depth: &[f32], config: &SsrConfig, ray: SsrRay) -> Result<SsrHit, LookError> {
    config.validate()?;
    if depth.len() != config.full_texels() {
        return Err(LookError::Invalid(format!(
            "ssr depth length {} mismatches frame {}x{}",
            depth.len(),
            config.width,
            config.height
        )));
    }
    if !ray.x_px.is_finite()
        || !ray.y_px.is_finite()
        || !ray.depth_m.is_finite()
        || !ray.dx_px.is_finite()
        || !ray.dy_px.is_finite()
        || !ray.dz_m.is_finite()
        || ray.depth_m <= 0.0
    {
        return Err(LookError::Invalid(format!(
            "ssr ray ({}, {}) depth {} is not positive finite",
            ray.x_px, ray.y_px, ray.depth_m
        )));
    }
    let w = config.width;
    let h = config.height;
    let limit = t_max(ray, w, h);
    let mut steps = 0;
    let mut last = SsrHit::miss(
        0.0,
        ray.x_px.max(0.0) as u32,
        ray.y_px.max(0.0) as u32,
        0.0,
        0,
    );
    for step in 0..config.steps {
        let t = config.hit_t(step);
        if t > limit {
            break;
        }
        steps += 1;
        let x = ray.x_px + ray.dx_px * t;
        let y = ray.y_px + ray.dy_px * t;
        if !(x >= 0.0 && y >= 0.0) {
            break;
        }
        let xi = x as u32;
        let yi = y as u32;
        if xi >= w || yi >= h {
            break;
        }
        let z = ray.depth_m + ray.dz_m * t;
        let d = depth[(yi * w + xi) as usize];
        last = SsrHit::miss(t, xi, yi, d, steps);
        // Inside the surface's own thickness: the ray entered geometry.
        if z >= d && z <= d + config.depth_tolerance {
            return Ok(SsrHit {
                hit: true,
                steps,
                ..last
            });
        }
    }
    last.steps = steps;
    Ok(last)
}

#[cfg(test)]
#[path = "ssr_tests.rs"]
mod tests;
