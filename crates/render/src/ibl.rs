//! ibl: GPU image-based lighting baked from the Poly Haven HDRI (RL2).
//!
//! Owner: render lane; consumers WIN1/ED1 material frame through the RA1
//! presenter and the Q1 chart. Units: texels for the prefiltered cubemap and
//! the BRDF LUT, roughness steps for the prefilter levels, steradians behind
//! the SH projection weights. Determinism: fixed index orders, f64
//! accumulation for the SH projection, bit-identical bytes for the same HDRI
//! bytes on every run. Errors are typed [`IblError`]: a missing HDRI refuses
//! loudly, and device failures name the call or the validation scope.
//!
//! Cook-time bake (behind `v1`, needs the R2 [`crate::v1_ibl_shadow`] HDRI
//! path): [`bake`] turns decoded HDRI bytes into an [`IblBake`] holding a
//! roughness-stepped prefiltered cubemap, 9-coefficient irradiance SH and a
//! GGX split-sum BRDF LUT. The forward pass samples the bake on the CPU with
//! [`sample_cube`], [`eval_sh`] and [`brdf_sample`], combined by
//! [`shade_ibl`]; the WGSL sampling chunk for the RA1 forward pipeline is
//! [`gpu::FORWARD_IBL_WGSL`], and [`IblBake::submit_gpu`] (behind `window`)
//! records the live material-chart pass binding the shared
//! [`crate::gtao::LookFrame`] plus the full bake.

use std::f32::consts::PI;
use std::fmt;
#[cfg(feature = "v1")]
use std::time::Instant;

// ---------------------------------------------------------------------------
// Bake shape: fixed sizes bound every allocation up front.
// ---------------------------------------------------------------------------

/// Prefiltered cubemap face edge in texels.
pub const IBL_CUBE_SIZE: u32 = 64;
/// Roughness steps: level `l` holds roughness `l / (LEVELS - 1)`.
pub const IBL_LEVELS: usize = 5;
/// Cubemap faces in stored order: +X, -X, +Y, -Y, +Z, -Z.
pub const IBL_FACES: usize = 6;
/// BRDF LUT edge in texels (NdotV by roughness).
pub const BRDF_LUT_SIZE: u32 = 64;
/// Hammersley samples per BRDF LUT texel; fixed, deterministic.
pub const BRDF_SAMPLES: u32 = 128;
/// Irradiance SH coefficients (L2); matches [`crate::gi::SH_COEFFS`].
pub const IBL_SH_COEFFS: usize = 9;
/// Largest chart target edge the submit sizes in pixels. The test chart
/// runs 32x32; the cap keeps readback buffers bounded like the graph
/// transient budget instead of growing with the viewport.
pub const IBL_CHART_MAX_EDGE: u32 = 256;

/// Size the chart target for a look viewport in pixels: identity while both
/// edges fit [`IBL_CHART_MAX_EDGE`], otherwise a uniform downscale by
/// `edge / long_edge` so the long edge lands exactly on the cap and the
/// aspect is preserved (1600x900 becomes 256x144). Pure integer math in
/// fixed order; the same viewport gives the same target on every run.
/// Zero-area viewports refuse with [`IblError::Invalid`]; see
/// [`check_chart_target_size`] for the direct target guard.
pub fn chart_target_size(viewport_w: u32, viewport_h: u32) -> Result<(u32, u32), IblError> {
    if viewport_w == 0 || viewport_h == 0 {
        return Err(IblError::Invalid(format!(
            "ibl chart viewport {viewport_w}x{viewport_h} has zero area"
        )));
    }
    if viewport_w <= IBL_CHART_MAX_EDGE && viewport_h <= IBL_CHART_MAX_EDGE {
        return Ok((viewport_w, viewport_h));
    }
    let wide = u64::from(viewport_w);
    let high = u64::from(viewport_h);
    let edge = u64::from(IBL_CHART_MAX_EDGE);
    let long = wide.max(high);
    let wide_scaled = wide
        .checked_mul(edge)
        .ok_or_else(|| IblError::Invalid("ibl chart viewport product overflows".into()))?;
    let high_scaled = high
        .checked_mul(edge)
        .ok_or_else(|| IblError::Invalid("ibl chart viewport product overflows".into()))?;
    let clamped_wide = (wide_scaled / long).clamp(1, edge);
    let clamped_high = (high_scaled / long).clamp(1, edge);
    let w = u32::try_from(clamped_wide)
        .map_err(|_| IblError::Invalid("ibl chart width exceeds u32".into()))?;
    let h = u32::try_from(clamped_high)
        .map_err(|_| IblError::Invalid("ibl chart height exceeds u32".into()))?;
    Ok((w, h))
}

/// Guard the chart target itself in pixels: both edges must be nonzero and
/// within [`IBL_CHART_MAX_EDGE`]. The viewport scaler above clamps window
/// sizes into this bound; direct target sizes over the edge refuse loudly
/// with [`IblError::Invalid`] before any texture sizes, so the refusal never
/// reaches the presenter validation scope.
pub fn check_chart_target_size(w: u32, h: u32) -> Result<(), IblError> {
    if w == 0 || h == 0 {
        return Err(IblError::Invalid(format!(
            "ibl chart target {w}x{h} has zero area"
        )));
    }
    if w > IBL_CHART_MAX_EDGE || h > IBL_CHART_MAX_EDGE {
        return Err(IblError::Invalid(format!(
            "ibl chart target {w}x{h} exceeds edge {IBL_CHART_MAX_EDGE}"
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Forward wiring: prove the forward pass binds the IBL chunk instead of
// only shipping it. The chart pass below already samples the full bake on a
// device; the forward pipeline must inline the same sampling chunk and bind
// the same prefilter, BRDF and SH images every frame.
// ---------------------------------------------------------------------------

/// Markers a compiled forward WGSL must contain to sample the IBL bake: the
/// prefiltered cubemap, the BRDF LUT, the sampler, the SH uniform and the
/// sampling entry. The window-only [`gpu::FORWARD_IBL_WGSL`] chunk carries
/// all five; the forward pipeline inlines that chunk when the material bind
/// group lands.
pub const FORWARD_IBL_MARKERS: [&str; 5] = [
    "ibl_prefilter",
    "ibl_brdf",
    "ibl_sampler",
    "ibl_sh",
    "ibl_sample",
];

/// Assert a compiled forward WGSL binds the IBL chunk.
///
/// Owner: render lane; consumer: the forward presenter pipeline plus the
/// wiring gate. Units: none (string markers). Threading: pure with no IO,
/// locks or clock; safe on any thread. Determinism: the same string gives
/// the same verdict on every run. Errors: [`IblError::Invalid`] naming the
/// first missing marker; empty shaders refuse instead of passing silently.
///
/// ```
/// use engine_render::ibl::{assert_forward_wgsl_binds_ibl, FORWARD_IBL_MARKERS};
/// let stitched = FORWARD_IBL_MARKERS.join("\n");
/// assert!(assert_forward_wgsl_binds_ibl(&stitched).is_ok());
/// ```
pub fn assert_forward_wgsl_binds_ibl(forward_wgsl: &str) -> Result<(), IblError> {
    for marker in FORWARD_IBL_MARKERS {
        if !forward_wgsl.contains(marker) {
            return Err(IblError::Invalid(format!(
                "forward pass misses IBL bind marker '{marker}': inline FORWARD_IBL_WGSL"
            )));
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Errors: typed; missing HDRI refuses, bad input names the cause, device
// failures name the call or the validation scope.
// ---------------------------------------------------------------------------

/// IBL failures. `MissingHdri` is the cook-time refusal when no HDRI bytes
/// were supplied; decode problems wrap the R2 error they came from.
/// `Invalid` covers unusable caller input; `Device`/`Validation` (behind
/// `window`) cover device calls and the wgpu validation scope. No path
/// ever reports silent success.
#[derive(Clone, PartialEq, Debug)]
pub enum IblError {
    /// No HDRI bytes at all; cook with the Poly Haven fixture first.
    MissingHdri,
    /// The R2 HDRI decoder refused the bytes.
    #[cfg(feature = "v1")]
    Hdri(crate::v1_ibl_shadow::HdriError),
    /// A direction or roughness input failed R2 validation.
    #[cfg(feature = "v1")]
    Sample(crate::v1_ibl_shadow::SampleError),
    /// Caller input is unusable; reason carried.
    Invalid(String),
    /// A device or queue call failed (map, poll, texture write); message
    /// carried. Only behind `window`, where a live device is in hand.
    #[cfg(feature = "window")]
    Device(String),
    /// A wgpu validation error fired inside the submit scope; kept loud
    /// with the scope debug text. Only behind `window`.
    #[cfg(feature = "window")]
    Validation(String),
}

impl fmt::Display for IblError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHdri => write!(f, "ibl needs HDRI bytes: none supplied"),
            #[cfg(feature = "v1")]
            Self::Hdri(e) => write!(f, "ibl hdri: {e}"),
            #[cfg(feature = "v1")]
            Self::Sample(e) => write!(f, "ibl sample: {e}"),
            Self::Invalid(e) => write!(f, "ibl input: {e}"),
            #[cfg(feature = "window")]
            Self::Device(e) => write!(f, "ibl device: {e}"),
            #[cfg(feature = "window")]
            Self::Validation(e) => write!(f, "ibl validation: {e}"),
        }
    }
}

impl std::error::Error for IblError {}

#[cfg(feature = "v1")]
impl From<crate::v1_ibl_shadow::HdriError> for IblError {
    fn from(e: crate::v1_ibl_shadow::HdriError) -> Self {
        Self::Hdri(e)
    }
}

#[cfg(feature = "v1")]
impl From<crate::v1_ibl_shadow::SampleError> for IblError {
    fn from(e: crate::v1_ibl_shadow::SampleError) -> Self {
        Self::Sample(e)
    }
}

// ---------------------------------------------------------------------------
// Receipts: texel counts plus wall-clock bake ms for the frame report.
// ---------------------------------------------------------------------------

/// One IBL cook receipt: what baked, how big, how long.
#[derive(Clone, PartialEq, Debug)]
pub struct IblReceipt {
    /// Prefiltered texels resident (`levels * 6 * size * size`).
    pub cube_texels: u64,
    /// BRDF LUT texels resident (`size * size`).
    pub lut_texels: u64,
    /// SH coefficients resident (always [`IBL_SH_COEFFS`]).
    pub sh_coeffs: u32,
    /// Wall-clock bake time in ms; evidence only, never hashed.
    pub ms: f64,
}

// ---------------------------------------------------------------------------
// Prefiltered cubemap: levels of roughness-blurred radiance faces.
// ---------------------------------------------------------------------------

/// Roughness-stepped cubemap: level `l` blurs with roughness
/// `l / (LEVELS - 1)` per the R2 [`crate::v1_ibl_shadow::prefilter`] kernel.
/// Layout is level-major, face-major (`+X,-X,+Y,-Y,+Z,-Z`), row-major texels.
#[derive(Clone, PartialEq, Debug)]
pub struct PrefilterCube {
    /// Face edge in texels.
    pub size: u32,
    /// Roughness levels.
    pub levels: usize,
    /// `levels * 6 * size * size` linear radiance texels.
    pub faces: Vec<[f32; 3]>,
}

impl PrefilterCube {
    /// Zero-filled cube (test and placeholder backing, never a cook result).
    pub fn new(size: u32, levels: usize) -> Self {
        Self {
            size,
            levels,
            faces: vec![[0.0; 3]; levels * IBL_FACES * size as usize * size as usize],
        }
    }

    /// Texel count across all levels and faces.
    pub fn texels(&self) -> usize {
        self.faces.len()
    }

    /// Roughness stored at `level`.
    pub fn level_roughness(level: usize) -> Result<f32, IblError> {
        if level >= IBL_LEVELS {
            return Err(IblError::Invalid(format!(
                "ibl level {level} outside 0..{IBL_LEVELS}"
            )));
        }
        Ok(level as f32 / (IBL_LEVELS - 1) as f32)
    }

    fn face_base(&self, level: usize, face: usize) -> Result<usize, IblError> {
        if level >= self.levels {
            return Err(IblError::Invalid(format!(
                "ibl level {level} outside 0..{}",
                self.levels
            )));
        }
        if face >= IBL_FACES {
            return Err(IblError::Invalid(format!(
                "ibl face {face} outside 0..{IBL_FACES}"
            )));
        }
        Ok((level * IBL_FACES + face) * self.size as usize * self.size as usize)
    }

    /// Read one texel; bounds-checked for callers outside the bake.
    pub fn texel(&self, level: usize, face: usize, x: u32, y: u32) -> Result<[f32; 3], IblError> {
        if x >= self.size || y >= self.size {
            return Err(IblError::Invalid(format!(
                "ibl texel ({x},{y}) outside {}x{}",
                self.size, self.size
            )));
        }
        let base = self.face_base(level, face)?;
        Ok(self.faces[base + y as usize * self.size as usize + x as usize])
    }

    /// Deterministic little-endian bytes: level-major, face-major,
    /// row-major texels, RGB component order.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.faces.len() * 12);
        for t in &self.faces {
            for c in t {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        out
    }
}

/// Unit direction through a texel center. Row 0 is the top (`-v`), matching
/// the image convention; the exact orientation is engine-internal, but bake
/// and sample share it, so round-trips are exact.
pub fn cube_texel_dir(face: usize, x: u32, y: u32, size: u32) -> [f32; 3] {
    let u = (2.0 * (x as f32 + 0.5) / size as f32) - 1.0;
    let v = (2.0 * (y as f32 + 0.5) / size as f32) - 1.0;
    let d = match face {
        0 => [1.0, -v, -u],
        1 => [-1.0, -v, u],
        2 => [u, 1.0, v],
        3 => [u, -1.0, -v],
        4 => [u, -v, 1.0],
        _ => [-u, -v, -1.0],
    };
    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    [d[0] / l, d[1] / l, d[2] / l]
}

fn check_unit_dir(dir: [f32; 3], what: &'static str) -> Result<[f32; 3], IblError> {
    if !dir.iter().all(|c| c.is_finite()) {
        return Err(IblError::Invalid(format!("ibl {what} is not finite")));
    }
    let l2 = dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2];
    if l2 < 1e-12 {
        return Err(IblError::Invalid(format!("ibl {what} has ~zero length")));
    }
    let l = l2.sqrt();
    Ok([dir[0] / l, dir[1] / l, dir[2] / l])
}

fn check_roughness(roughness: f32) -> Result<f32, IblError> {
    if !roughness.is_finite() || !(0.0..=1.0).contains(&roughness) {
        return Err(IblError::Invalid(format!(
            "ibl roughness {roughness} outside 0..=1"
        )));
    }
    Ok(roughness)
}

/// Bilinear sample of one cube level for `dir`: dominant axis picks the face
/// (the inverse of [`cube_texel_dir`]), UV clamps at face edges. Level blends
/// toward `roughness`: `f = roughness * (levels - 1)` lerps the two nearest
/// stored levels, so stored roughness steps reproduce the bake exactly.
pub fn sample_cube(
    cube: &PrefilterCube,
    dir: [f32; 3],
    roughness: f32,
) -> Result<[f32; 3], IblError> {
    let d = check_unit_dir(dir, "sample direction")?;
    let r = check_roughness(roughness)?;
    if cube.levels == 0 || cube.size == 0 {
        return Err(IblError::Invalid("ibl cube is empty".into()));
    }
    let f = r * (cube.levels - 1) as f32;
    let l0 = (f.floor() as usize).min(cube.levels - 1);
    let l1 = (l0 + 1).min(cube.levels - 1);
    let t = f - l0 as f32;
    let a = sample_level(cube, l0, d)?;
    if l1 == l0 {
        return Ok(a);
    }
    let b = sample_level(cube, l1, d)?;
    Ok([
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ])
}

fn sample_level(cube: &PrefilterCube, level: usize, d: [f32; 3]) -> Result<[f32; 3], IblError> {
    let ax = d[0].abs();
    let ay = d[1].abs();
    let az = d[2].abs();
    // Dominant axis selects the face; the (u, v) projections invert
    // `cube_texel_dir` exactly, so texel centers round-trip bit-for-bit.
    let (face, u, v): (usize, f32, f32) = if ax >= ay && ax >= az {
        if d[0] > 0.0 {
            (0, -d[2] / ax, -d[1] / ax)
        } else {
            (1, d[2] / ax, -d[1] / ax)
        }
    } else if ay >= ax && ay >= az {
        if d[1] > 0.0 {
            (2, d[0] / ay, d[2] / ay)
        } else {
            (3, d[0] / ay, -d[2] / ay)
        }
    } else if d[2] > 0.0 {
        (4, d[0] / az, -d[1] / az)
    } else {
        (5, -d[0] / az, -d[1] / az)
    };
    let size = cube.size;
    let x = ((u + 1.0) * 0.5 * size as f32 - 0.5).clamp(0.0, size as f32 - 1.0);
    let y = ((v + 1.0) * 0.5 * size as f32 - 0.5).clamp(0.0, size as f32 - 1.0);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(size - 1);
    let y1 = (y0 + 1).min(size - 1);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let base = cube.face_base(level, face)?;
    let n = size as usize;
    let at = |xx: u32, yy: u32| cube.faces[base + yy as usize * n + xx as usize];
    let a = at(x0, y0);
    let b = at(x1, y0);
    let c = at(x0, y1);
    let e = at(x1, y1);
    let mix = |p: [f32; 3], q: [f32; 3], t: f32| {
        [
            p[0] + (q[0] - p[0]) * t,
            p[1] + (q[1] - p[1]) * t,
            p[2] + (q[2] - p[2]) * t,
        ]
    };
    Ok(mix(mix(a, b, fx), mix(c, e, fx), fy))
}

// ---------------------------------------------------------------------------
// Irradiance SH: cosine-convolved projection shared with `crate::gi`.
// ---------------------------------------------------------------------------

/// Irradiance spherical harmonics: 9 RGB coefficients convolved with the
/// clamped-cosine lobe (`A0 = PI`, `A1 = 2PI/3`, `A2 = PI/4` per Ram-Ng), so
/// [`eval_sh`] returns irradiance directly. The basis is
/// [`crate::gi::sh_basis`]; bake and eval share it by construction.
#[derive(Clone, PartialEq, Debug)]
pub struct IrradianceSh {
    /// `A_l`-convolved RGB coefficients, L0..L2 order of `gi::sh_basis`.
    pub coeffs: [[f32; 3]; IBL_SH_COEFFS],
}

impl IrradianceSh {
    /// Zero coefficients (test backing, never a cook result).
    pub fn zero() -> Self {
        Self {
            coeffs: [[0.0; 3]; IBL_SH_COEFFS],
        }
    }

    /// Deterministic little-endian bytes: coefficient-major, RGB order.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(IBL_SH_COEFFS * 12);
        for c in &self.coeffs {
            for v in c {
                out.extend_from_slice(&v.to_le_bytes());
            }
        }
        out
    }
}

/// Diffuse irradiance for `normal` from convolved coefficients: the same dot
/// product as [`crate::gi::sh_irradiance`], so probe and IBL eval agree.
pub fn eval_sh(sh: &IrradianceSh, normal: [f32; 3]) -> Result<[f32; 3], IblError> {
    let n = check_unit_dir(normal, "sh normal")?;
    let rgb = crate::gi::sh_irradiance(&sh.coeffs, n);
    Ok([rgb[0].max(0.0), rgb[1].max(0.0), rgb[2].max(0.0)])
}

// ---------------------------------------------------------------------------
// BRDF LUT: GGX split-sum scale/bias over (NdotV, roughness).
// ---------------------------------------------------------------------------

/// Split-sum BRDF LUT: texel `(ix, iy)` holds the `(scale, bias)` pair for
/// `NdotV = (ix + 0.5) / size`, `roughness = (iy + 0.5) / size`, integrated
/// with [`BRDF_SAMPLES`] Hammersley taps (Epic/Karis GGX importance sample).
/// Row-major, `size * size` texels.
#[derive(Clone, PartialEq, Debug)]
pub struct BrdfLut {
    /// LUT edge in texels.
    pub size: u32,
    /// Row-major `(scale, bias)` pairs.
    pub data: Vec<[f32; 2]>,
}

impl BrdfLut {
    /// Zero-filled LUT (test backing, never a cook result).
    pub fn new(size: u32) -> Self {
        Self {
            size,
            data: vec![[0.0; 2]; size as usize * size as usize],
        }
    }

    /// Texel count.
    pub fn texels(&self) -> usize {
        self.data.len()
    }

    /// Read one texel; bounds-checked for callers outside the bake.
    pub fn texel(&self, x: u32, y: u32) -> Result<[f32; 2], IblError> {
        if x >= self.size || y >= self.size {
            return Err(IblError::Invalid(format!(
                "ibl brdf texel ({x},{y}) outside {}x{}",
                self.size, self.size
            )));
        }
        Ok(self.data[y as usize * self.size as usize + x as usize])
    }

    /// Deterministic little-endian bytes: row-major, `(scale, bias)` order.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.data.len() * 8);
        for t in &self.data {
            for c in t {
                out.extend_from_slice(&c.to_le_bytes());
            }
        }
        out
    }
}

fn radical_inverse_vdc(mut bits: u32) -> f32 {
    bits = bits.swap_bytes();
    bits = ((bits & 0x5555_5555) << 1) | ((bits & 0xAAAA_AAAA) >> 1);
    bits = ((bits & 0x3333_3333) << 2) | ((bits & 0xCCCC_CCCC) >> 2);
    bits = ((bits & 0x0F0F_0F0F) << 4) | ((bits & 0xF0F0_F0F0) >> 4);
    bits = ((bits & 0x00FF_00FF) << 8) | ((bits & 0xFF00_FF00) >> 8);
    (bits as f64 * 2.328_306_436_538_696_3e-10) as f32
}

fn hammersley(i: u32, n: u32) -> [f32; 2] {
    [i as f32 / n as f32, radical_inverse_vdc(i)]
}

/// GGX importance-sampled split-sum integral for one `(NdotV, roughness)`:
/// returns `(scale, bias)` so `spec = prefiltered * (F0 * scale + bias)`.
/// Fixed Hammersley sequence, fixed tap count: bit-identical per input.
pub fn brdf_integrate(ndotv: f32, roughness: f32) -> Result<[f32; 2], IblError> {
    if !ndotv.is_finite() || !(0.0..=1.0).contains(&ndotv) {
        return Err(IblError::Invalid(format!(
            "ibl NdotV {ndotv} outside 0..=1"
        )));
    }
    let roughness = check_roughness(roughness)?;
    let ndotv = ndotv.max(1e-4);
    let vx = (1.0 - ndotv * ndotv).max(0.0).sqrt();
    let v = [vx, 0.0, ndotv];
    let a = roughness * roughness;
    let a2 = a * a;
    // Schlick-GGX IBL geometry factor `k = alpha / 2` with
    // `alpha = roughness^2` (Karis course notes; the NDF above squares it
    // again, the geometry term does not).
    let k = a * 0.5;
    let ggx = |ndot: f32| ndot / (ndot * (1.0 - k) + k).max(1e-9);
    let mut acc = [0.0f64; 2];
    for i in 0..BRDF_SAMPLES {
        let xi = hammersley(i, BRDF_SAMPLES);
        let phi = 2.0 * PI * xi[0];
        let cos_theta = ((1.0 - xi[1]) / (1.0 + (a2 - 1.0) * xi[1]).max(1e-9)).sqrt();
        let sin_theta = (1.0 - cos_theta * cos_theta).max(0.0).sqrt();
        let h = [sin_theta * phi.cos(), sin_theta * phi.sin(), cos_theta];
        let vdot_h = (v[0] * h[0] + v[1] * h[1] + v[2] * h[2]).max(0.0);
        let ndot_h = h[2].max(0.0);
        let ndot_l = (2.0 * vdot_h * h[2] - v[2]).max(0.0);
        if ndot_l > 0.0 {
            let g = ggx(ndotv) * ggx(ndot_l);
            let g_vis = (g * vdot_h / (ndot_h * ndotv).max(1e-9)) as f64;
            let fc = (1.0 - vdot_h).powi(5) as f64;
            acc[0] += (1.0 - fc) * g_vis;
            acc[1] += fc * g_vis;
        }
    }
    let n = f64::from(BRDF_SAMPLES);
    Ok([(acc[0] / n) as f32, (acc[1] / n) as f32])
}

/// Full LUT bake over the `(NdotV, roughness)` grid: texel centers, fixed
/// order. Bit-identical per size on every run.
pub fn brdf_lut_build(size: u32) -> Result<BrdfLut, IblError> {
    if size == 0 || size > 512 {
        return Err(IblError::Invalid(format!(
            "ibl brdf size {size} outside 1..=512"
        )));
    }
    let mut lut = BrdfLut::new(size);
    for y in 0..size {
        for x in 0..size {
            let ndotv = (x as f32 + 0.5) / size as f32;
            let roughness = (y as f32 + 0.5) / size as f32;
            lut.data[y as usize * size as usize + x as usize] = brdf_integrate(ndotv, roughness)?;
        }
    }
    Ok(lut)
}

/// Bilinear sample of the LUT: edges clamp, so `NdotV = 0/1` stays defined.
pub fn brdf_sample(lut: &BrdfLut, ndotv: f32, roughness: f32) -> Result<[f32; 2], IblError> {
    if !ndotv.is_finite() || !(0.0..=1.0).contains(&ndotv) {
        return Err(IblError::Invalid(format!(
            "ibl NdotV {ndotv} outside 0..=1"
        )));
    }
    let r = check_roughness(roughness)?;
    if lut.size == 0 {
        return Err(IblError::Invalid("ibl brdf lut is empty".into()));
    }
    let w = lut.size as f32;
    let x = (ndotv * w - 0.5).clamp(0.0, w - 1.0);
    let y = (r * w - 0.5).clamp(0.0, w - 1.0);
    let x0 = x.floor() as u32;
    let y0 = y.floor() as u32;
    let x1 = (x0 + 1).min(lut.size - 1);
    let y1 = (y0 + 1).min(lut.size - 1);
    let fx = x - x0 as f32;
    let fy = y - y0 as f32;
    let n = lut.size as usize;
    let at = |xx: u32, yy: u32| lut.data[yy as usize * n + xx as usize];
    let a = at(x0, y0);
    let b = at(x1, y0);
    let c = at(x0, y1);
    let e = at(x1, y1);
    Ok([
        a[0] + ((b[0] - a[0]) * fx + (c[0] - a[0]) * fy + (a[0] - b[0] - c[0] + e[0]) * fx * fy),
        a[1] + ((b[1] - a[1]) * fx + (c[1] - a[1]) * fy + (a[1] - b[1] - c[1] + e[1]) * fx * fy),
    ])
}

// ---------------------------------------------------------------------------
// Cook-time bake behind `v1`: HDRI bytes in, deterministic bake out.
// ---------------------------------------------------------------------------

/// Cooked IBL: prefiltered cubemap plus irradiance SH plus BRDF LUT, baked
/// from one HDRI. `to_bytes` is the deterministic cook artifact (level-major
/// cube, then SH, then LUT); same HDRI bytes give bit-identical bakes.
#[cfg(feature = "v1")]
#[derive(Clone, PartialEq, Debug)]
pub struct IblBake {
    /// Roughness-stepped prefiltered cubemap.
    pub cube: PrefilterCube,
    /// Cosine-convolved irradiance SH.
    pub sh: IrradianceSh,
    /// GGX split-sum LUT.
    pub brdf: BrdfLut,
    /// Mean HDRI texel (exposure calibration, the R2 `env_average`).
    pub source_average: [f32; 3],
}

#[cfg(feature = "v1")]
impl IblBake {
    /// Receipt for the last bake (pass ms supplied by the caller).
    pub fn receipt(&self, ms: f64) -> IblReceipt {
        IblReceipt {
            cube_texels: self.cube.texels() as u64,
            lut_texels: self.brdf.texels() as u64,
            sh_coeffs: IBL_SH_COEFFS as u32,
            ms,
        }
    }

    /// Deterministic cook artifact bytes: cube, then SH, then LUT.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = self.cube.to_bytes();
        out.extend_from_slice(&self.sh.to_bytes());
        out.extend_from_slice(&self.brdf.to_bytes());
        out
    }

    /// Headless submit seam (no device in this build): validates the shared
    /// [`crate::gtao::LookFrame`], then refuses loudly with the reason. The
    /// CPU bake above is the reference here; the `window` build below
    /// records the live pass instead. Never silent success.
    #[cfg(all(feature = "v1", not(feature = "window")))]
    pub fn submit_gpu(&self, frame: &crate::gtao::LookFrame) -> Result<IblReceipt, IblError> {
        frame
            .validate()
            .map_err(|e| IblError::Invalid(format!("ibl look frame: {e}")))?;
        Err(IblError::Invalid(
            "ibl gpu pass needs the window build with a live device; CPU bake is the reference"
                .into(),
        ))
    }
}

/// Decode HDRI bytes and bake the full IBL set. Empty bytes refuse with
/// [`IblError::MissingHdri`]; undecodable bytes wrap the R2 error. The same
/// complete bytes give bit-identical bakes on every run.
#[cfg(feature = "v1")]
pub fn bake(hdri_bytes: &[u8]) -> Result<(IblBake, f64), IblError> {
    if hdri_bytes.is_empty() {
        return Err(IblError::MissingHdri);
    }
    let started = Instant::now();
    let env = crate::v1_ibl_shadow::load_hdri(hdri_bytes).map_err(IblError::from)?;
    let cube = bake_cube(&env)?;
    let sh = project_sh(&env);
    let brdf = brdf_lut_build(BRDF_LUT_SIZE)?;
    let source_average = crate::v1_ibl_shadow::env_average(&env);
    let ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok((
        IblBake {
            cube,
            sh,
            brdf,
            source_average,
        },
        ms,
    ))
}

/// Prefilter every (level, face, texel) through the R2 roughness kernel:
/// parity with the R2 CPU reference is exact by construction.
#[cfg(feature = "v1")]
fn bake_cube(env: &crate::v1_ibl_shadow::EnvMap) -> Result<PrefilterCube, IblError> {
    let mut cube = PrefilterCube::new(IBL_CUBE_SIZE, IBL_LEVELS);
    for level in 0..IBL_LEVELS {
        let roughness = PrefilterCube::level_roughness(level)?;
        for face in 0..IBL_FACES {
            for y in 0..IBL_CUBE_SIZE {
                for x in 0..IBL_CUBE_SIZE {
                    let dir = cube_texel_dir(face, x, y, IBL_CUBE_SIZE);
                    let tap = crate::v1_ibl_shadow::prefilter(env, dir, roughness)
                        .map_err(IblError::from)?;
                    let idx = (level * IBL_FACES + face)
                        * IBL_CUBE_SIZE as usize
                        * IBL_CUBE_SIZE as usize
                        + y as usize * IBL_CUBE_SIZE as usize
                        + x as usize;
                    cube.faces[idx] = tap;
                }
            }
        }
    }
    Ok(cube)
}

/// Solid-angle-weighted SH projection with the `gi` basis, convolved with
/// the clamped-cosine lobe. f64 accumulation in fixed texel order.
#[cfg(feature = "v1")]
fn project_sh(env: &crate::v1_ibl_shadow::EnvMap) -> IrradianceSh {
    const A0: f64 = std::f64::consts::PI;
    const A1: f64 = 2.0 * std::f64::consts::PI / 3.0;
    const A2: f64 = std::f64::consts::PI / 4.0;
    const LOBE: [f64; IBL_SH_COEFFS] = [A0, A1, A1, A1, A2, A2, A2, A2, A2];
    let w = env.width as usize;
    let h = env.height as usize;
    let d_phi = std::f64::consts::PI / h as f64;
    let d_theta = 2.0 * std::f64::consts::PI / w as f64;
    let mut acc = [[0.0f64; 3]; IBL_SH_COEFFS];
    for y in 0..h {
        // Texel-center solid angle: sin(phi) * dphi * dtheta.
        let phi = (y as f64 + 0.5) / h as f64 * std::f64::consts::PI;
        let domega = phi.sin() * d_phi * d_theta;
        for x in 0..w {
            let p = env.pixels[y * w + x];
            let dir = crate::v1_ibl_shadow::uv_to_dir([
                (x as f32 + 0.5) / env.width as f32,
                (y as f32 + 0.5) / env.height as f32,
            ]);
            let b = crate::gi::sh_basis(dir);
            for i in 0..IBL_SH_COEFFS {
                acc[i][0] += f64::from(p[0]) * f64::from(b[i]) * domega;
                acc[i][1] += f64::from(p[1]) * f64::from(b[i]) * domega;
                acc[i][2] += f64::from(p[2]) * f64::from(b[i]) * domega;
            }
        }
    }
    let mut coeffs = [[0.0f32; 3]; IBL_SH_COEFFS];
    for i in 0..IBL_SH_COEFFS {
        coeffs[i] = [
            (acc[i][0] * LOBE[i]) as f32,
            (acc[i][1] * LOBE[i]) as f32,
            (acc[i][2] * LOBE[i]) as f32,
        ];
    }
    IrradianceSh { coeffs }
}

/// Forward-pass IBL shade in linear space: diffuse from the irradiance SH
/// plus split-sum specular from the prefiltered cubemap and the BRDF LUT.
/// `base_color` is linear `0..=1`, `metallic`/`roughness` in `0..=1`.
#[cfg(feature = "v1")]
pub fn shade_ibl(
    bake: &IblBake,
    normal: [f32; 3],
    view_dir: [f32; 3],
    base_color: [f32; 3],
    metallic: f32,
    roughness: f32,
) -> Result<[f32; 3], IblError> {
    let n = check_unit_dir(normal, "shade normal")?;
    let v = check_unit_dir(view_dir, "shade view direction")?;
    if !base_color.iter().all(|c| c.is_finite()) {
        return Err(IblError::Invalid("ibl base color is not finite".into()));
    }
    if base_color.iter().any(|c| *c < 0.0 || *c > 1.0) {
        return Err(IblError::Invalid(format!(
            "ibl base color {base_color:?} outside 0..=1"
        )));
    }
    if !metallic.is_finite() || !(0.0..=1.0).contains(&metallic) {
        return Err(IblError::Invalid(format!(
            "ibl metallic {metallic} outside 0..=1"
        )));
    }
    let r = check_roughness(roughness)?;
    let e = eval_sh(&bake.sh, n)?;
    let diffuse = [
        (1.0 - metallic) * base_color[0] * e[0] / PI,
        (1.0 - metallic) * base_color[1] * e[1] / PI,
        (1.0 - metallic) * base_color[2] * e[2] / PI,
    ];
    let ndotv = (n[0] * v[0] + n[1] * v[1] + n[2] * v[2]).clamp(0.0, 1.0);
    let refl = [
        2.0 * ndotv * n[0] - v[0],
        2.0 * ndotv * n[1] - v[1],
        2.0 * ndotv * n[2] - v[2],
    ];
    let pre = sample_cube(&bake.cube, refl, r)?;
    let lut = brdf_sample(&bake.brdf, ndotv, r)?;
    let f0 = [
        0.04 + (base_color[0] - 0.04) * metallic,
        0.04 + (base_color[1] - 0.04) * metallic,
        0.04 + (base_color[2] - 0.04) * metallic,
    ];
    let spec = [
        pre[0] * (f0[0] * lut[0] + lut[1]),
        pre[1] * (f0[1] * lut[0] + lut[1]),
        pre[2] * (f0[2] * lut[0] + lut[1]),
    ];
    Ok([
        (diffuse[0] + spec[0]).max(0.0),
        (diffuse[1] + spec[1]).max(0.0),
        (diffuse[2] + spec[2]).max(0.0),
    ])
}

// ---------------------------------------------------------------------------
// Forward IBL bind (headless): the sampling chunk plus the SH uniform image
// the forward pass binds. These carry no wgpu types, so they live outside
// `mod gpu`: the headless bind lock below hashes and packs them with no
// device, and `mod gpu` re-exports both for the live submit path. New module
// edge: `graph` reads `FORWARD_IBL_INPUTS` for the bake -> forward wiring.
// ---------------------------------------------------------------------------

/// Forward-pass IBL sampling chunk (WGSL): prefiltered cubemap sample
/// plus SH irradiance dot plus BRDF LUT scale/bias. Engine-owned code
/// (no port markers); the forward pipeline inlines this chunk when
/// the material bind group lands. The SH basis matches
/// [`crate::gi::sh_basis`] term for term; coefficients pack as 9 vec4s
/// (RGB in xyz, w pads) so the GPU dot matches [`eval_sh`].
pub const FORWARD_IBL_WGSL: &str = r#"
@group(1) @binding(0) var ibl_prefilter: texture_cube<f32>;
@group(1) @binding(1) var ibl_brdf: texture_2d<f32>;
@group(1) @binding(2) var ibl_sampler: sampler;
@group(1) @binding(3) var<uniform> ibl_sh: array<vec4<f32>, 9>;
fn ibl_sh_basis(d: vec3<f32>) -> array<f32, 9> {
    var b: array<f32, 9>;
    b[0] = 0.282095;
    b[1] = -0.488603 * d.y;
    b[2] = 0.488603 * d.z;
    b[3] = -0.488603 * d.x;
    b[4] = 1.092548 * d.x * d.y;
    b[5] = -1.092548 * d.y * d.z;
    b[6] = 0.315392 * (3.0 * d.z * d.z - 1.0);
    b[7] = -1.092548 * d.x * d.z;
    b[8] = 0.546274 * (d.x * d.x - d.y * d.y);
    return b;
}
fn ibl_irradiance(n: vec3<f32>) -> vec3<f32> {
    let b = ibl_sh_basis(n);
    var e = vec3<f32>(0.0, 0.0, 0.0);
    e += ibl_sh[0].xyz * b[0];
    e += ibl_sh[1].xyz * b[1];
    e += ibl_sh[2].xyz * b[2];
    e += ibl_sh[3].xyz * b[3];
    e += ibl_sh[4].xyz * b[4];
    e += ibl_sh[5].xyz * b[5];
    e += ibl_sh[6].xyz * b[6];
    e += ibl_sh[7].xyz * b[7];
    e += ibl_sh[8].xyz * b[8];
    return max(e, vec3<f32>(0.0, 0.0, 0.0));
}
fn ibl_sample(n: vec3<f32>, v: vec3<f32>, f0: vec3<f32>, roughness: f32, albedo: vec3<f32>, metallic: f32) -> vec3<f32> {
    let ndotv = clamp(dot(n, v), 0.0, 1.0);
    let r = reflect(-v, n);
    let lod = roughness * 4.0;
    let pre = textureSampleLevel(ibl_prefilter, ibl_sampler, r, lod).rgb;
    let fab = textureSampleLevel(ibl_brdf, ibl_sampler, vec2<f32>(ndotv, roughness), 0.0).rg;
    let diffuse = (1.0 - metallic) * albedo * ibl_irradiance(n) / 3.141592653589793;
    return diffuse + pre * (f0 * fab.x + vec3<f32>(fab.y, fab.y, fab.y));
}
"#;

/// Graph hash of the bound forward chunk as a single node, pinned at cook
/// review time. The bind lock asserts this hash, so the bound chunk is
/// exactly the reviewed bytes: any drift fails loudly instead of rebinding
/// silently. Hashed through [`crate::graph::Graph::graph_hash`] (the
/// crate's hash owner) so the algorithm is owned once.
pub const FORWARD_IBL_CHUNK_HASH: u64 = 15150056934146913161;

/// Graph resources the forward pass reads from the IBL bake: prefiltered
/// cubemap, BRDF LUT and SH uniform. Owned here so the graph names the same
/// strings the bind carries; [`compile_ibl_for_forward`] wires them.
pub const FORWARD_IBL_INPUTS: [&str; 3] = ["ibl_prefilter", "ibl_brdf", "ibl_sh"];
/// Bake pass name producing [`FORWARD_IBL_INPUTS`] in
/// [`compile_ibl_for_forward`].
pub const IBL_BAKE_PASS: &str = "ibl-bake";
/// Forward pass name consuming [`FORWARD_IBL_INPUTS`] (matches the arena
/// fixture forward name, but this wiring compiles its own mini-graph).
pub const IBL_FORWARD_PASS: &str = "forward";
/// SH uniform image size in bytes: 9 coefficients as vec4s (RGB plus pad).
pub const FORWARD_IBL_SH_UNIFORM_BYTES: usize = 144;
/// Compiled forward bind image size in bytes: 8-byte chunk hash plus the
/// [`FORWARD_IBL_SH_UNIFORM_BYTES`]-byte SH uniform.
pub const FORWARD_IBL_BIND_BYTES: usize = 152;

/// Hash of the bound forward chunk through the crate's graph owner; equals
/// [`FORWARD_IBL_CHUNK_HASH`]. Test-only (builds one graph node);
/// per-submit packing copies the pinned const instead, so steady-state
/// submits allocate nothing.
#[cfg(test)]
fn forward_ibl_chunk_probe() -> u64 {
    let mut probe = crate::graph::Graph::new();
    probe.add_node(FORWARD_IBL_WGSL);
    probe.graph_hash()
}

/// Pack the SH uniform image into caller-owned storage (exactly 144 bytes):
/// 9 RGB coefficients as vec4s (RGB in xyz, w pads), same order as
/// [`IrradianceSh::coeffs`], so the GPU dot matches [`eval_sh`] term for
/// term. Slices in, slices out: steady-state submits pack into a reused
/// scratch uniform and grow nothing.
pub fn sh_uniform_bytes_into(sh: &IrradianceSh, out: &mut [u8; 144]) {
    for (i, c) in sh.coeffs.iter().enumerate() {
        let base = i * 16;
        out[base..base + 4].copy_from_slice(&c[0].to_le_bytes());
        out[base + 4..base + 8].copy_from_slice(&c[1].to_le_bytes());
        out[base + 8..base + 12].copy_from_slice(&c[2].to_le_bytes());
        out[base + 12..base + 16].copy_from_slice(&0.0f32.to_le_bytes());
    }
}

/// Compiled forward bind image (headless): bytes `[0..8]` carry
/// [`FORWARD_IBL_CHUNK_HASH`] little-endian, bytes `[8..152]` carry the SH
/// uniform from [`sh_uniform_bytes_into`]. Assembled into caller-owned
/// storage only, so steady-state repacks allocate nothing; the same SH gives
/// the same bytes on every run. Infallible: every SH packs.
pub fn forward_ibl_bind_image(sh: &IrradianceSh, out: &mut [u8; FORWARD_IBL_BIND_BYTES]) {
    out[0..8].copy_from_slice(&FORWARD_IBL_CHUNK_HASH.to_le_bytes());
    let mut uniform = [0u8; FORWARD_IBL_SH_UNIFORM_BYTES];
    sh_uniform_bytes_into(sh, &mut uniform);
    out[8..FORWARD_IBL_BIND_BYTES].copy_from_slice(&uniform);
}

/// Bake -> forward wiring for the IBL bind (headless): producer
/// [`IBL_BAKE_PASS`] outputs [`FORWARD_IBL_INPUTS`], consumer
/// [`IBL_FORWARD_PASS`] reads them, and compile infers the handover edges,
/// so the compiled graph carries one Transition barrier per resource from
/// bake to forward. Mirrors fog's `compile_fog_for_forward`: planning-time
/// only (the plan allocates once); per-submit packing stays on
/// [`forward_ibl_bind_image`] with caller-owned storage. Fails loudly with
/// [`crate::graph::GraphError`] when the wiring stops compiling.
pub fn compile_ibl_for_forward() -> Result<crate::graph::CompiledGraph, crate::graph::GraphError> {
    let mut graph = crate::graph::FrameGraph::new();
    let bake = graph.add_pass(
        crate::graph::PassDesc::new(IBL_BAKE_PASS)
            .output(FORWARD_IBL_INPUTS[0])
            .output(FORWARD_IBL_INPUTS[1])
            .output(FORWARD_IBL_INPUTS[2])
            .work(1, 4),
    );
    let forward = graph.add_pass(
        crate::graph::PassDesc::new(IBL_FORWARD_PASS)
            .input(FORWARD_IBL_INPUTS[0])
            .input(FORWARD_IBL_INPUTS[1])
            .input(FORWARD_IBL_INPUTS[2])
            .work(2, 6),
    );
    graph.add_edge(bake, forward)?;
    graph.compile()
}

// ---------------------------------------------------------------------------
// GPU descriptors behind `window`: real texture allocation for the bake.
// Allocation tests skip without an adapter, exactly like the presenter
// tests. The forward chunk plus the SH pack live above (no wgpu types) and
// are re-exported here for the live submit path.
// ---------------------------------------------------------------------------

/// IBL GPU descriptors for the forward presenter: one `Rgba32Float` cubemap
/// (level 0 resident; rougher levels follow as mips) plus the `Rg32Float`
/// BRDF LUT. Cook bytes upload verbatim; the llvmpipe round-trip test pins
/// byte parity.
#[cfg(feature = "window")]
pub mod gpu {
    use super::IblError;

    pub use super::{sh_uniform_bytes_into, FORWARD_IBL_WGSL};

    /// Prefiltered cubemap descriptor: `Rgba32Float`, 6 array layers (one per
    /// face), mip count = roughness levels, sampler-readable for the forward
    /// pass plus copy-dest for the cook upload and copy-src for readback.
    pub fn prefilter_cube_desc(size: u32, levels: u32) -> wgpu::TextureDescriptor<'static> {
        wgpu::TextureDescriptor {
            label: Some("rl2-ibl-prefilter"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: super::IBL_FACES as u32,
            },
            mip_level_count: levels,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        }
    }

    /// BRDF LUT descriptor: `Rg32Float` (scale, bias), sampler-readable plus
    /// copy-dest for the cook upload.
    pub fn brdf_lut_desc(size: u32) -> wgpu::TextureDescriptor<'static> {
        wgpu::TextureDescriptor {
            label: Some("rl2-ibl-brdf-lut"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rg32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        }
    }

    /// SH uniform image: 9 RGB coefficients packed as 9 vec4s (RGB in xyz,
    /// w pads), 144 bytes for the `ibl_sh` uniform above. Same order as
    /// [`super::IrradianceSh::coeffs`], so the GPU dot matches
    /// [`super::eval_sh`] term for term. Packing lives in
    /// [`super::sh_uniform_bytes_into`]; this wrapper owns the one-shot Vec.
    pub fn sh_uniform_bytes(sh: &super::IrradianceSh) -> Vec<u8> {
        let mut out = [0u8; 144];
        sh_uniform_bytes_into(sh, &mut out);
        out.to_vec()
    }

    /// BRDF LUT cook bytes in upload order: row-major `(scale, bias)` f32
    /// pairs, the same order as [`super::BrdfLut::to_bytes`]. Shared by the
    /// one-shot upload below and the chart submitter re-upload, so both
    /// paths push verbatim cook bytes.
    pub fn brdf_upload_bytes(lut: &super::BrdfLut) -> Vec<u8> {
        let mut bytes = vec![0u8; lut.data.len() * 8];
        brdf_upload_bytes_into(lut, &mut bytes);
        bytes
    }

    /// Pack the BRDF LUT cook bytes into caller-owned storage (exactly
    /// `data.len() * 8` bytes). The submitter reuses one scratch LUT, so
    /// steady-state submits grow nothing.
    pub fn brdf_upload_bytes_into(lut: &super::BrdfLut, out: &mut [u8]) {
        debug_assert_eq!(out.len(), lut.data.len() * 8);
        for (t, chunk) in lut.data.iter().zip(out.chunks_exact_mut(8)) {
            chunk[0..4].copy_from_slice(&t[0].to_le_bytes());
            chunk[4..8].copy_from_slice(&t[1].to_le_bytes());
        }
    }

    /// Upload the BRDF LUT cook bytes verbatim into a GPU texture. Real GPU
    /// work for the presenter material bind group; callers keep last-good on
    /// error.
    pub fn upload_brdf_lut(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        lut: &super::BrdfLut,
    ) -> Result<wgpu::Texture, IblError> {
        if lut.size == 0 || lut.size > 512 {
            return Err(IblError::Invalid(format!(
                "ibl brdf gpu size {} outside 1..=512",
                lut.size
            )));
        }
        if lut.data.len() != lut.size as usize * lut.size as usize {
            return Err(IblError::Invalid(
                "ibl brdf lut data length mismatches size".into(),
            ));
        }
        let texture = device.create_texture(&brdf_lut_desc(lut.size));
        let bytes = brdf_upload_bytes(lut);
        let size = wgpu::Extent3d {
            width: lut.size,
            height: lut.size,
            depth_or_array_layers: 1,
        };
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(lut.size * 8),
                rows_per_image: Some(lut.size),
            },
            size,
        );
        Ok(texture)
    }

    /// Prefiltered cook bytes as a 2D array for exact texel-fetch sampling:
    /// `Rgba32Float`, one layer per (level, face) with layer
    /// `level * 6 + face`, level-major then face-minor like
    /// [`super::PrefilterCube::faces`]. The 32-bit float format is not
    /// hardware-filterable, so the chart shader below fetches texels with
    /// `textureLoad` and does the bilinear plus level blend in-shader: the
    /// same math as [`super::sample_cube`], bit for bit up to f32 fusing.
    pub fn prefilter_array_desc(size: u32, levels: u32) -> wgpu::TextureDescriptor<'static> {
        wgpu::TextureDescriptor {
            label: Some("rl2-ibl-prefilter-array"),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: levels * super::IBL_FACES as u32,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        }
    }

    /// Exact byte length of one packed layer (`edge * edge * 16`).
    /// Fails loudly on overflow instead of wrapping the size.
    pub fn layer_byte_len(cube: &super::PrefilterCube) -> Result<usize, super::IblError> {
        let edge = usize::try_from(cube.size)
            .map_err(|_| super::IblError::Invalid("ibl cube edge exceeds usize".into()))?;
        edge.checked_mul(edge)
            .and_then(|n| n.checked_mul(16))
            .ok_or_else(|| super::IblError::Invalid("ibl layer bytes overflow".into()))
    }

    /// Pack one (level, face) layer as RGBA32F upload bytes into
    /// caller-owned storage (exactly [`layer_byte_len`] bytes). The
    /// submitter reuses one scratch layer for all 30 uploads, so
    /// steady-state submits grow nothing.
    pub fn prefilter_layer_bytes_into(
        cube: &super::PrefilterCube,
        level: usize,
        face: usize,
        out: &mut [u8],
    ) -> Result<(), super::IblError> {
        if out.len() != layer_byte_len(cube)? {
            return Err(super::IblError::Invalid(
                "ibl chart layer scratch has the wrong length".into(),
            ));
        }
        let edge = usize::try_from(cube.size)
            .map_err(|_| super::IblError::Invalid("ibl cube edge exceeds usize".into()))?;
        let one = 1.0f32.to_le_bytes();
        for (y_us, y) in (0..cube.size).enumerate() {
            for (x_us, x) in (0..cube.size).enumerate() {
                let t = cube.texel(level, face, x, y)?;
                let base = (y_us * edge + x_us) * 16;
                out[base..base + 4].copy_from_slice(&t[0].to_le_bytes());
                out[base + 4..base + 8].copy_from_slice(&t[1].to_le_bytes());
                out[base + 8..base + 12].copy_from_slice(&t[2].to_le_bytes());
                out[base + 12..base + 16].copy_from_slice(&one);
            }
        }
        Ok(())
    }

    /// Posted encode + submit + readback budget for the chart pass in ms
    /// (debug build on the reference machine, evidence only, never hashed;
    /// arena-size timing belongs to PF2).
    pub const IBL_SUBMIT_BUDGET_MS: f64 = 16.0;
    /// Posted GPU-vs-CPU chart parity: absolute linear-RGB units. The shader
    /// repeats the CPU sampler math texel for texel, so only f32 fusing
    /// separates the legs (observed 2e-5 on the reference GPU); an axis flip
    /// or a missing lobe would miss by 10x this gate.
    pub const IBL_CHART_ABS_TOL: f32 = 0.005;
    /// Posted GPU-vs-CPU chart parity: relative error on chart pixels whose
    /// reference channel exceeds 0.2 (observed 0.002% on the reference GPU).
    pub const IBL_CHART_REL_TOL: f32 = 0.01;

    /// Material-chart pass shader over the shared look uniform plus the full
    /// bake: one `Rgba32Float` target, no vertex buffers, no depth. Each
    /// pixel derives its material from its coordinates (see
    /// [`chart_inputs`]): columns alternate dielectric/metal, halves swap
    /// albedo, rows sweep roughness, normals fan out over the faces. SH,
    /// prefilter and BRDF sampling repeat the CPU [`super::shade_ibl`] math;
    /// `textureLoad` fetches exact cook texels (32-bit float is not
    /// hardware-filterable), so the pass is a real GPU sampling of the bake
    /// with the look exposure applied. Engine-owned code.
    pub const CHART_WGSL: &str = r#"
struct LookU {
    frame_idx: u32,
    width_px: u32,
    height_px: u32,
    exposure: f32,
    sun_dir: vec3<f32>,
    _pad: f32,
};
@group(0) @binding(0) var<uniform> look: LookU;
@group(1) @binding(0) var pre: texture_2d_array<f32>;
@group(1) @binding(1) var brdf: texture_2d<f32>;
@group(1) @binding(2) var<uniform> sh: array<vec4<f32>, 9>;
struct VOut {
    @builtin(position) pos: vec4<f32>,
};
@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VOut {
    var p = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0)
    );
    var out: VOut;
    out.pos = vec4<f32>(p[vi], 0.0, 1.0);
    return out;
}
fn chart_basis(d: vec3<f32>) -> array<f32, 9> {
    var b: array<f32, 9>;
    b[0] = 0.282095;
    b[1] = -0.488603 * d.y;
    b[2] = 0.488603 * d.z;
    b[3] = -0.488603 * d.x;
    b[4] = 1.092548 * d.x * d.y;
    b[5] = -1.092548 * d.y * d.z;
    b[6] = 0.315392 * (3.0 * d.z * d.z - 1.0);
    b[7] = -1.092548 * d.x * d.z;
    b[8] = 0.546274 * (d.x * d.x - d.y * d.y);
    return b;
}
fn pre_level(layer_base: i32, face: i32, u: f32, v: f32) -> vec3<f32> {
    let x = clamp((u + 1.0) * 0.5 * 64.0 - 0.5, 0.0, 63.0);
    let y = clamp((v + 1.0) * 0.5 * 64.0 - 0.5, 0.0, 63.0);
    let x0 = i32(floor(x));
    let y0 = i32(floor(y));
    let x1 = min(x0 + 1, 63);
    let y1 = min(y0 + 1, 63);
    let fx = x - f32(x0);
    let fy = y - f32(y0);
    let layer = layer_base + face;
    let a = textureLoad(pre, vec2<i32>(x0, y0), layer, 0).rgb;
    let bb = textureLoad(pre, vec2<i32>(x1, y0), layer, 0).rgb;
    let c = textureLoad(pre, vec2<i32>(x0, y1), layer, 0).rgb;
    let e = textureLoad(pre, vec2<i32>(x1, y1), layer, 0).rgb;
    return mix(mix(a, bb, fx), mix(c, e, fx), fy);
}
fn brdf_pair(ndotv: f32, roughness: f32) -> vec2<f32> {
    let x = clamp(ndotv * 64.0 - 0.5, 0.0, 63.0);
    let y = clamp(roughness * 64.0 - 0.5, 0.0, 63.0);
    let x0 = i32(floor(x));
    let y0 = i32(floor(y));
    let x1 = min(x0 + 1, 63);
    let y1 = min(y0 + 1, 63);
    let fx = x - f32(x0);
    let fy = y - f32(y0);
    let a = textureLoad(brdf, vec2<i32>(x0, y0), 0).rg;
    let bb = textureLoad(brdf, vec2<i32>(x1, y0), 0).rg;
    let c = textureLoad(brdf, vec2<i32>(x0, y1), 0).rg;
    let e = textureLoad(brdf, vec2<i32>(x1, y1), 0).rg;
    return mix(mix(a, bb, fx), mix(c, e, fx), fy);
}
@fragment
fn fs(in: VOut) -> @location(0) vec4<f32> {
    let ix = i32(in.pos.x);
    let iy = i32(in.pos.y);
    let w = f32(look.width_px);
    let h = f32(look.height_px);
    let fx = (f32(ix) + 0.5) / w;
    let fy = (f32(iy) + 0.5) / h;
    let n = normalize(vec3<f32>((fx - 0.5) * 2.0, 0.6, (fy - 0.5) * 2.0));
    let vv = vec3<f32>(0.4, 0.6, 0.5);
    let v = vv / length(vv);
    var albedo = vec3<f32>(0.5, 0.5, 0.5);
    if (fx >= 0.5) {
        albedo = vec3<f32>(0.8, 0.2, 0.1);
    }
    let metallic = f32(ix % 2);
    let roughness = fy;
    let b = chart_basis(n);
    var e = vec3<f32>(0.0, 0.0, 0.0);
    e += sh[0].xyz * b[0];
    e += sh[1].xyz * b[1];
    e += sh[2].xyz * b[2];
    e += sh[3].xyz * b[3];
    e += sh[4].xyz * b[4];
    e += sh[5].xyz * b[5];
    e += sh[6].xyz * b[6];
    e += sh[7].xyz * b[7];
    e += sh[8].xyz * b[8];
    e = max(e, vec3<f32>(0.0, 0.0, 0.0));
    let diffuse = (1.0 - metallic) * albedo * e / 3.141592653589793;
    let ndotv = clamp(dot(n, v), 0.0, 1.0);
    let r = 2.0 * ndotv * n - v;
    let ax = abs(r.x);
    let ay = abs(r.y);
    let az = abs(r.z);
    var face = 5;
    var uu = 0.0;
    var vw = 0.0;
    if (ax >= ay && ax >= az) {
        if (r.x > 0.0) {
            face = 0;
            uu = -r.z / ax;
            vw = -r.y / ax;
        } else {
            face = 1;
            uu = r.z / ax;
            vw = -r.y / ax;
        }
    } else if (ay >= ax && ay >= az) {
        if (r.y > 0.0) {
            face = 2;
            uu = r.x / ay;
            vw = r.z / ay;
        } else {
            face = 3;
            uu = r.x / ay;
            vw = -r.z / ay;
        }
    } else {
        if (r.z > 0.0) {
            face = 4;
            uu = r.x / az;
            vw = -r.y / az;
        } else {
            face = 5;
            uu = -r.x / az;
            vw = -r.y / az;
        }
    }
    let f = roughness * 4.0;
    let l0 = min(i32(floor(f)), 3);
    let l1 = l0 + 1;
    let t = f - f32(l0);
    let p0 = pre_level(l0 * 6, face, uu, vw);
    let p1 = pre_level(l1 * 6, face, uu, vw);
    let sampled = mix(p0, p1, t);
    let fab = brdf_pair(ndotv, roughness);
    let f0 = vec3<f32>(0.04, 0.04, 0.04) + (albedo - vec3<f32>(0.04, 0.04, 0.04)) * metallic;
    let spec = sampled * (f0 * fab.x + vec3<f32>(fab.y, fab.y, fab.y));
    let escale = clamp(look.exposure / 10.0, 0.5, 2.0);
    return vec4<f32>((diffuse + spec) * escale, 1.0);
}
"#;

    /// Chart material inputs for pixel `(x, y)` in a `w` by `h` target:
    /// returns `(normal, view_dir, albedo, metallic, roughness)`. The exact
    /// formulas the [`CHART_WGSL`] fragment runs, so the CPU
    /// [`super::shade_ibl`] reference and the GPU pass read the same chart.
    pub fn chart_inputs(
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    ) -> ([f32; 3], [f32; 3], [f32; 3], f32, f32) {
        let fx = (x as f32 + 0.5) / w as f32;
        let fy = (y as f32 + 0.5) / h as f32;
        let raw = [(fx - 0.5) * 2.0, 0.6, (fy - 0.5) * 2.0];
        let l = (raw[0] * raw[0] + raw[1] * raw[1] + raw[2] * raw[2]).sqrt();
        let n = [raw[0] / l, raw[1] / l, raw[2] / l];
        let vv = [0.4f32, 0.6, 0.5];
        let vl = (vv[0] * vv[0] + vv[1] * vv[1] + vv[2] * vv[2]).sqrt();
        let v = [vv[0] / vl, vv[1] / vl, vv[2] / vl];
        let albedo = if fx < 0.5 {
            [0.5, 0.5, 0.5]
        } else {
            [0.8, 0.2, 0.1]
        };
        (n, v, albedo, (x % 2) as f32, fy)
    }

    /// One live chart submit: pass counts from the bake, pixel proof from the
    /// readback, timing as evidence. Units: counts, texels, ms.
    #[derive(Clone, Debug)]
    pub struct IblGpuReceipt {
        /// Frame index from the submitted look frame.
        pub frame_index: u64,
        /// Chart target width in pixels.
        pub width_px: u32,
        /// Chart target height in pixels.
        pub height_px: u32,
        /// wgpu passes recorded (the chart pass).
        pub passes_submitted: u32,
        /// Prefiltered texels bound (`levels * 6 * size * size`).
        pub cube_texels: u64,
        /// BRDF LUT texels bound (`size * size`).
        pub lut_texels: u64,
        /// SH coefficients bound (always 9).
        pub sh_coeffs: u32,
        /// wgpu validation errors observed; green means 0.
        pub validation_errors: u32,
        /// Pixels in the target differing from the clear color.
        pub covered_pixels: u64,
        /// Largest absolute GPU-vs-CPU channel gap over the chart.
        pub max_abs_diff: f32,
        /// Largest relative GPU-vs-CPU channel gap (reference above 0.2).
        pub max_rel_diff: f32,
        /// Wall time for upload + encode + submit + readback in ms.
        pub submit_ms: f64,
    }

    impl IblGpuReceipt {
        /// Posted budget this submit is charged against.
        pub fn budget_ms(&self) -> f64 {
            IBL_SUBMIT_BUDGET_MS
        }
        /// True while the submit stayed inside the posted budget.
        pub fn within_budget(&self) -> bool {
            self.submit_ms <= IBL_SUBMIT_BUDGET_MS
        }
        /// True when the submit observed zero validation errors.
        pub fn is_clean(&self) -> bool {
            self.validation_errors == 0
        }
        /// True while the GPU chart tracks the CPU shade inside the posted
        /// absolute and relative tolerances.
        pub fn within_chart_tolerance(&self) -> bool {
            self.max_abs_diff <= IBL_CHART_ABS_TOL && self.max_rel_diff <= IBL_CHART_REL_TOL
        }
    }

    /// Reusable chart submit state: the chart pipeline, the look bind layout
    /// over the 32-byte look uniform plus the bake bind layout over the
    /// prefilter array, the BRDF LUT and the 144-byte SH uniform, with the
    /// upload textures and the chart target cached. Built once (WIN1 and ED1
    /// each own one); submits on matching shape reuse every texture and bind
    /// group, so steady-state frames create no pipelines, bind groups or
    /// targets (the reuse test proves the counts stay flat) and grow nothing
    /// on the host either: uploads pack into the owned scratch below (the
    /// scratch test proves the capacities stay flat). Per-submit encoder
    /// and readback buffers are driver resources, same as the graph
    /// submitter, and sit outside the reuse guarantee.
    pub struct IblSubmitter {
        look_layout: wgpu::BindGroupLayout,
        bake_layout: wgpu::BindGroupLayout,
        pub(crate) pipeline: wgpu::RenderPipeline,
        pub(crate) look_buf: wgpu::Buffer,
        sh_buf: wgpu::Buffer,
        array_tex: Option<wgpu::Texture>,
        brdf_tex: Option<wgpu::Texture>,
        pub(crate) target: Option<wgpu::Texture>,
        pub(crate) target_view: Option<wgpu::TextureView>,
        pub(crate) look_bind: Option<wgpu::BindGroup>,
        pub(crate) bake_bind: Option<wgpu::BindGroup>,
        backing_ready: bool,
        cached_target: Option<(u32, u32)>,
        /// One packed prefilter layer (`64x64x16` bytes), refilled per
        /// layer upload.
        layer_scratch: Vec<u8>,
        /// Packed BRDF LUT (`64x64x8` bytes), refilled per submit.
        lut_scratch: Vec<u8>,
        /// Packed SH uniform (144 bytes), refilled per submit.
        sh_scratch: [u8; 144],
    }

    impl IblSubmitter {
        /// Compile the chart pipeline and both bind layouts; allocate the
        /// look and SH uniform buffers. No bake or target state yet: upload
        /// textures size on the first submit and the target resizes only
        /// when the look viewport changes.
        pub fn new(device: &wgpu::Device) -> Self {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("rl2-ibl-chart"),
                source: wgpu::ShaderSource::Wgsl(CHART_WGSL.into()),
            });
            let look_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("rl2-ibl-chart-look-layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
            let bake_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("rl2-ibl-chart-bake-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2Array,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: false },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
            let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("rl2-ibl-chart-layout"),
                bind_group_layouts: &[Some(&look_layout), Some(&bake_layout)],
                immediate_size: 0,
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("rl2-ibl-chart-pipeline"),
                layout: Some(&pipe_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba32Float,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
            let look_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rl2-ibl-chart-look"),
                size: crate::graph::LOOK_UNIFORM_BYTES,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let sh_buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rl2-ibl-chart-sh"),
                size: 9 * 16,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            Self {
                look_layout,
                bake_layout,
                pipeline,
                look_buf,
                sh_buf,
                array_tex: None,
                brdf_tex: None,
                target: None,
                target_view: None,
                look_bind: None,
                bake_bind: None,
                backing_ready: false,
                cached_target: None,
                layer_scratch: Vec::new(),
                lut_scratch: Vec::new(),
                sh_scratch: [0u8; 144],
            }
        }

        /// Resident upload plus target texture count (probe for the reuse test).
        pub fn texture_count(&self) -> usize {
            usize::from(self.array_tex.is_some())
                + usize::from(self.brdf_tex.is_some())
                + usize::from(self.target.is_some())
        }

        /// Resident bind-group count (probe for the reuse test).
        pub fn bind_group_count(&self) -> usize {
            usize::from(self.look_bind.is_some()) + usize::from(self.bake_bind.is_some())
        }

        /// Host scratch capacities in bytes (layer, LUT). Probe for the
        /// scratch test: flat across matching-shape submits.
        pub fn scratch_capacities(&self) -> (usize, usize) {
            (self.layer_scratch.capacity(), self.lut_scratch.capacity())
        }

        /// Upload the bake into the cached textures, creating the backing
        /// plus both bind groups on the first call.
        /// `pub(crate)` for the file-scope live submit; tests use the probes.
        pub(crate) fn ensure_backing(
            &mut self,
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            bake: &super::IblBake,
        ) -> Result<(), super::IblError> {
            if !self.backing_ready {
                self.create_backing(device)?;
            }
            self.upload_backing(queue, bake)
        }

        /// Create the prefilter array plus the LUT plus both bind groups.
        /// Runs once; later submits re-upload without recreating.
        fn create_backing(&mut self, device: &wgpu::Device) -> Result<(), super::IblError> {
            let levels = u32::try_from(super::IBL_LEVELS)
                .map_err(|_| super::IblError::Invalid("ibl levels exceed u32".into()))?;
            let array = device.create_texture(&prefilter_array_desc(super::IBL_CUBE_SIZE, levels));
            let lut = device.create_texture(&brdf_lut_desc(super::BRDF_LUT_SIZE));
            let array_view = array.create_view(&wgpu::TextureViewDescriptor {
                label: Some("rl2-ibl-prefilter-array-view"),
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            });
            let lut_view = lut.create_view(&wgpu::TextureViewDescriptor {
                label: Some("rl2-ibl-brdf-view"),
                dimension: Some(wgpu::TextureViewDimension::D2),
                ..Default::default()
            });
            let look_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("rl2-ibl-chart-look-bind"),
                layout: &self.look_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.look_buf.as_entire_binding(),
                }],
            });
            let bake_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("rl2-ibl-chart-bake-bind"),
                layout: &self.bake_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&array_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&lut_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.sh_buf.as_entire_binding(),
                    },
                ],
            });
            // Views are held alive by the bind groups; textures stay
            // resident so later submits re-upload without recreating.
            self.array_tex = Some(array);
            self.brdf_tex = Some(lut);
            self.look_bind = Some(look_bind);
            self.bake_bind = Some(bake_bind);
            self.backing_ready = true;
            Ok(())
        }

        /// Re-upload every layer plus the LUT plus the SH uniform through
        /// owned scratch, so steady-state submits grow nothing on the host.
        fn upload_backing(
            &mut self,
            queue: &wgpu::Queue,
            bake: &super::IblBake,
        ) -> Result<(), super::IblError> {
            let array = self
                .array_tex
                .as_ref()
                .ok_or_else(|| super::IblError::Invalid("ibl chart backing vanished".into()))?;
            let layer_need = layer_byte_len(&bake.cube)?;
            if self.layer_scratch.len() != layer_need {
                self.layer_scratch.resize(layer_need, 0);
            }
            for level in 0..super::IBL_LEVELS {
                for face in 0..super::IBL_FACES {
                    prefilter_layer_bytes_into(&bake.cube, level, face, &mut self.layer_scratch)?;
                    let layer = u32::try_from(level * super::IBL_FACES + face).map_err(|_| {
                        super::IblError::Invalid("ibl chart layer exceeds u32".into())
                    })?;
                    queue.write_texture(
                        wgpu::TexelCopyTextureInfo {
                            texture: array,
                            mip_level: 0,
                            origin: wgpu::Origin3d {
                                x: 0,
                                y: 0,
                                z: layer,
                            },
                            aspect: wgpu::TextureAspect::All,
                        },
                        &self.layer_scratch,
                        wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(super::IBL_CUBE_SIZE * 16),
                            rows_per_image: Some(super::IBL_CUBE_SIZE),
                        },
                        wgpu::Extent3d {
                            width: super::IBL_CUBE_SIZE,
                            height: super::IBL_CUBE_SIZE,
                            depth_or_array_layers: 1,
                        },
                    );
                }
            }
            let lut_tex = self
                .brdf_tex
                .as_ref()
                .ok_or_else(|| super::IblError::Invalid("ibl chart backing vanished".into()))?;
            let lut_need = bake.brdf.data.len() * 8;
            if self.lut_scratch.len() != lut_need {
                self.lut_scratch.resize(lut_need, 0);
            }
            brdf_upload_bytes_into(&bake.brdf, &mut self.lut_scratch);
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: lut_tex,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &self.lut_scratch,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(super::BRDF_LUT_SIZE * 8),
                    rows_per_image: Some(super::BRDF_LUT_SIZE),
                },
                wgpu::Extent3d {
                    width: super::BRDF_LUT_SIZE,
                    height: super::BRDF_LUT_SIZE,
                    depth_or_array_layers: 1,
                },
            );
            sh_uniform_bytes_into(&bake.sh, &mut self.sh_scratch);
            queue.write_buffer(&self.sh_buf, 0, &self.sh_scratch);
            Ok(())
        }

        /// Size the chart target from the look viewport, recreating only on
        /// size change so steady-state frames reuse the target and its view.
        /// `pub(crate)` for the file-scope live submit; tests use the probes.
        pub(crate) fn ensure_target(
            &mut self,
            device: &wgpu::Device,
            w: u32,
            h: u32,
        ) -> Result<(), super::IblError> {
            super::check_chart_target_size(w, h)?;
            if self.cached_target != Some((w, h)) {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("rl2-ibl-chart-target"),
                    size: wgpu::Extent3d {
                        width: w,
                        height: h,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba32Float,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                self.target = Some(texture);
                self.target_view = Some(view);
                self.cached_target = Some((w, h));
            }
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Live chart submit behind `window`: the bake sampled on a real device.
// ---------------------------------------------------------------------------

/// Window submit over the cook (RL2 live): one wgpu chart pass sampling the
/// prefilter array, the BRDF LUT and the SH uniform under the shared
/// [`crate::gtao::LookFrame`] bind, with a float-target readback compared
/// against the CPU [`shade_ibl`] reference. Owner: render lane; consumers
/// WIN1 (match material chart) and the ED1 viewport: both own one
/// [`gpu::IblSubmitter`] and call this per bake with a validated frame.
/// Units: texels, pixels, ms. Budget: [`gpu::IBL_SUBMIT_BUDGET_MS`] at test
/// sizes; the chart target downscales the look viewport through
/// [`chart_target_size`] like the graph slot targets (bounded by
/// [`IBL_CHART_MAX_EDGE`], so a 1600x900 window submits 256x144).
/// Determinism: fixed pixel
/// order, exact cook texels in, same bytes out on every run; pixels are
/// evidence only, never hashed. Errors: [`IblError`] names the bad input,
/// the device call or the validation scope; this path has no silent variant
/// because the caller hands over a live device.
/// Validate the frame plus the bake shape before the submitter sizes
/// anything, so a refusal sizes nothing. Returns the chart target size.
/// Only the standard cook shape submits: the chart shader addresses those
/// extents directly.
#[cfg(all(feature = "v1", feature = "window"))]
fn check_chart_bake(
    frame: &crate::gtao::LookFrame,
    bake: &IblBake,
) -> Result<(u32, u32), IblError> {
    frame.validate().map_err(chart_frame_error)?;
    let (w, h) = chart_target_size(frame.viewport_px[0], frame.viewport_px[1])?;
    if bake.cube.size != IBL_CUBE_SIZE || bake.cube.levels != IBL_LEVELS {
        return Err(IblError::Invalid(format!(
            "ibl chart needs the standard cook cube {}x{}x{IBL_LEVELS}, got {}x{}x{}",
            IBL_CUBE_SIZE, IBL_CUBE_SIZE, IBL_CUBE_SIZE, bake.cube.size, bake.cube.levels
        )));
    }
    let edge = usize::try_from(IBL_CUBE_SIZE)
        .map_err(|_| IblError::Invalid("ibl cube edge exceeds usize".into()))?;
    if bake.cube.faces.len() != IBL_LEVELS * IBL_FACES * edge * edge {
        return Err(IblError::Invalid(
            "ibl chart cube data length mismatches the standard shape".into(),
        ));
    }
    let lut_edge = usize::try_from(BRDF_LUT_SIZE)
        .map_err(|_| IblError::Invalid("ibl lut edge exceeds usize".into()))?;
    if bake.brdf.size != BRDF_LUT_SIZE || bake.brdf.data.len() != lut_edge * lut_edge {
        return Err(IblError::Invalid(format!(
            "ibl chart needs the standard BRDF edge {BRDF_LUT_SIZE}, got {}",
            bake.brdf.size
        )));
    }
    Ok((w, h))
}

/// Refusal text for an invalid look frame. Cold path only.
#[cfg(all(feature = "v1", feature = "window"))]
fn chart_frame_error(e: crate::gtao::LookError) -> IblError {
    IblError::Invalid(format!("ibl look frame: {e}"))
}

/// Refusal text for a validation-scope hit. Cold path only.
#[cfg(all(feature = "v1", feature = "window"))]
fn chart_validation(err: wgpu::Error) -> IblError {
    IblError::Validation(format!("{err:?}"))
}

/// Refusal text for a chart reference failure. Cold path only.
#[cfg(all(feature = "v1", feature = "window"))]
fn chart_reference(e: IblError) -> IblError {
    IblError::Invalid(format!("ibl chart reference: {e}"))
}

/// Record the chart pass and submit once, then hand back the readback
/// buffer plus its padded row stride. Owns the validation scope for the
/// encode, so the caller only maps and scores.
#[cfg(all(feature = "v1", feature = "window"))]
fn run_chart_pass(
    submitter: &gpu::IblSubmitter,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    w: u32,
    h: u32,
) -> Result<(wgpu::Buffer, u32), IblError> {
    let view = submitter
        .target_view
        .as_ref()
        .ok_or_else(|| IblError::Invalid("ibl chart target view vanished".into()))?;
    let look_bind = submitter
        .look_bind
        .as_ref()
        .ok_or_else(|| IblError::Invalid("ibl chart look bind vanished".into()))?;
    let bake_bind = submitter
        .bake_bind
        .as_ref()
        .ok_or_else(|| IblError::Invalid("ibl chart bake bind vanished".into()))?;
    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("rl2-ibl-chart"),
    });
    {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("rl2-ibl-chart-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&submitter.pipeline);
        pass.set_bind_group(0, look_bind, &[]);
        pass.set_bind_group(1, bake_bind, &[]);
        pass.draw(0..3, 0..1);
    }
    let bytes_per_row = w * 16;
    let padded = bytes_per_row.div_ceil(256) * 256;
    let target = submitter
        .target
        .as_ref()
        .ok_or_else(|| IblError::Invalid("ibl chart target vanished".into()))?;
    let readback = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("rl2-ibl-chart-readback"),
        size: u64::from(padded) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: target,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    queue.submit(Some(encoder.finish()));
    if let Some(err) = pollster::block_on(scope.pop()) {
        return Err(chart_validation(err));
    }
    Ok((readback, padded))
}

/// Map the chart readback and wait for it. Cold path helper: the device
/// poll and the channel hop live here, not in the submit body.
#[cfg(all(feature = "v1", feature = "window"))]
fn map_chart_readback(device: &wgpu::Device, readback: &wgpu::Buffer) -> Result<(), IblError> {
    let slice = readback.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    device
        .poll(wgpu::PollType::wait_indefinitely())
        .map_err(|e| IblError::Device(format!("chart poll: {e}")))?;
    rx.recv()
        .map_err(|e| IblError::Device(format!("chart map channel: {e}")))?
        .map_err(|e| IblError::Device(format!("chart map: {e}")))?;
    Ok(())
}

/// Score the mapped chart readback against the CPU [`shade_ibl`]
/// reference: covered pixels plus the worst absolute and relative channel
/// gaps. Pure pixel loop, no allocation; unmaps before returning.
#[cfg(all(feature = "v1", feature = "window"))]
fn score_chart_pixels(
    readback: &wgpu::Buffer,
    padded: u32,
    w: u32,
    h: u32,
    bake: &IblBake,
    escale: f32,
) -> Result<(u64, f32, f32), IblError> {
    let stride = usize::try_from(padded)
        .map_err(|_| IblError::Invalid("ibl chart stride exceeds usize".into()))?;
    let slice = readback.slice(..);
    let mapped = slice
        .get_mapped_range()
        .map_err(|e| IblError::Device(format!("chart mapped range: {e}")))?;
    let mut covered = 0u64;
    let mut max_abs = 0.0f32;
    let mut max_rel = 0.0f32;
    for (y_us, y) in (0..h).enumerate() {
        let row = y_us * stride;
        for (x_us, x) in (0..w).enumerate() {
            let px = row + x_us * 16;
            let g = [
                f32::from_le_bytes([mapped[px], mapped[px + 1], mapped[px + 2], mapped[px + 3]]),
                f32::from_le_bytes([
                    mapped[px + 4],
                    mapped[px + 5],
                    mapped[px + 6],
                    mapped[px + 7],
                ]),
                f32::from_le_bytes([
                    mapped[px + 8],
                    mapped[px + 9],
                    mapped[px + 10],
                    mapped[px + 11],
                ]),
            ];
            if g[0].abs().max(g[1].abs()).max(g[2].abs()) > 1e-3 {
                covered += 1;
            }
            let (n, v, albedo, metallic, roughness) = gpu::chart_inputs(x, y, w, h);
            let e = eval_sh(&bake.sh, n).map_err(chart_reference)?;
            let diffuse = [
                (1.0 - metallic) * albedo[0] * e[0] / PI,
                (1.0 - metallic) * albedo[1] * e[1] / PI,
                (1.0 - metallic) * albedo[2] * e[2] / PI,
            ];
            let ndotv = (n[0] * v[0] + n[1] * v[1] + n[2] * v[2]).clamp(0.0, 1.0);
            let refl = [
                2.0 * ndotv * n[0] - v[0],
                2.0 * ndotv * n[1] - v[1],
                2.0 * ndotv * n[2] - v[2],
            ];
            let pre = sample_cube(&bake.cube, refl, roughness).map_err(chart_reference)?;
            let lut = brdf_sample(&bake.brdf, ndotv, roughness).map_err(chart_reference)?;
            let f0 = [
                0.04 + (albedo[0] - 0.04) * metallic,
                0.04 + (albedo[1] - 0.04) * metallic,
                0.04 + (albedo[2] - 0.04) * metallic,
            ];
            for c in 0..3 {
                let want = (diffuse[c] + pre[c] * (f0[c] * lut[0] + lut[1])) * escale;
                let a = (g[c] - want).abs();
                max_abs = max_abs.max(a);
                if want.abs() > 0.2 {
                    max_rel = max_rel.max(a / want.abs());
                }
            }
        }
    }
    drop(mapped);
    readback.unmap();
    Ok((covered, max_abs, max_rel))
}

#[cfg(all(feature = "v1", feature = "window"))]
impl IblBake {
    /// Record the chart pass on `device` and submit once. Consumes no
    /// `submitter` capacity on matching shape; fails loudly (never silent)
    /// on invalid frames, non-standard bake shapes or a validation hit.
    /// Only the standard cook shape submits (`IBL_CUBE_SIZE` by
    /// `IBL_LEVELS`, `BRDF_LUT_SIZE`): the chart shader addresses those
    /// extents directly, so anything else refuses before any target sizes.
    pub fn submit_gpu(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &crate::gtao::LookFrame,
        submitter: &mut gpu::IblSubmitter,
    ) -> Result<gpu::IblGpuReceipt, IblError> {
        let (w, h) = check_chart_bake(frame, self)?;
        submitter.ensure_backing(device, queue, self)?;
        submitter.ensure_target(device, w, h)?;
        // The chart shader normalizes by the look size, so the bound look
        // carries the chart size while the frame index, exposure and sun
        // stay the caller's: the CPU score reads the same chart geometry.
        let chart_frame = crate::gtao::LookFrame {
            viewport_px: [w, h],
            ..*frame
        };
        queue.write_buffer(
            &submitter.look_buf,
            0,
            &crate::graph::look_frame_uniform_bytes(&chart_frame),
        );
        let started = std::time::Instant::now();
        let (readback, padded) = run_chart_pass(submitter, device, queue, w, h)?;
        map_chart_readback(device, &readback)?;
        let escale = (frame.exposure_ev100 / 10.0).clamp(0.5, 2.0);
        let (covered, max_abs, max_rel) =
            score_chart_pixels(&readback, padded, w, h, self, escale)?;
        Ok(gpu::IblGpuReceipt {
            frame_index: frame.frame_index,
            width_px: w,
            height_px: h,
            passes_submitted: 1,
            cube_texels: u64::try_from(self.cube.texels())
                .map_err(|_| IblError::Invalid("ibl cube texels exceed u64".into()))?,
            lut_texels: u64::try_from(self.brdf.texels())
                .map_err(|_| IblError::Invalid("ibl lut texels exceed u64".into()))?,
            sh_coeffs: u32::try_from(IBL_SH_COEFFS)
                .map_err(|_| IblError::Invalid("ibl sh count exceeds u32".into()))?,
            validation_errors: 0,
            covered_pixels: covered,
            max_abs_diff: max_abs,
            max_rel_diff: max_rel,
            submit_ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests: std-only core always; HDRI bake plus cook parity behind `v1`;
// llvmpipe upload round-trip plus the live chart submit behind `window`.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_texel_dirs_are_unit_and_face_centers_hit_axes() {
        // Face centers reproduce the axis directions, and every texel dir is
        // unit: the bake basis covers the sphere with no degenerate taps.
        let size = 8u32;
        let centers = [
            (0, [1.0, 0.0, 0.0]),
            (1, [-1.0, 0.0, 0.0]),
            (2, [0.0, 1.0, 0.0]),
            (3, [0.0, -1.0, 0.0]),
            (4, [0.0, 0.0, 1.0]),
            (5, [0.0, 0.0, -1.0]),
        ];
        for (face, axis) in centers {
            for y in 0..size {
                for x in 0..size {
                    let d = cube_texel_dir(face, x, y, size);
                    let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                    assert!((l - 1.0).abs() < 1e-6, "texel dir is unit, got {d:?}");
                }
            }
            // Even-sized faces straddle the axis: the 2x2 center quad must
            // agree with the axis on the dominant component.
            let h = size / 2;
            for (x, y) in [(h - 1, h - 1), (h, h - 1), (h - 1, h), (h, h)] {
                let d = cube_texel_dir(face, x, y, size);
                let dom = if axis[0] != 0.0 {
                    d[0] * axis[0]
                } else if axis[1] != 0.0 {
                    d[1] * axis[1]
                } else {
                    d[2] * axis[2]
                };
                assert!(
                    dom > 0.9,
                    "face {face} center quad points along {axis:?}, got {d:?}"
                );
            }
        }
        assert_eq!(PrefilterCube::level_roughness(0).expect("level 0"), 0.0);
        assert_eq!(PrefilterCube::level_roughness(4).expect("level 4"), 1.0);
        assert!(matches!(
            PrefilterCube::level_roughness(5),
            Err(IblError::Invalid(_))
        ));
    }

    #[test]
    fn cube_sample_roundtrips_texel_centers_exactly() {
        // Sampling a texel-center direction reproduces the stored texel: the
        // face/UV inverse is exact, so the forward pass reads what cook wrote.
        let mut cube = PrefilterCube::new(8, IBL_LEVELS);
        for (i, t) in cube.faces.iter_mut().enumerate() {
            let f = (i as f32 * 0.37).fract();
            *t = [f, 1.0 - f, 0.25 + f * 0.5];
        }
        let mut max_diff = 0.0f32;
        for level in [0, 2, 4] {
            let r = PrefilterCube::level_roughness(level).expect("level roughness");
            for face in 0..IBL_FACES {
                for (x, y) in [(0, 0), (7, 0), (0, 7), (7, 7), (3, 4), (5, 2)] {
                    let dir = cube_texel_dir(face, x, y, 8);
                    let got = sample_cube(&cube, dir, r).expect("center samples");
                    let want = cube.texel(level, face, x, y).expect("texel reads");
                    for c in 0..3 {
                        max_diff = max_diff.max((got[c] - want[c]).abs());
                    }
                }
            }
        }
        assert_eq!(max_diff, 0.0, "texel centers round-trip exactly");
    }

    #[test]
    fn cube_sample_rejects_bad_inputs() {
        let cube = PrefilterCube::new(8, IBL_LEVELS);
        assert!(matches!(
            sample_cube(&cube, [f32::NAN, 0.0, 0.0], 0.0),
            Err(IblError::Invalid(_))
        ));
        assert!(matches!(
            sample_cube(&cube, [0.0, 0.0, 0.0], 0.0),
            Err(IblError::Invalid(_))
        ));
        assert!(matches!(
            sample_cube(&cube, [0.0, 1.0, 0.0], 2.0),
            Err(IblError::Invalid(_))
        ));
        assert!(matches!(
            sample_cube(&cube, [0.0, 1.0, 0.0], f32::NAN),
            Err(IblError::Invalid(_))
        ));
        assert!(matches!(cube.texel(0, 0, 8, 0), Err(IblError::Invalid(_))));
        assert!(matches!(cube.texel(0, 6, 0, 0), Err(IblError::Invalid(_))));
        assert!(matches!(cube.texel(9, 0, 0, 0), Err(IblError::Invalid(_))));
    }

    #[test]
    fn brdf_lut_is_deterministic_with_sane_corners() {
        // Same inputs twice give bit-identical LUTs; corners pin the
        // split-sum shape: smooth dielectric at normal incidence reflects
        // ~fully (scale near 1, bias near 0), grazing angles bias upward,
        // and rough surfaces spread energy out of the scale term.
        let (a, b) = (
            brdf_lut_build(16).expect("lut builds"),
            brdf_lut_build(16).expect("lut builds"),
        );
        assert_eq!(a, b, "brdf bake is bit-identical twice");
        assert_eq!(a.to_bytes(), b.to_bytes(), "lut bytes are stable");
        let smooth_normal = a.texel(15, 0).expect("corner reads");
        assert!(
            (smooth_normal[0] - 1.0).abs() < 0.05 && smooth_normal[1].abs() < 0.05,
            "smooth normal-incidence keeps energy in scale, got {smooth_normal:?}"
        );
        let grazing = a.texel(0, 0).expect("grazing reads");
        assert!(
            grazing[1] > 0.3,
            "grazing angles carry fresnel bias, got {grazing:?}"
        );
        let rough = a.texel(15, 15).expect("rough reads");
        assert!(
            rough[0] < smooth_normal[0],
            "roughness spreads energy out of scale: smooth {smooth_normal:?}, rough {rough:?}"
        );
        for t in &a.data {
            assert!(
                t.iter().all(|c| c.is_finite() && *c >= -0.01 && *c <= 1.01),
                "lut stays in range, got {t:?}"
            );
        }
        assert!(matches!(brdf_lut_build(0), Err(IblError::Invalid(_))));
        assert!(matches!(
            brdf_sample(&a, 2.0, 0.5),
            Err(IblError::Invalid(_))
        ));
        assert!(matches!(
            brdf_sample(&a, 0.5, f32::NAN),
            Err(IblError::Invalid(_))
        ));
    }

    #[test]
    fn brdf_sample_matches_direct_integration() {
        // Bilinear sampling reproduces the integral it was baked from: cell
        // centers match to 1e-3, off-grid points to 1e-2 (the LUT is smooth).
        let lut = brdf_lut_build(32).expect("lut builds");
        let mut max_center = 0.0f32;
        for (x, y) in [(0, 0), (31, 0), (0, 31), (31, 31), (16, 16), (7, 23)] {
            let ndotv = (x as f32 + 0.5) / 32.0;
            let r = (y as f32 + 0.5) / 32.0;
            let direct = brdf_integrate(ndotv, r).expect("direct integrates");
            let got = brdf_sample(&lut, ndotv, r).expect("center samples");
            max_center = max_center.max((got[0] - direct[0]).abs());
            max_center = max_center.max((got[1] - direct[1]).abs());
        }
        assert!(
            max_center <= 1e-3,
            "cell centers match within 1e-3, got {max_center}"
        );
        let mut max_off = 0.0f32;
        for (ndotv, r) in [(0.33, 0.17), (0.9, 0.62), (0.05, 0.95), (0.71, 0.29)] {
            let direct = brdf_integrate(ndotv, r).expect("direct integrates");
            let got = brdf_sample(&lut, ndotv, r).expect("point samples");
            max_off = max_off.max((got[0] - direct[0]).abs());
            max_off = max_off.max((got[1] - direct[1]).abs());
        }
        assert!(max_off <= 1e-2, "off-grid within 1e-2, got {max_off}");
    }

    #[test]
    fn sh_eval_rejects_bad_normals() {
        let sh = IrradianceSh::zero();
        let n = [0.0, 1.0, 0.0];
        assert_eq!(
            eval_sh(&sh, n),
            Ok([0.0, 0.0, 0.0]),
            "zero coefficients give zero irradiance"
        );
        assert!(matches!(
            eval_sh(&sh, [0.0, 0.0, 0.0]),
            Err(IblError::Invalid(_))
        ));
        assert!(matches!(
            eval_sh(&sh, [f32::INFINITY, 0.0, 0.0]),
            Err(IblError::Invalid(_))
        ));
        assert!(matches!(
            eval_sh(&sh, [1.0, 1.0, f32::NAN]),
            Err(IblError::Invalid(_))
        ));
    }

    /// Studio SH fixture for the bind lock: varied magnitudes per band and
    /// per channel, so packing swaps or truncations move the bytes.
    fn bind_lock_sh() -> IrradianceSh {
        IrradianceSh {
            coeffs: [
                [0.50, 0.40, 0.30],
                [0.10, -0.20, 0.05],
                [-0.15, 0.25, 0.35],
                [0.05, 0.10, -0.10],
                [0.20, 0.05, 0.15],
                [-0.05, 0.30, -0.25],
                [0.45, -0.35, 0.20],
                [0.12, 0.22, -0.02],
                [-0.28, 0.18, 0.08],
            ],
        }
    }

    #[test]
    fn forward_ibl_chunk_is_pinned_for_the_forward_bind() {
        // The bound chunk is exactly the reviewed bytes: the hash pins drift,
        // the shape asserts pin silent rebinding (a stub chunk would miss the
        // entries, a renumbered bind group would miss the bindings).
        assert_eq!(
            forward_ibl_chunk_probe(),
            FORWARD_IBL_CHUNK_HASH,
            "forward chunk drifted without updating the pin"
        );
        assert!(!FORWARD_IBL_WGSL.is_empty(), "forward chunk ships");
        for binding in [
            "@group(1) @binding(0)",
            "@group(1) @binding(1)",
            "@group(1) @binding(2)",
            "@group(1) @binding(3)",
        ] {
            assert!(
                FORWARD_IBL_WGSL.contains(binding),
                "forward bind carries {binding}"
            );
        }
        for entry in [
            "ibl_sample",
            "ibl_irradiance",
            "ibl_sh_basis",
            "array<vec4<f32>, 9>",
            "textureSampleLevel",
        ] {
            assert!(
                FORWARD_IBL_WGSL.contains(entry),
                "forward chunk keeps {entry}"
            );
        }
        // The basis literals the GPU dots are named here; the oracle test
        // below pins their values against pi, so a flipped sign fails there.
        for literal in ["0.282095", "0.488603", "1.092548", "0.315392", "0.546274"] {
            assert!(
                FORWARD_IBL_WGSL.contains(literal),
                "forward chunk carries basis {literal}"
            );
        }
    }

    #[test]
    fn forward_ibl_bind_image_carries_hash_and_sh_bytes() {
        // Compiled forward bind: hash head plus SH tail, bit-exact pack into
        // caller-owned storage (steady-state repacks grow nothing).
        let sh = bind_lock_sh();
        let mut image = [0u8; FORWARD_IBL_BIND_BYTES];
        forward_ibl_bind_image(&sh, &mut image);
        let mut head = [0u8; 8];
        head.copy_from_slice(&image[0..8]);
        assert_eq!(
            head,
            FORWARD_IBL_CHUNK_HASH.to_le_bytes(),
            "bind head is the pinned chunk hash"
        );
        let mut packed = [0u8; FORWARD_IBL_SH_UNIFORM_BYTES];
        sh_uniform_bytes_into(&sh, &mut packed);
        assert_eq!(
            &image[8..FORWARD_IBL_BIND_BYTES],
            &packed[..],
            "bind tail is the SH uniform image"
        );
        // Layout: RGB little-endian floats with zero w pads; decode
        // round-trips the coefficients exactly, so field swaps fail here.
        for (i, c) in sh.coeffs.iter().enumerate() {
            let base = 8 + i * 16;
            for (lane, v) in c.iter().enumerate() {
                let bytes: [u8; 4] = image[base + lane * 4..base + lane * 4 + 4]
                    .try_into()
                    .expect("uniform lanes are 4 bytes wide");
                assert_eq!(
                    f32::from_le_bytes(bytes),
                    *v,
                    "coefficient {i} lane {lane} round-trips"
                );
            }
            assert_eq!(
                &image[base + 12..base + 16],
                &[0, 0, 0, 0],
                "coefficient {i} w pad stays zero"
            );
        }
        // Deterministic twice, sensitive once, zero baseline last.
        let mut again = [0u8; FORWARD_IBL_BIND_BYTES];
        forward_ibl_bind_image(&sh, &mut again);
        assert_eq!(image, again, "same SH binds the same bytes twice");
        let mut zero = [0u8; FORWARD_IBL_BIND_BYTES];
        forward_ibl_bind_image(&IrradianceSh::zero(), &mut zero);
        let mut zero_head = [0u8; 8];
        zero_head.copy_from_slice(&zero[0..8]);
        assert_eq!(
            zero_head,
            FORWARD_IBL_CHUNK_HASH.to_le_bytes(),
            "zero SH keeps the chunk hash head"
        );
        assert!(
            zero[8..].iter().all(|b| *b == 0),
            "zero SH packs a zero tail"
        );
        assert_ne!(image, zero, "distinct SH binds distinctly");
    }

    /// L2 real-SH basis from pi only (independent of the f32 literals in
    /// [`crate::gi::sh_basis`] and the WGSL chunk): order L00, L1y, L10,
    /// L11, L2-2, L2-1, L20, L21, L22.
    fn basis_oracle(x: f64, y: f64, z: f64) -> [f64; 9] {
        use std::f64::consts::PI;
        let s1 = 0.5 * (1.0 / PI).sqrt();
        let s3 = (3.0 / (4.0 * PI)).sqrt();
        let s15 = 0.5 * (15.0 / PI).sqrt();
        let s5 = 0.25 * (5.0 / PI).sqrt();
        let s15q = 0.25 * (15.0 / PI).sqrt();
        [
            s1,
            -s3 * y,
            s3 * z,
            -s3 * x,
            s15 * x * y,
            -s15 * y * z,
            s5 * (3.0 * z * z - 1.0),
            -s15 * x * z,
            s15q * (x * x - y * y),
        ]
    }

    #[test]
    fn forward_sh_matches_independent_oracle_within_gpu_tol() {
        // Headless parity leg for the forward bind: the CPU SH the bind
        // carries tracks an independent f64 oracle within the posted 2e-5
        // GPU-fusing budget (reference-GPU chart observation: shade abs
        // 0.00002), so a flipped basis sign or a dropped band fails here
        // before any device.
        const TOL: f32 = 0.00002;
        let sh = bind_lock_sh();
        let normals: [[f32; 3]; 7] = [
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
            [1.0, 0.0, 0.0],
            [0.3, -0.5, 0.8],
            [0.4, 0.35, 0.8],
            [-1.0, 0.2, 0.1],
            [0.2, 0.9, -0.4],
        ];
        let mut max_abs = 0.0f32;
        for n in normals {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            let n = [n[0] / len, n[1] / len, n[2] / len];
            let got = eval_sh(&sh, n).expect("fixture SH evals");
            // WGSL-order f32 emulation: sequential `e += sh[i] * b[i]` per
            // channel, clamped like `ibl_irradiance`. `eval_sh` renormalizes
            // once more inside `sh_irradiance`, so this trails by float
            // rounding (~1e-7); the 2e-5 oracle gate below is the budget.
            let basis = crate::gi::sh_basis(n);
            let mut emu = [0.0f32; 3];
            for (coeff, b) in sh.coeffs.iter().zip(basis.iter()) {
                for (slot, v) in emu.iter_mut().zip(coeff.iter()) {
                    *slot += *v * *b;
                }
            }
            emu = [emu[0].max(0.0), emu[1].max(0.0), emu[2].max(0.0)];
            let mut emu_abs = 0.0f32;
            for (e, g) in emu.iter().zip(got.iter()) {
                emu_abs = emu_abs.max((*e - *g).abs());
            }
            assert!(
                emu_abs <= 1e-6,
                "WGSL-order accumulation matches eval at {n:?}, worst {emu_abs}"
            );
            let (x, y, z) = (f64::from(n[0]), f64::from(n[1]), f64::from(n[2]));
            let oracle = basis_oracle(x, y, z);
            let mut exact = [0.0f64; 3];
            for (coeff, o) in sh.coeffs.iter().zip(oracle.iter()) {
                for (slot, v) in exact.iter_mut().zip(coeff.iter()) {
                    *slot += f64::from(*v) * *o;
                }
            }
            for (g, e) in got.iter().zip(exact.iter()) {
                let e = e.max(0.0);
                max_abs = max_abs.max((*g - e as f32).abs());
            }
        }
        assert!(
            max_abs <= TOL,
            "forward SH within abs {TOL} of the oracle, worst {max_abs}"
        );
        eprintln!("GX12 forward SH parity: max abs {max_abs:.7} within {TOL}");
    }

    #[test]
    fn ibl_bake_hands_bake_resources_to_forward() {
        // Unwired is FAIL: the compiled bake -> forward graph hands every
        // bake resource to forward with a Transition barrier, forward reads
        // all three, bake runs first, and the receipt keeps both passes'
        // draws.
        let compiled = compile_ibl_for_forward().expect("ibl wiring compiles");
        assert!(
            compiled.order_names_match(&[IBL_BAKE_PASS, IBL_FORWARD_PASS]),
            "bake runs before forward"
        );
        let (bake_id, forward_id) = (compiled.order()[0], compiled.order()[1]);
        assert_eq!(
            compiled.pass(bake_id).expect("dense ids").name.as_str(),
            IBL_BAKE_PASS
        );
        assert_eq!(
            compiled.pass(forward_id).expect("dense ids").name.as_str(),
            IBL_FORWARD_PASS
        );
        for resource in FORWARD_IBL_INPUTS {
            assert!(
                compiled.barriers().iter().any(|b| {
                    b.resource == resource
                        && b.kind == crate::graph::BarrierKind::Transition
                        && b.from == bake_id
                        && b.to == forward_id
                }),
                "{resource} hands over to forward"
            );
        }
        let forward = compiled.pass(forward_id).expect("forward reads");
        for resource in FORWARD_IBL_INPUTS {
            assert!(
                forward.inputs.iter().any(|i| i == resource),
                "forward reads {resource}: {:?}",
                forward.inputs
            );
        }
        let mut compiled = compiled;
        let receipt = compiled.execute(&crate::graph::FrameInfo {
            frame_index: 7,
            width_px: 16,
            height_px: 16,
        });
        assert_eq!(receipt.draws, 3, "both passes record draws");
        assert_eq!(receipt.triangles, 10, "both passes record triangles");
        assert_eq!(receipt.per_pass_cpu_ms.len(), 2);
    }

    #[test]
    fn chart_target_downscales_window_viewport_within_edge() {
        // The release window (1600x900) must submit a chart inside the edge
        // instead of failing the frame: uniform downscale, aspect preserved.
        assert_eq!(
            chart_target_size(32, 32).expect("test chart fits"),
            (32, 32),
            "fitting viewports pass through untouched"
        );
        let window = chart_target_size(1600, 900).expect("window downscales");
        assert_eq!(window, (256, 144), "1600x900 downscales to 256x144");
        assert!(
            window.0 <= IBL_CHART_MAX_EDGE && window.1 <= IBL_CHART_MAX_EDGE,
            "chart target stays within the edge, got {window:?}"
        );
        let wide = u64::from(window.0) * u64::from(900u32);
        let high = u64::from(window.1) * u64::from(1600u32);
        assert_eq!(wide, high, "downscale preserves aspect, got {window:?}");
        assert_eq!(
            chart_target_size(512, 32).expect("wide bar downscales"),
            (256, 16),
            "long edge lands exactly on the cap"
        );
        assert_eq!(
            chart_target_size(900, 1600).expect("portrait downscales"),
            (144, 256),
            "portrait keeps the tall edge on the cap"
        );
        assert_eq!(
            chart_target_size(2048, 2048).expect("full look downscales"),
            (256, 256),
            "largest look viewport still fits"
        );
        assert_eq!(
            chart_target_size(1600, 900).expect("deterministic"),
            window,
            "same viewport sizes the same target twice"
        );
        assert!(
            matches!(chart_target_size(0, 32), Err(IblError::Invalid(_))),
            "zero width refuses loudly"
        );
        assert!(
            matches!(chart_target_size(1600, 0), Err(IblError::Invalid(_))),
            "zero height refuses loudly"
        );
    }

    #[test]
    fn chart_target_guard_refuses_over_edge_directly() {
        // Direct target sizes over the edge never reach a texture: the guard
        // refuses loudly with the cap named, so the presenter scope stays
        // clean.
        assert!(check_chart_target_size(32, 32).is_ok());
        assert!(check_chart_target_size(256, 144).is_ok());
        match check_chart_target_size(512, 32) {
            Err(IblError::Invalid(detail)) => {
                assert!(detail.contains("exceeds"), "names the cap: {detail}");
            }
            other => panic!("512x32 target must refuse, got {other:?}"),
        }
        match check_chart_target_size(1600, 900) {
            Err(IblError::Invalid(detail)) => {
                assert!(detail.contains("exceeds"), "names the cap: {detail}");
            }
            other => panic!("1600x900 target must refuse, got {other:?}"),
        }
        assert!(
            matches!(check_chart_target_size(0, 32), Err(IblError::Invalid(_))),
            "zero target refuses loudly"
        );
        assert!(
            matches!(check_chart_target_size(256, 257), Err(IblError::Invalid(_))),
            "one pixel over the edge refuses loudly"
        );
    }

    #[test]
    fn forward_bind_assert_accepts_chunk_and_rejects_plain() {
        // The wiring gate accepts a forward shader that inlines the chunk
        // and refuses one that only ships it: a missing marker names itself
        // instead of passing silently.
        let stitched = FORWARD_IBL_MARKERS.join("\n");
        assert!(
            assert_forward_wgsl_binds_ibl(&stitched).is_ok(),
            "all five markers together bind the bake"
        );
        match assert_forward_wgsl_binds_ibl("fn fs() {}") {
            Err(IblError::Invalid(detail)) => {
                assert!(
                    detail.contains("ibl_prefilter"),
                    "names the first missing marker: {detail}"
                );
            }
            other => panic!("plain forward must refuse, got {other:?}"),
        }
        match assert_forward_wgsl_binds_ibl("") {
            Err(IblError::Invalid(detail)) => {
                assert!(detail.contains("inline"), "points at the chunk: {detail}");
            }
            other => panic!("empty forward must refuse, got {other:?}"),
        }
        let partial = stitched.replace("ibl_sample", "ibl_missing");
        match assert_forward_wgsl_binds_ibl(&partial) {
            Err(IblError::Invalid(detail)) => {
                assert!(
                    detail.contains("ibl_sample"),
                    "names the dropped entry point: {detail}"
                );
            }
            other => panic!("partial forward must refuse, got {other:?}"),
        }
    }

    #[test]
    fn bake_counts_quote_texels() {
        // The receipt quotes honest texel counts for the frame report; the
        // live submit path is covered by the chart tests behind `window`.
        let bake_like = PrefilterCube::new(8, IBL_LEVELS);
        assert_eq!(bake_like.texels(), IBL_LEVELS * IBL_FACES * 64);
        let lut = BrdfLut::new(8);
        assert_eq!(lut.texels(), 64);
        let sh = IrradianceSh::zero();
        assert_eq!(sh.to_bytes().len(), IBL_SH_COEFFS * 12);
        assert_eq!(bake_like.to_bytes().len(), bake_like.texels() * 12);
        assert_eq!(lut.to_bytes().len(), 64 * 8);
    }

    // ------------------------------------------------------------------
    // HDRI cook behind `v1`: deterministic bytes, exact R2 prefilter
    // parity, SH-vs-cosine-integral parity, forward shade sanity.
    // ------------------------------------------------------------------

    #[cfg(feature = "v1")]
    const HDRI_BYTES: &[u8] = include_bytes!("../../../content/incoming/studio_small_09_1k.hdr");

    #[cfg(feature = "v1")]
    #[test]
    fn studio_hdri_bakes_deterministically() {
        // Same complete HDRI bytes twice give bit-identical bakes and bytes:
        // the cook artifact is deterministic.
        let (a, ms_a) = bake(HDRI_BYTES).expect("studio HDRI bakes");
        let (b, ms_b) = bake(HDRI_BYTES).expect("studio HDRI bakes again");
        assert_eq!(a, b, "bake is bit-identical twice");
        assert_eq!(a.to_bytes(), b.to_bytes(), "cook bytes are stable");
        assert!(ms_a >= 0.0 && ms_b >= 0.0, "bake records pass ms");
        let receipt = a.receipt(ms_a);
        assert_eq!(
            receipt.cube_texels,
            IBL_LEVELS as u64 * IBL_FACES as u64 * IBL_CUBE_SIZE as u64 * IBL_CUBE_SIZE as u64
        );
        assert_eq!(
            receipt.lut_texels,
            BRDF_LUT_SIZE as u64 * BRDF_LUT_SIZE as u64
        );
        assert_eq!(receipt.sh_coeffs, IBL_SH_COEFFS as u32);
        assert_eq!(
            a.to_bytes().len(),
            receipt.cube_texels as usize * 12
                + IBL_SH_COEFFS * 12
                + receipt.lut_texels as usize * 8
        );
        assert!(
            a.source_average.iter().all(|c| c.is_finite() && *c >= 0.0),
            "source average is finite/nonnegative"
        );
        #[cfg(not(feature = "window"))]
        {
            // Headless build: no device in hand, so the submit seam refuses
            // loudly after validating the shared look frame.
            let frame = crate::gtao::LookFrame {
                frame_index: 3,
                viewport_px: [32, 32],
                exposure_ev100: 10.0,
                sun_dir: [0.3, 0.8, 0.5],
            };
            assert!(matches!(a.submit_gpu(&frame), Err(IblError::Invalid(_))));
            let bad = crate::gtao::LookFrame {
                viewport_px: [0, 32],
                ..frame
            };
            assert!(matches!(a.submit_gpu(&bad), Err(IblError::Invalid(_))));
        }
        eprintln!(
            "RL2 cook: cube {} texels, lut {} texels, sh {} coeffs, bake {ms_a:.1}ms",
            receipt.cube_texels, receipt.lut_texels, receipt.sh_coeffs
        );
    }

    #[cfg(feature = "v1")]
    #[test]
    fn prefilter_parity_with_r2_is_exact() {
        // Every baked texel equals the R2 `prefilter` at the same direction
        // and roughness: parity is exact (0.0) by construction, and level 0
        // equals the direct R2 `sample`.
        use crate::v1_ibl_shadow as r2;
        let env = r2::load_hdri(HDRI_BYTES).expect("fixture decodes");
        let (bake, _) = bake(HDRI_BYTES).expect("studio HDRI bakes");
        let mut max_diff = 0.0f32;
        let mut worst = String::new();
        for level in 0..IBL_LEVELS {
            let r = PrefilterCube::level_roughness(level).expect("level roughness");
            for face in 0..IBL_FACES {
                for y in (0..IBL_CUBE_SIZE).step_by(7) {
                    for x in (0..IBL_CUBE_SIZE).step_by(7) {
                        let dir = cube_texel_dir(face, x, y, IBL_CUBE_SIZE);
                        let want = r2::prefilter(&env, dir, r).expect("R2 prefilters");
                        let got = bake.cube.texel(level, face, x, y).expect("texel reads");
                        for c in 0..3 {
                            let d = (got[c] - want[c]).abs();
                            if d > max_diff {
                                max_diff = d;
                                worst = format!("level {level} face {face} ({x},{y}) ch {c}");
                            }
                        }
                        if r == 0.0 {
                            let direct = r2::sample(&env, dir).expect("R2 samples");
                            for c in 0..3 {
                                // Level 0 routes through `prefilter`, which
                                // renormalizes the already-unit cube dir: 1
                                // ulp of double normalization, not a kernel
                                // difference. The kernel path above is exact.
                                assert!(
                                    (got[c] - direct[c]).abs() <= 1e-6,
                                    "level 0 is the direct sample at {face} ({x},{y})"
                                );
                            }
                        }
                    }
                }
            }
        }
        assert_eq!(max_diff, 0.0, "prefilter parity exact, worst {worst}");
    }

    #[cfg(feature = "v1")]
    #[test]
    fn sh_reproduces_uniform_environment() {
        // Analytic anchor: a uniform L=1 environment has irradiance PI in
        // every direction. This pins the solid-angle weights and the cosine
        // lobe factors independent of any HDRI content.
        let env = crate::v1_ibl_shadow::EnvMap {
            width: 64,
            height: 32,
            pixels: vec![[1.0, 1.0, 1.0]; 64 * 32],
        };
        let sh = project_sh(&env);
        for n in [
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.3, -0.5, 0.8],
        ] {
            let got = eval_sh(&sh, n).expect("sh evals");
            for c in 0..3 {
                assert!(
                    (got[c] - PI).abs() <= 0.01,
                    "uniform env gives PI at {n:?}, got {got:?}"
                );
            }
        }
    }

    #[cfg(feature = "v1")]
    #[test]
    fn sh_matches_cosine_integral_within_posted_tolerance() {
        // Brute-force diffuse reference over a fixed Fibonacci sphere:
        // `E(n) = 4PI/N * sum L(w) max(dot(n,w), 0)` from direct R2 samples.
        // L2 SH must track it within abs 0.50 / rel 15% (posted tolerance).
        // The uniform-environment test above pins weights and lobe factors
        // exactly, so the residual here is pure L2 truncation on the peaky
        // studio HDRI (observed: abs 0.425, rel 12.9% on the -X probe facing
        // the softbox edge); an axis flip or missing lobe would miss by 2x.
        use crate::v1_ibl_shadow as r2;
        let env = r2::load_hdri(HDRI_BYTES).expect("fixture decodes");
        let (bake, _) = bake(HDRI_BYTES).expect("studio HDRI bakes");
        const N: usize = 4096;
        let golden = std::f32::consts::PI * (1.0 + (5.0f32).sqrt());
        let mut dirs = Vec::with_capacity(N);
        let mut rad = Vec::with_capacity(N);
        for i in 0..N {
            let z = 1.0 - 2.0 * (i as f32 + 0.5) / N as f32;
            let rr = (1.0 - z * z).max(0.0).sqrt();
            let theta = golden * i as f32;
            let d = [rr * theta.cos(), rr * theta.sin(), z];
            // R2 convention is +Y up; the Fibonacci frame is +Z up, so swap
            // axes into the env frame before sampling.
            let e = [d[0], d[2], d[1]];
            dirs.push(e);
            rad.push(r2::sample(&env, e).expect("reference samples"));
        }
        let weight = 4.0 * std::f32::consts::PI / N as f32;
        let normals: [[f32; 3]; 7] = [
            [0.0, 1.0, 0.0],
            [0.0, -1.0, 0.0],
            [1.0, 0.0, 0.0],
            [-1.0, 0.2, 0.1],
            [0.0, 0.0, 1.0],
            [0.3, -0.5, 0.8],
            [0.4, 0.35, 0.8],
        ];
        let mut max_abs = 0.0f32;
        let mut max_rel = 0.0f32;
        for n in normals {
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            let n = [n[0] / l, n[1] / l, n[2] / l];
            let mut e_ref = [0.0f32; 3];
            for (d, p) in dirs.iter().zip(rad.iter()) {
                let cos = (n[0] * d[0] + n[1] * d[1] + n[2] * d[2]).max(0.0);
                e_ref[0] += p[0] * cos;
                e_ref[1] += p[1] * cos;
                e_ref[2] += p[2] * cos;
            }
            e_ref = [e_ref[0] * weight, e_ref[1] * weight, e_ref[2] * weight];
            let got = eval_sh(&bake.sh, n).expect("sh evals");
            let mut n_abs = 0.0f32;
            for c in 0..3 {
                let a = (got[c] - e_ref[c]).abs();
                n_abs = n_abs.max(a);
                max_abs = max_abs.max(a);
                if e_ref[c] > 0.5 {
                    max_rel = max_rel.max(a / e_ref[c]);
                }
            }
            eprintln!("RL2 SH probe n={n:?} ref={e_ref:?} got={got:?} abs={n_abs:.4}");
        }
        assert!(
            max_abs <= 0.50,
            "SH irradiance within abs 0.50 of the cosine integral, max {max_abs}"
        );
        assert!(
            max_rel <= 0.15,
            "SH irradiance within rel 15% of the cosine integral, max {max_rel}"
        );
        eprintln!(
            "RL2 SH parity: max abs {max_abs:.4}, max rel {:.4}%",
            max_rel * 100.0
        );
    }

    #[cfg(feature = "v1")]
    #[test]
    fn forward_shade_is_sane_and_material_aware() {
        // The forward pass combines both bakes: finite/nonnegative output,
        // metals drop the diffuse term, roughness moves the specular term,
        // and a black albedo still reflects the environment through F0.
        let (bake, _) = bake(HDRI_BYTES).expect("studio HDRI bakes");
        let n = [0.0, 1.0, 0.0];
        let v = [0.4, 0.6, 0.5];
        let diel = shade_ibl(&bake, n, v, [0.5, 0.5, 0.5], 0.0, 0.5).expect("dielectric shades");
        let metal = shade_ibl(&bake, n, v, [0.5, 0.5, 0.5], 1.0, 0.5).expect("metal shades");
        let sharp = shade_ibl(&bake, n, v, [0.5, 0.5, 0.5], 0.0, 0.0).expect("sharp shades");
        let black = shade_ibl(&bake, n, v, [0.0, 0.0, 0.0], 0.0, 0.2).expect("black shades");
        for (name, s) in [
            ("diel", diel),
            ("metal", metal),
            ("sharp", sharp),
            ("black", black),
        ] {
            assert!(
                s.iter().all(|c| c.is_finite() && *c >= 0.0),
                "{name} shade is finite/nonnegative, got {s:?}"
            );
        }
        let lum = crate::v1_ibl_shadow::luminance;
        assert!(
            lum(diel) > lum(metal),
            "metal drops diffuse: diel {diel:?} vs metal {metal:?}"
        );
        assert!(
            (lum(sharp) - lum(diel)).abs() > 1e-3,
            "roughness moves specular: sharp {sharp:?} vs mid {diel:?}"
        );
        assert!(
            lum(black) > 0.0,
            "black albedo still reflects through F0 0.04, got {black:?}"
        );
        assert!(matches!(
            shade_ibl(&bake, [0.0, 0.0, 0.0], v, [0.5, 0.5, 0.5], 0.0, 0.5),
            Err(IblError::Invalid(_))
        ));
        assert!(matches!(
            shade_ibl(&bake, n, v, [2.0, 0.5, 0.5], 0.0, 0.5),
            Err(IblError::Invalid(_))
        ));
        assert!(matches!(
            shade_ibl(&bake, n, v, [0.5, 0.5, 0.5], 0.0, 1.5),
            Err(IblError::Invalid(_))
        ));
        // Same complete inputs twice give identical shades.
        let again = shade_ibl(&bake, n, v, [0.5, 0.5, 0.5], 0.0, 0.5).expect("shades again");
        assert_eq!(diel, again, "forward shade is deterministic");
    }

    #[cfg(feature = "v1")]
    #[test]
    fn missing_hdri_refuses_never_silent() {
        // No bytes refuse with MissingHdri (not an empty bake); undecodable
        // bytes wrap the R2 decode error; oversize bytes wrap TooLarge.
        assert_eq!(bake(&[]), Err(IblError::MissingHdri));
        assert!(matches!(
            bake(b"definitely not an image at all ........"),
            Err(IblError::Hdri(_))
        ));
        let huge = vec![0u8; crate::v1_ibl_shadow::MAX_HDRI_BYTES + 1];
        assert!(matches!(bake(&huge), Err(IblError::Hdri(_))));
    }

    // ------------------------------------------------------------------
    // GPU descriptors behind `window`: pure format/extent checks, a real
    // cook-bytes upload round-trip, and the live chart submit that skips
    // without an adapter (same skip rule as the RA1 presenter tests).
    // ------------------------------------------------------------------

    #[cfg(feature = "window")]
    #[test]
    fn ibl_descs_are_sampler_readable() {
        let cube = gpu::prefilter_cube_desc(IBL_CUBE_SIZE, IBL_LEVELS as u32);
        assert_eq!(cube.format, wgpu::TextureFormat::Rgba32Float);
        assert_eq!(
            (cube.size.width, cube.size.height),
            (IBL_CUBE_SIZE, IBL_CUBE_SIZE)
        );
        assert_eq!(cube.size.depth_or_array_layers, IBL_FACES as u32);
        assert_eq!(cube.mip_level_count, IBL_LEVELS as u32);
        assert!(cube.usage.contains(wgpu::TextureUsages::TEXTURE_BINDING));
        assert!(cube.usage.contains(wgpu::TextureUsages::COPY_DST));
        assert!(cube.usage.contains(wgpu::TextureUsages::COPY_SRC));
        let array = gpu::prefilter_array_desc(IBL_CUBE_SIZE, IBL_LEVELS as u32);
        assert_eq!(array.format, wgpu::TextureFormat::Rgba32Float);
        assert_eq!(
            array.size.depth_or_array_layers,
            IBL_LEVELS as u32 * IBL_FACES as u32
        );
        assert_eq!(array.mip_level_count, 1);
        assert!(array.usage.contains(wgpu::TextureUsages::TEXTURE_BINDING));
        assert!(array.usage.contains(wgpu::TextureUsages::COPY_DST));
        let lut = gpu::brdf_lut_desc(BRDF_LUT_SIZE);
        assert_eq!(lut.format, wgpu::TextureFormat::Rg32Float);
        assert_eq!(
            (lut.size.width, lut.size.height),
            (BRDF_LUT_SIZE, BRDF_LUT_SIZE)
        );
        assert!(lut.usage.contains(wgpu::TextureUsages::TEXTURE_BINDING));
        assert!(lut.usage.contains(wgpu::TextureUsages::COPY_SRC));
        assert!(!gpu::FORWARD_IBL_WGSL.is_empty(), "forward chunk ships");
        for marker in FORWARD_IBL_MARKERS {
            assert!(
                gpu::FORWARD_IBL_WGSL.contains(marker),
                "forward chunk carries marker '{marker}'"
            );
        }
        assert!(
            gpu::FORWARD_IBL_WGSL.contains("ibl_sh[8]"),
            "forward chunk dots all 9 SH coefficients"
        );
        assert!(
            assert_forward_wgsl_binds_ibl(gpu::FORWARD_IBL_WGSL).is_ok(),
            "the shipped chunk passes its own wiring gate"
        );
        assert!(!gpu::CHART_WGSL.is_empty(), "chart pass ships");
        assert!(
            gpu::CHART_WGSL.contains("textureLoad"),
            "chart pass fetches exact cook texels"
        );
        let sh_bytes = gpu::sh_uniform_bytes(&IrradianceSh::zero());
        assert_eq!(sh_bytes.len(), 9 * 16, "SH uniform is 9 vec4s");
        assert!(
            sh_bytes.iter().all(|b| *b == 0),
            "zero coefficients pack to zero bytes"
        );
        // Chart inputs pin the CPU/GPU contract: columns alternate metal,
        // halves swap albedo, rows sweep roughness, normals stay unit.
        let (n, _, albedo, metal, rough) = gpu::chart_inputs(0, 0, 32, 32);
        assert_eq!((metal, albedo), (0.0, [0.5, 0.5, 0.5]));
        let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        assert!((l - 1.0).abs() < 1e-6, "chart normal is unit");
        let (_, _, warm, metal_r, rough_top) = gpu::chart_inputs(17, 31, 32, 32);
        assert_eq!((metal_r, warm), (1.0, [0.8, 0.2, 0.1]));
        assert!(
            (rough_top - 31.5 / 32.0).abs() < 1e-6 && rough < rough_top,
            "rows sweep roughness upward"
        );
    }

    #[cfg(feature = "window")]
    #[test]
    fn brdf_lut_upload_roundtrips_on_llvmpipe() {
        // Cook bytes upload verbatim and read back bit-identical: the GPU
        // bake matches the CPU reference texel-for-texel (max diff 0.0).
        let lut = brdf_lut_build(16).expect("small lut builds");
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter =
            match pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })) {
                Ok(a) => a,
                Err(_) => {
                    eprintln!("SKIPPED ibl gpu test: no wgpu adapter on this machine");
                    return;
                }
            };
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("rl2-ibl-lut"),
            ..Default::default()
        }))
        .expect("device requests succeed when the adapter exists");
        let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let texture = gpu::upload_brdf_lut(&device, &queue, &lut).expect("lut uploads");
        let size = 16u32;
        let padded: u32 = 256;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("rl2-ibl-lut-readback"),
            size: (padded as u64) * (size as u64),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("rl2-ibl-lut-copy"),
        });
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(size),
                },
            },
            wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: 1,
            },
        );
        queue.submit(Some(enc.finish()));
        let slice = readback.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("poll succeeds");
        rx.recv().expect("map channel").expect("map succeeds");
        let mut max_diff = 0.0f32;
        {
            let mapped = slice.get_mapped_range().expect("mapped range");
            for (row, t_row) in lut.data.chunks_exact(size as usize).enumerate() {
                let off = row * padded as usize;
                for (i, t) in t_row.iter().enumerate() {
                    let bits = &mapped[off + i * 8..off + i * 8 + 8];
                    let g0 = f32::from_le_bytes([bits[0], bits[1], bits[2], bits[3]]);
                    let g1 = f32::from_le_bytes([bits[4], bits[5], bits[6], bits[7]]);
                    max_diff = max_diff.max((g0 - t[0]).abs());
                    max_diff = max_diff.max((g1 - t[1]).abs());
                }
            }
        }
        readback.unmap();
        let scope_err = pollster::block_on(scope.pop());
        assert!(
            scope_err.is_none(),
            "zero validation errors, got {scope_err:?}"
        );
        assert_eq!(max_diff, 0.0, "GPU LUT matches the CPU bake exactly");
        eprintln!("RL2 GPU LUT round-trip: 16x16 texels, max diff {max_diff}");
    }

    #[cfg(all(feature = "v1", feature = "window"))]
    #[test]
    fn ibl_chart_submits_with_look_binds_on_llvmpipe() {
        // RL2 live: the full bake binds (prefilter array, BRDF LUT, SH) next
        // to the shared look frame in one real wgpu pass, and the float
        // readback tracks the CPU `shade_ibl` reference inside the posted
        // chart tolerance with zero validation errors.
        let Some((device, queue, info)) = crate::graph::gx1_test_device() else {
            eprintln!("SKIPPED rl2 chart submit: no wgpu adapter on this machine");
            return;
        };
        let (bake, _) = bake(HDRI_BYTES).expect("studio HDRI bakes");
        let mut submitter = gpu::IblSubmitter::new(device);
        let frame = crate::gtao::LookFrame {
            frame_index: 5,
            viewport_px: [32, 32],
            exposure_ev100: 10.0,
            sun_dir: [0.3, 0.8, 0.5],
        };
        let receipt = bake
            .submit_gpu(device, queue, &frame, &mut submitter)
            .expect("chart submits");
        assert_eq!(receipt.passes_submitted, 1, "one chart pass recorded");
        assert!(
            receipt.is_clean(),
            "0 validation errors, got {}",
            receipt.validation_errors
        );
        assert_eq!(
            receipt.covered_pixels,
            32 * 32,
            "every chart pixel shades, got {}",
            receipt.covered_pixels
        );
        assert_eq!(
            (receipt.frame_index, receipt.width_px, receipt.height_px),
            (5, 32, 32),
            "receipt names the submitted frame"
        );
        assert_eq!(
            receipt.cube_texels,
            IBL_LEVELS as u64 * IBL_FACES as u64 * IBL_CUBE_SIZE as u64 * IBL_CUBE_SIZE as u64
        );
        assert_eq!(
            receipt.lut_texels,
            BRDF_LUT_SIZE as u64 * BRDF_LUT_SIZE as u64
        );
        assert_eq!(receipt.sh_coeffs, IBL_SH_COEFFS as u32);
        assert!(
            receipt.within_chart_tolerance(),
            "chart tracks CPU shade: abs {} over {} / rel {:.4}% over {}",
            receipt.max_abs_diff,
            gpu::IBL_CHART_ABS_TOL,
            receipt.max_rel_diff * 100.0,
            gpu::IBL_CHART_REL_TOL * 100.0
        );
        // Wall-clock budget lives in ibl_chart_submit_wall_budget (--perf
        // only): cold submits pay pipeline compile and idle clocks (observed
        // 210MHz vs ~1500MHz boost), never a default gate.
        eprintln!(
            "RL2 chart on {}: covered={} abs={:.6} rel={:.4}% submit_ms={:.2}",
            info.name,
            receipt.covered_pixels,
            receipt.max_abs_diff,
            receipt.max_rel_diff * 100.0,
            receipt.submit_ms
        );
    }

    #[cfg(all(feature = "v1", feature = "window"))]
    #[test]
    fn ibl_submit_reuses_backing_and_binds() {
        // Steady state creates nothing: the second submit on matching shape
        // reuses every texture and bind group, and both receipts agree
        // exactly (deterministic GPU bytes twice).
        let Some((device, queue, _)) = crate::graph::gx1_test_device() else {
            eprintln!("SKIPPED rl2 reuse: no wgpu adapter on this machine");
            return;
        };
        let (bake, _) = bake(HDRI_BYTES).expect("studio HDRI bakes");
        let mut submitter = gpu::IblSubmitter::new(device);
        assert_eq!(
            (submitter.texture_count(), submitter.bind_group_count()),
            (0, 0),
            "submitter sizes nothing up front"
        );
        let frame = |index: u64| crate::gtao::LookFrame {
            frame_index: index,
            viewport_px: [32, 32],
            exposure_ev100: 10.0,
            sun_dir: [0.3, 0.8, 0.5],
        };
        let first = bake
            .submit_gpu(device, queue, &frame(5), &mut submitter)
            .expect("first submits");
        assert_eq!(
            (submitter.texture_count(), submitter.bind_group_count()),
            (3, 2),
            "one array plus one LUT plus one target, one bind per group"
        );
        let second = bake
            .submit_gpu(device, queue, &frame(6), &mut submitter)
            .expect("second submits");
        assert_eq!(
            (submitter.texture_count(), submitter.bind_group_count()),
            (3, 2),
            "steady-state submit creates no new textures or bind groups"
        );
        assert_eq!(
            (
                second.covered_pixels,
                second.max_abs_diff,
                second.max_rel_diff
            ),
            (first.covered_pixels, first.max_abs_diff, first.max_rel_diff),
            "same bake twice gives identical chart pixels"
        );
        assert!(second.is_clean() && second.within_chart_tolerance());
        // Wall-clock budget lives in ibl_chart_submit_wall_budget (--perf only).
    }

    #[cfg(all(feature = "v1", feature = "window"))]
    #[test]
    fn ibl_submit_grows_no_host_scratch() {
        // Steady-state submits grow nothing on the host: both scratch
        // capacities stay flat across submits, and the chart still tracks
        // the CPU reference. Sizing happens once, on the first upload.
        let Some((device, queue, _)) = crate::graph::gx1_test_device() else {
            eprintln!("SKIPPED rl2 scratch: no wgpu adapter on this machine");
            return;
        };
        let (bake, _) = bake(HDRI_BYTES).expect("studio HDRI bakes");
        let mut submitter = gpu::IblSubmitter::new(device);
        let frame = |index: u64| crate::gtao::LookFrame {
            frame_index: index,
            viewport_px: [32, 32],
            exposure_ev100: 10.0,
            sun_dir: [0.3, 0.8, 0.5],
        };
        bake.submit_gpu(device, queue, &frame(5), &mut submitter)
            .expect("first submits");
        let caps = submitter.scratch_capacities();
        assert_eq!(
            caps,
            (64 * 64 * 16, 64 * 64 * 8),
            "first submit sizes staging exactly, got {caps:?}"
        );
        for index in 6..9 {
            let receipt = bake
                .submit_gpu(device, queue, &frame(index), &mut submitter)
                .expect("steady state submits");
            assert_eq!(
                submitter.scratch_capacities(),
                caps,
                "steady-state submit grows no host scratch"
            );
            assert!(receipt.is_clean() && receipt.within_chart_tolerance());
        }
    }

    #[cfg(all(feature = "v1", feature = "window"))]
    #[test]
    #[ignore = "timing: chart submit wall-clock needs release --perf on the boosted reference GPU (cold/idle-clock submits miss the budget)"]
    fn ibl_chart_submit_wall_budget() {
        let Some((device, queue, _)) = crate::graph::gx1_test_device() else {
            eprintln!("SKIPPED rl2 wall budget: no wgpu adapter on this machine");
            return;
        };
        let (bake, _) = bake(HDRI_BYTES).expect("studio HDRI bakes");
        let mut submitter = gpu::IblSubmitter::new(device);
        let frame = |index: u64| crate::gtao::LookFrame {
            frame_index: index,
            viewport_px: [32, 32],
            exposure_ev100: 10.0,
            sun_dir: [0.3, 0.8, 0.5],
        };
        // Warm the pipeline once so the budget reads steady state, not compile.
        let _ = bake
            .submit_gpu(device, queue, &frame(5), &mut submitter)
            .expect("warmup submits");
        let second = bake
            .submit_gpu(device, queue, &frame(6), &mut submitter)
            .expect("budget submit runs");
        // Perf harness line: log the measured submit so --perf medians are readable.
        eprintln!(
            "ibl_chart_submit_wall_budget: steady submit {:.2}ms vs {:.1}ms budget",
            second.submit_ms,
            second.budget_ms()
        );
        assert!(second.is_clean(), "0 validation errors");
        assert!(
            second.within_budget(),
            "submit {:.2}ms over budget {:.1}ms",
            second.submit_ms,
            second.budget_ms()
        );
    }

    #[cfg(all(feature = "v1", feature = "window"))]
    #[test]
    fn ibl_submit_rejects_bad_inputs() {
        // Bad frames and non-standard bakes refuse before any target sizes;
        // device errors stay loud and typed.
        let Some((device, queue, _)) = crate::graph::gx1_test_device() else {
            eprintln!("SKIPPED rl2 bad inputs: no wgpu adapter on this machine");
            return;
        };
        let (bake, _) = bake(HDRI_BYTES).expect("studio HDRI bakes");
        let mut submitter = gpu::IblSubmitter::new(device);
        let frame = crate::gtao::LookFrame {
            frame_index: 5,
            viewport_px: [32, 32],
            exposure_ev100: 10.0,
            sun_dir: [0.3, 0.8, 0.5],
        };
        let bad_view = crate::gtao::LookFrame {
            viewport_px: [0, 32],
            ..frame
        };
        match bake.submit_gpu(device, queue, &bad_view, &mut submitter) {
            Err(IblError::Invalid(detail)) => assert!(!detail.is_empty(), "never silent"),
            other => panic!("zero viewport must fail Invalid, got {other:?}"),
        }
        // Window viewports downscale into the edge instead of failing the
        // frame: 512x32 submits the 256x16 chart on its own submitter and
        // still tracks the CPU reference.
        let huge = crate::gtao::LookFrame {
            viewport_px: [512, 32],
            ..frame
        };
        let mut wide = gpu::IblSubmitter::new(device);
        let wide_receipt = bake
            .submit_gpu(device, queue, &huge, &mut wide)
            .expect("wide viewport downscales into the edge");
        assert_eq!(
            (wide_receipt.width_px, wide_receipt.height_px),
            (256, 16),
            "512x32 downscales to 256x16"
        );
        assert!(wide_receipt.is_clean(), "0 validation errors");
        assert!(
            wide_receipt.within_chart_tolerance(),
            "downscaled chart tracks CPU shade: abs {} / rel {:.4}%",
            wide_receipt.max_abs_diff,
            wide_receipt.max_rel_diff * 100.0
        );
        // Direct over-edge targets still refuse loudly before sizing.
        match check_chart_target_size(512, 32) {
            Err(IblError::Invalid(detail)) => {
                assert!(detail.contains("exceeds"), "names the cap: {detail}");
            }
            other => panic!("oversize target must fail Invalid, got {other:?}"),
        }
        let small_cube = IblBake {
            cube: PrefilterCube::new(8, IBL_LEVELS),
            sh: IrradianceSh::zero(),
            brdf: BrdfLut::new(8),
            source_average: [0.5, 0.5, 0.5],
        };
        match small_cube.submit_gpu(device, queue, &frame, &mut submitter) {
            Err(IblError::Invalid(detail)) => {
                assert!(detail.contains("standard"), "names the shape: {detail}");
            }
            other => panic!("non-standard cube must fail Invalid, got {other:?}"),
        }
        let small_lut = IblBake {
            brdf: BrdfLut::new(8),
            ..bake.clone()
        };
        match small_lut.submit_gpu(device, queue, &frame, &mut submitter) {
            Err(IblError::Invalid(detail)) => {
                assert!(detail.contains("BRDF"), "names the shape: {detail}");
            }
            other => panic!("non-standard LUT must fail Invalid, got {other:?}"),
        }
        assert_eq!(
            (submitter.texture_count(), submitter.bind_group_count()),
            (0, 0),
            "refused submits size nothing"
        );
        for err in [
            IblError::Device("poll".to_string()),
            IblError::Validation("scope".to_string()),
            IblError::Invalid("bad frame".to_string()),
        ] {
            let text = format!("{err}");
            assert!(!text.is_empty(), "errors are never silent");
        }
    }
}
