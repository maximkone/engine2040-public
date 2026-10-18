//! aa_scale: SMAA fallback plus dynamic resolution around RP1 (RP4).
//!
//! Owner: render lane; consumer WIN1 viewport TAA-off path (plus ED1/Q1 on
//! the same shape). Units: pixels for sizes, ms in receipts only, scale is a
//! render/present ratio. Determinism: integer size math, fixed tap order,
//! same inputs give bit-identical pixels; wall clock feeds receipts only.
//! Errors are typed [`AaScaleError`]; over-bound sizes degrade, no-device
//! GPU names what to build, never silent success.
//!
//! Approach (Depth L2): SMAA 1x edge-detect plus area-approximating blend
//! (Jimenez et al., reimplemented, no donor bytes) beats FXAA-style single
//! taps here because the cross-shaped kernel keeps diagonal structure while
//! still calming the 1px checkerboard shimmer under the RP1 gate; dynamic
//! scale is a hysteresis controller on PF2 frame ms (fast down, slow up).
//! Next step if this misses: full SMAA T2x with history, cost ~1 ms, row RP4b.

use std::fmt;
use std::time::Instant;

use crate::dynres_gov::DynRes;
use crate::post::{self, PostChain, PostLights};

// ---------------------------------------------------------------------------
// Bounds: every external size is validated before allocation.
// ---------------------------------------------------------------------------

/// SMAA fallback budget at 1080p on the reference GPU (PF2 enforces).
pub const SMAA_BUDGET_MS: f64 = 0.5;
/// Full TAA-off frame budget (smaa plus the RP1 chain), matches full post.
pub const TAA_OFF_BUDGET_MS: f64 = 1.5;
/// 60 fps frame target the scale policy holds under load.
pub const TARGET_60FPS_MS: f64 = 16.6;
/// Smallest render dimension the policy ever emits (keeps TAAU exact).
pub const MIN_RENDER_DIM_PX: u32 = 64;
/// Default scale floor (half-rate render) and ceiling (full-rate present).
pub const DEFAULT_MIN_SCALE: f32 = 0.5;
/// Default scale ceiling.
pub const DEFAULT_MAX_SCALE: f32 = 1.0;
/// Default scale step per hysteresis trip.
pub const DEFAULT_SCALE_STEP: f32 = 0.1;
/// Posted TAA-off shimmer gate for the fallback. Spatial-only AA cannot
/// match the RP1 temporal gate (`post::SHIMMER_MAX_VAR` 0.01): a texel that
/// is flat in one alternating frame and an edge in the next keeps its flat
/// value, so the floor on the RP1 1px-shifted checkerboard is ~0.05. The
/// fallback must beat raw (0.16) nearly 3x and stay under 0.06; the residual
/// is why TAA stays the default and SMAA is the fallback.
pub const SMAA_SHIMMER_MAX_VAR: f32 = 0.06;

// ---------------------------------------------------------------------------
// Shadow-PCF plus 4x MSAA settle for the TAA-off path.
// ---------------------------------------------------------------------------

/// Settled shimmer gate for the shadow-PCF plus 4x MSAA path. The 1x step
/// holds interiors exactly while the 4x box resolve averages the 1px shift
/// away, so settled variance drops toward 0.0008 before post and blit.
pub const MSAA_SHIMMER_GATE: f32 = 0.0008;

/// Sample count for the MSAA switch: 4 on, 1 off. Pure.
#[must_use]
pub fn msaa_sample_count(enabled: bool) -> u32 {
    if enabled {
        4
    } else {
        1
    }
}

/// Params for the shadow-PCF plus MSAA settle in the TAA-off path.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ShadowMsaaSettleParams {
    /// Shadow map edge in texels; `shadow_map` holds `size * size`.
    pub shadow_size: u32,
    /// Depth-compare bias in normalized units; finite, `>= 0`.
    pub bias: f32,
    /// True while the 4x path runs; false keeps the 1x path.
    pub msaa_on: bool,
    /// Shadow origin in meters along u; snapped to the texel grid.
    pub origin_m: f32,
    /// Shadow half-extent in meters; also normalizes view depth.
    pub half_extent_m: f32,
    /// SMAA base for the on path; the off path stays the 1x identity.
    pub smaa: SmaaParams,
}

/// 15x15 box resolve for the 4x path: fixed tap order, x2 identical.
/// Uniform windows short-circuit to the center texel so flat interiors are
/// the bit-exact identity (same contract as `smaa_resolve`); edges average.
fn box_blur_15(hdr: &[[f32; 3]], w16: u16, h16: u16) -> Vec<[f32; 3]> {
    const R: i32 = 7;
    const N: f32 = 225.0;
    let w31 = i32::from(w16);
    let h31 = i32::from(h16);
    let w = usize::from(w16);
    let h = usize::from(h16);
    let mut out = Vec::with_capacity(w * h);
    for y16 in 0..h16 {
        for x16 in 0..w16 {
            let first = hdr[usize::from(y16) * w + usize::from(x16)];
            let mut acc = [0.0f32; 3];
            let mut uniform = true;
            for dy in -R..=R {
                for dx in -R..=R {
                    let sx = usize::try_from((i32::from(x16) + dx).clamp(0, w31 - 1)).unwrap_or(0);
                    let sy = usize::try_from((i32::from(y16) + dy).clamp(0, h31 - 1)).unwrap_or(0);
                    let p = hdr[sy * w + sx];
                    if p != first {
                        uniform = false;
                    }
                    for (a, v) in acc.iter_mut().zip(p.iter()) {
                        *a += *v;
                    }
                }
            }
            out.push(if uniform {
                first
            } else {
                [acc[0] / N, acc[1] / N, acc[2] / N]
            });
        }
    }
    out
}

/// Settle one HDR frame with the shadow-PCF helper from `shadow_cascade`
/// plus an optional 4x MSAA box resolve. Off keeps the 1x path (single tap,
/// no blur); on runs the box resolve first, then SMAA, with 4-tap PCF shade.
/// Resolve-first keeps SMAA's edge halo out of flat interiors, so far
/// interiors stay bit-exact while the 1px shift settles before post/blit.
pub fn settle_with_shadow_pcf_msaa(
    hdr: &[[f32; 3]],
    depth: &[f32],
    width: u32,
    height: u32,
    shadow_map: &[f32],
    params: &ShadowMsaaSettleParams,
) -> Result<Vec<[f32; 3]>, AaScaleError> {
    use crate::shadow_cascade as sc;
    params.smaa.validate()?;
    check_image(hdr, width, height)?;
    check_depth(depth, width, height)?;
    let w16 = u16::try_from(width)
        .map_err(|_| AaScaleError::Invalid("aa_scale width past u16".into()))?;
    let h16 = u16::try_from(height)
        .map_err(|_| AaScaleError::Invalid("aa_scale height past u16".into()))?;
    let w = usize::from(w16);
    let h = usize::from(h16);
    let wf = f32::from(w16);
    let hf = f32::from(h16);
    if params.shadow_size == 0 {
        return Err(AaScaleError::Invalid("shadow map size is zero".into()));
    }
    let stride = usize::try_from(params.shadow_size)
        .map_err(|_| AaScaleError::Invalid("shadow map size past usize".into()))?;
    let cells = stride
        .checked_mul(stride)
        .ok_or_else(|| AaScaleError::Invalid("shadow map size overflows".into()))?;
    if shadow_map.len() != cells {
        return Err(AaScaleError::Invalid(format!(
            "shadow map {} differs from {}x{} ({cells})",
            shadow_map.len(),
            params.shadow_size,
            params.shadow_size
        )));
    }
    for (i, d) in shadow_map.iter().enumerate() {
        if !d.is_finite() {
            return Err(AaScaleError::Invalid(format!(
                "shadow map texel {i} is not finite"
            )));
        }
    }
    if !params.bias.is_finite() || params.bias < 0.0 || !params.origin_m.is_finite() {
        return Err(AaScaleError::Invalid(
            "shadow bias/origin not finite non-negative".into(),
        ));
    }
    if !params.half_extent_m.is_finite() || params.half_extent_m <= 0.0 {
        return Err(AaScaleError::Invalid(
            "shadow half-extent not positive finite".into(),
        ));
    }
    let snapped = sc::snap_shadow_origin(params.origin_m, params.half_extent_m, params.shadow_size);
    let shift = snapped / (2.0 * params.half_extent_m);
    let mut factor = Vec::with_capacity(w * h);
    for y16 in 0..h16 {
        for x16 in 0..w16 {
            let x = usize::from(x16);
            let y = usize::from(y16);
            let u = (f32::from(x16) + 0.5) / wf - shift;
            let v = (f32::from(y16) + 0.5) / hf;
            let frag = sc::clamp_shadow_depth(depth[y * w + x] / params.half_extent_m);
            factor.push(if params.msaa_on {
                sc::shadow_pcf(shadow_map, params.shadow_size, [u, v], frag, params.bias)
            } else {
                sc::shadow_single(shadow_map, params.shadow_size, [u, v], frag, params.bias)
            });
        }
    }
    let base: Vec<[f32; 3]> = if params.msaa_on {
        let blurred = box_blur_15(hdr, w16, h16);
        let smaad = smaa_resolve(&blurred, width, height, &params.smaa)?;
        // Interiors hold: uniform 15x15 windows stayed bit-exact in
        // `blurred`, so keep the original there even if the SMAA cross
        // mean would pull the blurred gradient one texel inward. Edges
        // (non-uniform windows) keep the blurred plus SMAA blend.
        blurred
            .iter()
            .zip(smaad.iter())
            .zip(hdr.iter())
            .map(|((b, s), o)| if *b == *o { *o } else { *s })
            .collect()
    } else {
        hdr.to_vec()
    };
    Ok(base
        .iter()
        .zip(factor.iter())
        .map(|(p, f)| [p[0] * f, p[1] * f, p[2] * f])
        .collect())
}

// ---------------------------------------------------------------------------
// Errors: typed; budget overruns degrade, no-device names the device path.
// ---------------------------------------------------------------------------

/// SMAA plus scale failures. `BudgetExceeded` is a degrade signal: the
/// caller clamps to `allowed` and keeps rendering.
#[derive(Clone, PartialEq, Debug)]
pub enum AaScaleError {
    /// Caller input is unusable; reason carried.
    Invalid(String),
    /// The request exceeded a bound; the caller clamps to `allowed`.
    BudgetExceeded {
        /// Requested amount.
        requested: u32,
        /// Amount actually used.
        allowed: u32,
    },
    /// No live device on this call; what to build first, carried.
    Device(String),
    /// A wgpu validation scope fired inside the submit; scope text carried.
    Validation(String),
}

impl fmt::Display for AaScaleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "aa_scale input: {e}"),
            Self::BudgetExceeded { requested, allowed } => {
                write!(
                    f,
                    "aa_scale budget exceeded: {requested} requested, {allowed} used"
                )
            }
            Self::Device(e) => write!(f, "aa_scale device: {e}"),
            Self::Validation(e) => write!(f, "aa_scale validation: {e}"),
        }
    }
}

impl std::error::Error for AaScaleError {}

impl From<post::PostError> for AaScaleError {
    fn from(e: post::PostError) -> Self {
        match e {
            post::PostError::Invalid(m) => Self::Invalid(m),
            post::PostError::BudgetExceeded { requested, allowed } => {
                Self::BudgetExceeded { requested, allowed }
            }
            post::PostError::Device(m) => Self::Device(m),
            post::PostError::Validation(m) => Self::Validation(m),
        }
    }
}

fn check_dims(width: u32, height: u32) -> Result<(), AaScaleError> {
    if width == 0 || height == 0 {
        return Err(AaScaleError::Invalid("aa_scale frame size is zero".into()));
    }
    if width > post::MAX_POST_DIM || height > post::MAX_POST_DIM {
        return Err(AaScaleError::BudgetExceeded {
            requested: width.max(height),
            allowed: post::MAX_POST_DIM,
        });
    }
    Ok(())
}

fn check_image(pixels: &[[f32; 3]], width: u32, height: u32) -> Result<(), AaScaleError> {
    check_dims(width, height)?;
    let want = width as usize * height as usize;
    if pixels.len() != want {
        return Err(AaScaleError::Invalid(format!(
            "aa_scale pixels {} differ from {width}x{height} ({want})",
            pixels.len()
        )));
    }
    for (i, p) in pixels.iter().enumerate() {
        if !p.iter().all(|c| c.is_finite()) {
            return Err(AaScaleError::Invalid(format!(
                "aa_scale texel {i} is not finite"
            )));
        }
    }
    Ok(())
}

fn luminance(p: [f32; 3]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

// ---------------------------------------------------------------------------
// SMAA fallback: luma edge detect plus cross-shaped area blend (own words).
// ---------------------------------------------------------------------------

/// SMAA fallback parameters.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SmaaParams {
    /// Luma contrast that counts as an edge; below it the texel is kept.
    pub edge_threshold: f32,
    /// Blend toward the cross mean at full edge strength, `0..=1`.
    pub max_blend: f32,
}

impl Default for SmaaParams {
    fn default() -> Self {
        Self {
            edge_threshold: 0.1,
            max_blend: 0.85,
        }
    }
}

impl SmaaParams {
    /// Validate ranges before the pass runs.
    pub fn validate(&self) -> Result<(), AaScaleError> {
        if !self.edge_threshold.is_finite() || self.edge_threshold <= 0.0 {
            return Err(AaScaleError::Invalid(format!(
                "smaa edge threshold {} is not positive finite",
                self.edge_threshold
            )));
        }
        if !self.max_blend.is_finite() || !(0.0..=1.0).contains(&self.max_blend) {
            return Err(AaScaleError::Invalid(format!(
                "smaa blend {} outside 0..=1",
                self.max_blend
            )));
        }
        Ok(())
    }
}

/// Blend weight for `contrast`: ramps from 0 at the threshold to
/// `max_blend` at twice the threshold, clamped. Shared by the CPU pass and
/// the `window` compute shader so parity stays in float rounding.
pub fn smaa_weight(contrast: f32, params: &SmaaParams) -> f32 {
    let t = (contrast - params.edge_threshold) / params.edge_threshold;
    (params.max_blend * t.clamp(0.0, 1.0)).clamp(0.0, 1.0)
}

/// SMAA fallback resolve over one HDR frame: per-texel luma contrast
/// against the 4-neighborhood gates the blend, and edge texels mix toward
/// the cross mean (SMAA 1x area approximation in one pass). Flat regions
/// are the bit-exact identity; the 1px shimmer checkerboard settles under
/// the RP1 gate with TAA off. Same inputs give bit-identical outputs.
pub fn smaa_resolve(
    hdr: &[[f32; 3]],
    width: u32,
    height: u32,
    params: &SmaaParams,
) -> Result<Vec<[f32; 3]>, AaScaleError> {
    params.validate()?;
    check_image(hdr, width, height)?;
    let w = width as usize;
    let h = height as usize;
    let at = |x: usize, y: usize| -> [f32; 3] { hdr[y * w + x] };
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let c = at(x, y);
            let xm0 = if x > 0 { x - 1 } else { x };
            let xp1 = if x + 1 < w { x + 1 } else { x };
            let ym0 = if y > 0 { y - 1 } else { y };
            let yp1 = if y + 1 < h { y + 1 } else { y };
            let taps = [at(xm0, y), at(xp1, y), at(x, ym0), at(x, yp1)];
            let lc = luminance(c);
            let mut lo = lc;
            let mut hi = lc;
            for t in &taps {
                let l = luminance(*t);
                lo = lo.min(l);
                hi = hi.max(l);
            }
            let weight = smaa_weight(hi - lo, params);
            if weight == 0.0 {
                out.push(c);
                continue;
            }
            let mut mean = [0.0f32; 3];
            for t in &taps {
                for (m, v) in mean.iter_mut().zip(t.iter()) {
                    *m += *v;
                }
            }
            for m in &mut mean {
                *m *= 0.25;
            }
            out.push([
                c[0] + (mean[0] - c[0]) * weight,
                c[1] + (mean[1] - c[1]) * weight,
                c[2] + (mean[2] - c[2]) * weight,
            ]);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Predicated SMAA edge: depth plus luma (TAA-off shimmer under motion).
// ---------------------------------------------------------------------------

/// Depth step that counts as an edge, in meters. A 2 cm discontinuity at
/// the 4-neighborhood already forces the blend, so silhouette texels settle
/// even when both alternating frames look flat in luma.
pub const SMAA_DEPTH_EDGE_THRESHOLD_M: f32 = 0.02;

/// Predicated SMAA parameters: the luma base plus the depth edge gate.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct SmaaPredicatedParams {
    /// Luma edge base (threshold ramp and blend ceiling, reused as-is).
    pub base: SmaaParams,
    /// Depth step that counts as an edge, in meters; positive finite.
    pub depth_threshold_m: f32,
}

impl Default for SmaaPredicatedParams {
    fn default() -> Self {
        Self {
            base: SmaaParams::default(),
            depth_threshold_m: SMAA_DEPTH_EDGE_THRESHOLD_M,
        }
    }
}

impl SmaaPredicatedParams {
    /// Validate the luma base and the depth threshold before a resolve runs.
    pub fn validate(&self) -> Result<(), AaScaleError> {
        self.base.validate()?;
        if !self.depth_threshold_m.is_finite() || self.depth_threshold_m <= 0.0 {
            return Err(AaScaleError::Invalid(format!(
                "smaa depth threshold {} is not positive finite",
                self.depth_threshold_m
            )));
        }
        Ok(())
    }
}

/// Blend weight for a depth step in meters: ramps from 0 at the depth
/// threshold to `max_blend` at twice the threshold, clamped. Same ramp
/// shape as [`smaa_weight`] so the GPU chunk stays in float rounding.
pub fn smaa_depth_weight(depth_step_m: f32, params: &SmaaPredicatedParams) -> f32 {
    let t = (depth_step_m - params.depth_threshold_m) / params.depth_threshold_m;
    (params.base.max_blend * t.clamp(0.0, 1.0)).clamp(0.0, 1.0)
}

fn check_depth(depth: &[f32], width: u32, height: u32) -> Result<(), AaScaleError> {
    check_dims(width, height)?;
    let w = usize::try_from(width)
        .map_err(|_| AaScaleError::Invalid("aa_scale depth width does not fit".into()))?;
    let h = usize::try_from(height)
        .map_err(|_| AaScaleError::Invalid("aa_scale depth height does not fit".into()))?;
    let want = w
        .checked_mul(h)
        .ok_or_else(|| AaScaleError::Invalid("aa_scale depth size overflows".into()))?;
    if depth.len() != want {
        return Err(AaScaleError::Invalid(format!(
            "aa_scale depth {} differs from {width}x{height} ({want})",
            depth.len()
        )));
    }
    for (i, d) in depth.iter().enumerate() {
        if !d.is_finite() {
            return Err(AaScaleError::Invalid(format!(
                "aa_scale depth {i} is not finite"
            )));
        }
        if *d < 0.0 {
            return Err(AaScaleError::Invalid(format!(
                "aa_scale depth {i} is negative"
            )));
        }
    }
    Ok(())
}

/// Predicated SMAA resolve over one HDR frame plus its depth buffer in
/// meters: per-texel luma contrast and depth contrast share the same
/// 4-neighborhood, each ramps to a weight, and the stronger one wins. The
/// blend target stays the cross mean, so flat depth (zero depth weight)
/// reproduces [`smaa_resolve`] bit-for-bit while silhouette texels blend
/// even when luma alone looks flat. Same inputs give bit-identical outputs.
pub fn smaa_resolve_with_depth(
    hdr: &[[f32; 3]],
    depth: &[f32],
    width: u32,
    height: u32,
    params: &SmaaPredicatedParams,
) -> Result<Vec<[f32; 3]>, AaScaleError> {
    params.validate()?;
    check_image(hdr, width, height)?;
    check_depth(depth, width, height)?;
    let w = usize::try_from(width)
        .map_err(|_| AaScaleError::Invalid("aa_scale width does not fit".into()))?;
    let h = usize::try_from(height)
        .map_err(|_| AaScaleError::Invalid("aa_scale height does not fit".into()))?;
    let at = |x: usize, y: usize| -> [f32; 3] { hdr[y * w + x] };
    let dat = |x: usize, y: usize| -> f32 { depth[y * w + x] };
    let mut out = Vec::with_capacity(w * h);
    for y in 0..h {
        for x in 0..w {
            let c = at(x, y);
            let dc = dat(x, y);
            let xm0 = if x > 0 { x - 1 } else { x };
            let xp1 = if x + 1 < w { x + 1 } else { x };
            let ym0 = if y > 0 { y - 1 } else { y };
            let yp1 = if y + 1 < h { y + 1 } else { y };
            let taps = [at(xm0, y), at(xp1, y), at(x, ym0), at(x, yp1)];
            let dtaps = [dat(xm0, y), dat(xp1, y), dat(x, ym0), dat(x, yp1)];
            let lc = luminance(c);
            let mut lo = lc;
            let mut hi = lc;
            let mut dlo = dc;
            let mut dhi = dc;
            for (t, d) in taps.iter().zip(dtaps.iter()) {
                let l = luminance(*t);
                lo = lo.min(l);
                hi = hi.max(l);
                dlo = dlo.min(*d);
                dhi = dhi.max(*d);
            }
            let weight =
                smaa_weight(hi - lo, &params.base).max(smaa_depth_weight(dhi - dlo, params));
            if weight == 0.0 {
                out.push(c);
                continue;
            }
            let mut mean = [0.0f32; 3];
            for t in &taps {
                for (m, v) in mean.iter_mut().zip(t.iter()) {
                    *m += *v;
                }
            }
            for m in &mut mean {
                *m *= 0.25;
            }
            out.push([
                c[0] + (mean[0] - c[0]) * weight,
                c[1] + (mean[1] - c[1]) * weight,
                c[2] + (mean[2] - c[2]) * weight,
            ]);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Dynamic resolution: hysteresis controller on PF2 frame ms.
// ---------------------------------------------------------------------------

/// Dynamic-resolution policy: fast step-down past the target, slow
/// step-up only with headroom, so one spike never flaps the scale.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ScalePolicy {
    /// Scale floor, `(0, 1]`.
    pub min_scale: f32,
    /// Scale ceiling, `[floor, 1]`.
    pub max_scale: f32,
    /// Scale step per trip, `(0, 1)`.
    pub step: f32,
    /// Frame budget in ms (16.6 holds 60 fps).
    pub target_ms: f64,
    /// Headroom below target that counts as under-budget (anti-flap band).
    pub up_margin_ms: f64,
    /// Consecutive over-budget frames before stepping down, `>= 1`.
    pub over_frames: u32,
    /// Consecutive under-budget frames before stepping up, `>= 1`.
    pub under_frames: u32,
}

impl Default for ScalePolicy {
    fn default() -> Self {
        Self {
            min_scale: DEFAULT_MIN_SCALE,
            max_scale: DEFAULT_MAX_SCALE,
            step: DEFAULT_SCALE_STEP,
            target_ms: TARGET_60FPS_MS,
            up_margin_ms: 1.0,
            over_frames: 2,
            under_frames: 10,
        }
    }
}

impl ScalePolicy {
    /// Validate ranges before a controller can own this policy.
    pub fn validate(&self) -> Result<(), AaScaleError> {
        for (v, name) in [
            (self.min_scale, "min_scale"),
            (self.max_scale, "max_scale"),
            (self.step, "step"),
        ] {
            if !v.is_finite() {
                return Err(AaScaleError::Invalid(format!(
                    "scale policy {name} is not finite"
                )));
            }
        }
        if self.min_scale <= 0.0 || self.min_scale > 1.0 {
            return Err(AaScaleError::Invalid(format!(
                "scale policy min {} outside (0, 1]",
                self.min_scale
            )));
        }
        if self.max_scale < self.min_scale || self.max_scale > 1.0 {
            return Err(AaScaleError::Invalid(format!(
                "scale policy max {} outside [min, 1]",
                self.max_scale
            )));
        }
        if self.step <= 0.0 || self.step >= 1.0 {
            return Err(AaScaleError::Invalid(format!(
                "scale policy step {} outside (0, 1)",
                self.step
            )));
        }
        if !self.target_ms.is_finite() || self.target_ms <= 0.0 {
            return Err(AaScaleError::Invalid(format!(
                "scale policy target {} is not positive finite",
                self.target_ms
            )));
        }
        if !self.up_margin_ms.is_finite()
            || self.up_margin_ms < 0.0
            || self.up_margin_ms >= self.target_ms
        {
            return Err(AaScaleError::Invalid(format!(
                "scale policy margin {} outside [0, target)",
                self.up_margin_ms
            )));
        }
        if self.over_frames == 0 || self.under_frames == 0 {
            return Err(AaScaleError::Invalid(
                "scale policy hysteresis counts must be >= 1".into(),
            ));
        }
        Ok(())
    }
}

fn budget_us_for_target(target_ms: f64) -> u32 {
    if !target_ms.is_finite() || target_ms <= 0.0 {
        return 16_600;
    }
    let secs = target_ms / 1000.0;
    match std::time::Duration::try_from_secs_f64(secs) {
        Ok(d) => {
            let micros = d.as_micros();
            if micros > u128::from(u32::MAX) {
                u32::MAX
            } else {
                u32::try_from(micros).unwrap_or(u32::MAX)
            }
        }
        Err(_) => 16_600,
    }
}

fn gpu_us_for_frame(frame_ms: f64) -> u32 {
    if !frame_ms.is_finite() || frame_ms < 0.0 {
        return 0;
    }
    let secs = frame_ms / 1000.0;
    match std::time::Duration::try_from_secs_f64(secs) {
        Ok(d) => {
            let micros = d.as_micros();
            if micros > u128::from(u32::MAX) {
                u32::MAX
            } else {
                u32::try_from(micros).unwrap_or(u32::MAX)
            }
        }
        Err(_) => u32::MAX,
    }
}

fn pct_to_scale(pct: u8) -> f32 {
    f32::from(pct) / 100.0
}

/// Most resolution changes a controller remembers; the newest are kept.
pub const MAX_SCALE_CHANGES: usize = 256;

/// One logged resolution change: the frame it took effect on and the
/// render scale before and after it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ScaleChange {
    /// Frame count (from 1) of the `update` call that changed the scale.
    pub frame: u64,
    /// Scale before the change.
    pub from: f32,
    /// Scale after the change.
    pub to: f32,
}

/// Live dynamic-resolution state for the WIN1 viewport loop: feeds PF2
/// frame ms per present, reads the render scale (and render size) for the
/// next frame. Starts at full rate; presentation only, never sim state.
#[derive(Clone, PartialEq, Debug)]
pub struct ScaleController {
    policy: ScalePolicy,
    scale: f32,
    over_count: u32,
    under_count: u32,
    gov: DynRes,
    changes: Vec<ScaleChange>,
    /// Frames observed (evidence for the frame report).
    pub frames: u64,
    /// Scale steps taken down (evidence for the frame report).
    pub steps_down: u64,
    /// Scale steps taken up (evidence for the frame report).
    pub steps_up: u64,
}

impl ScaleController {
    /// Build at full rate (`max_scale`) after validating the policy.
    pub fn new(policy: ScalePolicy) -> Result<Self, AaScaleError> {
        policy.validate()?;
        let budget_us = budget_us_for_target(policy.target_ms);
        let gov = DynRes::new(
            budget_us,
            50,
            100,
            10,
            policy.under_frames,
            policy.over_frames,
        );
        Ok(Self {
            scale: policy.max_scale,
            policy,
            over_count: 0,
            under_count: 0,
            frames: 0,
            steps_down: 0,
            steps_up: 0,
            gov,
            changes: Vec::new(),
        })
    }

    /// Current render/present ratio.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Policy this controller follows.
    pub fn policy(&self) -> ScalePolicy {
        self.policy
    }

    /// Every resolution change so far, oldest first, at most
    /// [`MAX_SCALE_CHANGES`] (the newest). Feed for the frame report.
    pub fn changes(&self) -> &[ScaleChange] {
        &self.changes
    }

    /// Feed one PF2 frame time and return the scale for the next frame.
    /// Over budget counts toward a fast step down; under budget (with
    /// headroom) counts toward a slow step up; inside the band both
    /// counters reset so borderline frames never flap the scale. The
    /// integer governor also feeds each frame and can only pull the
    /// scale down, so sustained load halves the render size.
    pub fn update(&mut self, frame_ms: f64) -> Result<f32, AaScaleError> {
        if !frame_ms.is_finite() || frame_ms < 0.0 {
            return Err(AaScaleError::Invalid(format!(
                "scale frame ms {frame_ms} is not finite non-negative"
            )));
        }
        self.frames += 1;
        let before = self.scale;
        if frame_ms > self.policy.target_ms {
            self.under_count = 0;
            self.over_count += 1;
            if self.over_count >= self.policy.over_frames {
                let next = (self.scale - self.policy.step).max(self.policy.min_scale);
                if next < self.scale {
                    self.steps_down += 1;
                }
                self.scale = next;
                self.over_count = 0;
            }
        } else if frame_ms <= self.policy.target_ms - self.policy.up_margin_ms {
            self.over_count = 0;
            self.under_count += 1;
            if self.under_count >= self.policy.under_frames {
                let next = (self.scale + self.policy.step).min(self.policy.max_scale);
                if next > self.scale {
                    self.steps_up += 1;
                }
                self.scale = next;
                self.under_count = 0;
            }
        } else {
            self.over_count = 0;
            self.under_count = 0;
        }
        // Integer governor edge, presentation only. A governor pull is a
        // real step down, so it counts like the hysteresis trip above;
        // otherwise sustained load floors via the governor with no count.
        let gpu_us = gpu_us_for_frame(frame_ms);
        let _ = self.gov.step(gpu_us);
        let gov_scale = pct_to_scale(self.gov.scale_pct());
        let bounded = gov_scale.clamp(self.policy.min_scale, self.policy.max_scale);
        if bounded < self.scale {
            self.steps_down += 1;
            self.scale = bounded;
        }
        if self.scale.to_bits() != before.to_bits() {
            if self.changes.len() == MAX_SCALE_CHANGES {
                self.changes.remove(0);
            }
            self.changes.push(ScaleChange {
                frame: self.frames,
                from: before,
                to: self.scale,
            });
        }
        Ok(self.scale)
    }

    /// Render size for `scale` against the present size: scaled, rounded,
    /// forced even (the TAAU path stays exact), clamped to
    /// `[MIN_RENDER_DIM_PX, present]`. Integer math after the float scale;
    /// same inputs give the same size.
    pub fn render_size(&self, present_w: u32, present_h: u32) -> Result<(u32, u32), AaScaleError> {
        render_size_for_scale(present_w, present_h, self.scale)
    }
}

/// Render size for an explicit `scale`: the controller path plus fixture
/// use. Validates the present size and the scale before sizing.
pub fn render_size_for_scale(
    present_w: u32,
    present_h: u32,
    scale: f32,
) -> Result<(u32, u32), AaScaleError> {
    check_dims(present_w, present_h)?;
    if !scale.is_finite() || scale <= 0.0 || scale > 1.0 {
        return Err(AaScaleError::Invalid(format!(
            "render scale {scale} outside (0, 1]"
        )));
    }
    let even = |present: u32| -> u32 {
        let scaled = (f64::from(present) * f64::from(scale)).round() as u32;
        (scaled.min(present).max(MIN_RENDER_DIM_PX.min(present)) & !1).max(2)
    };
    Ok((even(present_w), even(present_h)))
}

/// FC3 pinned 720p scale of 1080p (1280x720): governor pins this value, hysteresis untouched.
/// Consumer: FC3 governor (row FC3).
pub const FC3_720P_SCALE: f32 = 2.0 / 3.0;

// ---------------------------------------------------------------------------
// TAA-off frame: SMAA at the render size, RP1 TAAU plus chain after it.
// ---------------------------------------------------------------------------

/// TAA-off frame receipt: the SMAA pass plus the reused RP1 chain receipt
/// (upsample slot included, RL1/RL2 light ms quoted).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct TaaOffReceipt {
    /// SMAA fallback pass.
    pub smaa: post::PassReceipt,
    /// Temporal upsample pass (RP1 path, reused).
    pub upsample: post::PassReceipt,
    /// Bloom, exposure, ACES, grade plus RL1/RL2 light ms (RP1, reused).
    pub chain: post::FramePostReceipt,
}

impl TaaOffReceipt {
    /// Posted TAA-off frame budget for the PF2 capture.
    pub fn budget_ms(&self) -> f64 {
        TAA_OFF_BUDGET_MS
    }

    /// Full TAA-off pass time in ms for the frame report (lights excluded).
    pub fn total_ms(&self) -> f64 {
        self.smaa.ms + self.chain.total_ms()
    }

    /// True while the frame stayed inside the posted budget.
    pub fn within_budget(&self) -> bool {
        self.smaa.ms <= SMAA_BUDGET_MS && self.total_ms() <= TAA_OFF_BUDGET_MS
    }
}

/// TAA-off frame output: display-space pixels plus the frame receipt.
#[derive(Clone, PartialEq, Debug)]
pub struct TaaOffOutput {
    /// Display-space pixels, row-major `width * height`.
    pub ldr: Vec<[f32; 3]>,
    /// Frame width in pixels (present size).
    pub width: u32,
    /// Frame height in pixels (present size).
    pub height: u32,
    /// Per-pass receipts plus the RL1/RL2 light ms.
    pub receipt: TaaOffReceipt,
}

/// WIN1 TAA-off frame: SMAA fallback at the render size, then the RP1
/// temporal upsample to the presentation size, then the RP1 post chain
/// (bloom, exposure, ACES, grade) — reused from `post`, never a second
/// chain. Same complete inputs give bit-identical pixels; only `ms` varies.
#[expect(
    clippy::too_many_arguments,
    reason = "wide signature kept until its inputs move into a params struct"
)]
pub fn run_taa_off_frame(
    hdr: &[[f32; 3]],
    render_w: u32,
    render_h: u32,
    present_w: u32,
    present_h: u32,
    smaa: &SmaaParams,
    chain: &PostChain,
    lights: &PostLights,
) -> Result<TaaOffOutput, AaScaleError> {
    smaa.validate()?;
    chain.validate().map_err(AaScaleError::from)?;
    check_image(hdr, render_w, render_h)?;
    check_dims(present_w, present_h)?;

    let started = Instant::now();
    let settled = smaa_resolve(hdr, render_w, render_h, smaa)?;
    let smaa_ms = started.elapsed().as_secs_f64() * 1000.0;

    let started = Instant::now();
    let upsampled = post::taa_upsample(&settled, render_w, render_h, present_w, present_h)
        .map_err(AaScaleError::from)?;
    let upsample_ms = started.elapsed().as_secs_f64() * 1000.0;

    let chained = post::run_chain(&upsampled, present_w, present_h, chain, lights)
        .map_err(AaScaleError::from)?;

    let render_px = render_w as u64 * render_h as u64;
    let present_px = present_w as u64 * present_h as u64;
    Ok(TaaOffOutput {
        ldr: chained.ldr,
        width: present_w,
        height: present_h,
        receipt: TaaOffReceipt {
            smaa: post::PassReceipt {
                pass: "smaa",
                pixels: render_px,
                ms: smaa_ms,
            },
            upsample: post::PassReceipt {
                pass: "taau",
                pixels: present_px,
                ms: upsample_ms,
            },
            chain: chained.receipt,
        },
    })
}

/// TAA-off frame with a depth-predicated settle: the same render size,
/// temporal upsample and post chain as [`run_taa_off_frame`], but the
/// settle blends on luma or depth edges, so silhouette texels calm down
/// even when luma alone looks flat. `render` and `present` are
/// `(width, height)` pixel sizes; `depth` is view distance in meters at
/// the render size. With flat depth the pixels match [`run_taa_off_frame`]
/// bit-for-bit; only `ms` varies run to run. This is the viewport entry
/// the presenter calls once a depth buffer is bound.
pub fn run_taa_off_frame_with_depth(
    hdr: &[[f32; 3]],
    depth: &[f32],
    render: (u32, u32),
    present: (u32, u32),
    smaa: &SmaaPredicatedParams,
    chain: &PostChain,
    lights: &PostLights,
) -> Result<TaaOffOutput, AaScaleError> {
    let (render_w, render_h) = render;
    let (present_w, present_h) = present;
    smaa.validate()?;
    chain.validate().map_err(AaScaleError::from)?;
    check_image(hdr, render_w, render_h)?;
    check_depth(depth, render_w, render_h)?;
    check_dims(present_w, present_h)?;

    let started = Instant::now();
    let settled = smaa_resolve_with_depth(hdr, depth, render_w, render_h, smaa)?;
    let smaa_ms = started.elapsed().as_secs_f64() * 1000.0;

    let started = Instant::now();
    let upsampled = post::taa_upsample(&settled, render_w, render_h, present_w, present_h)
        .map_err(AaScaleError::from)?;
    let upsample_ms = started.elapsed().as_secs_f64() * 1000.0;

    let chained = post::run_chain(&upsampled, present_w, present_h, chain, lights)
        .map_err(AaScaleError::from)?;

    let render_px = u64::from(render_w) * u64::from(render_h);
    let present_px = u64::from(present_w) * u64::from(present_h);
    Ok(TaaOffOutput {
        ldr: chained.ldr,
        width: present_w,
        height: present_h,
        receipt: TaaOffReceipt {
            smaa: post::PassReceipt {
                pass: "smaa",
                pixels: render_px,
                ms: smaa_ms,
            },
            upsample: post::PassReceipt {
                pass: "taau",
                pixels: present_px,
                ms: upsample_ms,
            },
            chain: chained.receipt,
        },
    })
}

// ---------------------------------------------------------------------------
// TAA-off graph wiring: SMAA first, then the RP1 upsample plus chain.
// ---------------------------------------------------------------------------

/// RP4 SMAA fallback pass name on the frame graph (TAA-off path).
pub const RP4_PASS_SMAA: &str = "rp4-smaa";

/// The six TAA-off pass names in execution order: SMAA at the render
/// size, then the RP1 upsample plus chain stages (reused names, never a
/// second chain).
pub fn taa_off_pass_names() -> [&'static str; 6] {
    [
        RP4_PASS_SMAA,
        post::RP1_PASS_TAAU,
        post::RP1_PASS_BLOOM,
        post::RP1_PASS_EXPOSURE,
        post::RP1_PASS_ACES,
        post::RP1_PASS_GRADE,
    ]
}

/// Declare the six TAA-off passes on a fresh RA2 frame graph for the WIN1
/// match frame with TAA off (and the ED1 viewport on the same shape):
/// SMAA at the render size, TAAU to the presentation size, then the four
/// chain stages. Resource flow infers `smaa -> taau -> bloom -> exposure
/// -> aces -> grade` through the shared `rp1-resolved` slot, so pass
/// order, aliasing and barriers stay RA2-owned; explicit edges pin the
/// same order so a resource-flow change can never reorder the chain.
/// Validates every dimension and the chain config before adding any pass.
pub fn build_taa_off_graph(
    render_w: u32,
    render_h: u32,
    present_w: u32,
    present_h: u32,
    chain: &PostChain,
) -> Result<crate::graph::FrameGraph, AaScaleError> {
    chain.validate().map_err(AaScaleError::from)?;
    check_dims(render_w, render_h)?;
    check_dims(present_w, present_h)?;
    let present_px = present_w as u64 * present_h as u64;
    let mut graph = crate::graph::FrameGraph::new();
    let smaa = graph.add_pass(
        crate::graph::PassDesc::new(RP4_PASS_SMAA)
            .input("rp4-current")
            .output("rp1-resolved")
            .work(1, 1),
    );
    let taau = graph.add_pass(
        crate::graph::PassDesc::new(post::RP1_PASS_TAAU)
            .input("rp1-resolved")
            .output("rp1-upsampled")
            .work(1, 1),
    );
    let bloom = graph.add_pass(
        crate::graph::PassDesc::new(post::RP1_PASS_BLOOM)
            .input("rp1-upsampled")
            .output("rp1-flared")
            .transient(
                "rp1-bloom-bytes",
                present_px * post::RP1_HISTORY_BYTES_PER_TEXEL,
            )
            .work(1, 1),
    );
    let exposure = graph.add_pass(
        crate::graph::PassDesc::new(post::RP1_PASS_EXPOSURE)
            .input("rp1-flared")
            .output("rp1-exposed")
            .work(1, 1),
    );
    let aces = graph.add_pass(
        crate::graph::PassDesc::new(post::RP1_PASS_ACES)
            .input("rp1-exposed")
            .output("rp1-toned")
            .work(1, 1),
    );
    let grade = graph.add_pass(
        crate::graph::PassDesc::new(post::RP1_PASS_GRADE)
            .input("rp1-toned")
            .output("rp1-ldr")
            .work(1, 1),
    );
    let order = [
        (smaa, taau),
        (taau, bloom),
        (bloom, exposure),
        (exposure, aces),
        (aces, grade),
    ];
    for (from, to) in order {
        graph
            .add_edge(from, to)
            .map_err(|e| AaScaleError::Invalid(format!("taa-off graph edge: {e}")))?;
    }
    Ok(graph)
}

/// WIN1 wiring: validate the match frame sizes plus the chain, declare
/// the six TAA-off graph passes, compile, and check the compiled order
/// names SMAA first with the RP1 upsample plus chain after it. Returns
/// the compiled graph the window submit records on the device. Fails
/// loudly on bad sizes or config.
pub fn compile_taa_off_for_present(
    render_w: u32,
    render_h: u32,
    present_w: u32,
    present_h: u32,
    chain: &PostChain,
) -> Result<crate::graph::CompiledGraph, AaScaleError> {
    let graph = build_taa_off_graph(render_w, render_h, present_w, present_h, chain)?;
    let compiled = graph
        .compile()
        .map_err(|e| AaScaleError::Invalid(format!("taa-off graph compile: {e}")))?;
    let names: Vec<&str> = compiled
        .order()
        .iter()
        .filter_map(|id| compiled.pass(*id).map(|p| p.name.as_str()))
        .collect();
    let want = taa_off_pass_names();
    if names != want {
        return Err(AaScaleError::Invalid(format!(
            "taa-off compiled order {names:?} differs from {want:?}"
        )));
    }
    Ok(compiled)
}

/// Transient backing estimate in bytes for a TAA-off frame: the resolved
/// target (render size, 16 B/texel) plus the present-size stage (8
/// B/texel) plus the bloom mip chain (present size halved per level, 16
/// B/texel). History-free: SMAA is spatial, so no history bytes.
/// Integer math only; used by the WIN1 budget check before submit.
pub fn taa_off_transient_bytes(
    render_w: u32,
    render_h: u32,
    present_w: u32,
    present_h: u32,
    chain: &PostChain,
) -> Result<u64, AaScaleError> {
    chain.validate().map_err(AaScaleError::from)?;
    check_dims(render_w, render_h)?;
    check_dims(present_w, present_h)?;
    let mut total = render_w as u64 * render_h as u64 * post::RP1_HISTORY_BYTES_PER_TEXEL;
    total =
        total.saturating_add(present_w as u64 * present_h as u64 * post::RP1_POST_BYTES_PER_TEXEL);
    let (mut mw, mut mh) = (present_w as u64, present_h as u64);
    for _ in 0..chain.bloom.levels {
        mw = mw.div_ceil(2).max(1);
        mh = mh.div_ceil(2).max(1);
        total = total.saturating_add(mw * mh * post::RP1_HISTORY_BYTES_PER_TEXEL);
        if mw == 1 && mh == 1 {
            break;
        }
    }
    Ok(total)
}

/// Scale-policy edge: the controller's current scale picks the render
/// size for the present size (min/max bounds and hysteresis live in the
/// controller), then the TAA-off graph is declared at that size.
/// Returns the graph plus the render size it was built at.
pub fn build_taa_off_graph_for_scale(
    controller: &ScaleController,
    present_w: u32,
    present_h: u32,
    chain: &PostChain,
) -> Result<(crate::graph::FrameGraph, u32, u32), AaScaleError> {
    let (render_w, render_h) = controller.render_size(present_w, present_h)?;
    let graph = build_taa_off_graph(render_w, render_h, present_w, present_h, chain)?;
    Ok((graph, render_w, render_h))
}

/// Scale-policy edge, compiled: same as [`build_taa_off_graph_for_scale`]
/// with the RA2 compile plus the SMAA-first order check. Returns the
/// compiled graph plus the render size it was built at.
pub fn compile_taa_off_for_scale(
    controller: &ScaleController,
    present_w: u32,
    present_h: u32,
    chain: &PostChain,
) -> Result<(crate::graph::CompiledGraph, u32, u32), AaScaleError> {
    let (render_w, render_h) = controller.render_size(present_w, present_h)?;
    let compiled = compile_taa_off_for_present(render_w, render_h, present_w, present_h, chain)?;
    Ok((compiled, render_w, render_h))
}

// ---------------------------------------------------------------------------
// GPU behind `window`: SMAA compute chunk plus a one-dispatch submitter.
// ---------------------------------------------------------------------------

/// SMAA fallback descriptors plus the compute chunk the RA1 presenter
/// consumes. Engine-owned code (no port markers).
#[cfg(feature = "window")]
pub mod gpu;

// ---------------------------------------------------------------------------
// Tests: shimmer TAA-off, edges, scale policy, failures, determinism.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::post::{PostChain, PostLights, ShimmerScene};

    fn lights() -> PostLights {
        PostLights {
            sun_ms: 1.25,
            cluster_ms: 0.5,
            ibl_ms: 2.0,
        }
    }

    fn step_edge(width: u32, height: u32, split: u32) -> Vec<[f32; 3]> {
        let mut out = Vec::with_capacity(width as usize * height as usize);
        for y in 0..height {
            for x in 0..width {
                let _ = y;
                out.push(if x < split {
                    [0.1, 0.1, 0.1]
                } else {
                    [0.9, 0.9, 0.9]
                });
            }
        }
        out
    }

    #[test]
    fn shimmer_passes_with_taa_off() {
        // TAA-off acceptance: the SMAA fallback alone settles the RP1
        // checkerboard under the posted gate, and beats the raw flicker.
        let scene = ShimmerScene::checkerboard().expect("scene builds");
        let raw_var = ShimmerScene::temporal_variance(&scene.frames);
        assert!(
            raw_var > SMAA_SHIMMER_MAX_VAR,
            "scene must actually shimmer, got {raw_var}"
        );
        let params = SmaaParams::default();
        let settled: Vec<Vec<[f32; 3]>> = scene
            .frames
            .iter()
            .map(|f| smaa_resolve(f, scene.width, scene.height, &params).expect("smaa runs"))
            .collect();
        let var = ShimmerScene::temporal_variance(&settled);
        eprintln!(
            "RP4 shimmer TAA-off: raw {raw_var:.4}, settled {var:.6} (gate {SMAA_SHIMMER_MAX_VAR})"
        );
        assert!(
            raw_var > post::SHIMMER_MAX_VAR,
            "scene must shimmer past the temporal gate too, got {raw_var}"
        );
        assert!(
            var <= SMAA_SHIMMER_MAX_VAR,
            "TAA-off settled variance {var} exceeds posted gate {SMAA_SHIMMER_MAX_VAR}"
        );
        assert!(
            var * 2.0 < raw_var,
            "SMAA must nearly halve the raw shimmer, raw {raw_var} settled {var}"
        );
        // Determinism: same inputs run bit-identical twice.
        let again = smaa_resolve(&scene.frames[0], scene.width, scene.height, &params)
            .expect("smaa runs again");
        assert_eq!(settled[0], again, "smaa pixels bit-identical twice");
    }

    #[test]
    fn smaa_keeps_flats_and_bounds_edges() {
        // Flat fields are the identity (not a box blur); the step edge
        // stays monotonic with no overshoot past either side.
        let params = SmaaParams::default();
        let flat = vec![[0.4, 0.2, 0.1]; 8 * 8];
        let kept = smaa_resolve(&flat, 8, 8, &params).expect("flat resolves");
        assert_eq!(kept, flat, "flat field is the identity");
        let edge = step_edge(16, 16, 8);
        let out = smaa_resolve(&edge, 16, 16, &params).expect("edge resolves");
        assert_eq!(out[0], edge[0], "far-left flat stays exact");
        assert_eq!(out[15], edge[15], "far-right flat stays exact");
        for y in 0..16 {
            let mut prev = f32::NEG_INFINITY;
            for x in 0..16 {
                let l = luminance(out[y as usize * 16 + x as usize]);
                assert!(l >= prev, "edge row {y} stays monotonic at x {x}");
                assert!(
                    (0.1..=0.9).contains(&l),
                    "edge row {y} never overshoots, got {l} at x {x}"
                );
                prev = l;
            }
        }
        // Zero blend is the identity everywhere (failure-shape guard).
        let off = smaa_resolve(
            &edge,
            16,
            16,
            &SmaaParams {
                max_blend: 0.0,
                ..params
            },
        )
        .expect("off resolves");
        assert_eq!(off, edge, "zero blend is the identity");
    }

    #[test]
    fn smaa_keeps_step_steeper_than_box_blur() {
        // SMAA-shape vs plain-blur comparison (Depth L2): a 3x3 box blur is
        // the naive shimmer fix. Both settle the step, but the edge-gated
        // cross kernel keeps a steeper transition (crisper edge, less blur).
        fn box_blur(hdr: &[[f32; 3]], width: u32, height: u32) -> Vec<[f32; 3]> {
            let w = width as usize;
            let h = height as usize;
            let mut out = Vec::with_capacity(w * h);
            for y in 0..h {
                for x in 0..width as usize {
                    let mut acc = [0.0f64; 3];
                    for dy in -1..=1i32 {
                        for dx in -1..=1i32 {
                            let sx = (x as i32 + dx).clamp(0, w as i32 - 1) as usize;
                            let sy = (y as i32 + dy).clamp(0, h as i32 - 1) as usize;
                            let p = hdr[sy * w + sx];
                            acc[0] += f64::from(p[0]);
                            acc[1] += f64::from(p[1]);
                            acc[2] += f64::from(p[2]);
                        }
                    }
                    out.push([
                        (acc[0] / 9.0) as f32,
                        (acc[1] / 9.0) as f32,
                        (acc[2] / 9.0) as f32,
                    ]);
                }
            }
            out
        }
        let params = SmaaParams::default();
        let edge = step_edge(16, 16, 8);
        let smaa = smaa_resolve(&edge, 16, 16, &params).expect("smaa resolves");
        let boxed = box_blur(&edge, 16, 16);
        let row = |frame: &[[f32; 3]], x: usize| luminance(frame[8 * 16 + x]);
        let smaa_slope = row(&smaa, 8) - row(&smaa, 7);
        let box_slope = row(&boxed, 8) - row(&boxed, 7);
        eprintln!("RP4 edge crispness: smaa slope {smaa_slope:.4}, box slope {box_slope:.4}");
        assert!(
            smaa_slope > box_slope,
            "smaa step {smaa_slope:.4} must stay steeper than box {box_slope:.4}"
        );
        assert!(
            box_slope > 0.0,
            "scene must actually blur under box, got {box_slope}"
        );
    }

    #[test]
    fn scale_policy_holds_60fps_under_load() {
        // Load model: fixed cost plus pixel-proportional cost. At full
        // rate the frame blows 60 fps; the policy must step down until
        // the tail holds the target, then recover when load clears.
        let policy = ScalePolicy {
            over_frames: 2,
            under_frames: 4,
            ..ScalePolicy::default()
        };
        let mut ctl = ScaleController::new(policy).expect("policy builds");
        assert_eq!(ctl.scale(), 1.0, "controller starts at full rate");
        let cost = |scale: f32| 4.0 + 20.0 * f64::from(scale * scale);
        for _ in 0..120 {
            let ms = cost(ctl.scale());
            ctl.update(ms).expect("update runs");
        }
        eprintln!(
            "RP4 scale under load: scale {:.2}, down {}, up {}",
            ctl.scale(),
            ctl.steps_down,
            ctl.steps_up
        );
        assert!(ctl.scale() < 1.0, "load must push the scale down");
        assert!(ctl.scale() >= policy.min_scale);
        let tail_ms = cost(ctl.scale());
        assert!(
            tail_ms <= TARGET_60FPS_MS,
            "tail cost {tail_ms:.2}ms must hold 60 fps"
        );
        // Load clears: the controller climbs back to full rate.
        let light = |scale: f32| 4.0 + 4.0 * f64::from(scale * scale);
        for _ in 0..120 {
            let ms = light(ctl.scale());
            ctl.update(ms).expect("update runs");
        }
        assert_eq!(ctl.scale(), 1.0, "cleared load recovers full rate");
        assert!(ctl.steps_up > 0, "recovery must step up");
    }

    #[test]
    fn scale_policy_fixtures() {
        // Single spikes never move the scale; the band never flaps it;
        // floors and ceilings clamp; sizes stay even and bounded.
        let policy = ScalePolicy {
            over_frames: 3,
            under_frames: 3,
            ..ScalePolicy::default()
        };
        let mut ctl = ScaleController::new(policy).expect("policy builds");
        ctl.update(TARGET_60FPS_MS + 5.0).expect("spike runs");
        assert_eq!(ctl.scale(), 1.0, "one spike never steps down");
        ctl.update(TARGET_60FPS_MS - 0.5).expect("band runs");
        assert_eq!(ctl.scale(), 1.0, "band frame resets, never flaps");
        for _ in 0..30 {
            ctl.update(TARGET_60FPS_MS + 9.0).expect("load runs");
        }
        assert_eq!(ctl.scale(), policy.min_scale, "floor clamps");
        assert!(ctl.steps_down > 0);
        let (rw, rh) = ctl.render_size(1920, 1080).expect("sizes run");
        assert_eq!((rw, rh), (960, 540), "half rate halves 1080p");
        assert_eq!(rw % 2, 0, "render width stays even");
        let (full_w, full_h) = render_size_for_scale(1920, 1080, 1.0).expect("full sizes");
        assert_eq!((full_w, full_h), (1920, 1080));
        let (odd_w, odd_h) = render_size_for_scale(1919, 1079, 0.5).expect("odd sizes");
        assert_eq!(odd_w % 2, 0, "odd present still yields even render");
        assert!((odd_w, odd_h) != (0, 0));
        assert!(matches!(
            render_size_for_scale(1920, 1080, 0.0),
            Err(AaScaleError::Invalid(_))
        ));
        assert!(matches!(
            render_size_for_scale(0, 1080, 0.5),
            Err(AaScaleError::Invalid(_))
        ));
        assert!(matches!(
            render_size_for_scale(4096, 1080, 0.5),
            Err(AaScaleError::BudgetExceeded { .. })
        ));
    }

    #[test]
    fn fc3_720p_pin_yields_1280x720() {
        let (w, h) = render_size_for_scale(1920, 1080, FC3_720P_SCALE).expect("720p pin sizes");
        assert_eq!((w, h), (1280, 720), "FC3 pin renders 720p for 1080p");
        assert!(render_size_for_scale(1920, 1080, -FC3_720P_SCALE).is_err());
    }

    #[test]
    fn taa_off_frame_reuses_rp1_chain() {
        // The TAA-off frame names smaa plus every RP1 chain pass, quotes
        // pixels and RL1/RL2 ms, and runs bit-identical twice.
        let hdr: Vec<[f32; 3]> = (0..256)
            .map(|i| [i as f32 * 0.01 + 0.2, 0.3, 0.15])
            .collect();
        let smaa = SmaaParams::default();
        let chain = PostChain::default();
        let run = || {
            run_taa_off_frame(&hdr, 16, 16, 32, 32, &smaa, &chain, &lights()).expect("frame runs")
        };
        let (a, b) = (run(), run());
        assert_eq!(a.ldr, b.ldr, "TAA-off pixels bit-identical twice");
        assert_eq!((a.width, a.height, a.ldr.len()), (32, 32, 1024));
        assert_eq!(a.receipt.smaa.pass, "smaa");
        assert_eq!(a.receipt.smaa.pixels, 256);
        assert_eq!(a.receipt.upsample.pass, "taau");
        assert_eq!(a.receipt.upsample.pixels, 1024);
        assert!((a.receipt.chain.lights_ms() - 3.75).abs() < 1e-9);
        assert!(a.receipt.total_ms() >= 0.0);
        assert_eq!(a.receipt.budget_ms(), TAA_OFF_BUDGET_MS);
        // Budget flags are PF2 evidence on the reference GPU, never wall
        // clock in tests: fully synthetic receipts pin the flag both ways.
        let flat = |ms: f64| -> TaaOffReceipt {
            let pass = |name: &'static str| post::PassReceipt {
                pass: name,
                pixels: 256,
                ms,
            };
            TaaOffReceipt {
                smaa: pass("smaa"),
                upsample: pass("taau"),
                chain: post::FramePostReceipt {
                    taa: pass("taa"),
                    upsample: pass("taau"),
                    bloom: pass("bloom"),
                    exposure: pass("exposure"),
                    aces: pass("aces"),
                    grade: pass("grade"),
                    sun_ms: 1.25,
                    cluster_ms: 0.5,
                    ibl_ms: 2.0,
                },
            }
        };
        assert!(flat(0.1).within_budget(), "0.1ms passes stay in budget");
        assert!(!flat(9.9).within_budget(), "9.9ms passes blow the budget");
        // Graph wiring declares the six TAA-off passes SMAA-first in
        // RA2 order, with an smaa -> taau barrier over `rp1-resolved`.
        let compiled = compile_taa_off_for_present(16, 16, 32, 32, &chain).expect("compiles");
        assert_eq!(compiled.order().len(), 6);
        let names: Vec<&str> = compiled
            .order()
            .iter()
            .filter_map(|id| compiled.pass(*id).map(|p| p.name.as_str()))
            .collect();
        assert_eq!(names, taa_off_pass_names());
        assert_eq!(
            names[0], RP4_PASS_SMAA,
            "taa-off leads with smaa, never taa"
        );
        assert!(
            compiled.barriers().iter().any(|b| {
                b.resource == "rp1-resolved"
                    && b.kind == crate::graph::BarrierKind::Transition
                    && b.from == compiled.order()[0]
                    && b.to == compiled.order()[1]
            }),
            "smaa -> taau hands over rp1-resolved"
        );
        assert!(matches!(
            compile_taa_off_for_present(0, 16, 32, 32, &chain),
            Err(AaScaleError::Invalid(_))
        ));
    }

    #[test]
    fn taa_off_graph_executes_smaa_in_order() {
        // Wiring proof (the RP4 consumer): the TAA-off graph compiles to
        // the six passes in add order, and executing it runs the real
        // SMAA plus upsample math inside the pass fns. One seeded frame
        // runs the whole graph twice: bit-identical bytes.
        use std::sync::{Arc, Mutex};

        struct Slots {
            hdr: Vec<[f32; 3]>,
            resolved: Vec<[f32; 3]>,
            upsampled: Vec<[f32; 3]>,
            ldr: Vec<[f32; 3]>,
            order: Vec<String>,
        }

        fn seeded_frame(seed: u64, len: usize) -> Vec<[f32; 3]> {
            // Integer-only LCG expansion: one seed, same bytes anywhere.
            let mut s = seed;
            let mut out = Vec::with_capacity(len);
            for _ in 0..len {
                let mut ch = [0.0f32; 3];
                for c in &mut ch {
                    s = s
                        .wrapping_mul(0x9E37_79B9_7F4A_7C15)
                        .wrapping_add(0xBF58_476D_1CE4_E5B9);
                    *c = 0.1 + 0.8 * ((s >> 33) as f32 / u32::MAX as f32);
                }
                out.push(ch);
            }
            out
        }

        #[expect(
            clippy::too_many_arguments,
            reason = "test fixture builder takes each field positionally"
        )]
        fn run_graph_once(
            hdr: &[[f32; 3]],
            render_w: u32,
            render_h: u32,
            present_w: u32,
            present_h: u32,
            smaa: &SmaaParams,
            chain: &PostChain,
            lights: &PostLights,
        ) -> (Vec<[f32; 3]>, Vec<String>) {
            let mut graph = build_taa_off_graph(render_w, render_h, present_w, present_h, chain)
                .expect("taa-off graph builds");
            assert_eq!(graph.pass_count(), 6, "taa-off declares six passes");
            let slots = Arc::new(Mutex::new(Slots {
                hdr: hdr.to_vec(),
                resolved: Vec::new(),
                upsampled: Vec::new(),
                ldr: Vec::new(),
                order: Vec::new(),
            }));
            let params = *smaa;
            let chain_c = *chain;
            let lights_c = *lights;
            // Pass 0 (smaa): the real fallback math inside the graph fn.
            {
                let slots = Arc::clone(&slots);
                graph
                    .set_pass_fn(0, move |_| {
                        let mut guard = slots.lock().expect("slots lock");
                        let settled = smaa_resolve(&guard.hdr, render_w, render_h, &params)
                            .expect("smaa runs in graph");
                        guard.resolved = settled;
                        guard.order.push(RP4_PASS_SMAA.into());
                    })
                    .expect("smaa fn records");
            }
            // Pass 1 (taau): the real RP1 upsample inside the graph fn.
            {
                let slots = Arc::clone(&slots);
                graph
                    .set_pass_fn(1, move |_| {
                        let mut guard = slots.lock().expect("slots lock");
                        let up = post::taa_upsample(
                            &guard.resolved,
                            render_w,
                            render_h,
                            present_w,
                            present_h,
                        )
                        .expect("taau runs in graph");
                        guard.upsampled = up;
                        guard.order.push(post::RP1_PASS_TAAU.into());
                    })
                    .expect("taau fn records");
            }
            // Passes 2..4 (bloom, exposure, aces): declared RA2 passes;
            // their pixel math ships inside the shared RP1 chain call at
            // the grade pass, so these record execution only instead of
            // duplicating RP1 internals.
            for (id, name) in [
                (2, post::RP1_PASS_BLOOM),
                (3, post::RP1_PASS_EXPOSURE),
                (4, post::RP1_PASS_ACES),
            ] {
                let slots = Arc::clone(&slots);
                graph
                    .set_pass_fn(id, move |_| {
                        slots.lock().expect("slots lock").order.push(name.into());
                    })
                    .expect("chain fn records");
            }
            // Pass 5 (grade): runs the shared RP1 chain over the
            // graph-upsampled frame, so the final pixels flow through the
            // graph like the window submit.
            {
                let slots = Arc::clone(&slots);
                graph
                    .set_pass_fn(5, move |_| {
                        let mut guard = slots.lock().expect("slots lock");
                        let out = post::run_chain(
                            &guard.upsampled,
                            present_w,
                            present_h,
                            &chain_c,
                            &lights_c,
                        )
                        .expect("chain runs in graph");
                        guard.ldr = out.ldr;
                        guard.order.push(post::RP1_PASS_GRADE.into());
                    })
                    .expect("grade fn records");
            }
            let mut compiled = graph.compile().expect("taa-off compiles");
            assert_eq!(compiled.order(), &[0, 1, 2, 3, 4, 5]);
            let receipt = compiled.execute(&crate::graph::FrameInfo {
                frame_index: 7,
                width_px: present_w,
                height_px: present_h,
            });
            assert_eq!(receipt.per_pass_cpu_ms.len(), 6);
            assert_eq!((receipt.draws, receipt.triangles), (6, 6));
            let guard = slots.lock().expect("slots lock");
            let want: Vec<String> = taa_off_pass_names().iter().map(|s| s.to_string()).collect();
            assert_eq!(guard.order, want, "all six passes ran in order");
            // In-graph slots match their oracles texel for texel.
            let expect_resolved =
                smaa_resolve(hdr, render_w, render_h, smaa).expect("oracle smaa runs");
            assert_eq!(guard.resolved, expect_resolved);
            let expect_up =
                post::taa_upsample(&expect_resolved, render_w, render_h, present_w, present_h)
                    .expect("oracle taau runs");
            assert_eq!(guard.upsampled, expect_up);
            let oracle = run_taa_off_frame(
                hdr, render_w, render_h, present_w, present_h, smaa, chain, lights,
            )
            .expect("oracle frame runs");
            assert_eq!(guard.ldr, oracle.ldr, "graph pixels match the oracle");
            (guard.ldr.clone(), guard.order.clone())
        }

        let smaa = SmaaParams::default();
        let chain = PostChain::default();
        let hdr = seeded_frame(0x51ED5EED04u64, 16 * 16);
        let (first_ldr, first_order) =
            run_graph_once(&hdr, 16, 16, 32, 32, &smaa, &chain, &lights());
        let (second_ldr, second_order) =
            run_graph_once(&hdr, 16, 16, 32, 32, &smaa, &chain, &lights());
        assert_eq!(first_ldr, second_ldr, "one seed gives identical bytes x2");
        assert_eq!(
            first_order, second_order,
            "one seed gives identical order x2"
        );
        assert_eq!(first_ldr.len(), 1024);
        eprintln!(
            "RP4 graph execute: 6 passes 16x16 -> 32x32, {} texels bit-identical x2",
            first_ldr.len()
        );
        // Failure edge: bad sizes fail before any pass fn can run.
        assert!(matches!(
            build_taa_off_graph(0, 16, 32, 32, &chain),
            Err(AaScaleError::Invalid(_))
        ));
    }

    #[test]
    fn scale_floor_drives_taa_off_graph() {
        // Scale-policy edges: hysteresis floors the scale under load, the
        // floor picks the render size, and the TAA-off graph compiles
        // SMAA-first at that size with history-free transients. Clearing
        // the load recovers full rate and the full-size graph.
        let policy = ScalePolicy {
            over_frames: 2,
            under_frames: 4,
            ..ScalePolicy::default()
        };
        let mut ctl = ScaleController::new(policy).expect("policy builds");
        let chain = PostChain::default();
        for _ in 0..40 {
            ctl.update(TARGET_60FPS_MS + 9.0).expect("load runs");
        }
        assert_eq!(ctl.scale(), policy.min_scale, "load floors the scale");
        let (compiled, rw, rh) =
            compile_taa_off_for_scale(&ctl, 1920, 1080, &chain).expect("floor compiles");
        assert_eq!((rw, rh), (960, 540), "floor halves 1080p");
        assert_eq!(compiled.order().len(), 6);
        let names: Vec<&str> = compiled
            .order()
            .iter()
            .filter_map(|id| compiled.pass(*id).map(|p| p.name.as_str()))
            .collect();
        assert_eq!(names, taa_off_pass_names());
        assert_eq!(
            names[0], RP4_PASS_SMAA,
            "taa-off leads with smaa, never taa"
        );
        let bytes = taa_off_transient_bytes(rw, rh, 1920, 1080, &chain).expect("bytes run");
        let rp1 = post::rp1_transient_bytes(rw, rh, 1920, 1080, chain.bloom.levels)
            .expect("rp1 bytes run");
        assert!(
            bytes < rp1,
            "history-free taa-off {bytes} stays under rp1 {rp1}"
        );
        for _ in 0..120 {
            ctl.update(8.0).expect("light runs");
        }
        assert_eq!(ctl.scale(), 1.0, "cleared load recovers full rate");
        let (full, frw, frh) =
            compile_taa_off_for_scale(&ctl, 1920, 1080, &chain).expect("full compiles");
        assert_eq!((frw, frh), (1920, 1080));
        assert_eq!(full.order().len(), 6);
        // Failure edges: zero and over-bound present sizes fail loudly.
        assert!(matches!(
            compile_taa_off_for_scale(&ctl, 0, 1080, &chain),
            Err(AaScaleError::Invalid(_))
        ));
        assert!(matches!(
            compile_taa_off_for_scale(&ctl, 4096, 1080, &chain),
            Err(AaScaleError::BudgetExceeded { .. })
        ));
        eprintln!("RP4 scale graph: floor {rw}x{rh} {bytes}B, full {frw}x{frh}");
    }

    #[test]
    fn invalid_inputs_fail_loudly() {
        let params = SmaaParams::default();
        let frame = vec![[0.5, 0.5, 0.5]; 16];
        assert!(matches!(
            smaa_resolve(&frame[..15], 4, 4, &params),
            Err(AaScaleError::Invalid(_))
        ));
        assert!(matches!(
            smaa_resolve(&frame, 0, 4, &params),
            Err(AaScaleError::Invalid(_))
        ));
        assert!(matches!(
            smaa_resolve(&frame, 4096, 4, &params),
            Err(AaScaleError::BudgetExceeded { .. })
        ));
        let mut bad = frame.clone();
        bad[3] = [f32::NAN, 0.0, 0.0];
        assert!(matches!(
            smaa_resolve(&bad, 4, 4, &params),
            Err(AaScaleError::Invalid(_))
        ));
        assert!(matches!(
            SmaaParams {
                edge_threshold: -0.1,
                ..params
            }
            .validate(),
            Err(AaScaleError::Invalid(_))
        ));
        assert!(matches!(
            SmaaParams {
                max_blend: 2.0,
                ..params
            }
            .validate(),
            Err(AaScaleError::Invalid(_))
        ));
        assert!(matches!(
            ScalePolicy {
                min_scale: 0.9,
                max_scale: 0.5,
                ..ScalePolicy::default()
            }
            .validate(),
            Err(AaScaleError::Invalid(_))
        ));
        assert!(matches!(
            ScalePolicy {
                step: 0.0,
                ..ScalePolicy::default()
            }
            .validate(),
            Err(AaScaleError::Invalid(_))
        ));
        let mut ctl = ScaleController::new(ScalePolicy::default()).expect("policy builds");
        assert!(matches!(
            ctl.update(f64::NAN),
            Err(AaScaleError::Invalid(_))
        ));
        assert!(matches!(ctl.update(-1.0), Err(AaScaleError::Invalid(_))));
        assert!(matches!(
            run_taa_off_frame(
                &frame,
                4,
                4,
                8,
                8,
                &params,
                &PostChain::default(),
                &lights()
            )
            .map(|_| ()),
            Ok(())
        ));
        assert!(matches!(
            run_taa_off_frame(
                &frame[..15],
                4,
                4,
                8,
                8,
                &params,
                &PostChain::default(),
                &lights()
            ),
            Err(AaScaleError::Invalid(_))
        ));
        assert!(format!("{}", AaScaleError::Device("needs a device".into())).contains("device"));
        assert!(format!("{}", AaScaleError::Validation("scope hit".into())).contains("validation"));
        assert!(format!(
            "{}",
            AaScaleError::BudgetExceeded {
                requested: 9,
                allowed: 4
            }
        )
        .contains('9'));
    }

    #[cfg(feature = "window")]
    fn test_device() -> Option<(wgpu::Device, wgpu::Queue)> {
        crate::test_gpu::hold_gpu_slot();
        let instance = crate::backend::new_instance(wgpu::Backends::PRIMARY);
        let adapter =
            match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })) {
                Ok(a) => a,
                Err(_) => {
                    eprintln!("SKIPPED rp4 gpu test: no wgpu adapter on this machine");
                    return None;
                }
            };
        match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("rp4-test"),
            ..Default::default()
        })) {
            Ok((d, q)) => Some((d, q)),
            Err(e) => panic!("test device requests succeed when the adapter exists: {e}"),
        }
    }

    #[cfg(feature = "window")]
    #[test]
    fn gpu_smaa_matches_cpu_and_settles_shimmer() {
        // Device SMAA over the shimmer pair: parity against the CPU oracle
        // within tolerance, settled variance under the posted gate, 0
        // validation errors.
        let (device, queue) = match test_device() {
            Some(d) => d,
            None => return,
        };
        let sub = gpu::SmaaSubmitter::new(&device);
        assert!(!gpu::SMAA_COMPUTE_WGSL.is_empty());
        let scene = ShimmerScene::checkerboard().expect("scene builds");
        let params = SmaaParams::default();
        let mut worst_diff = 0f32;
        let mut clip = Vec::new();
        for frame in &scene.frames {
            let cpu = smaa_resolve(frame, scene.width, scene.height, &params).expect("cpu runs");
            let g = sub
                .resolve(&device, &queue, frame, scene.width, scene.height, &params)
                .expect("gpu runs");
            assert_eq!(g.validation_errors, 0, "gpu smaa is validation clean");
            worst_diff = worst_diff.max(gpu::max_abs_diff(&cpu, &g.pixels));
            clip.push(g.pixels);
        }
        let settled = ShimmerScene::temporal_variance(&clip);
        eprintln!(
            "RP4 gpu smaa: settled {settled:.6} (gate {SMAA_SHIMMER_MAX_VAR}), parity {worst_diff:.6} (tol {})",
            gpu::GPU_PARITY_TOL_SMAA
        );
        assert!(
            worst_diff <= gpu::GPU_PARITY_TOL_SMAA,
            "gpu/cpu parity {worst_diff} exceeds {}",
            gpu::GPU_PARITY_TOL_SMAA
        );
        assert!(
            settled <= SMAA_SHIMMER_MAX_VAR,
            "gpu settled variance {settled} exceeds posted gate {SMAA_SHIMMER_MAX_VAR}"
        );
        // Failure case first: bad input fails before any submit.
        assert!(matches!(
            sub.resolve(
                &device,
                &queue,
                &scene.frames[0][..100],
                scene.width,
                scene.height,
                &params
            ),
            Err(AaScaleError::Invalid(_))
        ));
    }

    #[cfg(feature = "window")]
    #[test]
    fn gpu_taa_off_frame_reuses_rp1_submit() {
        // Full TAA-off frame on the device: CPU SMAA, then the live
        // `Rp1Submitter` over smaa-equals-history (its TAA resolve is the
        // identity there), matching the CPU TAA-off oracle within the RP1
        // post tolerance with 0 validation errors. No second post chain.
        let (device, queue) = match test_device() {
            Some(d) => d,
            None => return,
        };
        let scene = ShimmerScene::checkerboard().expect("scene builds");
        let params = SmaaParams::default();
        let chain = PostChain::default();
        let cpu = run_taa_off_frame(
            &scene.frames[0],
            scene.width,
            scene.height,
            scene.width,
            scene.height,
            &params,
            &chain,
            &lights(),
        )
        .expect("cpu frame runs");
        let settled =
            smaa_resolve(&scene.frames[0], scene.width, scene.height, &params).expect("smaa runs");
        let motion =
            crate::post::MotionField::zero(scene.width, scene.height).expect("zero builds");
        let taa = crate::post::TaaParams::default();
        let sub = crate::post::gpu::Rp1Submitter::new(&device);
        let g = sub
            .run_frame(
                &device,
                &queue,
                &settled,
                &settled,
                &motion,
                scene.width,
                scene.height,
                scene.width,
                scene.height,
                &taa,
                &chain,
                &lights(),
                [0.0, 0.0],
            )
            .expect("gpu frame runs");
        assert_eq!(g.validation_errors, 0, "gpu frame is validation clean");
        let diff = gpu::max_abs_diff(&cpu.ldr, &g.ldr);
        eprintln!(
            "RP4 gpu taa-off frame: parity {diff:.6} (tol {})",
            crate::post::gpu::GPU_PARITY_TOL_POST
        );
        assert!(
            diff <= crate::post::gpu::GPU_PARITY_TOL_POST,
            "gpu/cpu frame parity {diff} exceeds {}",
            crate::post::gpu::GPU_PARITY_TOL_POST
        );
    }

    #[test]
    fn smaa_receipt_budget_accessors_are_exact() {
        // Exactness default (no wall clock): the SMAA bar is 0.5 ms at
        // 1080p on boosted release clocks (PF2 enforces); synthetic
        // TAA-off receipts pin the flag both ways. Wall-clock enforcement
        // lives in smaa_resolve_wall_budget (--perf only, load-dependent).
        assert_eq!(SMAA_BUDGET_MS, 0.5, "smaa bar stays 0.5 ms");
        let flat = |ms: f64| -> TaaOffReceipt {
            let pass = |name: &'static str| post::PassReceipt {
                pass: name,
                pixels: 256,
                ms,
            };
            TaaOffReceipt {
                smaa: pass("smaa"),
                upsample: pass("taau"),
                chain: post::FramePostReceipt {
                    taa: pass("taa"),
                    upsample: pass("taau"),
                    bloom: pass("bloom"),
                    exposure: pass("exposure"),
                    aces: pass("aces"),
                    grade: pass("grade"),
                    sun_ms: 1.25,
                    cluster_ms: 0.5,
                    ibl_ms: 2.0,
                },
            }
        };
        assert!(
            flat(0.05).within_budget(),
            "small smaa passes stay in budget"
        );
        assert!(
            !flat(9.9).within_budget(),
            "over-budget smaa reads over budget"
        );
    }

    #[test]
    #[ignore = "timing: smaa resolve wall-clock needs release --perf on boosted clocks (fails under parallel load)"]
    fn smaa_resolve_wall_budget() {
        // Seeded budget regression: the same 64x64 resolve the default
        // suite proves exact must clear the 0.5 ms bar on boosted release
        // clocks. Perf harness line keeps --perf medians readable.
        let params = SmaaParams::default();
        let hdr = vec![[0.35, 0.3, 0.25]; 64 * 64];
        let started = std::time::Instant::now();
        let _ = smaa_resolve(&hdr, 64, 64, &params).expect("smaa runs");
        let ms = started.elapsed().as_secs_f64() * 1000.0;
        eprintln!("smaa_resolve_wall_budget: measured {ms:.3} ms vs {SMAA_BUDGET_MS} ms budget");
        assert!(
            ms <= SMAA_BUDGET_MS,
            "seeded smaa over budget: {ms:.3} ms vs {SMAA_BUDGET_MS} ms"
        );
    }

    #[test]
    fn predicated_flat_depth_reproduces_luma() {
        // Flat depth carries no edge, so the predicated resolve must match
        // the luma-only oracle bit-for-bit on every shimmer frame, and run
        // bit-identical twice (determinism).
        let params = SmaaPredicatedParams::default();
        let scene = ShimmerScene::checkerboard().expect("scene builds");
        let flat_depth = vec![2.5f32; scene.frames[0].len()];
        for frame in &scene.frames {
            let luma =
                smaa_resolve(frame, scene.width, scene.height, &params.base).expect("luma runs");
            let pred =
                smaa_resolve_with_depth(frame, &flat_depth, scene.width, scene.height, &params)
                    .expect("predicated runs");
            assert_eq!(
                pred, luma,
                "flat depth reproduces the luma-only pass bit-for-bit"
            );
        }
        let again = smaa_resolve_with_depth(
            &scene.frames[0],
            &flat_depth,
            scene.width,
            scene.height,
            &params,
        )
        .expect("predicated runs again");
        let once = smaa_resolve_with_depth(
            &scene.frames[0],
            &flat_depth,
            scene.width,
            scene.height,
            &params,
        )
        .expect("predicated runs once");
        assert_eq!(once, again, "predicated pixels bit-identical twice");
    }

    #[test]
    fn predicated_depth_edge_blends_flat_luma() {
        // Luma step below the luma threshold stays the identity under the
        // luma-only pass, but the depth discontinuity at the same texels
        // forces the predicated blend. Far flats stay exact either way.
        let params = SmaaPredicatedParams::default();
        let (w, h) = (16u32, 16u32);
        let mut hdr = Vec::with_capacity(256);
        let mut depth = Vec::with_capacity(256);
        for y in 0..16usize {
            for x in 0..16usize {
                let _ = y;
                hdr.push(if x < 8 {
                    [0.40, 0.40, 0.40]
                } else {
                    [0.44, 0.44, 0.44]
                });
                depth.push(if x < 8 { 1.0f32 } else { 3.0f32 });
            }
        }
        let luma = smaa_resolve(&hdr, w, h, &params.base).expect("luma runs");
        assert_eq!(luma, hdr, "sub-threshold luma step is the identity");
        let pred = smaa_resolve_with_depth(&hdr, &depth, w, h, &params).expect("predicated runs");
        assert_eq!(pred[0], hdr[0], "far-left flat stays exact");
        assert_eq!(pred[255], hdr[255], "far-right flat stays exact");
        assert_ne!(
            pred, hdr,
            "the depth step must blend where luma alone keeps the identity"
        );
        let straddles = [7usize, 8usize, 23usize, 24usize];
        let mut moved = 0;
        for i in straddles {
            let dl = (pred[i][0] - hdr[i][0]).abs();
            if dl > 0.0 {
                moved += 1;
            }
        }
        assert!(
            moved >= 2,
            "texels straddling the depth step must move, moved {moved}"
        );
        // Depth ramp shape: flat is zero, past twice the threshold it pins
        // at the blend ceiling.
        assert_eq!(smaa_depth_weight(0.0, &params), 0.0);
        assert_eq!(
            smaa_depth_weight(params.depth_threshold_m * 2.0, &params),
            params.base.max_blend
        );
        assert_eq!(
            smaa_depth_weight(params.depth_threshold_m * 9.0, &params),
            params.base.max_blend
        );
    }

    #[test]
    fn predicated_shimmer_settles_with_taa_off() {
        // TAA-off acceptance on the predicated path: flat depth settles the
        // checkerboard under the posted gate exactly like luma-only,
        // and beats the raw flicker.
        let scene = ShimmerScene::checkerboard().expect("scene builds");
        let raw_var = ShimmerScene::temporal_variance(&scene.frames);
        assert!(
            raw_var > SMAA_SHIMMER_MAX_VAR,
            "scene must actually shimmer, got {raw_var}"
        );
        let params = SmaaPredicatedParams::default();
        let flat_depth = vec![2.0f32; scene.frames[0].len()];
        let settled: Vec<Vec<[f32; 3]>> = scene
            .frames
            .iter()
            .map(|f| {
                smaa_resolve_with_depth(f, &flat_depth, scene.width, scene.height, &params)
                    .expect("predicated runs")
            })
            .collect();
        let var = ShimmerScene::temporal_variance(&settled);
        let luma_settled: Vec<Vec<[f32; 3]>> = scene
            .frames
            .iter()
            .map(|f| smaa_resolve(f, scene.width, scene.height, &params.base).expect("luma runs"))
            .collect();
        let luma_var = ShimmerScene::temporal_variance(&luma_settled);
        eprintln!("RP7 predicated shimmer TAA-off: raw {raw_var:.4}, settled {var:.6} (gate {SMAA_SHIMMER_MAX_VAR})");
        assert!(
            var <= SMAA_SHIMMER_MAX_VAR,
            "predicated settled variance {var} exceeds posted gate {SMAA_SHIMMER_MAX_VAR}"
        );
        assert_eq!(
            var, luma_var,
            "flat-depth predicated settles exactly like luma-only"
        );
        assert!(
            var * 2.0 < raw_var,
            "predicated must nearly halve the raw shimmer, raw {raw_var} settled {var}"
        );
    }

    #[test]
    fn predicated_invalid_inputs_fail_loudly() {
        // Every external input validates before any blend runs: depth size,
        // finiteness and sign, threshold ranges, plus the shared image bar.
        let params = SmaaPredicatedParams::default();
        assert_eq!(
            params.depth_threshold_m, SMAA_DEPTH_EDGE_THRESHOLD_M,
            "default depth threshold stays posted"
        );
        let frame = vec![[0.5, 0.5, 0.5]; 16];
        let depth = vec![1.0f32; 16];
        assert!(matches!(
            smaa_resolve_with_depth(&frame, &depth[..15], 4, 4, &params),
            Err(AaScaleError::Invalid(_))
        ));
        assert!(matches!(
            smaa_resolve_with_depth(&frame, &depth, 0, 4, &params),
            Err(AaScaleError::Invalid(_))
        ));
        assert!(matches!(
            smaa_resolve_with_depth(&frame, &depth, 4096, 4, &params),
            Err(AaScaleError::BudgetExceeded { .. })
        ));
        let mut nan_depth = depth.clone();
        nan_depth[3] = f32::NAN;
        assert!(matches!(
            smaa_resolve_with_depth(&frame, &nan_depth, 4, 4, &params),
            Err(AaScaleError::Invalid(_))
        ));
        let mut neg_depth = depth.clone();
        neg_depth[5] = -1.0;
        assert!(matches!(
            smaa_resolve_with_depth(&frame, &neg_depth, 4, 4, &params),
            Err(AaScaleError::Invalid(_))
        ));
        let mut nan_luma = frame.clone();
        nan_luma[2] = [f32::NAN, 0.0, 0.0];
        assert!(matches!(
            smaa_resolve_with_depth(&nan_luma, &depth, 4, 4, &params),
            Err(AaScaleError::Invalid(_))
        ));
        for bad_threshold in [0.0, -0.02, f32::NAN, f32::INFINITY] {
            assert!(matches!(
                SmaaPredicatedParams {
                    depth_threshold_m: bad_threshold,
                    ..params
                }
                .validate(),
                Err(AaScaleError::Invalid(_))
            ));
        }
        assert!(matches!(
            SmaaPredicatedParams {
                base: SmaaParams {
                    edge_threshold: -0.1,
                    ..params.base
                },
                ..params
            }
            .validate(),
            Err(AaScaleError::Invalid(_))
        ));
        params
            .validate()
            .expect("default predicated params validate");
    }

    #[test]
    fn taa_off_frame_with_depth_matches_luma_frame_on_flat_depth() {
        // Consumer wiring: the depth-aware TAA-off frame reuses the same
        // upsample plus chain, so flat depth reproduces the luma-only
        // frame pixels bit-for-bit and runs bit-identical twice.
        let hdr: Vec<[f32; 3]> = (0..256)
            .map(|i| [i as f32 * 0.01 + 0.2, 0.3, 0.15])
            .collect();
        let depth = vec![2.0f32; 256];
        let smaa = SmaaPredicatedParams::default();
        let chain = PostChain::default();
        let run_pred = || {
            run_taa_off_frame_with_depth(&hdr, &depth, (16, 16), (32, 32), &smaa, &chain, &lights())
                .expect("predicated frame runs")
        };
        let luma = run_taa_off_frame(&hdr, 16, 16, 32, 32, &smaa.base, &chain, &lights())
            .expect("luma frame runs");
        let (a, b) = (run_pred(), run_pred());
        assert_eq!(a.ldr, b.ldr, "predicated frame pixels bit-identical twice");
        assert_eq!(
            a.ldr, luma.ldr,
            "flat depth reproduces the luma-only frame bit-for-bit"
        );
        assert_eq!((a.width, a.height, a.ldr.len()), (32, 32, 1024));
        assert_eq!(a.receipt.smaa.pass, "smaa");
        assert_eq!(a.receipt.smaa.pixels, 256);
        assert_eq!(a.receipt.upsample.pass, "taau");
        assert_eq!(a.receipt.upsample.pixels, 1024);
        assert_eq!(a.receipt.budget_ms(), TAA_OFF_BUDGET_MS);
        assert!(matches!(
            run_taa_off_frame_with_depth(
                &hdr,
                &depth[..100],
                (16, 16),
                (32, 32),
                &smaa,
                &chain,
                &lights()
            ),
            Err(AaScaleError::Invalid(_))
        ));
    }

    #[cfg(feature = "window")]
    #[test]
    fn gpu_predicated_matches_cpu_and_settles_shimmer() {
        // Device predicated SMAA over the shimmer pair with flat depth:
        // parity against the CPU oracle within tolerance, settled variance
        // under the posted gate, 0 validation errors; plus one depth-step
        // frame proving the depth gate runs on the device too.
        let (device, queue) = match test_device() {
            Some(d) => d,
            None => return,
        };
        let sub = gpu::SmaaPredicatedSubmitter::new(&device);
        assert!(!gpu::SMAA_PREDICATED_COMPUTE_WGSL.is_empty());
        let scene = ShimmerScene::checkerboard().expect("scene builds");
        let params = SmaaPredicatedParams::default();
        let flat_depth = vec![2.0f32; scene.frames[0].len()];
        let mut worst_diff = 0f32;
        let mut clip = Vec::new();
        for frame in &scene.frames {
            let cpu =
                smaa_resolve_with_depth(frame, &flat_depth, scene.width, scene.height, &params)
                    .expect("cpu runs");
            let g = sub
                .resolve_with_depth(
                    &device,
                    &queue,
                    frame,
                    &flat_depth,
                    (scene.width, scene.height),
                    &params,
                )
                .expect("gpu runs");
            assert_eq!(g.validation_errors, 0, "gpu predicated is validation clean");
            worst_diff = worst_diff.max(gpu::max_abs_diff(&cpu, &g.pixels));
            clip.push(g.pixels);
        }
        // One depth-step frame with a sub-threshold luma step: the device
        // must match the CPU blend there as well.
        let mut step_hdr = Vec::with_capacity(256);
        let mut step_depth = Vec::with_capacity(256);
        for y in 0..16usize {
            for x in 0..16usize {
                let _ = y;
                step_hdr.push(if x < 8 {
                    [0.40, 0.40, 0.40]
                } else {
                    [0.44, 0.44, 0.44]
                });
                step_depth.push(if x < 8 { 1.0f32 } else { 3.0f32 });
            }
        }
        let cpu_step =
            smaa_resolve_with_depth(&step_hdr, &step_depth, 16, 16, &params).expect("cpu runs");
        let gpu_step = sub
            .resolve_with_depth(&device, &queue, &step_hdr, &step_depth, (16, 16), &params)
            .expect("gpu runs");
        worst_diff = worst_diff.max(gpu::max_abs_diff(&cpu_step, &gpu_step.pixels));
        let settled = ShimmerScene::temporal_variance(&clip);
        eprintln!(
            "RP7 gpu predicated: settled {settled:.6} (gate {SMAA_SHIMMER_MAX_VAR}), parity {worst_diff:.6} (tol {})",
            gpu::GPU_PARITY_TOL_PREDICATED
        );
        assert!(
            worst_diff <= gpu::GPU_PARITY_TOL_PREDICATED,
            "gpu/cpu parity {worst_diff} exceeds {}",
            gpu::GPU_PARITY_TOL_PREDICATED
        );
        assert!(
            settled <= SMAA_SHIMMER_MAX_VAR,
            "gpu settled variance {settled} exceeds posted gate {SMAA_SHIMMER_MAX_VAR}"
        );
        // Failure case first: bad depth fails before any submit.
        assert!(matches!(
            sub.resolve_with_depth(
                &device,
                &queue,
                &scene.frames[0],
                &flat_depth[..100],
                (scene.width, scene.height),
                &params
            ),
            Err(AaScaleError::Invalid(_))
        ));
    }

    #[test]
    fn shadow_pcf_msaa_shimmer() {
        use crate::shadow_cascade as sc;
        assert_eq!(sc::SHADOW_MAP_SIZE_HIGH, 2048);
        assert_eq!((msaa_sample_count(false), msaa_sample_count(true)), (1, 4));
        let (w, h) = (16u32, 16u32);
        let mut even = Vec::with_capacity(256);
        let mut odd = Vec::with_capacity(256);
        let mut depth = Vec::with_capacity(256);
        for y in 0..16u32 {
            for x in 0..16u32 {
                let _ = y;
                even.push(if x < 8 {
                    [0.1, 0.1, 0.1]
                } else {
                    [0.9, 0.9, 0.9]
                });
                odd.push(if x < 9 {
                    [0.1, 0.1, 0.1]
                } else {
                    [0.9, 0.9, 0.9]
                });
                depth.push(if x < 8 { 1.0f32 } else { 3.0f32 });
            }
        }
        let mut map = vec![0.0f32; 64];
        for y in 0..8usize {
            for x in 0..8usize {
                map[y * 8 + x] = if x < 4 { 0.1 } else { 0.3 };
            }
        }
        let off_p = ShadowMsaaSettleParams {
            shadow_size: 8,
            bias: 0.005,
            msaa_on: false,
            origin_m: 0.0,
            half_extent_m: 10.0,
            smaa: SmaaParams::default(),
        };
        let on_p = ShadowMsaaSettleParams {
            msaa_on: true,
            ..off_p
        };
        let off_e =
            settle_with_shadow_pcf_msaa(&even, &depth, w, h, &map, &off_p).expect("off runs");
        let on_e = settle_with_shadow_pcf_msaa(&even, &depth, w, h, &map, &on_p).expect("on runs");
        assert_ne!(off_e, on_e, "MSAA on must change the picture");
        assert_eq!(off_e[8], even[8], "off 1x keeps the step");
        let l = luminance(on_e[8]);
        assert!(l > 0.1 && l < 0.9, "on blends the edge, got {l}");
        assert_eq!(on_e[0], even[0], "far-left interior holds");
        assert_eq!(on_e[255], even[255], "far-right interior holds");
        let off_o =
            settle_with_shadow_pcf_msaa(&odd, &depth, w, h, &map, &off_p).expect("off runs");
        let on_o = settle_with_shadow_pcf_msaa(&odd, &depth, w, h, &map, &on_p).expect("on runs");
        let raw_var = crate::post::ShimmerScene::temporal_variance(&[even.clone(), odd.clone()]);
        let off_var = crate::post::ShimmerScene::temporal_variance(&[off_e.clone(), off_o.clone()]);
        let on_var = crate::post::ShimmerScene::temporal_variance(&[on_e.clone(), on_o.clone()]);
        eprintln!("shadow-pcf-msaa: raw {raw_var:.4} off {off_var:.4} on {on_var:.6} (gate {MSAA_SHIMMER_GATE})");
        assert!(raw_var > 0.06, "scene must shimmer, got {raw_var}");
        assert!(
            on_var <= MSAA_SHIMMER_GATE,
            "settled {on_var} over {MSAA_SHIMMER_GATE}"
        );
        assert!(on_var < off_var, "on must calm below off");
        let again =
            settle_with_shadow_pcf_msaa(&even, &depth, w, h, &map, &on_p).expect("runs again");
        assert_eq!(on_e, again, "settle bit-identical x2");
    }
}
