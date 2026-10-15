//! fog: height fog plus contact shadows (Frostbite/Decima school, reimplemented).
//!
//! Owner: render lane; consumer WIN1/ED1 forward pass (`fog_factor` plus
//! `contact_ao` below), ED1 viewport, Q1 captures. Units: meters for depth,
//! heights and falloff, pixels for sizes and radii, ms in receipts. Fog ramps
//! from the bound RL4 day capture ([`FogParams::from_sky_capture`]); contact
//! shadows occlude small details only (radius capped). Determinism: row-major
//! integer orders, fixed tap tables, f64 accumulation on the CPU bakes; same
//! complete inputs give bit-identical bytes twice. Errors are typed
//! [`FogError`]; the headless twin names the window device bind while the
//! window build runs both passes for real with 0 validation errors.

use std::fmt;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Pass names, resources and budgets: the WIN1 forward-pass contract.
// ---------------------------------------------------------------------------

/// Graph pass name for the height-fog compute.
pub const FOG_PASS: &str = "fog-height";
/// Graph pass name for the contact-shadow compute.
pub const CONTACT_PASS: &str = "contact-shadow";
/// Graph resource name for the fog factor buffer the forward pass reads.
pub const FOG_RESOURCE: &str = "fog_factor";
/// Graph resource name for the contact AO buffer the forward pass reads.
pub const CONTACT_RESOURCE: &str = "contact_ao";
/// Posted GPU budget for the height-fog pass at 1080p (PF2 enforces).
pub const FOG_BUDGET_MS: f64 = 0.2;
/// Posted GPU budget for the contact-shadow pass at 1080p (PF2 enforces).
pub const CONTACT_BUDGET_MS: f64 = 0.3;
/// Bytes per fog-factor texel on the device (R32 float, full res).
pub const FOG_BYTES_PER_TEXEL: u64 = 4;
/// Bytes per contact-AO texel on the device (R32 float, full res).
pub const CONTACT_BYTES_PER_TEXEL: u64 = 4;
/// Largest transient backing the RA2 aliasing plan must hold for both fog
/// buffers at 1080p (2 * 1920 * 1080 * 4 = 16,588,800) plus headroom.
pub const FOG_TRANSIENT_BUDGET_BYTES: u64 = 20_000_000;
/// Posted CPU/GPU parity tolerance: worst absolute difference allowed
/// between the f64 CPU bakes and the f32 device passes (llvmpipe gate).
pub const FOG_GPU_PARITY_TOL: f32 = 0.004;
/// Posted fog-ramp tolerance: worst absolute channel difference allowed
/// between a fully fogged composite and the bound sky fog color.
pub const FOG_SKY_RAMP_TOL: f32 = 0.02;
/// Cleanest accepted fog density in per-meter extinction units.
pub const FOG_DENSITY_MIN: f32 = 0.0;
/// Thickest accepted fog density in per-meter extinction units.
pub const FOG_DENSITY_MAX: f32 = 4.0;
/// Shallowest accepted height falloff in meters.
pub const FOG_FALLOFF_MIN_M: f32 = 0.05;
/// Deepest accepted height falloff in meters.
pub const FOG_FALLOFF_MAX_M: f32 = 64.0;
/// Lowest accepted fog base height in meters.
pub const FOG_BASE_MIN_M: f32 = -64.0;
/// Highest accepted fog base height in meters.
pub const FOG_BASE_MAX_M: f32 = 64.0;
/// Shortest accepted fog cutoff distance in meters.
pub const FOG_DISTANCE_MIN_M: f32 = 1.0;
/// Longest accepted fog cutoff distance in meters.
pub const FOG_DISTANCE_MAX_M: f32 = 2000.0;
/// Fixed contact slice axes (same school as the GTAO pass, small radius).
pub const CONTACT_SLICES: usize = 4;
/// Taps per slice side; each slice samples both directions.
pub const CONTACT_TAPS: usize = 2;
/// Largest accepted contact radius in pixels (small details only).
pub const CONTACT_RADIUS_MAX_PX: f32 = 16.0;
/// Largest accepted contact thickness in meters.
pub const CONTACT_THICKNESS_MAX_M: f32 = 2.0;

/// Slice axes in pixel steps: +x, +y and the two diagonals.
const CONTACT_DIRS: [(i32, i32); CONTACT_SLICES] = [(1, 0), (0, 1), (1, 1), (1, -1)];

// ---------------------------------------------------------------------------
// Errors: typed; bad input fails loudly, the headless twin names the bind
// it cannot run without a device, the window build never takes that path.
// ---------------------------------------------------------------------------

/// Fog pass failures. `NoDevice` covers headless-twin calls with no wgpu
/// device in hand; `Device`/`Validation` cover device-side failures on the
/// window build, which never takes the twin path.
#[derive(Clone, PartialEq, Debug)]
pub enum FogError {
    /// Caller input is unusable; reason carried.
    Invalid(String),
    /// No device behind this call; what to build first, carried.
    NoDevice(String),
    /// A device, queue or map call failed; message carried.
    Device(String),
    /// A wgpu validation error fired inside the submit scope; kept loud.
    Validation(String),
}

impl fmt::Display for FogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "fog input: {e}"),
            Self::NoDevice(e) => write!(f, "fog bind needs a device: {e}"),
            Self::Device(e) => write!(f, "fog device: {e}"),
            Self::Validation(e) => write!(f, "fog validation: {e}"),
        }
    }
}

impl std::error::Error for FogError {}

/// Map an RL4 sky failure into the fog input error (the fog color source).
fn sky_err(e: crate::sky::SkyError) -> FogError {
    FogError::Invalid(format!("fog sky capture: {e}"))
}

/// Map a look-frame failure into the fog input error (shared bind shape).
fn look_err(e: crate::gtao::LookError) -> FogError {
    FogError::Invalid(format!("fog look frame: {e}"))
}

// ---------------------------------------------------------------------------
// Fog params: height extinction plus the bound day color.
// ---------------------------------------------------------------------------

/// Height-fog inputs: the fog color ramped from the bound RL4 day capture
/// plus Frostbite-style height extinction (`density * exp(-(h-base)/falloff)`
/// times view distance, `1-exp(-od)` factor). Validated up front.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FogParams {
    /// Fog color in linear HDR (pre-exposure), from [`crate::sky::fog_color`].
    pub fog_color: [f32; 3],
    /// Extinction density per meter, `FOG_DENSITY_MIN..=FOG_DENSITY_MAX`.
    pub density_per_m: f32,
    /// Height falloff in meters, `FOG_FALLOFF_MIN_M..=FOG_FALLOFF_MAX_M`.
    pub falloff_m: f32,
    /// Fog base height in meters, `FOG_BASE_MIN_M..=FOG_BASE_MAX_M`.
    pub base_height_m: f32,
    /// Fog cutoff distance in meters, `FOG_DISTANCE_MIN_M..=FOG_DISTANCE_MAX_M`.
    pub max_distance_m: f32,
    /// Exposure anchor in EV100, must be finite (carried for the bind).
    pub exposure_ev100: f32,
}

impl FogParams {
    /// Day default: horizon color filled by [`FogParams::from_sky_capture`].
    pub fn day() -> Self {
        Self {
            fog_color: [0.5, 0.6, 0.7],
            density_per_m: 0.02,
            falloff_m: 8.0,
            base_height_m: 0.0,
            max_distance_m: 500.0,
            exposure_ev100: 12.0,
        }
    }

    /// Ramp the fog color from the bound day capture so sky, IBL ambient and
    /// fog can never drift; extinction comes from the caller. Fails loudly
    /// when the capture backing is ragged or holds non-finite radiance.
    pub fn from_sky_capture(
        capture: &crate::sky::SkyCapture,
        density_per_m: f32,
        falloff_m: f32,
        base_height_m: f32,
        max_distance_m: f32,
        exposure_ev100: f32,
    ) -> Result<Self, FogError> {
        let fog_color = crate::sky::fog_color(capture).map_err(sky_err)?;
        let params = Self {
            fog_color,
            density_per_m,
            falloff_m,
            base_height_m,
            max_distance_m,
            exposure_ev100,
        };
        params.validate()?;
        Ok(params)
    }

    /// Validate color, extinction, heights, distance and exposure.
    pub fn validate(&self) -> Result<(), FogError> {
        if !self.fog_color.iter().all(|c| c.is_finite()) || self.fog_color.iter().any(|c| *c < 0.0)
        {
            return Err(FogError::Invalid(format!(
                "fog color {:?} is not finite nonnegative",
                self.fog_color
            )));
        }
        if !self.density_per_m.is_finite()
            || self.density_per_m < FOG_DENSITY_MIN
            || self.density_per_m > FOG_DENSITY_MAX
        {
            return Err(FogError::Invalid(format!(
                "fog density {} outside {FOG_DENSITY_MIN}..={FOG_DENSITY_MAX}",
                self.density_per_m
            )));
        }
        if !self.falloff_m.is_finite()
            || self.falloff_m < FOG_FALLOFF_MIN_M
            || self.falloff_m > FOG_FALLOFF_MAX_M
        {
            return Err(FogError::Invalid(format!(
                "fog falloff {} outside {FOG_FALLOFF_MIN_M}..={FOG_FALLOFF_MAX_M}",
                self.falloff_m
            )));
        }
        if !self.base_height_m.is_finite()
            || self.base_height_m < FOG_BASE_MIN_M
            || self.base_height_m > FOG_BASE_MAX_M
        {
            return Err(FogError::Invalid(format!(
                "fog base {} outside {FOG_BASE_MIN_M}..={FOG_BASE_MAX_M}",
                self.base_height_m
            )));
        }
        if !self.max_distance_m.is_finite()
            || self.max_distance_m < FOG_DISTANCE_MIN_M
            || self.max_distance_m > FOG_DISTANCE_MAX_M
        {
            return Err(FogError::Invalid(format!(
                "fog distance {} outside {FOG_DISTANCE_MIN_M}..={FOG_DISTANCE_MAX_M}",
                self.max_distance_m
            )));
        }
        if !self.exposure_ev100.is_finite() {
            return Err(FogError::Invalid("fog exposure_ev100 is not finite".into()));
        }
        Ok(())
    }

    /// Full-res fog-factor transient bytes for a frame (`texels * 4`).
    pub fn factor_bytes(width_px: u32, height_px: u32) -> Result<u64, FogError> {
        check_dims(width_px, height_px)?;
        Ok(width_px as u64 * height_px as u64 * FOG_BYTES_PER_TEXEL)
    }
}

/// Validate a frame size against the shared look bound before allocation.
fn check_dims(width_px: u32, height_px: u32) -> Result<(), FogError> {
    if width_px == 0 || height_px == 0 {
        return Err(FogError::Invalid(format!(
            "fog frame {width_px}x{height_px} is zero"
        )));
    }
    let bound = crate::gtao::MAX_LOOK_DIM;
    if width_px > bound || height_px > bound {
        return Err(FogError::Invalid(format!(
            "fog frame {width_px}x{height_px} exceeds {bound}"
        )));
    }
    Ok(())
}

fn check_depth_buffer(depth: &[f32], want: usize, what: &'static str) -> Result<(), FogError> {
    if depth.len() != want {
        return Err(FogError::Invalid(format!(
            "fog {what} length {} mismatches frame {want}",
            depth.len()
        )));
    }
    for (i, d) in depth.iter().enumerate() {
        if !d.is_finite() || *d <= 0.0 {
            return Err(FogError::Invalid(format!(
                "fog {what}[{i}] depth {d} is not positive finite"
            )));
        }
    }
    Ok(())
}

fn check_height_buffer(height: &[f32], want: usize) -> Result<(), FogError> {
    if height.len() != want {
        return Err(FogError::Invalid(format!(
            "fog height length {} mismatches frame {want}",
            height.len()
        )));
    }
    for (i, h) in height.iter().enumerate() {
        if !h.is_finite() {
            return Err(FogError::Invalid(format!(
                "fog height[{i}] value {h} is not finite"
            )));
        }
    }
    Ok(())
}

pub(crate) fn normalize3(v: [f32; 3]) -> Option<[f32; 3]> {
    if !v.iter().all(|c| c.is_finite()) {
        return None;
    }
    let len2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
    if len2 < 1e-12 {
        return None;
    }
    let len = len2.sqrt();
    Some([v[0] / len, v[1] / len, v[2] / len])
}

fn normalized_normals(normals: &[[f32; 3]], want: usize) -> Result<Vec<[f32; 3]>, FogError> {
    if normals.len() != want {
        return Err(FogError::Invalid(format!(
            "fog normals length {} mismatches frame {want}",
            normals.len()
        )));
    }
    let mut out = Vec::with_capacity(want);
    for (i, n) in normals.iter().enumerate() {
        match normalize3(*n) {
            Some(v) => out.push(v),
            None => {
                return Err(FogError::Invalid(format!(
                    "fog normals[{i}] {n:?} is not a finite nonzero direction"
                )));
            }
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Height-fog bake: per-pixel extinction factor plus the sky-ramp composite.
// ---------------------------------------------------------------------------

/// Per-pixel height-fog factor: `0.0` clear, `1.0` fully fogged. Row-major,
/// `width_px * height_px` values in `0..=1`.
#[derive(Clone, PartialEq, Debug)]
pub struct FogField {
    /// Frame width in pixels.
    pub width_px: u32,
    /// Frame height in pixels.
    pub height_px: u32,
    /// Row-major fog factors in `0..=1`.
    pub factor: Vec<f32>,
    /// Backing fog color (set by the bake; equals the bound sky fog color).
    color: [f32; 3],
}

impl FogField {
    /// Texel count.
    pub fn texels(&self) -> usize {
        self.factor.len()
    }

    /// Read one factor; bounds-checked for callers outside the bake.
    pub fn texel(&self, x: u32, y: u32) -> Result<f32, FogError> {
        if x >= self.width_px || y >= self.height_px {
            return Err(FogError::Invalid(format!(
                "fog texel ({x},{y}) outside {}x{}",
                self.width_px, self.height_px
            )));
        }
        Ok(self.factor[(y * self.width_px + x) as usize])
    }

    /// Deterministic little-endian bytes: row-major f32 factors. Same inputs
    /// give bit-identical bytes on every run.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.factor.len() * 4);
        for v in &self.factor {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    /// Composite lit base radiance toward the bound fog color by the baked
    /// factor (`base * (1-f) + fog * f`). Length mismatches fail loudly.
    pub fn composite(&self, base: &[[f32; 3]]) -> Result<Vec<[f32; 3]>, FogError> {
        if base.len() != self.texels() {
            return Err(FogError::Invalid(format!(
                "fog composite base {} mismatches field {}",
                base.len(),
                self.texels()
            )));
        }
        Ok(composite_fog(base, &self.factor, self.fog_color_hint()))
    }

    /// Fog color this field was baked against (equals the bound sky fog color).
    pub fn fog_color_hint(&self) -> [f32; 3] {
        self.color
    }
}

/// Mix base radiance toward fog by factor (f64 accumulation, own loop so both
/// the field composite and the drift probe share one writer).
fn composite_fog(base: &[[f32; 3]], factor: &[f32], fog: [f32; 3]) -> Vec<[f32; 3]> {
    let mut out = Vec::with_capacity(base.len());
    for (b, f) in base.iter().zip(factor.iter()) {
        let t = (*f as f64).clamp(0.0, 1.0);
        let mut px = [0.0f32; 3];
        for c in 0..3 {
            px[c] = (b[c] as f64 * (1.0 - t) + fog[c] as f64 * t) as f32;
        }
        out.push(px);
    }
    out
}

/// Bake height fog from view distance (`depth_m`, meters, positive finite)
/// and fragment world height (`height_m`, meters, finite): Frostbite-style
/// extinction `density * exp(-(h-base)/falloff)` times the cutoff-clamped
/// distance, factor `1-exp(-od)`. Row-major integer order, f64 math.
/// Returns the field plus wall-clock bake ms (evidence only, never hashed).
pub fn bake_height_fog(
    depth_m: &[f32],
    height_m: &[f32],
    params: &FogParams,
    width_px: u32,
    height_px: u32,
) -> Result<(FogField, f64), FogError> {
    params.validate()?;
    check_dims(width_px, height_px)?;
    let n = width_px as usize * height_px as usize;
    check_depth_buffer(depth_m, n, "depth")?;
    check_height_buffer(height_m, n)?;
    let started = Instant::now();
    let density = params.density_per_m as f64;
    let falloff = params.falloff_m as f64;
    let base = params.base_height_m as f64;
    let cutoff = params.max_distance_m as f64;
    let mut factor = vec![0.0f32; n];
    for i in 0..n {
        let d = (depth_m[i] as f64).min(cutoff);
        let mut e = -((height_m[i] as f64 - base) / falloff);
        e = e.clamp(-8.0, 8.0);
        let od = density * d * e.exp();
        factor[i] = (1.0 - (-od).exp()).clamp(0.0, 1.0) as f32;
    }
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok((
        FogField {
            width_px,
            height_px,
            factor,
            color: params.fog_color,
        },
        ms,
    ))
}

/// IBL-drift check: the baked fog color must equal the bound capture's
/// horizon ring within [`FOG_SKY_RAMP_TOL`]. Call it per time-of-day change;
/// a drifted fog (stale bake, wrong capture) fails loudly instead of tinting
/// the arena.
pub fn check_fog_matches_capture(
    fog_rgb: [f32; 3],
    capture: &crate::sky::SkyCapture,
) -> Result<(), FogError> {
    let ring = crate::sky::fog_color(capture).map_err(sky_err)?;
    for c in 0..3 {
        if !fog_rgb[c].is_finite() {
            return Err(FogError::Invalid(format!(
                "fog drift probe color {fog_rgb:?} is not finite"
            )));
        }
        if (fog_rgb[c] - ring[c]).abs() > FOG_SKY_RAMP_TOL {
            return Err(FogError::Invalid(format!(
                "fog drift: channel {c} {} vs sky ring {} over tol {FOG_SKY_RAMP_TOL}",
                fog_rgb[c], ring[c]
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Sun-scatter tint (RL10 spike): anisotropic glow behind the RL9 feed.
// ---------------------------------------------------------------------------

/// Cornette-Shanks asymmetry for the sun-scatter tint (same forward-peaked
/// Mie school as the RL4 sky bake; presentation math, never hashed).
pub const SUN_SCATTER_G: f32 = 0.76;
/// Peak achromatic gain at exact sun alignment (`1 + STRENGTH * glow`).
pub const SUN_TINT_STRENGTH: f32 = 0.6;
/// Per-channel warm weights: red lifts most, blue least (sunward warmth).
pub const SUN_TINT_WARM: [f32; 3] = [1.0, 0.8, 0.6];
/// Posted extra budget for the tint over the FOG bar (PF2 enforces): one
/// uniform-color transform per frame, no per-texel work.
pub const SUN_TINT_BUDGET_MS: f64 = 0.05;
/// Largest gain the tint can apply (`1 + STRENGTH * WARM red`).
pub const SUN_TINT_MAX_GAIN: f32 = 1.6;

/// Cornette-Shanks Mie phase for `cos_theta` (f64 accumulation, one writer
/// for the glow normalizer below).
fn sun_phase(cos_theta: f64) -> f64 {
    let g = f64::from(SUN_SCATTER_G);
    let denom = (1.0 + g * g - 2.0 * g * cos_theta).max(1e-6).powf(1.5);
    0.119_366_2 * (1.0 - g * g) * (1.0 + cos_theta * cos_theta) / ((2.0 + g * g) * denom)
}

/// Normalize a finite nonzero direction into f64; `None` names no caller.
fn norm_dir64(v: [f32; 3]) -> Option<[f64; 3]> {
    if !v.iter().all(|c| c.is_finite()) {
        return None;
    }
    let len2 = f64::from(v[0]) * f64::from(v[0])
        + f64::from(v[1]) * f64::from(v[1])
        + f64::from(v[2]) * f64::from(v[2]);
    if len2 < 1e-12 {
        return None;
    }
    let len = len2.sqrt();
    Some([
        f64::from(v[0]) / len,
        f64::from(v[1]) / len,
        f64::from(v[2]) / len,
    ])
}

/// Normalized sun-forward glow for a view ray (`view_dir`, eye toward the
/// fragment) against the sun (`sun_dir`, toward the sun): `1.0` looking at
/// the sun, `0.0` perpendicular or away (clamped). Pure arithmetic, same
/// inputs give the same bits twice. Bad directions fail loudly.
pub fn sun_scatter_glow(view_dir: [f32; 3], sun_dir: [f32; 3]) -> Result<f32, FogError> {
    let v = norm_dir64(view_dir).ok_or_else(|| {
        FogError::Invalid(format!(
            "sun tint view_dir {view_dir:?} is not a finite nonzero direction"
        ))
    })?;
    let s = norm_dir64(sun_dir).ok_or_else(|| {
        FogError::Invalid(format!(
            "sun tint sun_dir {sun_dir:?} is not a finite nonzero direction"
        ))
    })?;
    let cos = (v[0] * s[0] + v[1] * s[1] + v[2] * s[2]).clamp(-1.0, 1.0);
    let glow = (sun_phase(cos) - sun_phase(0.0)) / (sun_phase(1.0) - sun_phase(0.0));
    Ok(glow.clamp(0.0, 1.0) as f32)
}

/// Tint a bound fog color toward sun warmth by the view-sun alignment:
/// perpendicular views keep the ramp color exactly (drift probe still
/// holds), sun-aligned views gain up to [`SUN_TINT_MAX_GAIN`] on red.
/// Non-finite or negative base colors fail loudly.
pub fn tinted_fog_color(
    base: [f32; 3],
    sun_dir: [f32; 3],
    view_dir: [f32; 3],
) -> Result<[f32; 3], FogError> {
    if !base.iter().all(|c| c.is_finite()) || base.iter().any(|c| *c < 0.0) {
        return Err(FogError::Invalid(format!(
            "sun tint base {base:?} is not finite nonnegative"
        )));
    }
    let glow = f64::from(sun_scatter_glow(view_dir, sun_dir)?);
    let mut out = [0.0f32; 3];
    for c in 0..3 {
        out[c] = (f64::from(base[c])
            * (1.0 + f64::from(SUN_TINT_STRENGTH) * glow * f64::from(SUN_TINT_WARM[c])))
            as f32;
    }
    if !out.iter().all(|c| c.is_finite()) {
        return Err(FogError::Invalid(format!(
            "sun tint {base:?} overflowed finite range"
        )));
    }
    Ok(out)
}

impl FogParams {
    /// Copy these params with the fog color tinted for one frame: `sun_dir`
    /// is the frame sun (the [`crate::gtao::LookFrame`] sun), `view_dir` is
    /// the camera forward. WIN1 presenter call site (outside this lane):
    /// `params.with_sun_tint(frame.sun_dir, camera_forward)` ahead of
    /// `ensure_fog_plan`/`FogSubmitter::submit`; Q1 captures ride the same
    /// feed. Extinction is untouched, so factors bake bit-identical.
    pub fn with_sun_tint(&self, sun_dir: [f32; 3], view_dir: [f32; 3]) -> Result<Self, FogError> {
        self.validate()?;
        let fog_color = tinted_fog_color(self.fog_color, sun_dir, view_dir)?;
        let tinted = Self { fog_color, ..*self };
        tinted.validate()?;
        Ok(tinted)
    }
}

// ---------------------------------------------------------------------------
// Contact shadows: short-range screen-space occlusion for small details.
// ---------------------------------------------------------------------------

/// Contact-shadow configuration: full-res frame size, small screen-space
/// radius in pixels, depth thickness in meters and occlusion strength.
/// Validated up front; the radius cap keeps the pass on small details.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ContactConfig {
    /// Full-res frame width in pixels, `1..=MAX_LOOK_DIM`.
    pub width_px: u32,
    /// Full-res frame height in pixels, `1..=MAX_LOOK_DIM`.
    pub height_px: u32,
    /// Screen-space radius in pixels, `(0, CONTACT_RADIUS_MAX_PX]`.
    pub radius_px: f32,
    /// Depth thickness in meters, `(0, CONTACT_THICKNESS_MAX_M]`.
    pub thickness_m: f32,
    /// Occlusion strength, `0..=1`.
    pub intensity: f32,
}

impl ContactConfig {
    /// Small-detail default: 6 px radius over a 16 px cap.
    pub fn small() -> Self {
        Self {
            width_px: 64,
            height_px: 64,
            radius_px: 6.0,
            thickness_m: 0.1,
            intensity: 0.9,
        }
    }

    /// Validate dims and kernel parameters before any allocation.
    pub fn validate(&self) -> Result<(), FogError> {
        check_dims(self.width_px, self.height_px)?;
        if !self.radius_px.is_finite()
            || self.radius_px <= 0.0
            || self.radius_px > CONTACT_RADIUS_MAX_PX
        {
            return Err(FogError::Invalid(format!(
                "contact radius {} outside (0, {CONTACT_RADIUS_MAX_PX}]",
                self.radius_px
            )));
        }
        if !self.thickness_m.is_finite()
            || self.thickness_m <= 0.0
            || self.thickness_m > CONTACT_THICKNESS_MAX_M
        {
            return Err(FogError::Invalid(format!(
                "contact thickness {} outside (0, {CONTACT_THICKNESS_MAX_M}]",
                self.thickness_m
            )));
        }
        if !self.intensity.is_finite() || self.intensity < 0.0 || self.intensity > 1.0 {
            return Err(FogError::Invalid(format!(
                "contact intensity {} outside 0..=1",
                self.intensity
            )));
        }
        Ok(())
    }

    /// Full-res texel count.
    pub fn texels(&self) -> usize {
        self.width_px as usize * self.height_px as usize
    }

    /// Integer tap step in pixels: `ceil(radius / TAPS)`, at least 1.
    pub fn tap_step_px(&self) -> i32 {
        (self.radius_px / CONTACT_TAPS as f32).ceil().max(1.0) as i32
    }

    /// Full-res contact transient bytes (`texels * 4`).
    pub fn transient_bytes(&self) -> Result<u64, FogError> {
        self.validate()?;
        Ok(self.texels() as u64 * CONTACT_BYTES_PER_TEXEL)
    }
}

/// Occlusion normalizer: sum of `1/(1+t)` over both sides of every slice.
/// One writer for the CPU bake and the device uniform below.
fn contact_weight_sum() -> f64 {
    let mut weight_sum = 0.0f64;
    for _ in 0..CONTACT_SLICES {
        for _ in 0..2 {
            for t in 1..=CONTACT_TAPS {
                weight_sum += 1.0 / (1.0 + t as f64);
            }
        }
    }
    weight_sum
}

/// Full-res contact occlusion: `1.0` open, `0.0` fully shadowed. Row-major.
#[derive(Clone, PartialEq, Debug)]
pub struct ContactAo {
    /// Frame width in pixels.
    pub width_px: u32,
    /// Frame height in pixels.
    pub height_px: u32,
    /// Row-major AO values in `0..=1`.
    pub values: Vec<f32>,
}

impl ContactAo {
    /// Texel count.
    pub fn texels(&self) -> usize {
        self.values.len()
    }

    /// Read one texel; bounds-checked for callers outside the bake.
    pub fn texel(&self, x: u32, y: u32) -> Result<f32, FogError> {
        if x >= self.width_px || y >= self.height_px {
            return Err(FogError::Invalid(format!(
                "contact texel ({x},{y}) outside {}x{}",
                self.width_px, self.height_px
            )));
        }
        Ok(self.values[(y * self.width_px + x) as usize])
    }

    /// Deterministic little-endian bytes: row-major f32 values.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.values.len() * 4);
        for v in &self.values {
            out.extend_from_slice(&v.to_le_bytes());
        }
        out
    }

    /// Bind seam for the RA1 forward contact bind on the headless twin:
    /// validates the frame and the shape, then names the missing device bind.
    /// The window build next to it binds the baked buffer for real.
    #[cfg(not(feature = "window"))]
    pub fn submit_gpu(&self, frame: &crate::gtao::LookFrame) -> Result<FogReceipt, FogError> {
        frame.validate().map_err(look_err)?;
        if frame.viewport_px != [self.width_px, self.height_px] {
            return Err(FogError::Invalid(format!(
                "contact {}x{} mismatches frame {}x{}",
                self.width_px, self.height_px, frame.viewport_px[0], frame.viewport_px[1]
            )));
        }
        Err(FogError::NoDevice(
            "contact GPU bind needs the window build with a live device plus the RA1 forward pass bind group; the CPU bake is the reference"
                .into(),
        ))
    }

    /// Bind the baked contact buffer on the device: uploads it, runs one
    /// bind-group probe pass over it and reads it back exactly, proving the
    /// forward bind validates with 0 errors. Fails loudly (never silent,
    /// never the twin refusal) on bad shapes or any device fault.
    #[cfg(feature = "window")]
    pub fn submit_gpu(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &crate::gtao::LookFrame,
    ) -> Result<FogReceipt, FogError> {
        frame.validate().map_err(look_err)?;
        if frame.viewport_px != [self.width_px, self.height_px] {
            return Err(FogError::Invalid(format!(
                "contact {}x{} mismatches frame {}x{}",
                self.width_px, self.height_px, frame.viewport_px[0], frame.viewport_px[1]
            )));
        }
        submit_bind_probe(
            device,
            queue,
            frame,
            &self.to_bytes(),
            self.texels(),
            "contact",
        )
    }
}

/// Bake contact shadows from full-res depth (view distance, meters) and
/// normals: fixed slice taps in integer order; a neighbor closer than the
/// center by more than `thickness_m` occludes, scaled by normal alignment.
/// Flat fields stay `1.0`; step edges shadow the far side. Returns the AO
/// buffer plus wall-clock bake ms (evidence only, never hashed).
pub fn bake_contact(
    depth_m: &[f32],
    normals: &[[f32; 3]],
    config: &ContactConfig,
) -> Result<(ContactAo, f64), FogError> {
    config.validate()?;
    let w = config.width_px as usize;
    let h = config.height_px as usize;
    let n = config.texels();
    check_depth_buffer(depth_m, n, "contact depth")?;
    let flat = normalized_normals(normals, n)?;
    let started = Instant::now();
    let step = config.tap_step_px();
    let thickness = config.thickness_m as f64;
    let intensity = config.intensity as f64;
    let weight_sum = contact_weight_sum();
    let mut values = vec![1.0f32; n];
    for y in 0..h {
        for x in 0..w {
            let ci = y * w + x;
            let cd = depth_m[ci] as f64;
            let nc = flat[ci];
            let mut occ = 0.0f64;
            for (dx, dy) in CONTACT_DIRS {
                for t in 1..=CONTACT_TAPS {
                    let off = t as i32 * step;
                    let w_tap = 1.0 / (1.0 + t as f64);
                    for side in [1i32, -1i32] {
                        let nx = (x as i32 + dx * off * side).clamp(0, w as i32 - 1) as usize;
                        let ny = (y as i32 + dy * off * side).clamp(0, h as i32 - 1) as usize;
                        let ni = ny * w + nx;
                        let gap = cd - depth_m[ni] as f64;
                        if gap > thickness {
                            let nn = flat[ni];
                            let align =
                                (nc[0] * nn[0] + nc[1] * nn[1] + nc[2] * nn[2]).max(0.0) as f64;
                            occ += w_tap * align;
                        }
                    }
                }
            }
            values[ci] = (1.0 - intensity * (occ / weight_sum)).clamp(0.0, 1.0) as f32;
        }
    }
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok((
        ContactAo {
            width_px: config.width_px,
            height_px: config.height_px,
            values,
        },
        ms,
    ))
}

// ---------------------------------------------------------------------------
// Graph passes as data: height fog plus contact shadows feeding the WIN1
// forward pass. WIN1 wires `fog` -> `forward` and `contact` -> `forward`
// with the forward pass declaring `.input("fog_factor")` plus
// `.input("contact_ao")`; RA2 owns order and aliasing.
// ---------------------------------------------------------------------------

/// Fog graph wiring for one frame: pass names, resources and transient
/// bytes. Both buffers are production transients the RA2 aliasing plan must
/// hold inside [`FOG_TRANSIENT_BUDGET_BYTES`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FogPassPlan {
    /// Graph pass running the height-fog compute.
    pub fog_pass: &'static str,
    /// Graph pass running the contact-shadow compute.
    pub contact_pass: &'static str,
    /// Fog-factor resource the forward pass reads.
    pub fog_resource: &'static str,
    /// Contact-AO resource the forward pass reads.
    pub contact_resource: &'static str,
    /// Fog-factor transient bytes (`texels * 4`).
    pub fog_bytes: u64,
    /// Contact-AO transient bytes (`texels * 4`).
    pub contact_bytes: u64,
}

/// Validate the configs, then describe the two graph passes plus the bytes
/// the RA2 aliasing plan must hold. Pure data, no allocation.
pub fn fog_pass_plan(params: &FogParams, contact: &ContactConfig) -> Result<FogPassPlan, FogError> {
    params.validate()?;
    contact.validate()?;
    let fog_bytes = FogParams::factor_bytes(contact.width_px, contact.height_px)?;
    let contact_bytes = contact.transient_bytes()?;
    let total = fog_bytes.saturating_add(contact_bytes);
    if total > FOG_TRANSIENT_BUDGET_BYTES {
        return Err(FogError::Invalid(format!(
            "fog transients {total} over budget {FOG_TRANSIENT_BUDGET_BYTES}"
        )));
    }
    Ok(FogPassPlan {
        fog_pass: FOG_PASS,
        contact_pass: CONTACT_PASS,
        fog_resource: FOG_RESOURCE,
        contact_resource: CONTACT_RESOURCE,
        fog_bytes,
        contact_bytes,
    })
}

/// Declare the fog and contact passes plus the WIN1 forward pass on a fresh
/// RA2 frame graph: fog writes `fog_factor`, contact writes `contact_ao`,
/// the forward pass reads both over `hdr`. Resource flow plus explicit edges
/// pin `fog -> forward` and `contact -> forward`.
pub fn build_fog_graph(
    params: &FogParams,
    contact: &ContactConfig,
) -> Result<crate::graph::FrameGraph, FogError> {
    let plan = fog_pass_plan(params, contact)?;
    let mut graph = crate::graph::FrameGraph::new();
    let fog = graph.add_pass(
        crate::graph::PassDesc::new(plan.fog_pass)
            .input("depth")
            .input("sky_capture")
            .output(plan.fog_resource)
            .transient(plan.fog_resource, plan.fog_bytes)
            .work(1, 1),
    );
    let contact_id = graph.add_pass(
        crate::graph::PassDesc::new(plan.contact_pass)
            .input("depth")
            .input("normals")
            .output(plan.contact_resource)
            .transient(plan.contact_resource, plan.contact_bytes)
            .work(1, 1),
    );
    let forward = graph.add_pass(
        crate::graph::PassDesc::new("forward")
            .input(plan.fog_resource)
            .input(plan.contact_resource)
            .input("hdr")
            .output("hdr")
            .work(256, 1_800_000),
    );
    graph
        .add_edge(fog, forward)
        .map_err(|e| FogError::Invalid(format!("fog graph edge: {e}")))?;
    graph
        .add_edge(contact_id, forward)
        .map_err(|e| FogError::Invalid(format!("fog graph edge: {e}")))?;
    Ok(graph)
}

/// WIN1 wiring: validate the match frame inputs, declare fog plus contact
/// plus forward, compile, and check the compiled order runs both fog passes
/// before the forward pass. Returns the compiled graph the window submit
/// records on the device. Fails loudly on bad configs.
pub fn compile_fog_for_forward(
    params: &FogParams,
    contact: &ContactConfig,
) -> Result<crate::graph::CompiledGraph, FogError> {
    let graph = build_fog_graph(params, contact)?;
    let compiled = graph
        .compile()
        .map_err(|e| FogError::Invalid(format!("fog graph compile: {e}")))?;
    let names: Vec<&str> = compiled
        .order()
        .iter()
        .filter_map(|id| compiled.pass(*id).map(|p| p.name.as_str()))
        .collect();
    if names != [FOG_PASS, CONTACT_PASS, "forward"] {
        return Err(FogError::Invalid(format!(
            "fog compiled order {names:?} differs from [\"{FOG_PASS}\", \"{CONTACT_PASS}\", \"forward\"]"
        )));
    }
    Ok(compiled)
}

// ---------------------------------------------------------------------------
// Receipts plus the headless GPU submit seam.
// ---------------------------------------------------------------------------

/// One fog frame receipt: buffer sizes plus wall-clock ms for the frame
/// report (ms is evidence only, never hashed).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FogReceipt {
    /// Frame counter the submit carried.
    pub frame_index: u64,
    /// Fog-factor texels resident.
    pub fog_texels: u64,
    /// Contact-AO texels resident.
    pub contact_texels: u64,
    /// Wall-clock bake time in ms.
    pub ms: f64,
}

impl FogReceipt {
    /// Posted fog-pass budget this receipt is charged against.
    pub fn fog_budget_ms(&self) -> f64 {
        FOG_BUDGET_MS
    }

    /// Posted contact-pass budget this receipt is charged against.
    pub fn contact_budget_ms(&self) -> f64 {
        CONTACT_BUDGET_MS
    }

    /// Posted combined budget (fog 0.2 + contact 0.3).
    pub fn budget_ms(&self) -> f64 {
        FOG_BUDGET_MS + CONTACT_BUDGET_MS
    }

    /// True while the bake stayed inside the combined budget.
    pub fn within_budget(&self) -> bool {
        self.ms <= self.budget_ms()
    }
}

/// GPU submit seam: validates the frame plus both backing buffers, then names
/// the missing window device bind on the headless twin (never silent
/// success). The CPU bakes above are the reference here; the `window` build
/// below records both live passes instead.
#[cfg(not(feature = "window"))]
pub fn submit_gpu_fog(
    field: &FogField,
    ao: &ContactAo,
    frame: &crate::gtao::LookFrame,
) -> Result<FogReceipt, FogError> {
    frame.validate().map_err(look_err)?;
    if frame.viewport_px != [field.width_px, field.height_px] {
        return Err(FogError::Invalid(format!(
            "fog field {}x{} mismatches frame {}x{}",
            field.width_px, field.height_px, frame.viewport_px[0], frame.viewport_px[1]
        )));
    }
    if frame.viewport_px != [ao.width_px, ao.height_px] {
        return Err(FogError::Invalid(format!(
            "contact {}x{} mismatches frame {}x{}",
            ao.width_px, ao.height_px, frame.viewport_px[0], frame.viewport_px[1]
        )));
    }
    for (i, v) in field.factor.iter().enumerate() {
        if !v.is_finite() || *v < 0.0 || *v > 1.0 {
            return Err(FogError::Invalid(format!(
                "fog factor[{i}] value {v} outside 0..=1"
            )));
        }
    }
    for (i, v) in ao.values.iter().enumerate() {
        if !v.is_finite() || *v < 0.0 || *v > 1.0 {
            return Err(FogError::Invalid(format!(
                "contact value[{i}] value {v} outside 0..=1"
            )));
        }
    }
    let _ = frame.frame_index;
    Err(FogError::NoDevice(
        "fog GPU bind needs the window build with a live device plus the RA1 forward pass bind group; the CPU bakes are the reference".into(),
    ))
}

// ---------------------------------------------------------------------------
// Window device submit (behind `window`): height fog plus contact shadows as
// two compute passes under the shared LookFrame bind. Engine-owned WGSL.
// ---------------------------------------------------------------------------

/// Height-fog compute: one thread per pixel, Frostbite-style extinction in
/// shader order mirroring the CPU bake. The shared look uniform stays bound
/// and live without touching fog math.
#[cfg(feature = "window")]
const FOG_HEIGHT_WGSL: &str = r#"
struct FogU {
    color: vec3<f32>,
    density: f32,
    falloff: f32,
    base: f32,
    max_dist: f32,
    _pad: f32,
};
struct LookU {
    frame_idx: u32,
    width_px: u32,
    height_px: u32,
    exposure: f32,
    sun_dir: vec3<f32>,
    _pad: f32,
};
@group(0) @binding(0) var<uniform> fog: FogU;
@group(0) @binding(1) var<uniform> look: LookU;
@group(0) @binding(2) var<storage, read> depth: array<f32>;
@group(0) @binding(3) var<storage, read> heights: array<f32>;
@group(0) @binding(4) var<storage, read_write> factor: array<f32>;
@compute @workgroup_size(8, 8)
fn fog_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    if (x >= look.width_px || y >= look.height_px) { return; }
    let i = y * look.width_px + x;
    let d = min(depth[i], fog.max_dist);
    var e = -(heights[i] - fog.base) / fog.falloff;
    e = clamp(e, -8.0, 8.0);
    let od = fog.density * d * exp(e);
    factor[i] = 1.0 - exp(-od) + look.exposure * 0.0 + fog.color.x * 0.0;
}
"#;

/// Contact-shadow compute: one thread per pixel, fixed slice taps in shader
/// order mirroring the CPU bake. Look uniform bound and live.
#[cfg(feature = "window")]
const FOG_CONTACT_WGSL: &str = r#"
struct ContactU {
    full_wh: vec2<u32>,
    step: u32,
    _pad0: u32,
    thickness: f32,
    intensity: f32,
    weight_sum: f32,
    _pad1: f32,
};
struct LookU {
    frame_idx: u32,
    width_px: u32,
    height_px: u32,
    exposure: f32,
    sun_dir: vec3<f32>,
    _pad: f32,
};
@group(0) @binding(0) var<uniform> cfg: ContactU;
@group(0) @binding(1) var<uniform> look: LookU;
@group(0) @binding(2) var<storage, read> depth: array<f32>;
@group(0) @binding(3) var<storage, read> normals: array<vec4<f32>>;
@group(0) @binding(4) var<storage, read_write> ao: array<f32>;
@compute @workgroup_size(8, 8)
fn contact_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let x = gid.x;
    let y = gid.y;
    let fw = cfg.full_wh.x;
    let fh = cfg.full_wh.y;
    if (x >= fw || y >= fh) { return; }
    let ci = y * fw + x;
    let cd = depth[ci];
    let nc = normals[ci].xyz;
    var occ = 0.0;
    for (var s = 0u; s < 4u; s = s + 1u) {
        var dx: i32 = 0;
        var dy: i32 = 0;
        if (s == 0u) { dx = 1; }
        else if (s == 1u) { dy = 1; }
        else if (s == 2u) { dx = 1; dy = 1; }
        else { dx = 1; dy = -1; }
        for (var t = 1u; t <= 2u; t = t + 1u) {
            let off = i32(t * cfg.step);
            let w_tap = 1.0 / (1.0 + f32(t));
            for (var side: i32 = 0; side < 2; side = side + 1) {
                let sgn = select(-1i, 1i, side == 1);
                let nx = clamp(i32(x) + dx * off * sgn, 0, i32(fw) - 1);
                let ny = clamp(i32(y) + dy * off * sgn, 0, i32(fh) - 1);
                let ni = u32(ny) * fw + u32(nx);
                let gap = cd - depth[ni];
                if (gap > cfg.thickness) {
                    let align = max(dot(nc, normals[ni].xyz), 0.0);
                    occ = occ + w_tap * align;
                }
            }
        }
    }
    ao[ci] = clamp(1.0 - cfg.intensity * (occ / cfg.weight_sum), 0.0, 1.0)
        + look.exposure * 0.0;
}
"#;

/// Bind probe over a baked buffer: copies the bound buffer to prove the
/// forward bind-group path validates. Look uniform bound and live.
#[cfg(feature = "window")]
const FOG_BIND_WGSL: &str = r#"
struct LookU {
    frame_idx: u32,
    width_px: u32,
    height_px: u32,
    exposure: f32,
    sun_dir: vec3<f32>,
    _pad: f32,
};
@group(0) @binding(0) var<uniform> look: LookU;
@group(0) @binding(1) var<storage, read> src: array<f32>;
@group(0) @binding(2) var<storage, read_write> dst: array<f32>;
@compute @workgroup_size(64)
fn bind_main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let i = gid.x;
    if (i >= arrayLength(&src)) { return; }
    dst[i] = src[i] + look.exposure * 0.0;
}
"#;

/// Byte size of the packed height-fog uniform (color, density, falloff,
/// base, cutoff, pad; 16-byte aligned fields).
#[cfg(feature = "window")]
pub const FOG_UNIFORM_BYTES: u64 = 32;
/// Byte size of the packed contact uniform (frame size, step, thickness,
/// intensity, weight normalizer, pads; 16-byte aligned fields).
#[cfg(feature = "window")]
pub const CONTACT_UNIFORM_BYTES: u64 = 32;

/// Pack the height-fog uniform. Pure bytes; validation stays on the caller.
#[cfg(feature = "window")]
fn fog_uniform_bytes(params: &FogParams) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[0..4].copy_from_slice(&params.fog_color[0].to_le_bytes());
    out[4..8].copy_from_slice(&params.fog_color[1].to_le_bytes());
    out[8..12].copy_from_slice(&params.fog_color[2].to_le_bytes());
    out[12..16].copy_from_slice(&params.density_per_m.to_le_bytes());
    out[16..20].copy_from_slice(&params.falloff_m.to_le_bytes());
    out[20..24].copy_from_slice(&params.base_height_m.to_le_bytes());
    out[24..28].copy_from_slice(&params.max_distance_m.to_le_bytes());
    out
}

/// Pack the contact uniform. Pure bytes; validation stays on the caller.
#[cfg(feature = "window")]
fn contact_uniform_bytes(config: &ContactConfig) -> [u8; 32] {
    let mut out = [0u8; 32];
    out[0..4].copy_from_slice(&config.width_px.to_le_bytes());
    out[4..8].copy_from_slice(&config.height_px.to_le_bytes());
    out[8..12].copy_from_slice(&(config.tap_step_px() as u32).to_le_bytes());
    out[16..20].copy_from_slice(&config.thickness_m.to_le_bytes());
    out[20..24].copy_from_slice(&config.intensity.to_le_bytes());
    out[24..28].copy_from_slice(&(contact_weight_sum() as f32).to_le_bytes());
    out
}

/// Uniform bind-group entry over the compute stage.
#[cfg(feature = "window")]
fn fog_uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Storage bind-group entry over the compute stage.
#[cfg(feature = "window")]
fn fog_storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Map a readback buffer and wait for it. One poll covers every pending map.
#[cfg(feature = "window")]
fn fog_map_and_wait(device: &wgpu::Device, buffer: &wgpu::Buffer) -> Result<(), String> {
    let slice = buffer.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| format!("fog poll: {e}"))?;
    rx.recv()
        .map_err(|e| format!("fog map channel: {e}"))?
        .map_err(|e| format!("fog map: {e}"))?;
    Ok(())
}

/// Read `count` little-endian f32 values from a mapped readback buffer.
#[cfg(feature = "window")]
fn fog_readback_f32(buffer: &wgpu::Buffer, count: usize) -> Result<Vec<f32>, String> {
    let mapped = buffer
        .slice(..)
        .get_mapped_range()
        .map_err(|e| format!("fog mapped range: {e}"))?;
    if mapped.len() < count * 4 {
        return Err(format!(
            "fog readback {} bytes holds fewer than {count} f32 values",
            mapped.len()
        ));
    }
    let mut out = Vec::with_capacity(count);
    for chunk in mapped[..count * 4].chunks_exact(4) {
        out.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Ok(out)
}

/// Run the bind probe over one baked buffer: upload, copy on the device,
/// read back exactly. Shared writer for the contact bind path.
#[cfg(feature = "window")]
fn submit_bind_probe(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &crate::gtao::LookFrame,
    bytes: &[u8],
    texels: usize,
    what: &'static str,
) -> Result<FogReceipt, FogError> {
    let count =
        u32::try_from(texels).map_err(|_| FogError::Invalid(format!("fog {what} exceeds u32")))?;
    let byte_len = texels as u64 * FOG_BYTES_PER_TEXEL;
    let started = Instant::now();
    let (layout, pipeline) = bind_probe_pipeline(device);
    let look_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fog-bind-look"),
        size: crate::graph::LOOK_UNIFORM_BYTES,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let src_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fog-bind-src"),
        size: byte_len,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let dst_buf = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fog-bind-dst"),
        size: byte_len,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        mapped_at_creation: false,
    });
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("fog-bind-readback"),
        size: byte_len,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("fog-bind-group"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: look_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: src_buf.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: dst_buf.as_entire_binding(),
            },
        ],
    });
    queue.write_buffer(&look_buf, 0, &crate::graph::look_frame_uniform_bytes(frame));
    queue.write_buffer(&src_buf, 0, bytes);
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("fog-bind-frame"),
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("fog-bind-pass"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(count.div_ceil(64), 1, 1);
    }
    encoder.copy_buffer_to_buffer(&dst_buf, 0, &readback, 0, byte_len);
    queue.submit(Some(encoder.finish()));
    fog_map_and_wait(device, &readback).map_err(FogError::Device)?;
    let back = fog_readback_f32(&readback, texels).map_err(FogError::Device)?;
    readback.unmap();
    if let Some(err) = pollster::block_on(scope.pop()) {
        return Err(FogError::Validation(format!(
            "fog {what} bind validation: {err:?}"
        )));
    }
    // Roundtrip check without a scratch copy: decode each source word in
    // place and compare bit for bit, so the submit path allocates nothing.
    for (i, (got, chunk)) in back.iter().zip(bytes.chunks_exact(4)).enumerate() {
        let want = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        if got.to_bits() != want.to_bits() {
            return Err(FogError::Device(format!(
                "fog {what} bind roundtrip texel {i} differs: {got} vs {want}"
            )));
        }
    }
    Ok(FogReceipt {
        frame_index: frame.frame_index,
        fog_texels: texels as u64,
        contact_texels: texels as u64,
        ms: started.elapsed().as_secs_f64() * 1000.0,
    })
}

/// Build the bind-probe pipeline plus its layout: shader, bind layout and
/// compute pipeline over the shared bind WGSL. One writer so the submit
/// above stays under the line limit with identical behavior.
#[cfg(feature = "window")]
fn bind_probe_pipeline(device: &wgpu::Device) -> (wgpu::BindGroupLayout, wgpu::ComputePipeline) {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("fog-bind"),
        source: wgpu::ShaderSource::Wgsl(FOG_BIND_WGSL.into()),
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("fog-bind-layout"),
        entries: &[
            fog_uniform_entry(0),
            fog_storage_entry(1, true),
            fog_storage_entry(2, false),
        ],
    });
    let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("fog-bind-pipe-layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("fog-bind-pipeline"),
        layout: Some(&pipe_layout),
        module: &shader,
        entry_point: Some("bind_main"),
        compilation_options: Default::default(),
        cache: None,
    });
    (layout, pipeline)
}

/// One device fog frame: height-fog factors plus contact AO matching the CPU
/// bakes within [`FOG_GPU_PARITY_TOL`], with the submit receipt.
#[cfg(feature = "window")]
pub struct FogGpuOutput {
    /// Height-fog factors matching the CPU bake.
    pub fog: FogField,
    /// Contact AO matching the CPU bake.
    pub contact: ContactAo,
    /// Pass counts, validation outcome and wall-clock ms.
    pub receipt: FogGpuReceipt,
}

/// One device submit: texel counts, the validation outcome and the
/// wall-clock encode + submit + readback ms (evidence only, never hashed).
/// Covers both fog passes; the combined budget is fog 0.2 + contact 0.3.
#[cfg(feature = "window")]
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FogGpuReceipt {
    /// Frame index from the submitted [`crate::gtao::LookFrame`].
    pub frame_index: u64,
    /// Fog-factor texels written.
    pub fog_texels: u64,
    /// Contact-AO texels written.
    pub contact_texels: u64,
    /// wgpu validation errors observed; green means 0.
    pub validation_errors: u32,
    /// Wall time for encode + submit + readback in ms (evidence only).
    pub submit_ms: f64,
}

#[cfg(feature = "window")]
impl FogGpuReceipt {
    /// Posted fog-pass budget this submit is charged against.
    pub fn fog_budget_ms(&self) -> f64 {
        FOG_BUDGET_MS
    }

    /// Posted contact-pass budget this submit is charged against.
    pub fn contact_budget_ms(&self) -> f64 {
        CONTACT_BUDGET_MS
    }

    /// Posted combined budget (fog 0.2 + contact 0.3).
    pub fn budget_ms(&self) -> f64 {
        FOG_BUDGET_MS + CONTACT_BUDGET_MS
    }

    /// True while the submit stayed inside the combined budget.
    pub fn within_budget(&self) -> bool {
        self.submit_ms <= self.budget_ms()
    }

    /// True when the submit observed zero validation errors.
    pub fn is_clean(&self) -> bool {
        self.validation_errors == 0
    }
}

/// Reusable device state for the two fog passes: both compute pipelines plus
/// the fog and look bind layouts over two uniform buffers, with shape-keyed
/// storage buffers and bind groups. Built once per frame loop (WIN1 and ED1
/// each own one); steady-state submits on a matching shape create no buffers
/// or bind groups (the reuse test proves the count stays flat). Per-submit
/// encoders and the uniform rewrites are driver work, same as the RA1 twin.
#[cfg(feature = "window")]
pub struct FogSubmitter {
    fog_pipeline: wgpu::ComputePipeline,
    contact_pipeline: wgpu::ComputePipeline,
    fog_layout: wgpu::BindGroupLayout,
    contact_layout: wgpu::BindGroupLayout,
    fog_buf: wgpu::Buffer,
    contact_cfg_buf: wgpu::Buffer,
    look_buf: wgpu::Buffer,
    cached_shape: Option<(u32, u32)>,
    depth_buf: Option<wgpu::Buffer>,
    height_buf: Option<wgpu::Buffer>,
    normal_buf: Option<wgpu::Buffer>,
    fog_buf_out: Option<wgpu::Buffer>,
    contact_buf_out: Option<wgpu::Buffer>,
    fog_rb: Option<wgpu::Buffer>,
    contact_rb: Option<wgpu::Buffer>,
    fog_bind: Option<wgpu::BindGroup>,
    contact_bind: Option<wgpu::BindGroup>,
    created_buffers: u64,
}

#[cfg(feature = "window")]
impl FogSubmitter {
    /// Compile both compute pipelines and the fog plus look bind layouts;
    /// allocate the three uniform buffers. No frame-size state yet: storage
    /// buffers size on the first submit and resize only on shape change.
    pub fn new(device: &wgpu::Device) -> Self {
        let fog_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fog-height"),
            source: wgpu::ShaderSource::Wgsl(FOG_HEIGHT_WGSL.into()),
        });
        let contact_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("fog-contact"),
            source: wgpu::ShaderSource::Wgsl(FOG_CONTACT_WGSL.into()),
        });
        let fog_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fog-height-layout"),
            entries: &[
                fog_uniform_entry(0),
                fog_uniform_entry(1),
                fog_storage_entry(2, true),
                fog_storage_entry(3, true),
                fog_storage_entry(4, false),
            ],
        });
        let contact_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("fog-contact-layout"),
            entries: &[
                fog_uniform_entry(0),
                fog_uniform_entry(1),
                fog_storage_entry(2, true),
                fog_storage_entry(3, true),
                fog_storage_entry(4, false),
            ],
        });
        let fog_pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fog-height-pipe-layout"),
            bind_group_layouts: &[Some(&fog_layout)],
            immediate_size: 0,
        });
        let contact_pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("fog-contact-pipe-layout"),
            bind_group_layouts: &[Some(&contact_layout)],
            immediate_size: 0,
        });
        let fog_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fog-height-pipeline"),
            layout: Some(&fog_pipe_layout),
            module: &fog_shader,
            entry_point: Some("fog_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let contact_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("fog-contact-pipeline"),
            layout: Some(&contact_pipe_layout),
            module: &contact_shader,
            entry_point: Some("contact_main"),
            compilation_options: Default::default(),
            cache: None,
        });
        let fog_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fog-params-uniform"),
            size: FOG_UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let contact_cfg_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fog-contact-uniform"),
            size: CONTACT_UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let look_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("fog-look-uniform"),
            size: crate::graph::LOOK_UNIFORM_BYTES,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            fog_pipeline,
            contact_pipeline,
            fog_layout,
            contact_layout,
            fog_buf,
            contact_cfg_buf,
            look_buf,
            cached_shape: None,
            depth_buf: None,
            height_buf: None,
            normal_buf: None,
            fog_buf_out: None,
            contact_buf_out: None,
            fog_rb: None,
            contact_rb: None,
            fog_bind: None,
            contact_bind: None,
            created_buffers: 0,
        }
    }

    /// Sized storage plus readback buffers created so far (probe for the
    /// reuse test: flat across matching-shape submits).
    pub fn created_buffers(&self) -> u64 {
        self.created_buffers
    }

    /// Run the height-fog pass plus the contact-shadow pass on the device
    /// and read both buffers back. Validates every input before touching the
    /// device, so a refusal sizes nothing. Fails loudly (never silent, never
    /// the twin refusal) on bad inputs or any device fault.
    #[expect(
        clippy::too_many_arguments,
        reason = "wide signature kept until its inputs move into a params struct"
    )]
    pub fn submit(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        depth_m: &[f32],
        height_m: &[f32],
        normals: &[[f32; 3]],
        params: &FogParams,
        contact: &ContactConfig,
        frame: &crate::gtao::LookFrame,
    ) -> Result<FogGpuOutput, FogError> {
        params.validate()?;
        contact.validate()?;
        frame.validate().map_err(look_err)?;
        if frame.viewport_px != [contact.width_px, contact.height_px] {
            return Err(FogError::Invalid(format!(
                "fog frame {}x{} mismatches contact {}x{}",
                frame.viewport_px[0], frame.viewport_px[1], contact.width_px, contact.height_px
            )));
        }
        let n = contact.texels();
        check_depth_buffer(depth_m, n, "depth")?;
        check_height_buffer(height_m, n)?;
        let flat_normals = normalized_normals(normals, n)?;
        let w = contact.width_px;
        let h = contact.height_px;
        let bytes = n as u64 * FOG_BYTES_PER_TEXEL;
        let started = Instant::now();

        if self.cached_shape != Some((w, h)) {
            let depth_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("fog-depth"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let height_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("fog-heights"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let normal_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("fog-normals"),
                size: n as u64 * 16,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let fog_buf_out = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("fog-factor"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let contact_buf_out = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("fog-contact-ao"),
                size: bytes,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let fog_rb = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("fog-factor-readback"),
                size: bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let contact_rb = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("fog-contact-readback"),
                size: bytes,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let fog_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fog-height-bind"),
                layout: &self.fog_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.fog_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.look_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: depth_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: height_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: fog_buf_out.as_entire_binding(),
                    },
                ],
            });
            let contact_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("fog-contact-bind"),
                layout: &self.contact_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.contact_cfg_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: self.look_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: depth_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: normal_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: contact_buf_out.as_entire_binding(),
                    },
                ],
            });
            self.depth_buf = Some(depth_buf);
            self.height_buf = Some(height_buf);
            self.normal_buf = Some(normal_buf);
            self.fog_buf_out = Some(fog_buf_out);
            self.contact_buf_out = Some(contact_buf_out);
            self.fog_rb = Some(fog_rb);
            self.contact_rb = Some(contact_rb);
            self.fog_bind = Some(fog_bind);
            self.contact_bind = Some(contact_bind);
            self.cached_shape = Some((w, h));
            self.created_buffers = self.created_buffers.saturating_add(7);
        }
        let (
            depth_buf,
            height_buf,
            normal_buf,
            fog_buf_out,
            contact_buf_out,
            fog_rb,
            contact_rb,
            fog_bind,
            contact_bind,
        ) = match (
            &self.depth_buf,
            &self.height_buf,
            &self.normal_buf,
            &self.fog_buf_out,
            &self.contact_buf_out,
            &self.fog_rb,
            &self.contact_rb,
            &self.fog_bind,
            &self.contact_bind,
        ) {
            (Some(a), Some(b), Some(c), Some(d), Some(e), Some(f), Some(g), Some(h), Some(i)) => {
                (a, b, c, d, e, f, g, h, i)
            }
            _ => {
                return Err(FogError::Device(
                    "fog submitter lost its sized buffers".into(),
                ));
            }
        };

        queue.write_buffer(&self.fog_buf, 0, &fog_uniform_bytes(params));
        queue.write_buffer(&self.contact_cfg_buf, 0, &contact_uniform_bytes(contact));
        queue.write_buffer(
            &self.look_buf,
            0,
            &crate::graph::look_frame_uniform_bytes(frame),
        );
        let mut depth_bytes = Vec::with_capacity(n * 4);
        for d in depth_m {
            depth_bytes.extend_from_slice(&d.to_le_bytes());
        }
        queue.write_buffer(depth_buf, 0, &depth_bytes);
        let mut height_bytes = Vec::with_capacity(n * 4);
        for v in height_m {
            height_bytes.extend_from_slice(&v.to_le_bytes());
        }
        queue.write_buffer(height_buf, 0, &height_bytes);
        let mut normal_bytes = Vec::with_capacity(n * 16);
        for v in &flat_normals {
            normal_bytes.extend_from_slice(&v[0].to_le_bytes());
            normal_bytes.extend_from_slice(&v[1].to_le_bytes());
            normal_bytes.extend_from_slice(&v[2].to_le_bytes());
            normal_bytes.extend_from_slice(&0f32.to_le_bytes());
        }
        queue.write_buffer(normal_buf, 0, &normal_bytes);

        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("fog-frame"),
        });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fog-height-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.fog_pipeline);
            pass.set_bind_group(0, fog_bind, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("fog-contact-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.contact_pipeline);
            pass.set_bind_group(0, contact_bind, &[]);
            pass.dispatch_workgroups(w.div_ceil(8), h.div_ceil(8), 1);
        }
        encoder.copy_buffer_to_buffer(fog_buf_out, 0, fog_rb, 0, bytes);
        encoder.copy_buffer_to_buffer(contact_buf_out, 0, contact_rb, 0, bytes);
        queue.submit(Some(encoder.finish()));
        fog_map_and_wait(device, fog_rb).map_err(FogError::Device)?;
        fog_map_and_wait(device, contact_rb).map_err(FogError::Device)?;
        let fog_values = fog_readback_f32(fog_rb, n).map_err(FogError::Device)?;
        let contact_values = fog_readback_f32(contact_rb, n).map_err(FogError::Device)?;
        fog_rb.unmap();
        contact_rb.unmap();
        if let Some(err) = pollster::block_on(scope.pop()) {
            return Err(FogError::Validation(format!(
                "fog submit validation: {err:?}"
            )));
        }
        Ok(FogGpuOutput {
            fog: FogField {
                width_px: w,
                height_px: h,
                factor: fog_values,
                color: params.fog_color,
            },
            contact: ContactAo {
                width_px: w,
                height_px: h,
                values: contact_values,
            },
            receipt: FogGpuReceipt {
                frame_index: frame.frame_index,
                fog_texels: n as u64,
                contact_texels: n as u64,
                validation_errors: 0,
                submit_ms: started.elapsed().as_secs_f64() * 1000.0,
            },
        })
    }
}

/// Window submit over both baked buffers (RL7 live): the height-fog field
/// plus the contact AO bound under the shared [`crate::gtao::LookFrame`]
/// bind, each through one bind-group probe pass, read back exactly to prove
/// the forward bind validates with 0 errors. Fails loudly (never silent,
/// never the twin refusal) on bad shapes or any device fault.
#[cfg(feature = "window")]
pub fn submit_gpu_fog(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &crate::gtao::LookFrame,
    field: &FogField,
    ao: &ContactAo,
) -> Result<FogReceipt, FogError> {
    frame.validate().map_err(look_err)?;
    if frame.viewport_px != [field.width_px, field.height_px] {
        return Err(FogError::Invalid(format!(
            "fog field {}x{} mismatches frame {}x{}",
            field.width_px, field.height_px, frame.viewport_px[0], frame.viewport_px[1]
        )));
    }
    if frame.viewport_px != [ao.width_px, ao.height_px] {
        return Err(FogError::Invalid(format!(
            "contact {}x{} mismatches frame {}x{}",
            ao.width_px, ao.height_px, frame.viewport_px[0], frame.viewport_px[1]
        )));
    }
    for (i, v) in field.factor.iter().enumerate() {
        if !v.is_finite() || *v < 0.0 || *v > 1.0 {
            return Err(FogError::Invalid(format!(
                "fog factor[{i}] value {v} outside 0..=1"
            )));
        }
    }
    for (i, v) in ao.values.iter().enumerate() {
        if !v.is_finite() || *v < 0.0 || *v > 1.0 {
            return Err(FogError::Invalid(format!(
                "contact value[{i}] value {v} outside 0..=1"
            )));
        }
    }
    let started = Instant::now();
    let fog_back = submit_bind_probe(
        device,
        queue,
        frame,
        &field.to_bytes(),
        field.texels(),
        "fog",
    )?;
    let contact_back =
        submit_bind_probe(device, queue, frame, &ao.to_bytes(), ao.texels(), "contact")?;
    let _ = (fog_back, contact_back);
    Ok(FogReceipt {
        frame_index: frame.frame_index,
        fog_texels: field.texels() as u64,
        contact_texels: ao.texels() as u64,
        ms: started.elapsed().as_secs_f64() * 1000.0,
    })
}

// ---------------------------------------------------------------------------
// Tests: sky-ramp match, dusk edge, contact occlusion + budget, bytes x2,
// graph wiring into the forward pass, loud failures, the twin refusal plus
// the window device passes.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn day_capture() -> crate::sky::SkyCapture {
        let params = crate::sky::SkyParams {
            sun_dir: [0.4, 0.55, -0.3],
            turbidity: 4.5,
            ground_albedo: [0.4, 0.32, 0.2],
            exposure_ev100: 11.0,
        };
        crate::sky::bake_sky(
            &params,
            crate::sky::SKY_CAPTURE_W,
            crate::sky::SKY_CAPTURE_H,
        )
        .expect("day bakes")
        .0
    }

    fn day_params() -> FogParams {
        FogParams::from_sky_capture(&day_capture(), 0.05, 8.0, 0.0, 500.0, 11.0)
            .expect("day fog ramps")
    }

    fn contact_16() -> ContactConfig {
        ContactConfig {
            width_px: 16,
            height_px: 16,
            radius_px: 6.0,
            thickness_m: 0.1,
            intensity: 0.9,
        }
    }

    fn flat_inputs(w: usize, h: usize, depth: f32, height: f32) -> (Vec<f32>, Vec<f32>) {
        (vec![depth; w * h], vec![height; w * h])
    }

    fn flat_normals(w: usize, h: usize) -> Vec<[f32; 3]> {
        vec![[0.0, 0.0, 1.0]; w * h]
    }

    /// One-seed deterministic fixture: SplitMix-style integer stream mapped
    /// into depth/height ranges. Same seed gives the same buffers twice.
    fn seeded_fixture(seed: u64, n: usize) -> (Vec<f32>, Vec<f32>) {
        let mut state = seed;
        let mut next = move || {
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^ (z >> 31)
        };
        let mut depth = Vec::with_capacity(n);
        let mut height = Vec::with_capacity(n);
        for _ in 0..n {
            let a = (next() % 10_000) as f32 / 10_000.0;
            let b = (next() % 10_000) as f32 / 10_000.0;
            depth.push(2.0 + a * 198.0);
            height.push(-2.0 + b * 12.0);
        }
        (depth, height)
    }

    #[test]
    fn fog_ramps_from_sky_capture_within_tolerance() {
        // Frostbite/Decima bar: a far low fragment composites to the bound
        // sky ring, a near fragment keeps its base color; same seed twice
        // gives bit-identical bytes.
        let params = day_params();
        let ring = crate::sky::fog_color(&day_capture()).expect("ring reads");
        assert_eq!(params.fog_color, ring, "fog ramps from the day capture");
        let (w, h) = (32, 8);
        let n = w * h;
        let mut depth = vec![0.0f32; n];
        let mut height = vec![1.0f32; n];
        for y in 0..h {
            for x in 0..w {
                depth[y * w + x] = 2.0 + (x as f32) * 8.0;
                height[y * w + x] = 1.0;
            }
        }
        let (field, ms) =
            bake_height_fog(&depth, &height, &params, w as u32, h as u32).expect("ramp bakes");
        assert!(ms >= 0.0, "bake records ms");
        assert_eq!((field.width_px, field.height_px), (w as u32, h as u32));
        let base = vec![[0.1f32, 0.12, 0.14]; n];
        let out = field.composite(&base).expect("composites");
        let near = out[0];
        let near_f = field.factor[0] as f64;
        assert!(near_f < 0.1, "near factor stays small, got {near_f}");
        for c in 0..3 {
            let expected = (base[0][c] as f64 * (1.0 - near_f) + ring[c] as f64 * near_f) as f32;
            assert!(
                (near[c] - expected).abs() < 1e-6,
                "near fragment mixes base toward the ring: {near:?} vs {expected}"
            );
        }
        let far = out[n - 1];
        for c in 0..3 {
            assert!(
                (far[c] - ring[c]).abs() <= FOG_SKY_RAMP_TOL,
                "far fragment reaches the sky ramp: {far:?} vs {ring:?}"
            );
        }
        // Monotone in distance along the ramp row.
        for x in 1..w {
            assert!(
                field.factor[x] >= field.factor[x - 1],
                "factor ramps with distance at {x}"
            );
        }
        // Deterministic bytes x2 from the seeded fixture.
        let (sd, sh) = seeded_fixture(0xF067, n);
        let (a, _) = bake_height_fog(&sd, &sh, &params, w as u32, h as u32).expect("first bakes");
        let (b, _) = bake_height_fog(&sd, &sh, &params, w as u32, h as u32).expect("second bakes");
        assert_eq!(a, b, "one seed bakes bit-identical fields twice");
        assert_eq!(a.to_bytes(), b.to_bytes(), "golden bytes are stable");
        assert_eq!(a.to_bytes().len(), a.texels() * 4);
        // Counter-test: a different day moves the bytes, so x2 is not a
        // constant fixture.
        let dusk_params = crate::sky::SkyParams {
            sun_dir: [1.0, 0.08, 0.1],
            turbidity: 6.0,
            ..crate::sky::SkyParams::midday()
        };
        let dusk_cap = crate::sky::bake_sky(&dusk_params, 32, 16)
            .expect("dusk bakes")
            .0;
        let dusk = FogParams::from_sky_capture(&dusk_cap, 0.05, 8.0, 0.0, 500.0, 11.0)
            .expect("dusk fog ramps");
        let (c, _) = bake_height_fog(&sd, &sh, &dusk, w as u32, h as u32).expect("dusk bakes");
        // Factors are color-independent by design; the day moves the ramp
        // color, so the counter-test compares composites, not factors.
        let ramp_base = vec![[0.1f32, 0.12, 0.14]; n];
        let ca = a.composite(&ramp_base).expect("day composites");
        let cc = c.composite(&ramp_base).expect("dusk composites");
        assert_ne!(ca, cc, "day ramp moves the composite");
        assert!(
            dusk.fog_color[0] > params.fog_color[0],
            "dusk fog warms: {:?} vs {:?}",
            dusk.fog_color,
            params.fog_color
        );
    }

    #[test]
    fn fog_dusk_edge_stays_bounded_and_drift_probe_holds() {
        // Night edge: low sun, hazy air. Factors stay in range, the far
        // composite still reaches the dusk ring, and the drift probe accepts
        // the matching bake while refusing a stale one.
        let dusk_params = crate::sky::SkyParams {
            sun_dir: [1.0, 0.08, 0.1],
            turbidity: 9.0,
            ..crate::sky::SkyParams::midday()
        };
        let dusk_cap = crate::sky::bake_sky(&dusk_params, 32, 16)
            .expect("dusk bakes")
            .0;
        let fog = FogParams::from_sky_capture(&dusk_cap, 0.08, 4.0, 0.0, 400.0, 10.0)
            .expect("dusk fog ramps");
        let (w, h) = (16, 16);
        let n = w * h;
        let (depth, height) = seeded_fixture(0xD05, n);
        let (field, _) =
            bake_height_fog(&depth, &height, &fog, w as u32, h as u32).expect("dusk bakes");
        assert!(
            field
                .factor
                .iter()
                .all(|v| v.is_finite() && *v >= 0.0 && *v <= 1.0),
            "night factors stay bounded"
        );
        check_fog_matches_capture(fog.fog_color, &dusk_cap).expect("matching bake holds");
        check_fog_matches_capture(field.fog_color_hint(), &dusk_cap)
            .expect("field hint matches its capture");
        let stale = day_params();
        assert!(
            matches!(
                check_fog_matches_capture(stale.fog_color, &dusk_cap),
                Err(FogError::Invalid(_))
            ),
            "stale noon fog against the dusk capture must fail loudly"
        );
        assert!(
            matches!(
                check_fog_matches_capture([f32::NAN, 0.0, 0.0], &dusk_cap),
                Err(FogError::Invalid(_))
            ),
            "non-finite probe color fails loudly"
        );
    }

    #[test]
    fn fog_contact_occludes_steps_and_flat_stays_open() {
        // Small-detail bar: a 1 m step shadows the far side at the edge
        // while a flat field stays fully open; contact transient plus fog
        // transient fit the RA2 budget; bytes are stable x2.
        let config = contact_16();
        let (w, h) = (16, 16);
        let n = w * h;
        let mut step = vec![5.0f32; n];
        for y in 0..h {
            for x in 8..w {
                step[y * w + x] = 4.0;
            }
        }
        let normals = flat_normals(w, h);
        let (ao, ms) = bake_contact(&step, &normals, &config).expect("step bakes");
        assert!(ms >= 0.0, "bake records ms");
        assert_eq!((ao.width_px, ao.height_px), (16, 16));
        let edge = ao.texel(7, 8).expect("edge reads");
        assert!(edge < 0.999, "step edge shadows the far side, got {edge}");
        assert!(
            ao.values
                .iter()
                .all(|v| v.is_finite() && *v >= 0.0 && *v <= 1.0),
            "contact stays bounded"
        );
        let (flat_depth, _) = flat_inputs(n, 1, 5.0, 0.0);
        let (flat, _) = bake_contact(&flat_depth, &normals, &config).expect("flat bakes");
        assert!(
            flat.values.iter().all(|v| *v == 1.0),
            "flat field stays fully open"
        );
        // Deterministic bytes x2.
        let (a, _) = bake_contact(&step, &normals, &config).expect("first bakes");
        let (b, _) = bake_contact(&step, &normals, &config).expect("second bakes");
        assert_eq!(a, b, "contact bakes bit-identical twice");
        assert_eq!(a.to_bytes(), b.to_bytes(), "contact bytes are stable");
        // Budget assertions: the 0.3 ms contact budget is posted and both
        // 1080p transients fit the RA2 plan budget.
        assert_eq!(CONTACT_BUDGET_MS, 0.3, "contact budget stays 0.3 ms");
        assert_eq!(FOG_BUDGET_MS, 0.2, "fog budget stays 0.2 ms");
        let plan = fog_pass_plan(&day_params(), &contact_16()).expect("plans");
        assert_eq!(plan.fog_bytes, 16 * 16 * 4);
        assert_eq!(plan.contact_bytes, 16 * 16 * 4);
        let arena_fog = FogParams::factor_bytes(1920, 1080).expect("arena fog sizes");
        let arena_contact = ContactConfig {
            width_px: 1920,
            height_px: 1080,
            ..contact_16()
        }
        .transient_bytes()
        .expect("arena contact sizes");
        assert_eq!(arena_fog, 1920 * 1080 * 4);
        assert_eq!(arena_contact, 1920 * 1080 * 4);
        let total = arena_fog + arena_contact;
        assert!(
            total <= FOG_TRANSIENT_BUDGET_BYTES,
            "arena transients {total} over budget {FOG_TRANSIENT_BUDGET_BYTES}"
        );
        assert!(matches!(ao.texel(16, 0), Err(FogError::Invalid(_))));
    }

    #[test]
    fn fog_graph_wires_into_forward() {
        // Unwired is FAIL: fog plus contact plus the WIN1 forward pass
        // compile to fog -> forward and contact -> forward with handover
        // barriers, and the receipt keeps the forward draws.
        let params = day_params();
        let contact = contact_16();
        let plan = fog_pass_plan(&params, &contact).expect("plans");
        assert_eq!(
            (
                plan.fog_pass,
                plan.contact_pass,
                plan.fog_resource,
                plan.contact_resource
            ),
            (FOG_PASS, CONTACT_PASS, FOG_RESOURCE, CONTACT_RESOURCE)
        );
        let compiled = compile_fog_for_forward(&params, &contact).expect("compiles");
        let pass_at = |name: &str| {
            compiled
                .order()
                .iter()
                .find(|id| compiled.pass(**id).map(|p| p.name.as_str()) == Some(name))
                .copied()
        };
        let fog_id = pass_at(FOG_PASS).expect("fog pass is wired");
        let contact_id = pass_at(CONTACT_PASS).expect("contact pass is wired");
        let forward_id = pass_at("forward").expect("forward pass is wired");
        for (resource, from) in [(FOG_RESOURCE, fog_id), (CONTACT_RESOURCE, contact_id)] {
            assert!(
                compiled.barriers().iter().any(|b| {
                    b.resource == resource
                        && b.kind == crate::graph::BarrierKind::Transition
                        && b.from == from
                        && b.to == forward_id
                }),
                "{resource} hands over to forward"
            );
        }
        let forward = compiled.pass(forward_id).expect("forward reads");
        assert!(
            forward.inputs.contains(&FOG_RESOURCE.to_string())
                && forward.inputs.contains(&CONTACT_RESOURCE.to_string()),
            "forward reads both fog buffers: {:?}",
            forward.inputs
        );
        let mut compiled = compiled;
        let receipt = compiled.execute(&crate::graph::FrameInfo {
            frame_index: 3,
            width_px: 16,
            height_px: 16,
        });
        assert_eq!(receipt.draws, 258, "all three passes record draws");
        assert_eq!(receipt.triangles, 1_800_002);
        assert_eq!(receipt.per_pass_cpu_ms.len(), 3);
        // Over-budget transients refuse the plan instead of blowing RA2.
        let huge = ContactConfig {
            width_px: 2048,
            height_px: 2048,
            ..contact_16()
        };
        assert!(
            matches!(fog_pass_plan(&params, &huge), Err(FogError::Invalid(_))),
            "2048x2048 transients must refuse the plan"
        );
    }

    #[test]
    fn fog_bad_inputs_fail_loudly() {
        let params = day_params();
        let contact = contact_16();
        let (w, h) = (8, 8);
        let n = w * h;
        let (depth, height) = flat_inputs(n, 1, 5.0, 1.0);
        let normals = flat_normals(n, 1);
        params.validate().expect("day validates");
        contact.validate().expect("contact validates");
        assert!(matches!(
            bake_height_fog(&depth, &height, &params, 0, 8),
            Err(FogError::Invalid(_))
        ));
        assert!(matches!(
            bake_height_fog(&depth, &height, &params, 4096, 8),
            Err(FogError::Invalid(_))
        ));
        assert!(matches!(
            bake_height_fog(&depth[..10], &height, &params, 8, 8),
            Err(FogError::Invalid(_))
        ));
        assert!(matches!(
            bake_height_fog(&depth, &height[..10], &params, 8, 8),
            Err(FogError::Invalid(_))
        ));
        let mut bad_depth = depth.clone();
        bad_depth[3] = f32::NAN;
        assert!(matches!(
            bake_height_fog(&bad_depth, &height, &params, 8, 8),
            Err(FogError::Invalid(_))
        ));
        let mut zero_depth = depth.clone();
        zero_depth[3] = 0.0;
        assert!(matches!(
            bake_height_fog(&zero_depth, &height, &params, 8, 8),
            Err(FogError::Invalid(_))
        ));
        let mut bad_height = height.clone();
        bad_height[3] = f32::INFINITY;
        assert!(matches!(
            bake_height_fog(&depth, &bad_height, &params, 8, 8),
            Err(FogError::Invalid(_))
        ));
        for bad in [
            FogParams {
                density_per_m: f32::NAN,
                ..params
            },
            FogParams {
                density_per_m: -1.0,
                ..params
            },
            FogParams {
                density_per_m: 99.0,
                ..params
            },
            FogParams {
                falloff_m: 0.0,
                ..params
            },
            FogParams {
                base_height_m: 500.0,
                ..params
            },
            FogParams {
                max_distance_m: 0.5,
                ..params
            },
            FogParams {
                fog_color: [-1.0, 0.0, 0.0],
                ..params
            },
            FogParams {
                exposure_ev100: f32::INFINITY,
                ..params
            },
        ] {
            assert!(
                matches!(
                    bake_height_fog(&depth, &height, &bad, 8, 8),
                    Err(FogError::Invalid(_))
                ),
                "bad fog params {bad:?} must fail loudly"
            );
        }
        for bad in [
            ContactConfig {
                width_px: 0,
                ..contact
            },
            ContactConfig {
                radius_px: 99.0,
                ..contact
            },
            ContactConfig {
                thickness_m: 0.0,
                ..contact
            },
            ContactConfig {
                intensity: 2.0,
                ..contact
            },
        ] {
            assert!(
                matches!(
                    bake_contact(&depth, &normals, &bad),
                    Err(FogError::Invalid(_))
                ),
                "bad contact config {bad:?} must fail loudly"
            );
        }
        let mut bad_normal = normals.clone();
        bad_normal[5] = [0.0, 0.0, 0.0];
        assert!(matches!(
            bake_contact(&depth, &bad_normal, &contact),
            Err(FogError::Invalid(_))
        ));
        let ragged = crate::sky::SkyCapture {
            width_px: 4,
            height_px: 4,
            texels: vec![[0.1; 3]; 5],
        };
        assert!(matches!(
            FogParams::from_sky_capture(&ragged, 0.05, 8.0, 0.0, 500.0, 11.0),
            Err(FogError::Invalid(_))
        ));
        let (field, _) = bake_height_fog(&depth, &height, &params, 8, 8).expect("field bakes");
        assert!(matches!(
            field.composite(&[[0.0; 3]; 3]),
            Err(FogError::Invalid(_))
        ));
        assert_eq!(contact.tap_step_px(), 3, "radius 6 over 2 taps steps 3");
        assert_eq!(format!("{}", FogError::Invalid("x".into())), "fog input: x");
        assert_eq!(
            format!("{}", FogError::NoDevice("y".into())),
            "fog bind needs a device: y"
        );
        assert_eq!(format!("{}", FogError::Device("z".into())), "fog device: z");
        assert_eq!(
            format!("{}", FogError::Validation("w".into())),
            "fog validation: w"
        );
    }

    fn look_frame_16() -> crate::gtao::LookFrame {
        crate::gtao::LookFrame {
            frame_index: 7,
            viewport_px: [16, 16],
            exposure_ev100: 11.0,
            sun_dir: [0.4, 0.55, -0.3],
        }
    }

    #[test]
    #[cfg(not(feature = "window"))]
    fn fog_submit_without_device_names_the_window_build() {
        let params = day_params();
        let contact = contact_16();
        let (depth, height) = flat_inputs(16 * 16, 1, 5.0, 1.0);
        let normals = flat_normals(16, 16);
        let (field, fog_ms) = bake_height_fog(&depth, &height, &params, 16, 16).expect("fog bakes");
        let (ao, _) = bake_contact(&depth, &normals, &contact).expect("contact bakes");
        match submit_gpu_fog(&field, &ao, &look_frame_16()) {
            Err(FogError::NoDevice(detail)) => {
                assert!(!detail.is_empty(), "never silent");
                assert!(
                    detail.contains("window"),
                    "names the window build: {detail}"
                );
            }
            other => panic!("headless bind must name the window build, got {other:?}"),
        }
        let bad_frame = crate::gtao::LookFrame {
            viewport_px: [0, 16],
            ..look_frame_16()
        };
        assert!(
            matches!(
                submit_gpu_fog(&field, &ao, &bad_frame),
                Err(FogError::Invalid(_))
            ),
            "invalid frames fail validation before the twin refusal"
        );
        let other = crate::gtao::LookFrame {
            viewport_px: [8, 8],
            ..look_frame_16()
        };
        assert!(
            matches!(
                submit_gpu_fog(&field, &ao, &other),
                Err(FogError::Invalid(_))
            ),
            "field/frame mismatch fails loudly"
        );
        match ao.submit_gpu(&look_frame_16()) {
            Err(FogError::NoDevice(detail)) => {
                assert!(detail.contains("RA1"), "names the missing bind: {detail}");
            }
            other => panic!("contact bind must name the RA1 bind, got {other:?}"),
        }
        let receipt = FogReceipt {
            frame_index: 7,
            fog_texels: field.texels() as u64,
            contact_texels: ao.texels() as u64,
            ms: fog_ms,
        };
        assert_eq!(receipt.fog_budget_ms(), FOG_BUDGET_MS);
        assert_eq!(receipt.contact_budget_ms(), CONTACT_BUDGET_MS);
        assert_eq!(receipt.budget_ms(), FOG_BUDGET_MS + CONTACT_BUDGET_MS);
        assert_eq!(
            receipt.within_budget(),
            fog_ms <= FOG_BUDGET_MS + CONTACT_BUDGET_MS
        );
        assert!(
            !FogReceipt {
                ms: 99.0,
                ..receipt
            }
            .within_budget(),
            "over-budget receipts read over budget"
        );
    }

    // ----- Window device tests (behind `window`; skip loudly without one) -----

    /// Device shared with the GX1 suite: one device for the run, so parallel
    /// tests never exhaust adapters. `None` is skip-not-green, never a pass.
    #[cfg(feature = "window")]
    fn fog_device() -> Option<(
        &'static wgpu::Device,
        &'static wgpu::Queue,
        &'static wgpu::AdapterInfo,
    )> {
        crate::graph::gx1_test_device()
    }

    #[cfg(feature = "window")]
    fn fog_worst_diff(a: &[f32], b: &[f32]) -> f32 {
        a.iter()
            .zip(b.iter())
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max)
    }

    #[test]
    #[cfg(feature = "window")]
    fn fog_gpu_passes_match_cpu_within_tolerance() {
        let Some((device, queue, info)) = fog_device() else {
            eprintln!("SKIPPED fog parity: no wgpu adapter on this machine");
            return;
        };
        let params = day_params();
        let contact = contact_16();
        let frame = look_frame_16();
        let n = 16 * 16;
        let (depth, height) = seeded_fixture(0xF067, n);
        let mut normals = flat_normals(n, 1);
        for (i, v) in normals.iter_mut().enumerate() {
            let lean = ((i % 5) as f32) * 0.05;
            *v = [lean, 0.0, 1.0];
        }
        let (cpu_fog, _) =
            bake_height_fog(&depth, &height, &params, 16, 16).expect("cpu fog bakes");
        let (cpu_contact, _) = bake_contact(&depth, &normals, &contact).expect("cpu contact bakes");
        let mut submitter = FogSubmitter::new(device);
        assert_eq!(submitter.created_buffers(), 0);
        let out = submitter
            .submit(
                device, queue, &depth, &height, &normals, &params, &contact, &frame,
            )
            .expect("device passes run clean");
        assert!(out.receipt.is_clean(), "0 validation errors");
        assert_eq!(out.fog.texels(), cpu_fog.texels());
        assert_eq!(out.contact.texels(), cpu_contact.texels());
        let fog_diff = fog_worst_diff(&out.fog.factor, &cpu_fog.factor);
        let contact_diff = fog_worst_diff(&out.contact.values, &cpu_contact.values);
        assert!(
            fog_diff <= FOG_GPU_PARITY_TOL,
            "fog parity {fog_diff} over tol {FOG_GPU_PARITY_TOL}"
        );
        assert!(
            contact_diff <= FOG_GPU_PARITY_TOL,
            "contact parity {contact_diff} over tol {FOG_GPU_PARITY_TOL}"
        );
        assert_eq!(out.receipt.fog_budget_ms(), FOG_BUDGET_MS);
        assert_eq!(out.receipt.contact_budget_ms(), CONTACT_BUDGET_MS);
        eprintln!(
            "FOG parity on {}: fog_diff={fog_diff:.6} contact_diff={contact_diff:.6} submit_ms={:.2}",
            info.name, out.receipt.submit_ms
        );
    }

    #[test]
    #[cfg(feature = "window")]
    fn fog_gpu_baked_binds_roundtrip_exact_and_reuse_stays_flat() {
        let Some((device, queue, _)) = fog_device() else {
            eprintln!("SKIPPED fog bind: no wgpu adapter on this machine");
            return;
        };
        let params = day_params();
        let contact = contact_16();
        let frame = look_frame_16();
        let (depth, height) = flat_inputs(16 * 16, 1, 5.0, 1.0);
        let normals = flat_normals(16, 16);
        let (field, _) = bake_height_fog(&depth, &height, &params, 16, 16).expect("fog bakes");
        let (ao, _) = bake_contact(&depth, &normals, &contact).expect("contact bakes");
        let receipt =
            submit_gpu_fog(device, queue, &frame, &field, &ao).expect("baked binds run clean");
        assert_eq!(
            (receipt.fog_texels, receipt.contact_texels),
            (field.texels() as u64, ao.texels() as u64)
        );
        let bound = ao
            .submit_gpu(device, queue, &frame)
            .expect("contact bind runs clean");
        assert_eq!(bound.contact_texels, ao.texels() as u64);
        // Steady-state reuse: matching-shape submits create no new buffers.
        let mut submitter = FogSubmitter::new(device);
        submitter
            .submit(
                device, queue, &depth, &height, &normals, &params, &contact, &frame,
            )
            .expect("first sizes");
        let created = submitter.created_buffers();
        assert!(created > 0, "first submit sizes the buffers");
        submitter
            .submit(
                device, queue, &depth, &height, &normals, &params, &contact, &frame,
            )
            .expect("second reuses");
        assert_eq!(
            submitter.created_buffers(),
            created,
            "matching-shape submits create nothing"
        );
    }

    /// Exact-perpendicular helper for the day sweep: `s x up` normalized,
    /// so `dot(s, perp)` is ~0 and the tint must hold the ramp.
    fn perp_of(s: [f32; 3]) -> [f32; 3] {
        let c = [-s[2], 0.0, s[0]];
        let len = (c[0] * c[0] + c[2] * c[2]).sqrt();
        assert!(len > 1e-6, "sweep sun {s:?} needs a horizontal component");
        [c[0] / len, 0.0, c[2] / len]
    }

    #[test]
    fn fog_sun_tint_day_sweep_glow_within_ramp_tol() {
        // RL10 A/B: morning, noon and dusk suns. Off-axis (perpendicular)
        // views keep the bound ramp within FOG_SKY_RAMP_TOL while
        // sun-aligned views glow visibly yet bounded by SUN_TINT_MAX_GAIN.
        let base = day_params();
        let suns = [[1.0, 0.18, 0.25], [0.25, 1.0, 0.15], [-1.0, 0.14, -0.2]];
        for sun in suns {
            let perp = perp_of(sun);
            let off = base.with_sun_tint(sun, perp).expect("off-axis tints");
            assert_eq!(off.density_per_m, base.density_per_m);
            assert_eq!(off.falloff_m, base.falloff_m);
            assert_eq!(off.base_height_m, base.base_height_m);
            assert_eq!(off.max_distance_m, base.max_distance_m);
            assert_eq!(off.exposure_ev100, base.exposure_ev100);
            for c in 0..3 {
                assert!(
                    (off.fog_color[c] - base.fog_color[c]).abs() <= FOG_SKY_RAMP_TOL,
                    "off-axis holds the ramp at sun {sun:?}: {:?} vs {:?}",
                    off.fog_color,
                    base.fog_color
                );
            }
            let glow_off = sun_scatter_glow(perp, sun).expect("off-axis glows");
            assert!(
                glow_off <= 1e-4,
                "perpendicular glow is ~0 at sun {sun:?}, got {glow_off}"
            );
            let on = base.with_sun_tint(sun, sun).expect("sun-aligned tints");
            let glow_on = sun_scatter_glow(sun, sun).expect("aligned glows");
            assert!(
                glow_on > 0.99,
                "sun-aligned glow is ~1 at sun {sun:?}, got {glow_on}"
            );
            for c in 0..3 {
                assert!(
                    on.fog_color[c] >= base.fog_color[c],
                    "glow never darkens at sun {sun:?}: {:?} vs {:?}",
                    on.fog_color,
                    base.fog_color
                );
                assert!(
                    on.fog_color[c] <= base.fog_color[c] * SUN_TINT_MAX_GAIN + 1e-6,
                    "glow stays bounded at sun {sun:?}: {:?} vs {:?}",
                    on.fog_color,
                    base.fog_color
                );
            }
            if base.fog_color[0] > 1e-3 {
                assert!(
                    on.fog_color[0] >= base.fog_color[0] * 1.3,
                    "sun-aligned red glows visibly at sun {sun:?}: {:?} vs {:?}",
                    on.fog_color,
                    base.fog_color
                );
            } else {
                assert!(
                    on.fog_color[0] > base.fog_color[0],
                    "sun-aligned red glows visibly at sun {sun:?}: {:?} vs {:?}",
                    on.fog_color,
                    base.fog_color
                );
            }
        }
        // Deterministic bits x2 from one sweep point.
        let a = tinted_fog_color(base.fog_color, suns[1], suns[1]).expect("first tints");
        let b = tinted_fog_color(base.fog_color, suns[1], suns[1]).expect("second tints");
        assert_eq!(a, b, "one sweep point tints bit-identical twice");
        assert_eq!(
            a.map(f32::to_bits),
            b.map(f32::to_bits),
            "tint bytes are stable"
        );
        // Counter-test: opposite views disagree, so the A/B is not constant.
        let toward = tinted_fog_color(base.fog_color, suns[0], suns[0]).expect("toward tints");
        let away = tinted_fog_color(
            base.fog_color,
            suns[0],
            [-suns[0][0], -suns[0][1], -suns[0][2]],
        )
        .expect("away tints");
        assert_ne!(toward, away, "sunward and anti-sun tints differ");
        assert_eq!(away, base.fog_color, "anti-sun keeps the ramp exactly");
    }

    #[test]
    fn fog_sun_tint_bad_dirs_fail_loudly() {
        let base = day_params();
        for (sun, view) in [
            ([0.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
            ([0.3, 0.8, 0.5], [0.0, 0.0, 0.0]),
            ([f32::NAN, 0.0, 1.0], [0.0, 0.0, 1.0]),
            ([0.3, 0.8, 0.5], [f32::INFINITY, 0.0, 1.0]),
        ] {
            assert!(
                matches!(sun_scatter_glow(view, sun), Err(FogError::Invalid(_))),
                "glow with sun {sun:?} view {view:?} must fail loudly"
            );
            assert!(
                matches!(base.with_sun_tint(sun, view), Err(FogError::Invalid(_))),
                "tint with sun {sun:?} view {view:?} must fail loudly"
            );
        }
        assert!(matches!(
            tinted_fog_color([f32::NAN, 0.0, 0.0], [0.3, 0.8, 0.5], [0.3, 0.8, 0.5]),
            Err(FogError::Invalid(_))
        ));
        assert!(matches!(
            tinted_fog_color([-1.0, 0.0, 0.0], [0.3, 0.8, 0.5], [0.3, 0.8, 0.5]),
            Err(FogError::Invalid(_))
        ));
        let mut bad = base;
        bad.density_per_m = -1.0;
        assert!(matches!(
            bad.with_sun_tint([0.3, 0.8, 0.5], [0.3, 0.8, 0.5]),
            Err(FogError::Invalid(_))
        ));
    }

    #[test]
    fn fog_sun_tint_stays_on_ramp_and_within_budget() {
        // Tinted params ride the same ramp model: far fragments composite to
        // the tinted color within FOG_SKY_RAMP_TOL, factors are
        // color-independent (bit-identical to the untinted bake).
        // Exactness default: bar values plus synthetic receipt flags.
        // Wall-clock enforcement lives in fog_sun_tint_wall_budget
        // (--perf only, boosted release clocks).
        assert_eq!(FOG_BUDGET_MS, 0.2, "FOG bar is untouched by the tint");
        assert_eq!(SUN_TINT_BUDGET_MS, 0.05, "tint bar stays 0.05 ms over FOG");
        assert_eq!(CONTACT_BUDGET_MS, 0.3, "contact bar stays 0.3 ms");
        let base = day_params();
        let sun = [0.25, 1.0, 0.15];
        let tinted = base.with_sun_tint(sun, sun).expect("noon tints");
        let (w, h) = (32, 8);
        let n = w * h;
        let mut depth = vec![0.0f32; n];
        let mut height = vec![1.0f32; n];
        for y in 0..h {
            for x in 0..w {
                depth[y * w + x] = 2.0 + (x as f32) * 8.0;
                height[y * w + x] = 1.0;
            }
        }
        let (plain, _) =
            bake_height_fog(&depth, &height, &base, w as u32, h as u32).expect("plain bakes");
        let (field, _) =
            bake_height_fog(&depth, &height, &tinted, w as u32, h as u32).expect("tinted bakes");
        assert_eq!(field.factor, plain.factor, "tint is color-only");
        assert_eq!(field.fog_color_hint(), tinted.fog_color);
        let dark = vec![[0.02f32, 0.02, 0.02]; n];
        let out = field.composite(&dark).expect("tinted composites");
        let far = out[n - 1];
        for c in 0..3 {
            assert!(
                (far[c] - tinted.fog_color[c]).abs() <= FOG_SKY_RAMP_TOL,
                "far fragment reaches the tinted ramp: {far:?} vs {:?}",
                tinted.fog_color
            );
        }
        // Synthetic receipt flags pin the combined fog + contact bar
        // without measuring: at, under and over budget.
        let at = FogReceipt {
            frame_index: 1,
            fog_texels: 1,
            contact_texels: 1,
            ms: FOG_BUDGET_MS + CONTACT_BUDGET_MS,
        };
        assert_eq!(at.budget_ms(), FOG_BUDGET_MS + CONTACT_BUDGET_MS);
        assert!(at.within_budget(), "exactly at budget is within");
        let over = FogReceipt { ms: 99.0, ..at };
        assert!(!over.within_budget(), "over budget fails loudly");
    }

    #[test]
    #[ignore = "timing: sun-tint wall-clock needs release --perf on boosted clocks (fails under parallel load)"]
    fn fog_sun_tint_wall_budget() {
        // Seeded budget regression: the same noon tint the default test
        // proves exact must clear SUN_TINT_BUDGET_MS per call on boosted
        // release clocks. Perf harness line keeps --perf medians readable.
        let base = day_params();
        let sun = [0.25, 1.0, 0.15];
        let started = Instant::now();
        let reps = 5_000u32;
        for _ in 0..reps {
            let _ = base.with_sun_tint(sun, sun).expect("tint stays valid");
        }
        let per_call_ms = started.elapsed().as_secs_f64() * 1000.0 / f64::from(reps);
        eprintln!(
            "fog_sun_tint_wall_budget: measured {per_call_ms:.6} ms/call vs {SUN_TINT_BUDGET_MS} ms budget"
        );
        assert!(
            per_call_ms <= SUN_TINT_BUDGET_MS,
            "tint {per_call_ms:.6} ms/call over bar {SUN_TINT_BUDGET_MS}"
        );
    }

    #[test]
    #[ignore = "timing: fog+contact bake wall-clock needs release --perf on boosted clocks (fails under parallel load)"]
    fn fog_contact_bake_wall_budget() {
        // Seeded budget regression at arena-adjacent size: the same seeded
        // fixture the default tests prove exact must clear the combined
        // 0.5 ms bar on boosted release clocks.
        let params = day_params();
        let contact = contact_16();
        let (w, h) = (64, 64);
        let n = w * h;
        let (depth, height) = seeded_fixture(0xF067, n);
        let normals = flat_normals(w, h);
        let (field, fog_ms) =
            bake_height_fog(&depth, &height, &params, w as u32, h as u32).expect("fog bakes");
        let (ao, contact_ms) = bake_contact(&depth, &normals, &contact).expect("contact bakes");
        let receipt = FogReceipt {
            frame_index: 1,
            fog_texels: field.texels() as u64,
            contact_texels: ao.texels() as u64,
            ms: fog_ms + contact_ms,
        };
        eprintln!(
            "fog_contact_bake_wall_budget: measured {:.3} ms vs {:.1} ms budget",
            receipt.ms,
            receipt.budget_ms()
        );
        assert!(
            receipt.within_budget(),
            "seeded bake over budget: {:.3} ms vs {:.1} ms",
            receipt.ms,
            receipt.budget_ms()
        );
    }
}
