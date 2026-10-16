//! post: engine-owned TAA/TAAU plus the GPU post chain (RP1).
//!
//! Owner: render lane; consumers WIN1/ED1 final frame through the RA1
//! presenter and the Q1 clips. Units: pixels for sizes and motion vectors,
//! ms per pass in receipts. Determinism: integer size math, fixed tap
//! orders, same complete inputs give bit-identical outputs; wall clock
//! appears in receipts, never in pixels. Errors are typed [`PostError`]:
//! bad caller input fails loudly, over-budget sizes degrade with
//! `BudgetExceeded`, and the no-device submit names the device-bound
//! `gpu::Rp1Submitter` to build first, never silent success.
//!
//! Pipeline order: jittered projection (per-frame Halton-2/3 offset into
//! the projection matrix) -> motion vectors (current minus history, in
//! pixels) -> TAA resolve (bilinear history reproject plus Karis-style
//! 3x3 neighborhood box clamp, then blend) -> temporal upsampling
//! (engine-owned bilinear TAAU present path; FSR1-inspired EASU plus RCAS
//! spatial path beside it, TAAU default frozen) ->
//! post chain (bloom extract/blur/add, exposure, ACES, grading). Exposure
//! is fixed stops by default; the auto-exposure state beside it histograms
//! the pre-exposure HDR (64 log bins, trimmed mean onto key) and slews
//! toward the retarget at 3 stops per second, so day sweeps track while
//! sparse highlights cannot pump the frame. The chain
//! reads the RL1/RL2 passes through [`PostLights`]: the caller hands the
//! `FrameLightsReceipt` (RL1) and `IblReceipt` (RL2) of the frame being
//! posted, and the output receipt quotes their pass ms alongside the post
//! pass ms, so the frame report names every light and post cost.
//!
//! Graph shape: [`build_rp1_graph`] declares the six RP1 passes
//! (`rp1-taa` -> `rp1-taau` -> `rp1-bloom` -> `rp1-exposure` ->
//! `rp1-aces` -> `rp1-grade`) on the RA2 frame graph, and the `window`
//! `gpu::Rp1Submitter` records the same six in compiled order on a live
//! device with motion-vector plus jitter reprojection and box-clamp
//! resolve, then the bloom/exposure/ACES/grade chain, all validation
//! clean. The CPU chain stays the parity oracle: same complete inputs
//! give bit-identical CPU pixels twice, and GPU pixels match it within
//! the posted tolerance.
//!
//! Provenance: Halton-2/3 jitter, box-clamp reprojection, soft-knee bloom,
//! Narkowicz ACES fit and contrast/saturation grading are textbook
//! formulations implemented directly (no upstream copy), so they carry no
//! `PORT()` marker. The spatial upscale reimplements the FSR1 EASU plus RCAS
//! shape (directional Lanczos-2 taps with 2x2 clamp plus lobe limiters) with
//! zero donor bytes; its items carry the donor `PORT()` marker.

use std::fmt;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Bounds: every external size is validated before allocation.
// ---------------------------------------------------------------------------

/// Largest accepted post dimension in pixels (mirrors the RA1 frame bound).
pub const MAX_POST_DIM: u32 = 2048;
/// Most bloom mip levels in the CPU chain.
pub const MAX_BLOOM_LEVELS: u32 = 6;
/// Default history blend: how much clamped history the resolve keeps.
pub const DEFAULT_HISTORY_BLEND: f32 = 0.9;
/// Contrast pivot (linear mid-grey), same convention as the R4 grade path.
pub const GRADE_MID_GREY: f32 = 0.18;
/// Posted ghosting gate: max trail-region luma after the TAA resolve.
pub const GHOST_MAX_TRAIL: f32 = 0.25;
/// Posted shimmer gate: max per-pixel temporal variance over the clip.
pub const SHIMMER_MAX_VAR: f32 = 0.01;

// ---------------------------------------------------------------------------
// Errors: typed; budget overruns degrade, no-device submit names the device
// path to build.
// ---------------------------------------------------------------------------

/// Post failures. `BudgetExceeded` is a degrade signal: the caller clamps
/// to `allowed` and keeps rendering. `Device` covers calls with no live
/// device handed over (use `gpu::Rp1Submitter` on a `window` device);
/// `Validation` carries a wgpu scope hit; neither path ever reports
/// silent success.
#[derive(Clone, PartialEq, Debug)]
pub enum PostError {
    /// Caller input is unusable; reason carried.
    Invalid(String),
    /// The request exceeded a bound; the caller clamps to `allowed`.
    BudgetExceeded {
        /// Requested amount.
        requested: u32,
        /// Amount the pass actually uses.
        allowed: u32,
    },
    /// No live device on this call; what to build first, carried.
    Device(String),
    /// A wgpu validation scope fired inside the submit; scope text carried.
    Validation(String),
}

impl fmt::Display for PostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "post input: {e}"),
            Self::BudgetExceeded { requested, allowed } => {
                write!(
                    f,
                    "post budget exceeded: {requested} requested, {allowed} used"
                )
            }
            Self::Device(e) => write!(f, "post device: {e}"),
            Self::Validation(e) => write!(f, "post validation: {e}"),
        }
    }
}

impl std::error::Error for PostError {}

// ---------------------------------------------------------------------------
// Receipts: per-pass ms plus pixel counts for the frame report.
// ---------------------------------------------------------------------------

/// One post pass receipt: what ran, how many pixels, how long.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PassReceipt {
    /// Pass name (`"taa"`, `"taau"`, `"bloom"`, `"exposure"`, `"aces"`, `"grade"`).
    pub pass: &'static str,
    /// Pixels written by this pass.
    pub pixels: u64,
    /// Wall-clock pass time in ms; evidence only, never hashed.
    pub ms: f64,
}

/// Frame-level post receipt: TAA resolve, TAAU upsample and the four chain
/// passes, plus the RL1/RL2 light ms the chain read.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FramePostReceipt {
    /// TAA resolve pass.
    pub taa: PassReceipt,
    /// Temporal upsample pass.
    pub upsample: PassReceipt,
    /// Bloom pass.
    pub bloom: PassReceipt,
    /// Exposure pass.
    pub exposure: PassReceipt,
    /// ACES tonemap pass.
    pub aces: PassReceipt,
    /// Grading pass.
    pub grade: PassReceipt,
    /// RL1 sun shadow pass ms read from the light receipt.
    pub sun_ms: f64,
    /// RL1 torch cluster pass ms read from the light receipt.
    pub cluster_ms: f64,
    /// RL2 IBL bake ms read from the IBL receipt.
    pub ibl_ms: f64,
}

impl FramePostReceipt {
    /// Total post pass time in ms for the frame report (lights excluded).
    pub fn total_ms(&self) -> f64 {
        self.taa.ms
            + self.upsample.ms
            + self.bloom.ms
            + self.exposure.ms
            + self.aces.ms
            + self.grade.ms
    }

    /// Total light pass time in ms read from RL1/RL2 for the frame report.
    pub fn lights_ms(&self) -> f64 {
        self.sun_ms + self.cluster_ms + self.ibl_ms
    }
}

fn receipt(pass: &'static str, pixels: u64, ms: f64) -> PassReceipt {
    PassReceipt { pass, pixels, ms }
}

// ---------------------------------------------------------------------------
// Jittered projection: Halton-2/3 per-frame offsets (engine-owned).
// ---------------------------------------------------------------------------

/// Radical inverse of `index` in `base`. Pure textbook math; frame callers
/// pass `frame + 1` so frame 0 still jitters off the origin.
pub fn halton(mut index: u32, base: u32) -> f32 {
    debug_assert!(base >= 2, "halton base must be >= 2");
    let (b, mut denom) = (f64::from(base), 1.0f64);
    let mut out = 0.0f64;
    while index > 0 {
        denom *= b;
        out += f64::from(index % base) / denom;
        index /= base;
    }
    out as f32
}

/// Jitter offset for `frame` in pixels, centered on `[-0.5, 0.5)`: x from
/// Halton base 2, y from Halton base 3. Depends only on the frame number,
/// never on wall time. Matches the R4 `TaaJitter` sequence by construction
/// (the `v1` parity test pins equality).
pub fn jitter_offset(frame: u64) -> [f32; 2] {
    let i = frame.wrapping_add(1) as u32;
    [halton(i, 2) - 0.5, halton(i, 3) - 0.5]
}

/// Jitter for `frame` in NDC units (`offset * 2 / res`), the value
/// [`jitter_projection`] adds to the projection matrix. Zero resolution
/// (suspended swap path) yields zero jitter.
pub fn jitter_ndc(frame: u64, width: u32, height: u32) -> [f32; 2] {
    if width == 0 || height == 0 {
        return [0.0, 0.0];
    }
    let [jx, jy] = jitter_offset(frame);
    [jx * 2.0 / width as f32, jy * 2.0 / height as f32]
}

/// GPU uniform `[jx_px, jy_px, jx_ndc, jy_ndc]` for the resolve pass.
pub fn jitter_uniform(frame: u64, width: u32, height: u32) -> [f32; 4] {
    let [px, py] = jitter_offset(frame);
    let [nx, ny] = jitter_ndc(frame, width, height);
    [px, py, nx, ny]
}

/// Jitter a row-major projection matrix for this frame: offsets the
/// perspective terms (`m[2][0]`, `m[2][1]`), leaving everything else
/// bit-identical. The `v1` wrapper pins the same terms on `glam::Mat4`.
pub fn jitter_projection(proj: [[f32; 4]; 4], ndc: [f32; 2]) -> [[f32; 4]; 4] {
    let mut out = proj;
    out[2][0] += ndc[0];
    out[2][1] += ndc[1];
    out
}

/// Jitter a `glam` projection matrix (same terms as [`jitter_projection`])
/// for the `v1_gpu` TAA pass. Behind `v1` with the camera math.
#[cfg(feature = "v1")]
pub fn jitter_projection_glam(proj: glam::Mat4, ndc: [f32; 2]) -> glam::Mat4 {
    let mut out = proj;
    out.z_axis.x += ndc[0];
    out.z_axis.y += ndc[1];
    out
}

// ---------------------------------------------------------------------------
// Motion vectors: current-minus-history flow in pixels.
// ---------------------------------------------------------------------------

/// Per-pixel motion in pixels: `motion[x, y] = current_pos - history_pos`,
/// so the history sample for `(x, y)` sits at `(x - mx, y - my)`.
/// Row-major, `width * height` entries.
#[derive(Clone, PartialEq, Debug)]
pub struct MotionField {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Motion vectors in pixels, row-major.
    pub flow: Vec<[f32; 2]>,
}

impl MotionField {
    /// Build from an explicit flow field: validates dims, length and
    /// finiteness before the resolve can read it.
    pub fn build(width: u32, height: u32, flow: Vec<[f32; 2]>) -> Result<Self, PostError> {
        check_dims(width, height)?;
        let want = width as usize * height as usize;
        if flow.len() != want {
            return Err(PostError::Invalid(format!(
                "motion flow {} differs from {width}x{height} ({want})",
                flow.len()
            )));
        }
        for (i, v) in flow.iter().enumerate() {
            if !v.iter().all(|c| c.is_finite()) {
                return Err(PostError::Invalid(format!(
                    "motion vector {i} is not finite"
                )));
            }
        }
        Ok(Self {
            width,
            height,
            flow,
        })
    }

    /// Zero motion everywhere (static camera): history reprojects 1:1.
    pub fn zero(width: u32, height: u32) -> Result<Self, PostError> {
        check_dims(width, height)?;
        Ok(Self {
            width,
            height,
            flow: vec![[0.0, 0.0]; width as usize * height as usize],
        })
    }

    /// Motion at texel `(x, y)` in pixels.
    pub fn at(&self, x: u32, y: u32) -> [f32; 2] {
        self.flow[y as usize * self.width as usize + x as usize]
    }

    /// Pixel count (`width * height`).
    pub fn len(&self) -> usize {
        self.width as usize * self.height as usize
    }

    /// Whether the field holds no texels (only when a dimension is 0,
    /// which [`Self::build`] never produces).
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

fn check_dims(width: u32, height: u32) -> Result<(), PostError> {
    if width == 0 || height == 0 {
        return Err(PostError::Invalid("post frame size is zero".into()));
    }
    if width > MAX_POST_DIM || height > MAX_POST_DIM {
        return Err(PostError::BudgetExceeded {
            requested: width.max(height),
            allowed: MAX_POST_DIM,
        });
    }
    Ok(())
}

fn check_image(
    pixels: &[[f32; 3]],
    width: u32,
    height: u32,
    what: &'static str,
) -> Result<(), PostError> {
    check_dims(width, height)?;
    let want = width as usize * height as usize;
    if pixels.len() != want {
        return Err(PostError::Invalid(format!(
            "{what} pixels {} differ from {width}x{height} ({want})",
            pixels.len()
        )));
    }
    for (i, p) in pixels.iter().enumerate() {
        if !p.iter().all(|c| c.is_finite()) {
            return Err(PostError::Invalid(format!(
                "{what} texel {i} is not finite"
            )));
        }
    }
    Ok(())
}

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
fn sample_bilinear(history: &[[f32; 3]], width: u32, height: u32, u: f32, v: f32) -> [f32; 3] {
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
fn neighborhood_minmax(
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

fn clamp_history(sample: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> [f32; 3] {
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
// Temporal upsampling: engine-owned TAAU present path (no FSR code).
// ---------------------------------------------------------------------------

/// Engine-owned temporal upsample: clamp-to-edge bilinear from the render
/// size to the presentation size. Same-size input is the bit-exact
/// identity; solid colors survive unchanged. Same complete inputs give
/// bit-identical outputs.
pub fn taa_upsample(
    src: &[[f32; 3]],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Result<Vec<[f32; 3]>, PostError> {
    for (dim, name) in [
        (src_w, "src_w"),
        (src_h, "src_h"),
        (dst_w, "dst_w"),
        (dst_h, "dst_h"),
    ] {
        if dim == 0 {
            return Err(PostError::Invalid(format!("taau {name} is zero")));
        }
        if dim > MAX_POST_DIM {
            return Err(PostError::BudgetExceeded {
                requested: dim,
                allowed: MAX_POST_DIM,
            });
        }
    }
    if src.len() != src_w as usize * src_h as usize {
        return Err(PostError::Invalid(format!(
            "taau src pixels {} differ from {src_w}x{src_h}",
            src.len()
        )));
    }
    for (i, p) in src.iter().enumerate() {
        if !p.iter().all(|c| c.is_finite()) {
            return Err(PostError::Invalid(format!(
                "taau src texel {i} is not finite"
            )));
        }
    }
    let sx = src_w as f32 / dst_w as f32;
    let sy = src_h as f32 / dst_h as f32;
    let mut out = Vec::with_capacity(dst_w as usize * dst_h as usize);
    for y in 0..dst_h as usize {
        for x in 0..dst_w as usize {
            let fx = (x as f32 + 0.5) * sx - 0.5;
            let fy = (y as f32 + 0.5) * sy - 0.5;
            out.push(sample_bilinear(src, src_w, src_h, fx, fy));
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Spatial upscale: FSR1-inspired EASU plus RCAS, engine-owned.
// ---------------------------------------------------------------------------

/// Lowest RCAS sharpness (the identity).
pub const FSR_SHARPEN_MIN: f32 = 0.0;
/// Highest RCAS sharpness.
pub const FSR_SHARPEN_MAX: f32 = 1.0;
/// Default RCAS sharpness for the combined path.
pub const FSR_DEFAULT_SHARPNESS: f32 = 0.4;
/// Posted shimmer gate for the spatial path (mirrors the fallback gate).
pub const FSR_SHIMMER_GATE: f32 = 0.06;
/// Smallest accepted MTF50 lift over bilinear at matched shimmer.
pub const FSR_MTF50_LIFT_MIN: f32 = 0.10;

/// Dynamic-resolution constants for the EASU pass (FSR1 `FsrEasuCon` shape).
///
/// PORT(GPUOpen-Effects/FidelityFX-FSR@a21ffb8:ffx-fsr/ffx_fsr1.h): MIT, shape
/// only reimplemented, zero HLSL bytes pasted. Own CPU plus WGSL, presentation
/// only, never in tick. The bilinear TAAU stays frozen as the default.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FsrEasuCon {
    /// Source width in pixels.
    pub src_w: u32,
    /// Source height in pixels.
    pub src_h: u32,
    /// Destination width in pixels.
    pub dst_w: u32,
    /// Destination height in pixels.
    pub dst_h: u32,
    /// Scale per axis (`src / dst`).
    pub scale: [f32; 2],
    /// Offset per axis (`0.5 * scale - 0.5`).
    pub offset: [f32; 2],
    /// Texel size per axis (`1 / src`).
    pub texel: [f32; 2],
}

// Small exact conversions without numeric casts (dims stay under u16).
fn f32_from_dim(v: u32) -> Result<f32, PostError> {
    let small =
        u16::try_from(v).map_err(|_| PostError::Invalid("fsr dim does not fit u16".into()))?;
    Ok(f32::from(small))
}

fn f32_from_tap(v: i64) -> Result<f32, PostError> {
    let small = i16::try_from(v).map_err(|_| PostError::Invalid("fsr tap out of range".into()))?;
    Ok(f32::from(small))
}

fn usize_from_dim(v: u32) -> Result<usize, PostError> {
    usize::try_from(v).map_err(|_| PostError::Invalid("fsr dim does not fit usize".into()))
}

fn fsr_capacity(w: u32, h: u32) -> Result<usize, PostError> {
    let total = u64::from(w) * u64::from(h);
    usize::try_from(total).map_err(|_| PostError::Invalid("fsr size does not fit".into()))
}

/// Build the EASU constants for a source to destination size pair.
pub fn fsr_easu_con(
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Result<FsrEasuCon, PostError> {
    check_dims(src_w, src_h)?;
    check_dims(dst_w, dst_h)?;
    let src_w_f = f32_from_dim(src_w)?;
    let src_h_f = f32_from_dim(src_h)?;
    let dst_w_f = f32_from_dim(dst_w)?;
    let dst_h_f = f32_from_dim(dst_h)?;
    let sx = src_w_f / dst_w_f;
    let sy = src_h_f / dst_h_f;
    Ok(FsrEasuCon {
        src_w,
        src_h,
        dst_w,
        dst_h,
        scale: [sx, sy],
        offset: [0.5 * sx - 0.5, 0.5 * sy - 0.5],
        texel: [1.0 / src_w_f, 1.0 / src_h_f],
    })
}

// Lanczos-2 kernel value at `x` (zero past `|x| >= 2`, one at zero).
fn fsr_lanczos2(x: f32) -> f32 {
    let ax = x.abs();
    if ax >= 2.0 {
        return 0.0;
    }
    if ax < 1e-6 {
        return 1.0;
    }
    let pi_ax = std::f32::consts::PI * ax;
    let sinc1 = pi_ax.sin() / pi_ax;
    let half = pi_ax * 0.5;
    sinc1 * (half.sin() / half)
}

fn fsr_texel_clamped(
    src: &[[f32; 3]],
    w: u32,
    h: u32,
    x: i64,
    y: i64,
) -> Result<[f32; 3], PostError> {
    let w64 = i64::from(w);
    let h64 = i64::from(h);
    let cx = x.clamp(0, w64 - 1);
    let cy = y.clamp(0, h64 - 1);
    let idx64 = cy * w64 + cx;
    let idx = usize::try_from(idx64)
        .map_err(|_| PostError::Invalid("fsr texel index out of range".into()))?;
    src.get(idx)
        .copied()
        .ok_or_else(|| PostError::Invalid("fsr texel index out of range".into()))
}

fn fsr_luma_clamped(src: &[[f32; 3]], w: u32, h: u32, x: i64, y: i64) -> Result<f32, PostError> {
    Ok(luminance(fsr_texel_clamped(src, w, h, x, y)?))
}

// Edge frame at one source texel: gradient axes plus stretch.
struct FsrDir {
    nx: f32,
    ny: f32,
    ex: f32,
    ey: f32,
    stretch: f32,
}

fn fsr_dir_at(src: &[[f32; 3]], w: u32, h: u32, ix: i64, iy: i64) -> Result<FsrDir, PostError> {
    let one: i64 = 1;
    let l_px = fsr_luma_clamped(src, w, h, ix + one, iy)?;
    let l_nx = fsr_luma_clamped(src, w, h, ix - one, iy)?;
    let l_py = fsr_luma_clamped(src, w, h, ix, iy + one)?;
    let l_ny = fsr_luma_clamped(src, w, h, ix, iy - one)?;
    let l_pp = fsr_luma_clamped(src, w, h, ix + one, iy + one)?;
    let l_nn = fsr_luma_clamped(src, w, h, ix - one, iy - one)?;
    let l_pn = fsr_luma_clamped(src, w, h, ix + one, iy - one)?;
    let l_np = fsr_luma_clamped(src, w, h, ix - one, iy + one)?;
    let gx = (l_px - l_nx).abs();
    let gy = (l_py - l_ny).abs();
    let gd1 = (l_pp - l_nn).abs();
    let gd2 = (l_pn - l_np).abs();
    let gx_all = gx + (gd1 + gd2) * 0.5;
    let gy_all = gy + (gd1 + gd2) * 0.5;
    let len = (gx_all * gx_all + gy_all * gy_all).sqrt();
    if !len.is_finite() || len < 1e-6 {
        return Ok(FsrDir {
            nx: 1.0,
            ny: 0.0,
            ex: 0.0,
            ey: 1.0,
            stretch: 1.0,
        });
    }
    let nx = gx_all / len;
    let ny = gy_all / len;
    let aniso = (len * 2.0).clamp(0.0, 1.0);
    Ok(FsrDir {
        nx,
        ny,
        ex: -ny,
        ey: nx,
        stretch: 1.0 + aniso,
    })
}

fn fsr_easu_pixel(
    src: &[[f32; 3]],
    w: u32,
    h: u32,
    con: &FsrEasuCon,
    x: u32,
    y: u32,
) -> Result<[f32; 3], PostError> {
    let x_f = f32_from_dim(x)?;
    let y_f = f32_from_dim(y)?;
    let fx = (x_f + 0.5) * con.scale[0] + con.offset[0];
    let fy = (y_f + 0.5) * con.scale[1] + con.offset[1];
    // Integer base with exact math (no float to int cast).
    let x64 = i64::from(x);
    let y64 = i64::from(y);
    let src_w64 = i64::from(con.src_w);
    let src_h64 = i64::from(con.src_h);
    let dst_w64 = i64::from(con.dst_w);
    let dst_h64 = i64::from(con.dst_h);
    let num_x = (2 * x64 + 1) * src_w64 - dst_w64;
    let num_y = (2 * y64 + 1) * src_h64 - dst_h64;
    let ix = num_x.div_euclid(2 * dst_w64);
    let iy = num_y.div_euclid(2 * dst_h64);
    let dir = fsr_dir_at(src, w, h, ix, iy)?;
    let ix_f = f32_from_tap(ix)?;
    let iy_f = f32_from_tap(iy)?;
    // 4x4 directional Lanczos-2 taps around the base.
    let mut acc = [0.0f32; 3];
    let mut wsum = 0.0f32;
    for dy in [-1i8, 0i8, 1i8, 2i8] {
        for dx in [-1i8, 0i8, 1i8, 2i8] {
            let tap = fsr_texel_clamped(src, w, h, ix + i64::from(dx), iy + i64::from(dy))?;
            let off_x = ix_f + f32::from(dx) - fx;
            let off_y = iy_f + f32::from(dy) - fy;
            let along = off_x * dir.ex + off_y * dir.ey;
            let across = off_x * dir.nx + off_y * dir.ny;
            let wgt = fsr_lanczos2(along / dir.stretch) * fsr_lanczos2(across * dir.stretch);
            if wgt.is_finite() {
                acc[0] += tap[0] * wgt;
                acc[1] += tap[1] * wgt;
                acc[2] += tap[2] * wgt;
                wsum += wgt;
            }
        }
    }
    if !wsum.is_finite() || wsum <= 0.0 {
        return Ok(sample_bilinear(src, w, h, fx, fy));
    }
    let mut pix = [acc[0] / wsum, acc[1] / wsum, acc[2] / wsum];
    // 2x2 min/max ringing clamp around the sample point.
    let one: i64 = 1;
    let c00 = fsr_texel_clamped(src, w, h, ix, iy)?;
    let c10 = fsr_texel_clamped(src, w, h, ix + one, iy)?;
    let c01 = fsr_texel_clamped(src, w, h, ix, iy + one)?;
    let c11 = fsr_texel_clamped(src, w, h, ix + one, iy + one)?;
    for c in 0..3 {
        let lo = c00[c].min(c10[c]).min(c01[c]).min(c11[c]);
        let hi = c00[c].max(c10[c]).max(c01[c]).max(c11[c]);
        pix[c] = pix[c].clamp(lo, hi);
    }
    Ok(pix)
}

/// Spatial upscale with directional Lanczos-2 taps plus a 2x2 ringing clamp
/// (EASU shape). Same complete inputs give bit-identical outputs. The frozen
/// bilinear TAAU stays the default; this path sits beside it.
pub fn fsr_easu_upsample(
    src: &[[f32; 3]],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Result<Vec<[f32; 3]>, PostError> {
    check_image(src, src_w, src_h, "fsr easu src")?;
    check_dims(dst_w, dst_h)?;
    let con = fsr_easu_con(src_w, src_h, dst_w, dst_h)?;
    let mut out = Vec::with_capacity(fsr_capacity(dst_w, dst_h)?);
    for y in 0..dst_h {
        for x in 0..dst_w {
            out.push(fsr_easu_pixel(src, src_w, src_h, &con, x, y)?);
        }
    }
    Ok(out)
}

/// Contrast-adaptive sharpen with 4-neighbour lobe limiters (RCAS shape).
///
/// `sharpness` rides `0..=1`; 0 is the identity. Output clamps per channel
/// to the neighbour min/max, so no new halo escapes the local lobe. Same
/// complete inputs give bit-identical outputs.
pub fn fsr_rcas_sharpen(
    img: &[[f32; 3]],
    w: u32,
    h: u32,
    sharpness: f32,
) -> Result<Vec<[f32; 3]>, PostError> {
    if !sharpness.is_finite() || sharpness < FSR_SHARPEN_MIN || sharpness > FSR_SHARPEN_MAX {
        return Err(PostError::Invalid("fsr sharpness outside 0..=1".into()));
    }
    check_image(img, w, h, "fsr rcas src")?;
    if sharpness <= 0.0 {
        return Ok(img.to_vec());
    }
    let mut out = Vec::with_capacity(fsr_capacity(w, h)?);
    let one: i64 = 1;
    for y in 0..h {
        for x in 0..w {
            let x64 = i64::from(x);
            let y64 = i64::from(y);
            let e = fsr_texel_clamped(img, w, h, x64, y64)?;
            let b = fsr_texel_clamped(img, w, h, x64, y64 - one)?;
            let d = fsr_texel_clamped(img, w, h, x64 - one, y64)?;
            let f = fsr_texel_clamped(img, w, h, x64 + one, y64)?;
            let hh = fsr_texel_clamped(img, w, h, x64, y64 + one)?;
            let mut pix = [0.0f32; 3];
            for c in 0..3 {
                let mn = b[c].min(d[c]).min(f[c]).min(hh[c]);
                let mx = b[c].max(d[c]).max(f[c]).max(hh[c]);
                let blur = (b[c] + d[c] + f[c] + hh[c]) * 0.25;
                pix[c] = (e[c] + (e[c] - blur) * sharpness).clamp(mn, mx);
            }
            out.push(pix);
        }
    }
    Ok(out)
}

/// Full spatial path: EASU upscale then RCAS sharpen (FSR1 shape).
/// Presentation only, never in tick. Same complete inputs give bit-identical
/// outputs.
pub fn fsr_upscale(
    src: &[[f32; 3]],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
    sharpness: f32,
) -> Result<Vec<[f32; 3]>, PostError> {
    let easu = fsr_easu_upsample(src, src_w, src_h, dst_w, dst_h)?;
    fsr_rcas_sharpen(&easu, dst_w, dst_h, sharpness)
}

/// Build a vertical step edge at low res for the MTF50 probe (dark left,
/// bright right, edge at half width). Deterministic, no wall clock.
pub fn fsr_edge_lowres(w: u32, h: u32) -> Result<Vec<[f32; 3]>, PostError> {
    check_dims(w, h)?;
    let mut out = Vec::with_capacity(fsr_capacity(w, h)?);
    let half = w / 2;
    for _ in 0..h {
        for x in 0..w {
            if x < half {
                out.push([0.1, 0.1, 0.1]);
            } else {
                out.push([0.9, 0.9, 0.9]);
            }
        }
    }
    Ok(out)
}

/// 10-90 percent edge width in output pixels, averaged over middle rows.
/// Flat rows fail loudly (no edge to measure).
pub fn fsr_edge_width_10_90(up: &[[f32; 3]], w: u32, h: u32) -> Result<f32, PostError> {
    check_image(up, w, h, "fsr edge up")?;
    if h < 4 {
        return Err(PostError::Invalid("fsr edge needs 4 rows".into()));
    }
    let w_us = usize_from_dim(w)?;
    let h_us = usize_from_dim(h)?;
    let y0_us = h_us / 4;
    let y1_us = h_us - h_us / 4;
    let mut sum = 0.0f32;
    let mut rows = 0u32;
    for y_us in y0_us..y1_us {
        let mut mn = f32::INFINITY;
        let mut mx = f32::NEG_INFINITY;
        for x_us in 0..w_us {
            let l = luminance(up[y_us * w_us + x_us]);
            mn = mn.min(l);
            mx = mx.max(l);
        }
        let span = mx - mn;
        if !span.is_finite() || span < 1e-6 {
            return Err(PostError::Invalid("fsr edge row is flat".into()));
        }
        let lo = mn + span * 0.1;
        let hi = mn + span * 0.9;
        let mut x10: Option<usize> = None;
        let mut x90: Option<usize> = None;
        for x_us in 0..w_us {
            let l = luminance(up[y_us * w_us + x_us]);
            if x10.is_none() && l >= lo {
                x10 = Some(x_us);
            }
            if l >= hi {
                x90 = Some(x_us);
                break;
            }
        }
        let (a_us, b_us) = match (x10, x90) {
            (Some(a), Some(b)) => (a, b),
            _ => return Err(PostError::Invalid("fsr edge never rises".into())),
        };
        let a32 = u32::try_from(a_us)
            .map_err(|_| PostError::Invalid("fsr edge x does not fit".into()))?;
        let b32 = u32::try_from(b_us)
            .map_err(|_| PostError::Invalid("fsr edge x does not fit".into()))?;
        sum += f32_from_dim(b32)? - f32_from_dim(a32)?;
        rows += 1;
    }
    if rows == 0 {
        return Err(PostError::Invalid("fsr edge has no rows".into()));
    }
    Ok(sum / f32_from_dim(rows)?)
}

/// MTF50 proxy from a 10-90 edge width (`0.5 / width`, floored at 0.5 px).
pub fn fsr_mtf50(edge_width_px: f32) -> Result<f32, PostError> {
    if !edge_width_px.is_finite() || edge_width_px < 0.0 {
        return Err(PostError::Invalid("fsr edge width not finite".into()));
    }
    Ok(0.5 / edge_width_px.max(0.5))
}

/// MTF50 lift of a candidate width over a baseline width.
pub fn fsr_mtf50_lift(base_width_px: f32, cand_width_px: f32) -> Result<f32, PostError> {
    let base = fsr_mtf50(base_width_px)?;
    let cand = fsr_mtf50(cand_width_px)?;
    if !base.is_finite() || base <= 0.0 {
        return Err(PostError::Invalid("fsr base mtf not positive".into()));
    }
    Ok((cand - base) / base)
}

// ---------------------------------------------------------------------------
// Post chain: bloom, exposure, ACES, grading.
// ---------------------------------------------------------------------------

/// Bloom configuration: soft-knee threshold in linear HDR, mip count and
/// add-back strength.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BloomConfig {
    /// Knee center in linear HDR; brighter texels bloom.
    pub threshold: f32,
    /// Soft-knee width in linear HDR, `>= 0`.
    pub knee: f32,
    /// Add-back strength, `0..=2`.
    pub intensity: f32,
    /// Mip levels in the blur chain, `1..=MAX_BLOOM_LEVELS`.
    pub levels: u32,
}

impl Default for BloomConfig {
    fn default() -> Self {
        Self {
            threshold: 1.0,
            knee: 0.5,
            intensity: 0.6,
            levels: 4,
        }
    }
}

impl BloomConfig {
    /// Validate ranges before the chain runs.
    pub fn validate(&self) -> Result<(), PostError> {
        if !self.threshold.is_finite() || self.threshold < 0.0 {
            return Err(PostError::Invalid(format!(
                "bloom threshold {} is not finite non-negative",
                self.threshold
            )));
        }
        if !self.knee.is_finite() || self.knee < 0.0 {
            return Err(PostError::Invalid(format!(
                "bloom knee {} is not finite non-negative",
                self.knee
            )));
        }
        if !self.intensity.is_finite() || !(0.0..=2.0).contains(&self.intensity) {
            return Err(PostError::Invalid(format!(
                "bloom intensity {} outside 0..=2",
                self.intensity
            )));
        }
        if self.levels == 0 || self.levels > MAX_BLOOM_LEVELS {
            return Err(PostError::Invalid(format!(
                "bloom levels {} outside 1..={MAX_BLOOM_LEVELS}",
                self.levels
            )));
        }
        Ok(())
    }
}

/// Exposure configuration: stops added before the tonemap.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ExposureConfig {
    /// Exposure in stops, `-8..=+8`.
    pub exposure_ev: f32,
}

impl Default for ExposureConfig {
    fn default() -> Self {
        Self { exposure_ev: 0.0 }
    }
}

impl ExposureConfig {
    /// Linear multiplier for the exposure (`2^ev`).
    pub fn mult(&self) -> Result<f32, PostError> {
        if !self.exposure_ev.is_finite() || self.exposure_ev < -8.0 || self.exposure_ev > 8.0 {
            return Err(PostError::Invalid(format!(
                "exposure {} outside -8..=+8 stops",
                self.exposure_ev
            )));
        }
        Ok(2f32.powf(self.exposure_ev))
    }
}

/// Color grading configuration applied after the tonemap in display space.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct GradeConfig {
    /// Contrast around [`GRADE_MID_GREY`], `>= 0`.
    pub contrast: f32,
    /// Saturation mix, `>= 0` (1 keeps color, 0 is grey).
    pub saturation: f32,
    /// Per-channel lift added after gain, finite.
    pub lift: [f32; 3],
    /// Per-channel gain, finite and non-negative.
    pub gain: [f32; 3],
}

impl Default for GradeConfig {
    fn default() -> Self {
        Self {
            contrast: 1.0,
            saturation: 1.0,
            lift: [0.0; 3],
            gain: [1.0; 3],
        }
    }
}

impl GradeConfig {
    /// Validate ranges before the chain runs.
    pub fn validate(&self) -> Result<(), PostError> {
        if !self.contrast.is_finite() || self.contrast < 0.0 {
            return Err(PostError::Invalid(format!(
                "grade contrast {} is not finite non-negative",
                self.contrast
            )));
        }
        if !self.saturation.is_finite() || self.saturation < 0.0 {
            return Err(PostError::Invalid(format!(
                "grade saturation {} is not finite non-negative",
                self.saturation
            )));
        }
        if !self.lift.iter().all(|c| c.is_finite()) {
            return Err(PostError::Invalid("grade lift is not finite".into()));
        }
        if !self.gain.iter().all(|c| c.is_finite() && *c >= 0.0) {
            return Err(PostError::Invalid(
                "grade gain is not finite non-negative".into(),
            ));
        }
        Ok(())
    }
}

/// Full post chain configuration: bloom plus exposure plus ACES white plus
/// grading. Validated as a unit before `run`.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PostChain {
    /// Bloom extract/blur/add settings.
    pub bloom: BloomConfig,
    /// Exposure stops.
    pub exposure: ExposureConfig,
    /// ACES white reference, `> 0`.
    pub aces_white: f32,
    /// Display-space grading.
    pub grade: GradeConfig,
}

impl Default for PostChain {
    fn default() -> Self {
        Self {
            bloom: BloomConfig::default(),
            exposure: ExposureConfig::default(),
            aces_white: 11.2,
            grade: GradeConfig::default(),
        }
    }
}

impl PostChain {
    /// Validate every stage before the chain runs.
    pub fn validate(&self) -> Result<(), PostError> {
        self.bloom.validate()?;
        self.exposure.mult()?;
        if !self.aces_white.is_finite() || self.aces_white <= 0.0 {
            return Err(PostError::Invalid(format!(
                "aces white {} is not positive finite",
                self.aces_white
            )));
        }
        self.grade.validate()?;
        Ok(())
    }

    /// No-device submit seam: names the device-bound `gpu::Rp1Submitter`
    /// (`window` build) that records the same six graph passes on a live
    /// device. The CPU chain above stays the parity oracle. Never silent
    /// success.
    pub fn submit_gpu(&self) -> Result<FramePostReceipt, PostError> {
        let _ = self;
        Err(PostError::Device(
            "post GPU needs a live device plus queue: use gpu::Rp1Submitter on the window build; CPU chain is the parity oracle".into(),
        ))
    }

    /// Chain with the exposure stops replaced (the auto-exposure feed):
    /// validates the stops through `ExposureConfig` so the returned chain
    /// always passes `validate`. Bloom, white and grade stay untouched.
    pub fn with_exposure_ev(&self, ev: f32) -> Result<PostChain, PostError> {
        let exposure = ExposureConfig { exposure_ev: ev };
        exposure.mult()?;
        Ok(PostChain { exposure, ..*self })
    }
}

/// RL1/RL2 taps the post chain reads: the light receipts of the frame
/// being posted. Their pass ms flow into [`FramePostReceipt`], so the
/// frame report names every light and post cost together.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PostLights {
    /// RL1 cascaded sun shadow pass ms.
    pub sun_ms: f64,
    /// RL1 torch cluster pass ms.
    pub cluster_ms: f64,
    /// RL2 IBL bake ms.
    pub ibl_ms: f64,
}

impl PostLights {
    /// Read the RL1/RL2 receipts of this frame.
    pub fn from_receipts(
        lights: &crate::shadow_cluster::FrameLightsReceipt,
        ibl: &crate::ibl::IblReceipt,
    ) -> Self {
        Self {
            sun_ms: lights.sun.ms,
            cluster_ms: lights.cluster.ms,
            ibl_ms: ibl.ms,
        }
    }
}

/// Post chain output: display-space pixels plus the frame receipt.
#[derive(Clone, PartialEq, Debug)]
pub struct PostOutput {
    /// Display-space `0..=1` pixels, row-major `width * height`.
    pub ldr: Vec<[f32; 3]>,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Per-pass receipts plus the RL1/RL2 light ms.
    pub receipt: FramePostReceipt,
}

fn luminance(p: [f32; 3]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

/// Soft-knee bloom extract: below `threshold - knee` nothing passes, above
/// `threshold + knee` everything passes, with a quadratic knee between.
fn bloom_weight(luma: f32, threshold: f32, knee: f32) -> f32 {
    if luma <= threshold - knee {
        0.0
    } else if knee <= 0.0 || luma >= threshold + knee {
        1.0
    } else {
        let t = (luma - (threshold - knee)) / (2.0 * knee);
        t * t
    }
}

/// SIMD slice: bright-extract over one HDR frame. The scalar loop is
/// canonical (std-only, runs everywhere); the `v1` glam loop is the SIMD
/// path (lane-wise `Vec3` multiply, same op order, bit-identical pixels).
/// Simulation code is untouched (presentation pixels only, never the tick
/// hash). Each change is A/B-measured by `bright_extract_ab` below.
fn bright_extract_scalar(hdr: &[[f32; 3]], threshold: f32, knee: f32) -> Vec<[f32; 3]> {
    let mut bright = Vec::with_capacity(hdr.len());
    for p in hdr {
        let w = bloom_weight(luminance(*p), threshold, knee);
        bright.push([p[0] * w, p[1] * w, p[2] * w]);
    }
    bright
}

/// SIMD bright-extract on `glam::Vec3` (`v1` only, no new dependency: glam
/// is already the render optional dep). Same op order as the scalar loop.
#[cfg(feature = "v1")]
fn bright_extract_glam(hdr: &[[f32; 3]], threshold: f32, knee: f32) -> Vec<[f32; 3]> {
    let mut bright = Vec::with_capacity(hdr.len());
    for p in hdr {
        let w = bloom_weight(luminance(*p), threshold, knee);
        let lit = glam::Vec3::from_array(*p) * w;
        bright.push(lit.to_array());
    }
    bright
}

/// Dispatcher: glam SIMD under `v1`, scalar fallback otherwise.
fn bright_extract(hdr: &[[f32; 3]], threshold: f32, knee: f32) -> Vec<[f32; 3]> {
    #[cfg(feature = "v1")]
    {
        bright_extract_glam(hdr, threshold, knee)
    }
    #[cfg(not(feature = "v1"))]
    {
        bright_extract_scalar(hdr, threshold, knee)
    }
}

/// A/B timing of the two extract paths in ms (wall clock only, evidence
/// for the speed receipt; pixels compared separately for bit-equality).
#[cfg(feature = "v1")]
pub fn bright_extract_ab(hdr: &[[f32; 3]], threshold: f32, knee: f32) -> (f64, f64) {
    let start = Instant::now();
    std::hint::black_box(bright_extract_scalar(hdr, threshold, knee));
    let scalar_ms = start.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    std::hint::black_box(bright_extract_glam(hdr, threshold, knee));
    let glam_ms = start.elapsed().as_secs_f64() * 1000.0;
    (scalar_ms, glam_ms)
}
fn downsample_box(src: &[[f32; 3]], w: u32, h: u32) -> (Vec<[f32; 3]>, u32, u32) {
    let dw = w.div_ceil(2).max(1);
    let dh = h.div_ceil(2).max(1);
    let mut out = Vec::with_capacity(dw as usize * dh as usize);
    for y in 0..dh {
        for x in 0..dw {
            let mut acc = [0.0f64; 3];
            let mut n = 0.0f64;
            for dy in 0..2 {
                for dx in 0..2 {
                    let sx = x * 2 + dx;
                    let sy = y * 2 + dy;
                    if sx < w && sy < h {
                        let p = src[sy as usize * w as usize + sx as usize];
                        acc[0] += f64::from(p[0]);
                        acc[1] += f64::from(p[1]);
                        acc[2] += f64::from(p[2]);
                        n += 1.0;
                    }
                }
            }
            out.push([
                (acc[0] / n) as f32,
                (acc[1] / n) as f32,
                (acc[2] / n) as f32,
            ]);
        }
    }
    (out, dw, dh)
}

/// Apply bloom: soft-knee extract, box-blur mip chain, bilinear add-back.
/// Returns the flared HDR frame.
pub fn apply_bloom(
    hdr: &[[f32; 3]],
    width: u32,
    height: u32,
    cfg: &BloomConfig,
) -> Result<Vec<[f32; 3]>, PostError> {
    cfg.validate()?;
    check_image(hdr, width, height, "bloom hdr")?;
    let bright = bright_extract(hdr, cfg.threshold, cfg.knee);
    // Blur chain: each level halves the previous bright mip.
    let mut chain: Vec<(Vec<[f32; 3]>, u32, u32)> = Vec::new();
    let (mut level, mut lw, mut lh) = (bright, width, height);
    for _ in 0..cfg.levels {
        let (next, nw, nh) = downsample_box(&level, lw, lh);
        level = next;
        lw = nw;
        lh = nh;
        chain.push((level.clone(), lw, lh));
        if lw == 1 && lh == 1 {
            break;
        }
    }
    // Add every blurred level back, spread evenly over the intensity.
    let per = if chain.is_empty() {
        0.0
    } else {
        cfg.intensity / chain.len() as f32
    };
    let mut out: Vec<[f32; 3]> = hdr.to_vec();
    for (mip, mw, mh) in &chain {
        let sx = *mw as f32 / width as f32;
        let sy = *mh as f32 / height as f32;
        for y in 0..height as usize {
            for x in 0..width as usize {
                let fx = (x as f32 + 0.5) * sx - 0.5;
                let fy = (y as f32 + 0.5) * sy - 0.5;
                let b = sample_bilinear(mip, *mw, *mh, fx, fy);
                let o = &mut out[y * width as usize + x];
                o[0] += b[0] * per;
                o[1] += b[1] * per;
                o[2] += b[2] * per;
            }
        }
    }
    Ok(out)
}

/// Exposure multiplier helper (`2^ev`); validates the stop range.
pub fn exposure_mult(ev: f32) -> Result<f32, PostError> {
    ExposureConfig { exposure_ev: ev }.mult()
}

// ---------------------------------------------------------------------------
// Auto-exposure: log-luma histogram with percentile trim plus
// slew-rate-limited adaptation.
//
// The presenter holds one `AutoExposure` across frames (the match loop
// owns it, the capture bundle reads its `ev` for the manifest): each
// frame ticks the state from the pre-exposure HDR with the frame dt in
// seconds, and the adapted stops feed the chain through
// `PostChain::with_exposure_ev`. The adapted value also rides the
// existing exposure uniform on the device path, so no new shader is
// needed for parity. Pixels stay presentation (never the tick hash).
// Budget: the histogram samples large frames on a fixed stride (one
// sample per 4x4 block, full sampling under 64 px sides) into stack
// `[f32; 64]` bins (no heap per frame, proven by
// `auto_exposure_costs_no_heap`); the tick is O(64); `run_chain_auto`
// folds the adapt ms into the exposure receipt slot, so the posted
// full-post bar does not move. Stride sampling can alias pathological
// perfectly periodic patterns; the percentile trim still excludes
// sparse highlights either way.
// ---------------------------------------------------------------------------

/// Histogram bins over log2 luma (the accept pins 64).
pub const AUTO_HIST_BINS: usize = 64;
/// Log2-luma floor of bin 0 in stops (deep shadow; non-positive luma
/// lands here so black never poisons the log).
pub const AUTO_LOG_MIN_EV: f32 = -12.0;
/// Log2-luma ceiling of the top bin in stops (sun and torch highlights).
pub const AUTO_LOG_MAX_EV: f32 = 8.0;
/// Low trim fraction: texels below the 10th percentile never move exposure.
pub const AUTO_TRIM_LO: f32 = 0.10;
/// High trim fraction: texels above the 95th percentile never move
/// exposure, so small bright lights cannot pump the frame.
pub const AUTO_TRIM_HI: f32 = 0.95;
/// Adaptation slew in stops per second: full daylight steps settle in
/// about a second, frame-to-frame steps stay far under the flicker gate.
pub const AUTO_ADAPT_EV_PER_S: f32 = 3.0;
/// Key luma the trimmed mean lands on (the grade mid-grey).
pub const AUTO_KEY_LUMA: f32 = GRADE_MID_GREY;
/// Flicker gate in stops: adapted steps between frames stay under this.
pub const AUTO_FLICKER_MAX_EV: f32 = 0.5;
/// Win gate: adapted exposure must cut the fixed-exposure clipped share
/// by at least this fraction on the day-sweep fixture.
pub const AUTO_CLIP_IMPROVE_MIN: f32 = 0.15;
/// Clipped-highlight threshold in display space: any channel at or above
/// this counts as saturated.
pub const AUTO_CLIP_LUMA: f32 = 0.99;

/// Log bin width in stops (20 over 64, exactly 0.3125).
const AUTO_BIN_WIDTH_EV: f32 = (AUTO_LOG_MAX_EV - AUTO_LOG_MIN_EV) / 64.0;
/// Halving steps locating one of [`AUTO_HIST_BINS`] bins (2 to the 6th
/// spans 64 bins).
const AUTO_HIST_STEPS: usize = 6;
/// Widest histogram stride in texels: large frames sample one texel per
/// 4x4 block, small frames sample every texel (see [`hist_stride`]).
const AUTO_HIST_STRIDE_MAX: u32 = 4;

/// Auto-exposure tuning: trim band, key, slew and clamp range. The clamp
/// range sits inside the chain `-8..=+8` stops so every adapted value
/// feeds `PostChain::with_exposure_ev` without a second validation.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AutoExposureConfig {
    /// Low trim fraction in `0..=1`.
    pub trim_lo: f32,
    /// High trim fraction in `0..=1`, above `trim_lo`.
    pub trim_hi: f32,
    /// Key luma in linear HDR the trimmed mean lands on, `> 0`.
    pub key_luma: f32,
    /// Adaptation slew in stops per second, `> 0`.
    pub adapt_ev_per_s: f32,
    /// Lowest adapted stops, inside `-8..=+8`.
    pub min_ev: f32,
    /// Highest adapted stops, inside `-8..=+8`, at or above `min_ev`.
    pub max_ev: f32,
    /// Starting adapted stops, inside `min_ev..=max_ev`.
    pub start_ev: f32,
}

impl Default for AutoExposureConfig {
    fn default() -> Self {
        Self {
            trim_lo: AUTO_TRIM_LO,
            trim_hi: AUTO_TRIM_HI,
            key_luma: AUTO_KEY_LUMA,
            adapt_ev_per_s: AUTO_ADAPT_EV_PER_S,
            min_ev: -8.0,
            max_ev: 8.0,
            start_ev: 0.0,
        }
    }
}

impl AutoExposureConfig {
    /// Validate every field before the state can run.
    pub fn validate(&self) -> Result<(), PostError> {
        if !self.trim_lo.is_finite() || !(0.0..=1.0).contains(&self.trim_lo) {
            return Err(PostError::Invalid(format!(
                "auto-exposure trim_lo {} outside 0..=1",
                self.trim_lo
            )));
        }
        if !self.trim_hi.is_finite() || !(0.0..=1.0).contains(&self.trim_hi) {
            return Err(PostError::Invalid(format!(
                "auto-exposure trim_hi {} outside 0..=1",
                self.trim_hi
            )));
        }
        if self.trim_lo >= self.trim_hi {
            return Err(PostError::Invalid(format!(
                "auto-exposure trim {}..={} is empty",
                self.trim_lo, self.trim_hi
            )));
        }
        if !self.key_luma.is_finite() || self.key_luma <= 0.0 {
            return Err(PostError::Invalid(format!(
                "auto-exposure key {} is not positive finite",
                self.key_luma
            )));
        }
        if !self.adapt_ev_per_s.is_finite() || self.adapt_ev_per_s <= 0.0 {
            return Err(PostError::Invalid(format!(
                "auto-exposure slew {} is not positive finite",
                self.adapt_ev_per_s
            )));
        }
        for (value, name) in [
            (self.min_ev, "min_ev"),
            (self.max_ev, "max_ev"),
            (self.start_ev, "start_ev"),
        ] {
            if !value.is_finite() || !(-8.0..=8.0).contains(&value) {
                return Err(PostError::Invalid(format!(
                    "auto-exposure {name} {value} outside -8..=+8 stops"
                )));
            }
        }
        if self.min_ev > self.max_ev {
            return Err(PostError::Invalid(format!(
                "auto-exposure range {}..={} is inverted",
                self.min_ev, self.max_ev
            )));
        }
        if self.start_ev < self.min_ev || self.start_ev > self.max_ev {
            return Err(PostError::Invalid(format!(
                "auto-exposure start {} outside {}..={}",
                self.start_ev, self.min_ev, self.max_ev
            )));
        }
        Ok(())
    }
}

/// Log2-luma histogram over one HDR frame: stack-only (`[f32; 64]`, no
/// heap), fixed bin order, so the same frame bins bit-identically twice.
/// Counts ride f32 (exact below 2 to the 24th texels; the 2048 bound
/// stays far under it) so the trim math converts nothing.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ExposureHistogram {
    /// Per-bin sample counts, bin 0 at [`AUTO_LOG_MIN_EV`].
    pub bins: [f32; AUTO_HIST_BINS],
    /// Samples binned (equals `width * height` at stride 1).
    pub texels: f32,
}

/// Histogram stride in texels for `width` by `height`: full sampling on
/// small frames, up to one sample per 4x4 block on large ones. A pure
/// function of the dims, so the pattern never varies frame to frame.
fn hist_stride(width: u32, height: u32) -> u32 {
    (width.min(height) / 16).clamp(1, AUTO_HIST_STRIDE_MAX)
}

/// Bin index for a scaled log-luma in `[0, 64]`: six halving steps, most
/// significant bit first, so no float-to-int conversion appears on the
/// hot path. Exact for finite inputs; 64.0 still lands on bin 63.
fn bin_for_scaled(scaled: f32) -> usize {
    let mut bin = 0usize;
    let mut base = 0.0f32;
    let mut half = 32.0f32;
    for _ in 0..AUTO_HIST_STEPS {
        bin <<= 1;
        if scaled >= base + half {
            bin += 1;
            base += half;
        }
        half *= 0.5;
    }
    bin.min(AUTO_HIST_BINS - 1)
}

/// Bin one HDR frame by log2 luma over the [`hist_stride`] pattern.
/// Non-positive luma lands in bin 0; the top edge clamps into the last
/// bin. Validates dims, length and finiteness exactly like the chain
/// inputs.
pub fn histogram_log_luma(
    hdr: &[[f32; 3]],
    width: u32,
    height: u32,
) -> Result<ExposureHistogram, PostError> {
    check_image(hdr, width, height, "auto-exposure hdr")?;
    let stride = hist_stride(width, height);
    let mut bins = [0.0f32; AUTO_HIST_BINS];
    let mut texels = 0.0f32;
    let mut idx = 0usize;
    for y in 0..height {
        for x in 0..width {
            if x % stride == 0 && y % stride == 0 {
                let luma = luminance(hdr[idx]).max(0.0);
                let log = if luma <= 0.0 {
                    AUTO_LOG_MIN_EV
                } else {
                    luma.log2().clamp(AUTO_LOG_MIN_EV, AUTO_LOG_MAX_EV)
                };
                bins[bin_for_scaled((log - AUTO_LOG_MIN_EV) / AUTO_BIN_WIDTH_EV)] += 1.0;
                texels += 1.0;
            }
            idx += 1;
        }
    }
    Ok(ExposureHistogram { bins, texels })
}

/// Mean log2 luma over the trimmed percentile band, in stops: samples
/// below `trim_lo` and above `trim_hi` never move the mean, so dark
/// borders and torch highlights cannot pump the exposure. Bin centers
/// stand in for sample values (half-bin quantization at most).
pub fn trimmed_log_mean(
    hist: &ExposureHistogram,
    trim_lo: f32,
    trim_hi: f32,
) -> Result<f32, PostError> {
    if !trim_lo.is_finite() || !(0.0..=1.0).contains(&trim_lo) {
        return Err(PostError::Invalid(format!(
            "auto-exposure trim_lo {trim_lo} outside 0..=1"
        )));
    }
    if !trim_hi.is_finite() || !(0.0..=1.0).contains(&trim_hi) {
        return Err(PostError::Invalid(format!(
            "auto-exposure trim_hi {trim_hi} outside 0..=1"
        )));
    }
    if trim_lo >= trim_hi {
        return Err(PostError::Invalid(format!(
            "auto-exposure trim {trim_lo}..={trim_hi} is empty"
        )));
    }
    if hist.texels <= 0.0 {
        return Err(PostError::Invalid(
            "auto-exposure histogram is empty".into(),
        ));
    }
    let lo_rank = trim_lo * hist.texels;
    let hi_rank = trim_hi * hist.texels;
    let mut acc = 0.0f32;
    let mut sum = 0.0f32;
    let mut weight = 0.0f32;
    let mut center = AUTO_LOG_MIN_EV + 0.5 * AUTO_BIN_WIDTH_EV;
    for count in hist.bins.iter() {
        if *count > 0.0 {
            let keep = (acc + *count).min(hi_rank) - acc.max(lo_rank);
            if keep > 0.0 {
                sum += keep * center;
                weight += keep;
            }
        }
        acc += *count;
        center += AUTO_BIN_WIDTH_EV;
    }
    if weight <= 0.0 {
        return Err(PostError::Invalid(format!(
            "auto-exposure trim {trim_lo}..={trim_hi} keeps no samples"
        )));
    }
    Ok(sum / weight)
}

/// Stops to add so `mean_log_luma` lands on `key_luma`: positive when the
/// frame is darker than key (brighten), negative when brighter (darken).
/// Feeds the chain exposure directly; units are stops.
pub fn target_ev_for_mean(mean_log_luma: f32, key_luma: f32) -> Result<f32, PostError> {
    if !mean_log_luma.is_finite() {
        return Err(PostError::Invalid(format!(
            "auto-exposure mean {mean_log_luma} is not finite"
        )));
    }
    if !key_luma.is_finite() || key_luma <= 0.0 {
        return Err(PostError::Invalid(format!(
            "auto-exposure key {key_luma} is not positive finite"
        )));
    }
    Ok(key_luma.log2() - mean_log_luma)
}

/// Adapted exposure state: one per presenter loop, ticked every frame
/// with the frame dt in seconds. Same frames plus same dts give
/// bit-identical stops twice; wall clock never enters (the caller passes
/// dt, so replays and captures agree exactly).
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct AutoExposure {
    /// Tuning behind this state.
    config: AutoExposureConfig,
    /// Currently adapted stops, always inside `min_ev..=max_ev`.
    ev: f32,
}

impl AutoExposure {
    /// Build from a validated config, starting at `start_ev`.
    pub fn new(config: AutoExposureConfig) -> Result<Self, PostError> {
        config.validate()?;
        Ok(Self {
            config,
            ev: config.start_ev,
        })
    }

    /// Tuning behind this state.
    pub fn config(&self) -> AutoExposureConfig {
        self.config
    }

    /// Currently adapted stops (feed the chain or quote in the manifest).
    pub fn ev(&self) -> f32 {
        self.ev
    }

    /// Linear multiplier for the adapted stops (`2^ev`); always finite
    /// because the state clamps into `-8..=+8`.
    pub fn mult(&self) -> f32 {
        self.ev.exp2()
    }

    /// Histogram the pre-exposure HDR, retarget the trimmed mean onto key
    /// and slew toward it at the configured stops per second. A `dt_s` of
    /// 0 holds; negative or non-finite dts fail loudly. Returns the
    /// adapted stops. Allocates nothing (stack histogram, scalar walk).
    pub fn tick(
        &mut self,
        hdr: &[[f32; 3]],
        width: u32,
        height: u32,
        dt_s: f32,
    ) -> Result<f32, PostError> {
        if !dt_s.is_finite() || dt_s < 0.0 {
            return Err(PostError::Invalid(
                "auto-exposure dt_s is not finite non-negative".into(),
            ));
        }
        let hist = histogram_log_luma(hdr, width, height)?;
        let mean = trimmed_log_mean(&hist, self.config.trim_lo, self.config.trim_hi)?;
        let target = target_ev_for_mean(mean, self.config.key_luma)?
            .clamp(self.config.min_ev, self.config.max_ev);
        let step = self.config.adapt_ev_per_s * dt_s;
        let delta = (target - self.ev).clamp(-step, step);
        self.ev = (self.ev + delta).clamp(self.config.min_ev, self.config.max_ev);
        Ok(self.ev)
    }
}

impl Default for AutoExposure {
    /// Default tuning starting at 0 stops (fixed-exposure parity on day one).
    fn default() -> Self {
        let config = AutoExposureConfig::default();
        Self {
            config,
            ev: config.start_ev,
        }
    }
}

/// Narkowicz ACES fit (textbook): cheap, monotonic, `0 -> 0`, `1 -> ~0.8`,
/// large inputs saturate at 1. Same formulation as the `shadow_post` path
/// (the `v1` parity test pins bit-equality); the V1 receipt uses the
/// Bevy-ported `ACESFitted` instead, a different formulation, so the two
/// tonemaps agree only qualitatively — Q1 reference-GPU clips stay
/// operator-verified.
pub fn aces_fitted(x: f32) -> f32 {
    let x = x.max(0.0);
    ((x * (2.51 * x + 0.03)) / (x * (2.43 * x + 0.59) + 0.14)).clamp(0.0, 1.0)
}

/// ACES tonemap with white normalization: exposure-scaled fit over the fit
/// at `white`, clamped to `0..=1`.
pub fn tonemap_aces(c: [f32; 3], exposure: f32, white: f32) -> Result<[f32; 3], PostError> {
    if !exposure.is_finite() || exposure <= 0.0 {
        return Err(PostError::Invalid(format!(
            "aces exposure {exposure} is not positive finite"
        )));
    }
    if !white.is_finite() || white <= 0.0 {
        return Err(PostError::Invalid(format!(
            "aces white {white} is not positive finite"
        )));
    }
    if !c.iter().all(|v| v.is_finite()) {
        return Err(PostError::Invalid("aces color is not finite".into()));
    }
    let w = aces_fitted(white).max(1e-4);
    Ok([
        (aces_fitted(c[0] * exposure) / w).clamp(0.0, 1.0),
        (aces_fitted(c[1] * exposure) / w).clamp(0.0, 1.0),
        (aces_fitted(c[2] * exposure) / w).clamp(0.0, 1.0),
    ])
}

/// Display-space grade: contrast around mid-grey, saturation mix toward
/// luma, then gain plus lift. Output stays `>= 0`.
pub fn apply_grade(c: [f32; 3], cfg: &GradeConfig) -> Result<[f32; 3], PostError> {
    cfg.validate()?;
    if !c.iter().all(|v| v.is_finite()) {
        return Err(PostError::Invalid("grade color is not finite".into()));
    }
    let mut out = [
        (c[0] - GRADE_MID_GREY) * cfg.contrast + GRADE_MID_GREY,
        (c[1] - GRADE_MID_GREY) * cfg.contrast + GRADE_MID_GREY,
        (c[2] - GRADE_MID_GREY) * cfg.contrast + GRADE_MID_GREY,
    ];
    let luma = luminance(out);
    let s = cfg.saturation.clamp(0.0, 4.0);
    for (o, (g, l)) in out.iter_mut().zip(cfg.gain.iter().zip(cfg.lift.iter())) {
        *o = ((luma + (*o - luma) * s) * *g + *l).max(0.0);
    }
    Ok(out)
}

/// Run the full chain over one HDR frame: bloom, exposure, ACES, grading.
/// Reads the RL1/RL2 receipts of this frame into the output receipt. Same
/// complete inputs give bit-identical pixels; only `ms` varies.
pub fn run_chain(
    hdr: &[[f32; 3]],
    width: u32,
    height: u32,
    chain: &PostChain,
    lights: &PostLights,
) -> Result<PostOutput, PostError> {
    chain.validate()?;
    check_image(hdr, width, height, "post hdr")?;
    run_chain_core(hdr, width, height, chain, lights)
}

/// Validated chain core: bloom, exposure, ACES, grading over inputs the
/// caller already checked. Exists so the auto path validates once (the
/// tick checks the frame, the core never rescans it); behavior past the
/// checks is exactly `run_chain`.
fn run_chain_core(
    hdr: &[[f32; 3]],
    width: u32,
    height: u32,
    chain: &PostChain,
    lights: &PostLights,
) -> Result<PostOutput, PostError> {
    let pixels = u64::from(width) * u64::from(height);

    let started = Instant::now();
    let flared = apply_bloom(hdr, width, height, &chain.bloom)?;
    let bloom_ms = started.elapsed().as_secs_f64() * 1000.0;

    let started = Instant::now();
    let mult = chain.exposure.mult()?;
    let exposed: Vec<[f32; 3]> = flared
        .iter()
        .map(|p| [p[0] * mult, p[1] * mult, p[2] * mult])
        .collect();
    let exposure_ms = started.elapsed().as_secs_f64() * 1000.0;

    let started = Instant::now();
    let mut toned = Vec::with_capacity(exposed.len());
    for p in &exposed {
        toned.push(tonemap_aces(*p, 1.0, chain.aces_white)?);
    }
    let aces_ms = started.elapsed().as_secs_f64() * 1000.0;

    let started = Instant::now();
    let mut ldr = Vec::with_capacity(toned.len());
    for p in &toned {
        ldr.push(apply_grade(*p, &chain.grade)?);
    }
    let grade_ms = started.elapsed().as_secs_f64() * 1000.0;

    Ok(PostOutput {
        ldr,
        width,
        height,
        receipt: FramePostReceipt {
            taa: receipt("taa-resolve", 0, 0.0),
            upsample: receipt("taau", 0, 0.0),
            bloom: receipt("bloom", pixels, bloom_ms),
            exposure: receipt("exposure", pixels, exposure_ms),
            aces: receipt("aces", pixels, aces_ms),
            grade: receipt("grade", pixels, grade_ms),
            sun_ms: lights.sun_ms,
            cluster_ms: lights.cluster_ms,
            ibl_ms: lights.ibl_ms,
        },
    })
}

/// TAA resolve at the render size plus TAAU to the presentation size, the
/// shared front of `run_frame` and `run_frame_auto`. Returns the upsampled
/// pre-exposure HDR (the buffer the auto-exposure histograms) plus the two
/// pass timings; receipts stay with the callers.
#[expect(
    clippy::too_many_arguments,
    reason = "wide signature kept until its inputs move into a params struct"
)]
fn resolve_and_upsample(
    current: &[[f32; 3]],
    history: &[[f32; 3]],
    motion: &MotionField,
    render_w: u32,
    render_h: u32,
    present_w: u32,
    present_h: u32,
    taa: &TaaParams,
) -> Result<(Vec<[f32; 3]>, f64, f64), PostError> {
    taa.validate()?;
    let started = Instant::now();
    let resolved = taa_resolve(current, history, motion, render_w, render_h, taa)?;
    let taa_ms = started.elapsed().as_secs_f64() * 1000.0;
    let started = Instant::now();
    let upsampled = taa_upsample(&resolved, render_w, render_h, present_w, present_h)?;
    let upsample_ms = started.elapsed().as_secs_f64() * 1000.0;
    Ok((upsampled, taa_ms, upsample_ms))
}

/// Full frame: TAA resolve at the render size, temporal upsample to the
/// presentation size, then the post chain. Same complete inputs give
/// bit-identical pixels; only `ms` varies.
#[expect(
    clippy::too_many_arguments,
    reason = "wide signature kept until its inputs move into a params struct"
)]
pub fn run_frame(
    current: &[[f32; 3]],
    history: &[[f32; 3]],
    motion: &MotionField,
    render_w: u32,
    render_h: u32,
    present_w: u32,
    present_h: u32,
    taa: &TaaParams,
    chain: &PostChain,
    lights: &PostLights,
) -> Result<PostOutput, PostError> {
    taa.validate()?;
    chain.validate()?;
    let (upsampled, taa_ms, upsample_ms) = resolve_and_upsample(
        current, history, motion, render_w, render_h, present_w, present_h, taa,
    )?;

    let mut out = run_chain(&upsampled, present_w, present_h, chain, lights)?;
    let pixels = u64::from(present_w) * u64::from(present_h);
    out.receipt.taa = receipt("taa", u64::from(render_w) * u64::from(render_h), taa_ms);
    out.receipt.upsample = receipt("taau", pixels, upsample_ms);
    Ok(out)
}

/// Run the chain under adapted exposure: tick the state from this HDR
/// with the frame dt in seconds, swap the adapted stops into the chain
/// and run it. The tick already validated the frame, so the core runs
/// without a second full-frame scan. The histogram-plus-adapt ms folds
/// into the exposure receipt slot, so the frame report prices adaptation
/// where it ran. Same complete inputs (plus same starting state and dt)
/// give bit-identical pixels; only `ms` varies.
pub fn run_chain_auto(
    hdr: &[[f32; 3]],
    width: u32,
    height: u32,
    chain: &PostChain,
    auto_state: &mut AutoExposure,
    dt_s: f32,
    lights: &PostLights,
) -> Result<PostOutput, PostError> {
    let adapt_started = Instant::now();
    let ev = auto_state.tick(hdr, width, height, dt_s)?;
    let adapt_ms = adapt_started.elapsed().as_secs_f64() * 1000.0;
    let adapted = chain.with_exposure_ev(ev)?;
    adapted.validate()?;
    let mut out = run_chain_core(hdr, width, height, &adapted, lights)?;
    out.receipt.exposure.ms += adapt_ms;
    Ok(out)
}

#[expect(
    clippy::too_many_arguments,
    reason = "frame-shaped args; struct when inputs move"
)]
/// Full frame under adapted exposure: TAA resolve at the render size,
/// temporal upsample to the presentation size, then the auto-exposed
/// chain ticked from the upsampled HDR. This is the match-loop frame the
/// presenter runs (the capture bundle reads the same LDR plus receipts):
/// exposure tracks the scene while TAA, bloom, tonemap and grade behave
/// exactly as `run_frame`. Same complete inputs (plus same starting
/// state and dt) give bit-identical pixels; only `ms` varies.
pub fn run_frame_auto(
    current: &[[f32; 3]],
    history: &[[f32; 3]],
    motion: &MotionField,
    render_w: u32,
    render_h: u32,
    present_w: u32,
    present_h: u32,
    taa: &TaaParams,
    chain: &PostChain,
    auto_state: &mut AutoExposure,
    dt_s: f32,
    lights: &PostLights,
) -> Result<PostOutput, PostError> {
    taa.validate()?;
    chain.validate()?;
    let (upsampled, taa_ms, upsample_ms) = resolve_and_upsample(
        current, history, motion, render_w, render_h, present_w, present_h, taa,
    )?;

    let mut out = run_chain_auto(
        &upsampled, present_w, present_h, chain, auto_state, dt_s, lights,
    )?;
    let pixels = u64::from(present_w) * u64::from(present_h);
    out.receipt.taa = receipt("taa", u64::from(render_w) * u64::from(render_h), taa_ms);
    out.receipt.upsample = receipt("taau", pixels, upsample_ms);
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
// RP1 graph passes: the six post stages as RA2 data for WIN1/the viewport.
// The `window` `gpu::Rp1Submitter` records the same six in compiled order
// on a live device; this builder is its CPU-side declaration so order,
// aliasing and barriers come from RA2, never raw.
// ---------------------------------------------------------------------------

/// RP1 TAA resolve pass name on the frame graph.
pub const RP1_PASS_TAA: &str = "rp1-taa";
/// RP1 temporal upsample pass name on the frame graph.
pub const RP1_PASS_TAAU: &str = "rp1-taau";
/// RP1 bloom pass name on the frame graph.
pub const RP1_PASS_BLOOM: &str = "rp1-bloom";
/// RP1 exposure pass name on the frame graph.
pub const RP1_PASS_EXPOSURE: &str = "rp1-exposure";
/// RP1 ACES pass name on the frame graph.
pub const RP1_PASS_ACES: &str = "rp1-aces";
/// RP1 grading pass name on the frame graph.
pub const RP1_PASS_GRADE: &str = "rp1-grade";
/// Bytes per HDR texel in the transient plan (`Rgba32Float` history).
pub const RP1_HISTORY_BYTES_PER_TEXEL: u64 = 16;
/// Bytes per LDR staging texel (`Rgba16Float` post target).
pub const RP1_POST_BYTES_PER_TEXEL: u64 = 8;

/// The six RP1 pass names in execution order.
pub fn rp1_pass_names() -> [&'static str; 6] {
    [
        RP1_PASS_TAA,
        RP1_PASS_TAAU,
        RP1_PASS_BLOOM,
        RP1_PASS_EXPOSURE,
        RP1_PASS_ACES,
        RP1_PASS_GRADE,
    ]
}

/// Transient backing estimate in bytes for an RP1 frame: history plus
/// resolved (render size, 16 B/texel) plus the present-size stage plus
/// the bloom mip chain (present size halved per level, 16 B/texel).
/// Integer math only; used by the WIN1 budget check before submit.
pub fn rp1_transient_bytes(
    render_w: u32,
    render_h: u32,
    present_w: u32,
    present_h: u32,
    bloom_levels: u32,
) -> Result<u64, PostError> {
    check_dims(render_w, render_h)?;
    check_dims(present_w, present_h)?;
    if bloom_levels == 0 || bloom_levels > MAX_BLOOM_LEVELS {
        return Err(PostError::Invalid(format!(
            "rp1 bloom levels {bloom_levels} outside 1..={MAX_BLOOM_LEVELS}"
        )));
    }
    let render = render_w as u64 * render_h as u64;
    let mut total = render * RP1_HISTORY_BYTES_PER_TEXEL * 2;
    total = total.saturating_add(present_w as u64 * present_h as u64 * RP1_POST_BYTES_PER_TEXEL);
    let (mut mw, mut mh) = (present_w as u64, present_h as u64);
    for _ in 0..bloom_levels {
        mw = mw.div_ceil(2).max(1);
        mh = mh.div_ceil(2).max(1);
        total = total.saturating_add(mw * mh * RP1_HISTORY_BYTES_PER_TEXEL);
        if mw == 1 && mh == 1 {
            break;
        }
    }
    Ok(total)
}

/// Declare the six RP1 passes on a fresh RA2 frame graph for the WIN1
/// match frame (and the ED1 viewport on the same shape): TAA at the
/// render size, TAAU to the presentation size, then the four chain
/// stages. Resource flow infers `taa -> taau -> bloom -> exposure ->
/// aces -> grade`; explicit edges pin the same order so a resource-flow
/// change can never reorder the chain. Validates every dimension and the
/// chain config before adding any pass.
pub fn build_rp1_graph(
    render_w: u32,
    render_h: u32,
    present_w: u32,
    present_h: u32,
    chain: &PostChain,
) -> Result<crate::graph::FrameGraph, PostError> {
    chain.validate()?;
    check_dims(render_w, render_h)?;
    check_dims(present_w, present_h)?;
    let render_px = render_w as u64 * render_h as u64;
    let present_px = present_w as u64 * present_h as u64;
    let mut graph = crate::graph::FrameGraph::new();
    let taa = graph.add_pass(
        crate::graph::PassDesc::new(RP1_PASS_TAA)
            .input("rp1-current")
            .input("rp1-history")
            .input("rp1-motion")
            .input("rp1-jitter")
            .output("rp1-resolved")
            .transient("rp1-history-bytes", render_px * RP1_HISTORY_BYTES_PER_TEXEL)
            .work(1, 1),
    );
    let taau = graph.add_pass(
        crate::graph::PassDesc::new(RP1_PASS_TAAU)
            .input("rp1-resolved")
            .output("rp1-upsampled")
            .work(1, 1),
    );
    let bloom = graph.add_pass(
        crate::graph::PassDesc::new(RP1_PASS_BLOOM)
            .input("rp1-upsampled")
            .output("rp1-flared")
            .transient("rp1-bloom-bytes", present_px * RP1_HISTORY_BYTES_PER_TEXEL)
            .work(1, 1),
    );
    let exposure = graph.add_pass(
        crate::graph::PassDesc::new(RP1_PASS_EXPOSURE)
            .input("rp1-flared")
            .output("rp1-exposed")
            .work(1, 1),
    );
    let aces = graph.add_pass(
        crate::graph::PassDesc::new(RP1_PASS_ACES)
            .input("rp1-exposed")
            .output("rp1-toned")
            .work(1, 1),
    );
    let grade = graph.add_pass(
        crate::graph::PassDesc::new(RP1_PASS_GRADE)
            .input("rp1-toned")
            .output("rp1-ldr")
            .work(1, 1),
    );
    let order = [
        (taa, taau),
        (taau, bloom),
        (bloom, exposure),
        (exposure, aces),
        (aces, grade),
    ];
    for (from, to) in order {
        graph
            .add_edge(from, to)
            .map_err(|e| PostError::Invalid(format!("rp1 graph edge: {e}")))?;
    }
    Ok(graph)
}

/// WIN1 wiring: validate the match frame sizes plus the chain, declare
/// the six RP1 graph passes, compile, and check the compiled order names
/// the six stages in order. Returns the compiled graph the window submit
/// records on the device. Fails loudly on bad sizes or config.
pub fn compile_rp1_for_present(
    render_w: u32,
    render_h: u32,
    present_w: u32,
    present_h: u32,
    chain: &PostChain,
) -> Result<crate::graph::CompiledGraph, PostError> {
    let graph = build_rp1_graph(render_w, render_h, present_w, present_h, chain)?;
    let compiled = graph
        .compile()
        .map_err(|e| PostError::Invalid(format!("rp1 graph compile: {e}")))?;
    let names: Vec<&str> = compiled
        .order()
        .iter()
        .filter_map(|id| compiled.pass(*id).map(|p| p.name.as_str()))
        .collect();
    let want = rp1_pass_names();
    if names != want {
        return Err(PostError::Invalid(format!(
            "rp1 compiled order {names:?} differs from {want:?}"
        )));
    }
    Ok(compiled)
}

// ---------------------------------------------------------------------------
// RP5 scale seam: post/TAAU production wiring over the RP4 TAA-off graph.
// The `aa_scale` API stays frozen: this seam only feeds the live
// `ScaleController` (hysteresis on PF2 frame ms) into
// `compile_taa_off_for_scale` and maps its errors into `PostError`, then
// re-checks the SMAA-first order through the `graph.rs` seam so the
// SMAA-first graph (never TAA) drives the presented size. Consumer: the
// WIN1 TAA-off loop (RP6 selection rides on top of this seam).
// Budget: `tick_scale_for_present` is the per-present call; the hysteresis
// tick itself allocates nothing, the compile allocates its RA2 plan only.
// ---------------------------------------------------------------------------

impl From<crate::aa_scale::AaScaleError> for PostError {
    fn from(e: crate::aa_scale::AaScaleError) -> Self {
        match e {
            crate::aa_scale::AaScaleError::Invalid(m) => Self::Invalid(m),
            crate::aa_scale::AaScaleError::BudgetExceeded { requested, allowed } => {
                Self::BudgetExceeded { requested, allowed }
            }
            crate::aa_scale::AaScaleError::Device(m) => Self::Device(m),
            crate::aa_scale::AaScaleError::Validation(m) => Self::Validation(m),
        }
    }
}

/// Render size the controller drives for this present: scaled, rounded,
/// forced even (the TAAU path stays exact), clamped to the present size.
/// Same inputs give the same size; errors map into [`PostError`].
pub fn taa_off_render_size_for_scale(
    controller: &crate::aa_scale::ScaleController,
    present_w: u32,
    present_h: u32,
) -> Result<(u32, u32), PostError> {
    controller
        .render_size(present_w, present_h)
        .map_err(PostError::from)
}

/// Production TAA-off compile at the controller scale: sizes from the
/// controller, declares the SMAA-first graph via
/// `aa_scale::compile_taa_off_for_scale` (frozen API), then re-checks the
/// compiled order through the `graph.rs` SMAA-first seam. Returns the
/// compiled graph plus the render size it was built at. Fails loudly on
/// bad sizes or a non-SMAA-first order.
pub fn compile_taa_off_for_scale(
    controller: &crate::aa_scale::ScaleController,
    present_w: u32,
    present_h: u32,
    chain: &PostChain,
) -> Result<(crate::graph::CompiledGraph, u32, u32), PostError> {
    let (compiled, render_w, render_h) =
        crate::aa_scale::compile_taa_off_for_scale(controller, present_w, present_h, chain)
            .map_err(PostError::from)?;
    if !compiled.order_names_match(&crate::aa_scale::taa_off_pass_names()) {
        return Err(PostError::Invalid(format!(
            "taa-off compiled order is not smaa-first at {render_w}x{render_h}"
        )));
    }
    Ok((compiled, render_w, render_h))
}

/// Per-present production tick: feed one PF2 frame time through the
/// controller hysteresis (fast step-down past target, slow step-up with
/// headroom, band resets so borderline frames never flap), then compile
/// the SMAA-first graph at the new scale. Returns the compiled graph, the
/// render size it was built at, and the scale for the frame report.
pub fn tick_scale_for_present(
    controller: &mut crate::aa_scale::ScaleController,
    frame_ms: f64,
    present_w: u32,
    present_h: u32,
    chain: &PostChain,
) -> Result<(crate::graph::CompiledGraph, u32, u32, f32), PostError> {
    let scale = controller.update(frame_ms).map_err(PostError::from)?;
    let (compiled, render_w, render_h) =
        compile_taa_off_for_scale(controller, present_w, present_h, chain)?;
    Ok((compiled, render_w, render_h, scale))
}

/// CPU TAA-off frame at the controller scale: sizes from the controller,
/// then runs the frozen `aa_scale::run_taa_off_frame` oracle (SMAA at the
/// render size, RP1 TAAU plus chain after it, never a second chain). Same
/// complete inputs give bit-identical pixels; only `ms` varies.
pub fn run_taa_off_frame_for_scale(
    hdr: &[[f32; 3]],
    controller: &crate::aa_scale::ScaleController,
    present_w: u32,
    present_h: u32,
    smaa: &crate::aa_scale::SmaaParams,
    chain: &PostChain,
    lights: &PostLights,
) -> Result<crate::aa_scale::TaaOffOutput, PostError> {
    let (render_w, render_h) = taa_off_render_size_for_scale(controller, present_w, present_h)?;
    crate::aa_scale::run_taa_off_frame(
        hdr, render_w, render_h, present_w, present_h, smaa, chain, lights,
    )
    .map_err(PostError::from)
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

/// Shimmer scene: a static high-frequency checkerboard sampled with
/// alternating sub-pixel offsets. Frame `k` shifts the sampling grid by
/// `(k % 2) * 0.5` px, so raw frames flicker while the TAA resolve must
/// converge to a stable mean.
#[derive(Clone, PartialEq, Debug)]
pub struct ShimmerScene {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Alternating raw frames (even/odd sampling offsets).
    pub frames: Vec<Vec<[f32; 3]>>,
}

impl ShimmerScene {
    /// Build the 16x16 checkerboard pair (2px cells, dark 0.1 / lit 0.9).
    pub fn checkerboard() -> Result<Self, PostError> {
        let (width, height) = (16u32, 16u32);
        let cell = |x: u32, y: u32| {
            if (x / 2 + y / 2).is_multiple_of(2) {
                [0.1, 0.1, 0.1]
            } else {
                [0.9, 0.9, 0.9]
            }
        };
        let mut even = Vec::with_capacity(256);
        let mut odd = Vec::with_capacity(256);
        for y in 0..height {
            for x in 0..width {
                even.push(cell(x, y));
                // Odd frames sample half a cell diagonally off: every other
                // texel flips, which is the aliasing shimmer under motion.
                odd.push(cell(x.wrapping_add(1) % width, y.wrapping_add(1) % height));
            }
        }
        check_image(&even, width, height, "shimmer even")?;
        check_image(&odd, width, height, "shimmer odd")?;
        Ok(Self {
            width,
            height,
            frames: vec![even, odd],
        })
    }

    /// Per-pixel temporal variance across `clip` frames (mean of channel
    /// variances, then the max over pixels). Must stay under
    /// [`SHIMMER_MAX_VAR`].
    pub fn temporal_variance(clip: &[Vec<[f32; 3]>]) -> f32 {
        if clip.is_empty() {
            return 0.0;
        }
        let n = clip.len() as f32;
        let len = clip[0].len();
        let mut max = 0.0f32;
        for i in 0..len {
            let mut mean = [0.0f32; 3];
            for frame in clip {
                for (m, p) in mean.iter_mut().zip(frame[i].iter()) {
                    *m += *p;
                }
            }
            for m in &mut mean {
                *m /= n;
            }
            let mut var = 0.0f32;
            for frame in clip {
                for c in 0..3 {
                    let d = frame[i][c] - mean[c];
                    var += d * d;
                }
            }
            max = max.max(var / (3.0 * n));
        }
        max
    }
}

// ---------------------------------------------------------------------------
// Fixtures: day-sweep plus torch-grid HDR frames for the auto-exposure
// accept (fixed baseline versus adapted, flicker under alternating
// frames). Deterministic data, no wall clock.
// ---------------------------------------------------------------------------

/// Share of texels with any display channel at or above
/// [`AUTO_CLIP_LUMA`]: the clipped-highlight metric. A total function
/// over display pixels (empty clips read 0.0); the chain output feeds it.
pub fn clipped_share(ldr: &[[f32; 3]]) -> f32 {
    let mut clipped = 0.0f32;
    let mut total = 0.0f32;
    for p in ldr {
        total += 1.0;
        if p[0] >= AUTO_CLIP_LUMA || p[1] >= AUTO_CLIP_LUMA || p[2] >= AUTO_CLIP_LUMA {
            clipped += 1.0;
        }
    }
    if total <= 0.0 {
        return 0.0;
    }
    clipped / total
}

/// Day-sweep plus torch-grid HDR fixtures. The sweep is one grey ramp at
/// five radiance doublings (four stops, dim to bright): fixed exposure
/// clips the bright end while the adapted state tracks it. The torch
/// grid is a dark field with a sparse bright grid (16 warm torches on
/// 256 texels): the percentile trim keeps those highlights out of the
/// mean, so striking the torches barely moves the adapted stops.
#[derive(Clone, PartialEq, Debug)]
pub struct ExposureScene {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Five ramp frames, dim to bright, one stop apart.
    pub sweep: Vec<Vec<[f32; 3]>>,
    /// Dark field with the torch grid lit.
    pub torch: Vec<[f32; 3]>,
    /// Same field with the torches out (the pumping probe).
    pub dark: Vec<[f32; 3]>,
}

impl ExposureScene {
    /// Build the 16x16 sweep plus torch pair. The ramp runs 0.05..3.05
    /// linear per stop frame; torches are warm-white 30-ish highlights.
    pub fn day_sweep_torch() -> Result<Self, PostError> {
        let (width, height) = (16u32, 16u32);
        let mut base = Vec::with_capacity(256);
        let mut luma = 0.05f32;
        let step = 3.0 / 255.0;
        for _ in 0..256 {
            base.push([luma, luma, luma]);
            luma += step;
        }
        check_image(&base, width, height, "exposure base")?;
        let mut sweep = Vec::with_capacity(5);
        for stops in [0.25f32, 0.5, 1.0, 2.0, 4.0] {
            sweep.push(
                base.iter()
                    .map(|p| [p[0] * stops, p[1] * stops, p[2] * stops])
                    .collect(),
            );
        }
        let dark = vec![[0.05, 0.05, 0.05]; 256];
        let mut torch = Vec::with_capacity(256);
        for y in 0..height {
            for x in 0..width {
                if x % 4 == 0 && y % 4 == 0 {
                    torch.push([30.0, 26.0, 22.0]);
                } else {
                    torch.push([0.05, 0.05, 0.05]);
                }
            }
        }
        check_image(&torch, width, height, "exposure torch")?;
        Ok(Self {
            width,
            height,
            sweep,
            dark,
            torch,
        })
    }
}

// ---------------------------------------------------------------------------
// GPU descriptors behind `window`: resolve/post targets plus the WGSL
// chunks the RA1 presenter consumes. Allocation tests skip without an
// adapter, exactly like the RL1 presenter tests.
// ---------------------------------------------------------------------------

/// Post GPU descriptors and the TAA-resolve plus post sampling chunks for
/// the RA1 presenter. Engine-owned code (no port markers); the presenter
/// inlines these chunks when the post bind group lands.
#[cfg(feature = "window")]
pub mod gpu {
    use super::PostError;

    /// TAA resolve chunk (WGSL): bilinear history reproject, 3x3 box clamp
    /// around the current texel, velocity rejection, blend.
    pub const TAA_RESOLVE_WGSL: &str = include_str!("shaders/post/taa_resolve.wgsl");

    /// Post chain chunk (WGSL): soft-knee bloom add, exposure, Narkowicz
    /// ACES with white normalization, contrast/saturation grading.
    pub const POST_CHAIN_WGSL: &str = include_str!("shaders/post/post_chain.wgsl");

    /// History color target descriptor: `Rgba32Float` render-attachment
    /// plus sampler binding for the resolve pass.
    pub fn history_desc(width: u32, height: u32) -> wgpu::TextureDescriptor<'static> {
        wgpu::TextureDescriptor {
            label: Some("rp1-taa-history"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        }
    }

    /// Post output target descriptor: `Rgba16Float` HDR staging target,
    /// sampler-readable for the chain pass.
    pub fn post_target_desc(width: u32, height: u32) -> wgpu::TextureDescriptor<'static> {
        wgpu::TextureDescriptor {
            label: Some("rp1-post-target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        }
    }

    /// Allocate the TAA history target on `device`. Real GPU work for the
    /// presenter post pass; callers keep last-good on error.
    pub fn allocate_history(
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> Result<wgpu::Texture, PostError> {
        check_history_dims(width, height)?;
        Ok(device.create_texture(&history_desc(width, height)))
    }

    /// Validate the history size before allocation: zero suspends (invalid),
    /// over-bound degrades with `BudgetExceeded`.
    pub fn check_history_dims(width: u32, height: u32) -> Result<(), PostError> {
        if width == 0 || height == 0 {
            return Err(PostError::Invalid("post GPU history size is zero".into()));
        }
        if width > super::MAX_POST_DIM || height > super::MAX_POST_DIM {
            return Err(PostError::BudgetExceeded {
                requested: width.max(height),
                allowed: super::MAX_POST_DIM,
            });
        }
        Ok(())
    }

    /// Posted CPU/GPU parity tolerances (halved-precision-free f32 path:
    /// TAA is pure `+ - * /` plus `sqrt` for speed, so 1-ulp drift only;
    /// the post chain adds the f64->f32 bloom accumulation drift).
    /// Ghost/shimmer gates still apply unchanged on GPU pixels.
    pub const GPU_PARITY_TOL_TAA: f32 = 3e-4;
    /// Posted CPU/GPU parity tolerance for the full post chain output.
    pub const GPU_PARITY_TOL_POST: f32 = 5e-3;
    /// Posted GPU submit budget at test sizes in ms (evidence only).
    pub const GPU_SUBMIT_BUDGET_MS: f64 = 16.0;

    /// TAA compute shader: motion-vector reprojection with the
    /// current-frame jitter offset, bilinear history sample, 3x3 box clamp
    /// around the current texel, velocity rejection, blend. Mirrors
    /// [`super::taa_resolve_jittered`] texel for texel.
    pub const TAA_COMPUTE_WGSL: &str = include_str!("shaders/post/taa_compute.wgsl");

    /// Temporal upsample shader: clamp-to-edge bilinear from the render
    /// size to the presentation size. Mirrors [`super::taa_upsample`].
    pub const UPSAMPLE_COMPUTE_WGSL: &str = include_str!("shaders/post/upsample_compute.wgsl");

    /// Bloom extract shader: soft-knee weight times the HDR texel.
    /// Mirrors [`super::apply_bloom`] extract (`bloom_weight`).
    pub const BRIGHT_COMPUTE_WGSL: &str = include_str!("shaders/post/bright_compute.wgsl");

    /// Box downsample shader: one 2x2 average step (ceil halves).
    /// Mirrors `downsample_box` (f32 accumulation; the CPU uses f64,
    /// inside [`GPU_PARITY_TOL_POST`]).
    pub const DOWNSAMPLE_COMPUTE_WGSL: &str = include_str!("shaders/post/downsample_compute.wgsl");

    /// Composite shader: bilinear add-back of every bloom mip with even
    /// intensity split, then exposure, Narkowicz ACES with white
    /// normalization, contrast/saturation grading. Mirrors
    /// [`super::apply_bloom`] add-back plus [`super::tonemap_aces`] plus
    /// [`super::apply_grade`]. All mip levels live in ONE concatenated
    /// storage buffer (`mipsN = (w, h, texel_offset, 0)`); with the source
    /// and the target that is three storage bindings total, inside every
    /// native minimum. The six slots are unrolled under the `count` guard.
    pub const COMPOSITE_COMPUTE_WGSL: &str = include_str!("shaders/post/composite_compute.wgsl");

    /// One GPU TAA resolve: pixels plus the validation outcome plus ms.
    #[derive(Clone, Debug)]
    pub struct GpuResolve {
        /// Resolved HDR pixels, row-major `width * height`.
        pub pixels: Vec<[f32; 3]>,
        /// Frame width in pixels.
        pub width: u32,
        /// Frame height in pixels.
        pub height: u32,
        /// wgpu validation errors observed; green means 0.
        pub validation_errors: u32,
        /// Wall time for encode plus submit plus readback in ms.
        pub submit_ms: f64,
    }

    /// One GPU post frame: display pixels plus the frame receipt plus the
    /// CPU/GPU parity figure. The receipt pass names match the RA2 graph
    /// (`taa`, `taau`, `bloom`, `exposure`, `aces`, `grade`).
    #[derive(Clone, Debug)]
    pub struct GpuFrame {
        /// Display-space pixels, row-major `width * height`.
        pub ldr: Vec<[f32; 3]>,
        /// Frame width in pixels.
        pub width: u32,
        /// Frame height in pixels.
        pub height: u32,
        /// Per-pass receipts plus the RL1/RL2 light ms.
        pub receipt: super::FramePostReceipt,
        /// Max per-channel abs diff against the CPU [`super::run_frame`].
        pub parity_max_diff: f32,
        /// wgpu validation errors observed; green means 0.
        pub validation_errors: u32,
        /// Wall time for the whole GPU frame in ms (evidence only).
        pub submit_ms: f64,
    }

    /// Reusable RP1 device state for the WIN1 match loop (and the ED1
    /// viewport on the same shape): the five compute pipelines plus their
    /// bind layouts. Built once; per-submit buffers are driver resources,
    /// same as the RA1 twin, and sit outside any reuse guarantee. Consumes
    /// [`super::build_rp1_graph`] on every full-frame submit so pass order
    /// stays RA2-owned.
    pub struct Rp1Submitter {
        taa_pipe: wgpu::ComputePipeline,
        up_pipe: wgpu::ComputePipeline,
        bright_pipe: wgpu::ComputePipeline,
        down_pipe: wgpu::ComputePipeline,
        comp_pipe: wgpu::ComputePipeline,
        taa_layout: wgpu::BindGroupLayout,
        up_layout: wgpu::BindGroupLayout,
        bright_layout: wgpu::BindGroupLayout,
        down_layout: wgpu::BindGroupLayout,
        comp_layout: wgpu::BindGroupLayout,
    }

    fn compute_pipeline(
        device: &wgpu::Device,
        label: &str,
        wgsl: &str,
        layout: &wgpu::BindGroupLayout,
    ) -> wgpu::ComputePipeline {
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(wgsl.into()),
        });
        let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(layout)],
            immediate_size: 0,
        });
        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some(label),
            layout: Some(&pipe_layout),
            module: &module,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        })
    }

    fn storage_layout(device: &wgpu::Device, label: &str, counts: &[u32]) -> wgpu::BindGroupLayout {
        let mut entries = Vec::with_capacity(counts.len());
        for (i, kind) in counts.iter().enumerate() {
            // kind 0 = uniform, 1 = read-only storage, 2 = read-write storage.
            let ty = match kind {
                0 => wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                1 => wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                _ => wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
            };
            entries.push(wgpu::BindGroupLayoutEntry {
                binding: i as u32,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty,
                count: None,
            });
        }
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
            entries: &entries,
        })
    }

    fn pack_pixels(pixels: &[[f32; 3]]) -> Vec<u8> {
        let mut out = Vec::with_capacity(pixels.len() * 16);
        for p in pixels {
            out.extend_from_slice(&p[0].to_le_bytes());
            out.extend_from_slice(&p[1].to_le_bytes());
            out.extend_from_slice(&p[2].to_le_bytes());
            out.extend_from_slice(&1f32.to_le_bytes());
        }
        out
    }

    fn pack_motion(flow: &[[f32; 2]]) -> Vec<u8> {
        let mut out = Vec::with_capacity(flow.len() * 16);
        for m in flow {
            out.extend_from_slice(&m[0].to_le_bytes());
            out.extend_from_slice(&m[1].to_le_bytes());
            out.extend_from_slice(&0f32.to_le_bytes());
            out.extend_from_slice(&0f32.to_le_bytes());
        }
        out
    }

    fn storage_buf(device: &wgpu::Device, label: &str, bytes: &[u8]) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
        })
    }

    fn zero_storage(device: &wgpu::Device, label: &str, texels: u64) -> wgpu::Buffer {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: texels * 16,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        })
    }

    fn uniform_buf(device: &wgpu::Device, label: &str, bytes: &[u8]) -> wgpu::Buffer {
        use wgpu::util::DeviceExt;
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    impl Rp1Submitter {
        /// Compile the five RP1 compute pipelines plus their bind layouts.
        /// No frame-size state: buffers size per submit.
        pub fn new(device: &wgpu::Device) -> Self {
            let taa_layout = storage_layout(device, "rp1-taa-layout", &[0, 1, 1, 1, 2]);
            let up_layout = storage_layout(device, "rp1-up-layout", &[0, 1, 2]);
            let bright_layout = storage_layout(device, "rp1-bright-layout", &[0, 1, 2]);
            let down_layout = storage_layout(device, "rp1-down-layout", &[0, 1, 2]);
            let comp_layout = storage_layout(device, "rp1-comp-layout", &[0, 1, 1, 2]);
            let taa_pipe = compute_pipeline(device, "rp1-taa", TAA_COMPUTE_WGSL, &taa_layout);
            let up_pipe = compute_pipeline(device, "rp1-up", UPSAMPLE_COMPUTE_WGSL, &up_layout);
            let bright_pipe =
                compute_pipeline(device, "rp1-bright", BRIGHT_COMPUTE_WGSL, &bright_layout);
            let down_pipe =
                compute_pipeline(device, "rp1-down", DOWNSAMPLE_COMPUTE_WGSL, &down_layout);
            let comp_pipe =
                compute_pipeline(device, "rp1-comp", COMPOSITE_COMPUTE_WGSL, &comp_layout);
            Self {
                taa_pipe,
                up_pipe,
                bright_pipe,
                down_pipe,
                comp_pipe,
                taa_layout,
                up_layout,
                bright_layout,
                down_layout,
                comp_layout,
            }
        }

        fn bind(
            &self,
            device: &wgpu::Device,
            layout: &wgpu::BindGroupLayout,
            buffers: &[&wgpu::Buffer],
        ) -> wgpu::BindGroup {
            let entries: Vec<wgpu::BindGroupEntry<'_>> = buffers
                .iter()
                .enumerate()
                .map(|(i, b)| wgpu::BindGroupEntry {
                    binding: i as u32,
                    resource: b.as_entire_binding(),
                })
                .collect();
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("rp1-bind"),
                layout,
                entries: &entries,
            })
        }

        /// GPU TAA resolve over one frame: motion-vector reprojection with
        /// the jitter offset, bilinear history sample, 3x3 box clamp,
        /// velocity rejection, blend. One compute dispatch plus readback
        /// inside a validation scope; 0 scope hits means green. This path
        /// has no `Device` refusal variant gap: bad input fails loudly,
        /// device work reports pixels.
        #[expect(
            clippy::too_many_arguments,
            reason = "wide signature kept until its inputs move into a params struct"
        )]
        pub fn resolve(
            &self,
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            current: &[[f32; 3]],
            history: &[[f32; 3]],
            motion: &super::MotionField,
            width: u32,
            height: u32,
            params: &super::TaaParams,
            jitter_px: [f32; 2],
        ) -> Result<GpuResolve, PostError> {
            params.validate()?;
            super::check_image(current, width, height, "taa current")?;
            super::check_image(history, width, height, "taa history")?;
            if motion.width != width || motion.height != height {
                return Err(PostError::Invalid(format!(
                    "taa motion {}x{} differs from frame {width}x{height}",
                    motion.width, motion.height
                )));
            }
            if !jitter_px.iter().all(|c| c.is_finite()) {
                return Err(PostError::Invalid("taa jitter is not finite".into()));
            }
            if jitter_px[0].abs() > 1.0 || jitter_px[1].abs() > 1.0 {
                return Err(PostError::Invalid(format!(
                    "taa jitter {jitter_px:?} outside +-1px"
                )));
            }
            let texels = width as usize * height as usize;
            let started = std::time::Instant::now();
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let mut uni = Vec::with_capacity(48);
            uni.extend_from_slice(&width.to_le_bytes());
            uni.extend_from_slice(&height.to_le_bytes());
            uni.extend_from_slice(&0u32.to_le_bytes());
            uni.extend_from_slice(&0u32.to_le_bytes());
            uni.extend_from_slice(&params.history_blend.to_le_bytes());
            uni.extend_from_slice(&params.motion_threshold_px.to_le_bytes());
            uni.extend_from_slice(&(if params.use_clamp { 1f32 } else { 0f32 }).to_le_bytes());
            uni.extend_from_slice(&0f32.to_le_bytes());
            uni.extend_from_slice(&jitter_px[0].to_le_bytes());
            uni.extend_from_slice(&jitter_px[1].to_le_bytes());
            uni.extend_from_slice(&0f32.to_le_bytes());
            uni.extend_from_slice(&0f32.to_le_bytes());
            let (u_buf, cur_buf, hist_buf, mot_buf, dst_buf) = (
                uniform_buf(device, "rp1-taa-u", &uni),
                storage_buf(device, "rp1-taa-cur", &pack_pixels(current)),
                storage_buf(device, "rp1-taa-hist", &pack_pixels(history)),
                storage_buf(device, "rp1-taa-mot", &pack_motion(&motion.flow)),
                zero_storage(device, "rp1-taa-dst", texels as u64),
            );
            let bind = self.bind(
                device,
                &self.taa_layout,
                &[&u_buf, &cur_buf, &hist_buf, &mot_buf, &dst_buf],
            );
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("rp1-taa"),
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("rp1-taa-pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.taa_pipe);
                pass.set_bind_group(0, &bind, &[]);
                pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
            }
            queue.submit(Some(encoder.finish()));
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .map_err(|e| PostError::Device(format!("poll: {e}")))?;
            let scope_err = pollster::block_on(scope.pop());
            // Copy out through a second encoder so the scope above covers
            // exactly the resolve dispatch.
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("rp1-taa-copy"),
            });
            let back = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rp1-taa-readback"),
                size: texels as u64 * 16,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            encoder.copy_buffer_to_buffer(&dst_buf, 0, &back, 0, texels as u64 * 16);
            queue.submit(Some(encoder.finish()));
            let slice = back.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .map_err(|e| PostError::Device(format!("poll: {e}")))?;
            rx.recv()
                .map_err(|e| PostError::Device(format!("map channel: {e}")))?
                .map_err(|e| PostError::Device(format!("map: {e}")))?;
            if let Some(err) = scope_err {
                return Err(PostError::Validation(format!("{err:?}")));
            }
            let mut pixels = Vec::with_capacity(texels);
            {
                let mapped = slice
                    .get_mapped_range()
                    .map_err(|e| PostError::Device(format!("mapped range: {e}")))?;
                for chunk in mapped.chunks_exact(16) {
                    let x = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    let y = f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
                    let z = f32::from_le_bytes([chunk[8], chunk[9], chunk[10], chunk[11]]);
                    if !x.is_finite() || !y.is_finite() || !z.is_finite() {
                        return Err(PostError::Validation(
                            "gpu taa pixels are not finite".into(),
                        ));
                    }
                    pixels.push([x, y, z]);
                }
            }
            back.unmap();
            Ok(GpuResolve {
                pixels,
                width,
                height,
                validation_errors: 0,
                submit_ms: started.elapsed().as_secs_f64() * 1000.0,
            })
        }

        /// Full RP1 frame on the device in compiled graph order: TAA at
        /// the render size, TAAU to the presentation size, bloom
        /// extract plus box-blur mip chain, then the composite
        /// (bloom add, exposure, ACES, grade). Consumes
        /// [`super::compile_rp1_for_present`] first so the six recorded
        /// dispatches always match the RA2 order; then compares against
        /// the CPU [`super::run_frame`] for the parity figure. One
        /// validation scope covers every dispatch; any hit fails loudly.
        #[expect(
            clippy::too_many_arguments,
            reason = "wide signature kept until its inputs move into a params struct"
        )]
        pub fn run_frame(
            &self,
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            current: &[[f32; 3]],
            history: &[[f32; 3]],
            motion: &super::MotionField,
            render_w: u32,
            render_h: u32,
            present_w: u32,
            present_h: u32,
            taa: &super::TaaParams,
            chain: &super::PostChain,
            lights: &super::PostLights,
            jitter_px: [f32; 2],
        ) -> Result<GpuFrame, PostError> {
            taa.validate()?;
            chain.validate()?;
            let compiled =
                super::compile_rp1_for_present(render_w, render_h, present_w, present_h, chain)?;
            if compiled.order().len() != 6 {
                return Err(PostError::Invalid(format!(
                    "rp1 compiled passes {} differ from 6",
                    compiled.order().len()
                )));
            }
            super::check_image(current, render_w, render_h, "taa current")?;
            super::check_image(history, render_w, render_h, "taa history")?;
            if motion.width != render_w || motion.height != render_h {
                return Err(PostError::Invalid(format!(
                    "taa motion {}x{} differs from render {render_w}x{render_h}",
                    motion.width, motion.height
                )));
            }
            let mult = chain.exposure.mult()?;
            let render_n = render_w as usize * render_h as usize;
            let present_n = present_w as usize * present_h as usize;
            let started = std::time::Instant::now();
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            // Stage 1 (taa): render-size resolve.
            let taa_started = std::time::Instant::now();
            let mut taa_uni = Vec::with_capacity(48);
            taa_uni.extend_from_slice(&render_w.to_le_bytes());
            taa_uni.extend_from_slice(&render_h.to_le_bytes());
            taa_uni.extend_from_slice(&0u32.to_le_bytes());
            taa_uni.extend_from_slice(&0u32.to_le_bytes());
            taa_uni.extend_from_slice(&taa.history_blend.to_le_bytes());
            taa_uni.extend_from_slice(&taa.motion_threshold_px.to_le_bytes());
            taa_uni.extend_from_slice(&(if taa.use_clamp { 1f32 } else { 0f32 }).to_le_bytes());
            taa_uni.extend_from_slice(&0f32.to_le_bytes());
            taa_uni.extend_from_slice(&jitter_px[0].to_le_bytes());
            taa_uni.extend_from_slice(&jitter_px[1].to_le_bytes());
            taa_uni.extend_from_slice(&0f32.to_le_bytes());
            taa_uni.extend_from_slice(&0f32.to_le_bytes());
            let (taa_u, cur_buf, hist_buf, mot_buf, resolved_buf) = (
                uniform_buf(device, "rp1-u-taa", &taa_uni),
                storage_buf(device, "rp1-cur", &pack_pixels(current)),
                storage_buf(device, "rp1-hist", &pack_pixels(history)),
                storage_buf(device, "rp1-mot", &pack_motion(&motion.flow)),
                zero_storage(device, "rp1-resolved", render_n as u64),
            );
            let taa_bind = self.bind(
                device,
                &self.taa_layout,
                &[&taa_u, &cur_buf, &hist_buf, &mot_buf, &resolved_buf],
            );
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("rp1-taa"),
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("rp1-taa-pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.taa_pipe);
                pass.set_bind_group(0, &taa_bind, &[]);
                pass.dispatch_workgroups(render_w.div_ceil(8), render_h.div_ceil(8), 1);
            }
            queue.submit(Some(encoder.finish()));
            let taa_ms = taa_started.elapsed().as_secs_f64() * 1000.0;
            // Stage 2 (taau): render -> present bilinear.
            let up_started = std::time::Instant::now();
            let sx = render_w as f32 / present_w as f32;
            let sy = render_h as f32 / present_h as f32;
            let mut up_uni = Vec::with_capacity(32);
            up_uni.extend_from_slice(&render_w.to_le_bytes());
            up_uni.extend_from_slice(&render_h.to_le_bytes());
            up_uni.extend_from_slice(&present_w.to_le_bytes());
            up_uni.extend_from_slice(&present_h.to_le_bytes());
            up_uni.extend_from_slice(&sx.to_le_bytes());
            up_uni.extend_from_slice(&sy.to_le_bytes());
            up_uni.extend_from_slice(&0f32.to_le_bytes());
            up_uni.extend_from_slice(&0f32.to_le_bytes());
            let (up_u, up_buf) = (
                uniform_buf(device, "rp1-u-up", &up_uni),
                zero_storage(device, "rp1-upsampled", present_n as u64),
            );
            let up_bind = self.bind(device, &self.up_layout, &[&up_u, &resolved_buf, &up_buf]);
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("rp1-taau"),
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("rp1-taau-pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.up_pipe);
                pass.set_bind_group(0, &up_bind, &[]);
                pass.dispatch_workgroups(present_w.div_ceil(8), present_h.div_ceil(8), 1);
            }
            queue.submit(Some(encoder.finish()));
            let up_ms = up_started.elapsed().as_secs_f64() * 1000.0;
            // Stage 3 (bloom): bright extract then box-blur mip chain.
            let bloom_started = std::time::Instant::now();
            let mut bright_uni = Vec::with_capacity(32);
            bright_uni.extend_from_slice(&chain.bloom.threshold.to_le_bytes());
            bright_uni.extend_from_slice(&chain.bloom.knee.to_le_bytes());
            bright_uni.extend_from_slice(&0f32.to_le_bytes());
            bright_uni.extend_from_slice(&0f32.to_le_bytes());
            bright_uni.extend_from_slice(&present_w.to_le_bytes());
            bright_uni.extend_from_slice(&present_h.to_le_bytes());
            bright_uni.extend_from_slice(&0u32.to_le_bytes());
            bright_uni.extend_from_slice(&0u32.to_le_bytes());
            let (bright_u, bright_buf) = (
                uniform_buf(device, "rp1-u-bright", &bright_uni),
                zero_storage(device, "rp1-bright", present_n as u64),
            );
            let bright_bind = self.bind(
                device,
                &self.bright_layout,
                &[&bright_u, &up_buf, &bright_buf],
            );
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("rp1-bright"),
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("rp1-bright-pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.bright_pipe);
                pass.set_bind_group(0, &bright_bind, &[]);
                pass.dispatch_workgroups(present_w.div_ceil(8), present_h.div_ceil(8), 1);
            }
            queue.submit(Some(encoder.finish()));
            // Box-blur mip chain: each level halves the previous bright mip,
            // mirroring `downsample_box` (ceil halves, stop at 1x1). The
            // chain vector owns every buffer: index 0 is the bright extract
            // (dropped before the composite), the rest are the sampled mips
            // in order. Borrows end at bind-group creation, so pushing the
            // next dst after each submit is borrow-clean.
            let mut chain_bufs: Vec<wgpu::Buffer> = vec![bright_buf];
            let mut chain_sizes: Vec<(u32, u32)> = vec![(present_w, present_h)];
            for _ in 0..chain.bloom.levels {
                let (lw, lh) = chain_sizes[chain_sizes.len() - 1];
                let nw = lw.div_ceil(2).max(1);
                let nh = lh.div_ceil(2).max(1);
                let mut down_uni = Vec::with_capacity(16);
                down_uni.extend_from_slice(&lw.to_le_bytes());
                down_uni.extend_from_slice(&lh.to_le_bytes());
                down_uni.extend_from_slice(&nw.to_le_bytes());
                down_uni.extend_from_slice(&nh.to_le_bytes());
                let down_u = uniform_buf(device, "rp1-u-down", &down_uni);
                let dst = zero_storage(device, "rp1-mip", nw as u64 * nh as u64);
                {
                    let src = &chain_bufs[chain_bufs.len() - 1];
                    let down_bind = self.bind(device, &self.down_layout, &[&down_u, src, &dst]);
                    let mut encoder =
                        device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                            label: Some("rp1-down"),
                        });
                    {
                        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                            label: Some("rp1-down-pass"),
                            timestamp_writes: None,
                        });
                        pass.set_pipeline(&self.down_pipe);
                        pass.set_bind_group(0, &down_bind, &[]);
                        pass.dispatch_workgroups(nw.div_ceil(8), nh.div_ceil(8), 1);
                    }
                    queue.submit(Some(encoder.finish()));
                }
                chain_bufs.push(dst);
                chain_sizes.push((nw, nh));
                if nw == 1 && nh == 1 {
                    break;
                }
            }
            // Drop the bright extract: the CPU add-back samples the chain
            // mips only, never the extract itself.
            chain_bufs.remove(0);
            chain_sizes.remove(0);
            let mip_bufs = chain_bufs;
            let mip_sizes = chain_sizes;
            let bloom_ms = bloom_started.elapsed().as_secs_f64() * 1000.0;
            // Stage 4..6 (exposure/aces/grade): single composite dispatch.
            // The mip levels are concatenated into one storage buffer first
            // (offsets in texels, packed as `mipsN = (w, h, off, 0)`), so
            // the composite binds three storage buffers total: source,
            // mips, target.
            let comp_started = std::time::Instant::now();
            let mut mip_total: u64 = 0;
            let mut mip_offs: Vec<u64> = Vec::with_capacity(mip_bufs.len());
            for (w, h) in mip_sizes.iter() {
                mip_offs.push(mip_total);
                mip_total += *w as u64 * *h as u64;
            }
            let mips_buf = zero_storage(device, "rp1-mips", mip_total.max(1));
            let mut full_sizes = [(1u32, 1u32, 0u64); 6];
            for (i, s) in mip_sizes.iter().enumerate().take(6) {
                full_sizes[i] = (s.0, s.1, mip_offs[i]);
            }
            let count = mip_sizes.len().min(6) as u32;
            let mut comp_uni = Vec::with_capacity(160);
            comp_uni.extend_from_slice(&present_w.to_le_bytes());
            comp_uni.extend_from_slice(&present_h.to_le_bytes());
            comp_uni.extend_from_slice(&count.to_le_bytes());
            comp_uni.extend_from_slice(&0u32.to_le_bytes());
            for (w, h, off) in full_sizes {
                comp_uni.extend_from_slice(&w.to_le_bytes());
                comp_uni.extend_from_slice(&h.to_le_bytes());
                comp_uni.extend_from_slice(&(off as u32).to_le_bytes());
                comp_uni.extend_from_slice(&0u32.to_le_bytes());
            }
            comp_uni.extend_from_slice(&chain.bloom.intensity.to_le_bytes());
            comp_uni.extend_from_slice(&mult.to_le_bytes());
            comp_uni.extend_from_slice(&chain.aces_white.to_le_bytes());
            comp_uni.extend_from_slice(&chain.grade.contrast.to_le_bytes());
            comp_uni.extend_from_slice(&chain.grade.saturation.to_le_bytes());
            comp_uni.extend_from_slice(&chain.grade.lift[0].to_le_bytes());
            comp_uni.extend_from_slice(&chain.grade.lift[1].to_le_bytes());
            comp_uni.extend_from_slice(&chain.grade.lift[2].to_le_bytes());
            comp_uni.extend_from_slice(&chain.grade.gain[0].to_le_bytes());
            comp_uni.extend_from_slice(&chain.grade.gain[1].to_le_bytes());
            comp_uni.extend_from_slice(&chain.grade.gain[2].to_le_bytes());
            comp_uni.extend_from_slice(&0f32.to_le_bytes());
            let (comp_u, ldr_buf) = (
                uniform_buf(device, "rp1-u-comp", &comp_uni),
                zero_storage(device, "rp1-ldr", present_n as u64),
            );
            let comp_bind = self.bind(
                device,
                &self.comp_layout,
                &[&comp_u, &up_buf, &mips_buf, &ldr_buf],
            );
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("rp1-comp"),
            });
            // Stage the mip levels into the concatenated buffer first;
            // encoder order is execution order, so the pass below reads
            // settled bytes.
            for (buf, off) in mip_bufs.iter().zip(mip_offs.iter()) {
                let bytes = buf.size();
                encoder.copy_buffer_to_buffer(buf, 0, &mips_buf, off * 16, bytes);
            }
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("rp1-comp-pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.comp_pipe);
                pass.set_bind_group(0, &comp_bind, &[]);
                pass.dispatch_workgroups(present_w.div_ceil(8), present_h.div_ceil(8), 1);
            }
            queue.submit(Some(encoder.finish()));
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .map_err(|e| PostError::Device(format!("poll: {e}")))?;
            let scope_err = pollster::block_on(scope.pop());
            if let Some(err) = scope_err {
                return Err(PostError::Validation(format!("{err:?}")));
            }
            let comp_ms_total = comp_started.elapsed().as_secs_f64() * 1000.0;
            // Single readback of the LDR buffer.
            let back = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("rp1-ldr-readback"),
                size: present_n as u64 * 16,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("rp1-ldr-copy"),
            });
            encoder.copy_buffer_to_buffer(&ldr_buf, 0, &back, 0, present_n as u64 * 16);
            queue.submit(Some(encoder.finish()));
            let slice = back.slice(..);
            let (tx, rx) = std::sync::mpsc::channel();
            slice.map_async(wgpu::MapMode::Read, move |r| {
                let _ = tx.send(r);
            });
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .map_err(|e| PostError::Device(format!("poll: {e}")))?;
            rx.recv()
                .map_err(|e| PostError::Device(format!("map channel: {e}")))?
                .map_err(|e| PostError::Device(format!("map: {e}")))?;
            let mut ldr = Vec::with_capacity(present_n);
            {
                let mapped = slice
                    .get_mapped_range()
                    .map_err(|e| PostError::Device(format!("mapped range: {e}")))?;
                for chunk in mapped.chunks_exact(16) {
                    let x = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    let y = f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
                    let z = f32::from_le_bytes([chunk[8], chunk[9], chunk[10], chunk[11]]);
                    if !x.is_finite() || !y.is_finite() || !z.is_finite() {
                        return Err(PostError::Validation(
                            "gpu ldr pixels are not finite".into(),
                        ));
                    }
                    ldr.push([x, y, z]);
                }
            }
            back.unmap();
            // CPU parity: the oracle runs on the same inputs.
            let cpu = super::run_frame(
                current, history, motion, render_w, render_h, present_w, present_h, taa, chain,
                lights,
            )?;
            let mut parity_max_diff = 0f32;
            for (a, b) in cpu.ldr.iter().zip(ldr.iter()) {
                for c in 0..3 {
                    parity_max_diff = parity_max_diff.max((a[c] - b[c]).abs());
                }
            }
            let submit_ms = started.elapsed().as_secs_f64() * 1000.0;
            let render_px = render_w as u64 * render_h as u64;
            let present_px = present_w as u64 * present_h as u64;
            let third = comp_ms_total / 3.0;
            Ok(GpuFrame {
                width: present_w,
                height: present_h,
                ldr,
                receipt: super::FramePostReceipt {
                    taa: super::PassReceipt {
                        pass: "taa",
                        pixels: render_px,
                        ms: taa_ms,
                    },
                    upsample: super::PassReceipt {
                        pass: "taau",
                        pixels: present_px,
                        ms: up_ms,
                    },
                    bloom: super::PassReceipt {
                        pass: "bloom",
                        pixels: present_px,
                        ms: bloom_ms,
                    },
                    exposure: super::PassReceipt {
                        pass: "exposure",
                        pixels: present_px,
                        ms: third,
                    },
                    aces: super::PassReceipt {
                        pass: "aces",
                        pixels: present_px,
                        ms: third,
                    },
                    grade: super::PassReceipt {
                        pass: "grade",
                        pixels: present_px,
                        ms: third,
                    },
                    sun_ms: lights.sun_ms,
                    cluster_ms: lights.cluster_ms,
                    ibl_ms: lights.ibl_ms,
                },
                parity_max_diff,
                validation_errors: 0,
                submit_ms,
            })
        }
    }

    /// Max per-channel abs diff between two clips (CPU/GPU parity figure).
    pub fn max_abs_diff(a: &[[f32; 3]], b: &[[f32; 3]]) -> f32 {
        let mut max = 0f32;
        for (x, y) in a.iter().zip(b.iter()) {
            for c in 0..3 {
                max = max.max((x[c] - y[c]).abs());
            }
        }
        max
    }
}

// ---------------------------------------------------------------------------
// Tests: jitter, motion, resolve/clamp, upsample, chain, scenes, failures.
// `v1` tests pin parity with the R4/R1 paths; `window` tests pin the GPU
// descriptors with the RA1 skip pattern.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32, tol: f32) -> bool {
        (a - b).abs() <= tol
    }

    fn lights() -> PostLights {
        PostLights {
            sun_ms: 1.25,
            cluster_ms: 0.5,
            ibl_ms: 2.0,
        }
    }

    #[test]
    fn jitter_matches_halton_values() {
        // Textbook anchors: halton(1,2)=1/2, halton(2,2)=1/4, halton(1,3)=1/3.
        assert!(approx(halton(1, 2), 0.5, 1e-7));
        assert!(approx(halton(2, 2), 0.25, 1e-7));
        assert!(approx(halton(1, 3), 1.0 / 3.0, 1e-7));
        // Frame 0 leaves half-pixel x (0.5-0.5) and -1/6 y (1/3-1/2).
        let j0 = jitter_offset(0);
        assert!(approx(j0[0], 0.0, 1e-7));
        assert!(approx(j0[1], -1.0 / 6.0, 1e-6));
        for frame in [0u64, 1, 2, 7, 15] {
            let j = jitter_offset(frame);
            assert!(
                (-0.5..0.5).contains(&j[0]) && (-0.5..0.5).contains(&j[1]),
                "frame {frame} jitter {j:?} stays in the pixel"
            );
        }
        // NDC scales pixels by 2/res; zero res suspends to zero.
        let ndc = jitter_ndc(1, 1920, 1080);
        let [px, py] = jitter_offset(1);
        assert!(approx(ndc[0], px * 2.0 / 1920.0, 1e-7));
        assert!(approx(ndc[1], py * 2.0 / 1080.0, 1e-7));
        assert_eq!(jitter_ndc(3, 0, 720), [0.0, 0.0]);
        let u = jitter_uniform(1, 1280, 720);
        let [ex, ey] = jitter_ndc(1, 1280, 720);
        assert_eq!(u, [px, py, ex, ey]);
    }

    #[test]
    fn jittered_projection_touches_only_perspective_terms() {
        let proj = [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 2.0, 0.0, 0.0],
            [0.3, -0.4, 1.0, 1.0],
            [0.0, 0.0, -0.5, 0.0],
        ];
        let out = jitter_projection(proj, [0.01, -0.02]);
        assert!(approx(out[2][0], 0.31, 1e-7));
        assert!(approx(out[2][1], -0.42, 1e-7));
        for r in [0, 1, 3] {
            assert_eq!(out[r], proj[r]);
        }
        assert_eq!(out[2][2], proj[2][2]);
        assert_eq!(out[2][3], proj[2][3]);
        assert_eq!(jitter_projection(proj, [0.0, 0.0]), proj);
    }

    #[test]
    fn motion_field_validates_inputs() {
        let m = MotionField::zero(4, 4).expect("zero builds");
        assert_eq!((m.width, m.height, m.len()), (4, 4, 16));
        assert!(!m.is_empty());
        assert_eq!(m.at(2, 1), [0.0, 0.0]);
        assert!(matches!(
            MotionField::zero(0, 4),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            MotionField::zero(4096, 4),
            Err(PostError::BudgetExceeded { .. })
        ));
        assert!(matches!(
            MotionField::build(2, 2, vec![[0.0, 0.0]; 3]),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            MotionField::build(2, 2, vec![[f32::NAN, 0.0]; 4]),
            Err(PostError::Invalid(_))
        ));
    }

    #[test]
    fn taa_static_scene_resolves_exactly() {
        // History equals current: every sample reprojects 1:1 and the box
        // clamp is a no-op, so the resolve reproduces the frame exactly.
        let frame: Vec<[f32; 3]> = (0..64).map(|i| [i as f32 * 0.01, 0.2, 0.05]).collect();
        let motion = MotionField::zero(8, 8).expect("zero builds");
        let params = TaaParams::default();
        let out = taa_resolve(&frame, &frame, &motion, 8, 8, &params).expect("resolves");
        assert_eq!(out, frame, "static resolve is the identity");
        // Zero blend follows the current frame even with stale history.
        let stale = vec![[9.0, 9.0, 9.0]; 64];
        let follow = TaaParams {
            history_blend: 0.0,
            ..params
        };
        let out = taa_resolve(&frame, &stale, &motion, 8, 8, &follow).expect("resolves");
        assert_eq!(out, frame);
    }

    #[test]
    fn neighborhood_clamp_kills_ghosting() {
        // Moving-bar scene: the clamped resolve keeps the vacated trail
        // dark (posted gate GHOST_MAX_TRAIL), while the naive resolve
        // without the clamp smears history across the trail.
        let scene = GhostingScene::moving_bar().expect("scene builds");
        let params = TaaParams::default();
        let clamped = taa_resolve(
            &scene.current,
            &scene.prev,
            &scene.motion,
            scene.width,
            scene.height,
            &params,
        )
        .expect("resolves");
        let trail = GhostingScene::trail_luma(&clamped, scene.width, scene.trail);
        eprintln!("RP1 ghosting: clamped trail luma {trail:.4} (gate {GHOST_MAX_TRAIL})");
        assert!(
            trail <= GHOST_MAX_TRAIL,
            "ghost trail {trail} exceeds posted gate {GHOST_MAX_TRAIL}"
        );
        let naive = taa_resolve(
            &scene.current,
            &scene.prev,
            &scene.motion,
            scene.width,
            scene.height,
            &TaaParams {
                use_clamp: false,
                ..params
            },
        )
        .expect("naive resolves");
        let naive_trail = GhostingScene::trail_luma(&naive, scene.width, scene.trail);
        eprintln!("RP1 ghosting: naive trail luma {naive_trail:.4}");
        assert!(
            naive_trail > trail,
            "naive {naive_trail} must ghost more than clamped {trail}"
        );
        assert!(
            naive_trail > GHOST_MAX_TRAIL,
            "scene must actually ghost without the clamp, got {naive_trail}"
        );
        // Velocity rejection: a huge jump follows the current frame.
        let fast = MotionField::build(16, 16, vec![[500.0, 0.0]; 256]).expect("fast builds");
        let out =
            taa_resolve(&scene.current, &scene.prev, &fast, 16, 16, &params).expect("resolves");
        assert_eq!(out, scene.current, "fast movers follow current");
    }

    #[test]
    fn taa_upsample_is_exact_on_identity_and_solids() {
        // Same-size upscale is the bit-exact identity (weights collapse).
        let src = vec![[0.3, 0.6, 0.9]; 4 * 3];
        let same = taa_upsample(&src, 4, 3, 4, 3).expect("same size upscales");
        assert_eq!(same, src, "same-size TAAU is the identity");
        // Solid colors survive any upscale unchanged (no invented detail).
        let solid = vec![[0.7, 0.2, 0.1]; 2 * 2];
        let big = taa_upsample(&solid, 2, 2, 8, 6).expect("solid upscales");
        assert_eq!(big.len(), 8 * 6);
        assert!(big.iter().all(|p| *p == [0.7, 0.2, 0.1]));
        // 1x1 replicates to every destination texel.
        let one = taa_upsample(&[[1.0, 0.5, 0.0]], 1, 1, 3, 2).expect("1x1 upscales");
        assert_eq!(one, vec![[1.0, 0.5, 0.0]; 6]);
    }

    #[test]
    fn fsr_540p_to_1080p_posts_mtf50_shimmer_and_ms() {
        // 540p step edge to 1080p: spatial path must lift MTF50 over the
        // frozen bilinear TAAU at matched shimmer, with ms posted as evidence
        // (GPU bars need the reference box; CPU ms never gates here).
        let (src_w, src_h) = (960u32, 540u32);
        let (dst_w, dst_h) = (1920u32, 1080u32);
        let low = fsr_edge_lowres(src_w, src_h).expect("edge builds");
        let start = Instant::now();
        let base = taa_upsample(&low, src_w, src_h, dst_w, dst_h).expect("bilinear runs");
        let base_ms = start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        let cand =
            fsr_upscale(&low, src_w, src_h, dst_w, dst_h, FSR_DEFAULT_SHARPNESS).expect("fsr runs");
        let cand_ms = start.elapsed().as_secs_f64() * 1000.0;
        let base_w = fsr_edge_width_10_90(&base, dst_w, dst_h).expect("base width measures");
        let cand_w = fsr_edge_width_10_90(&cand, dst_w, dst_h).expect("cand width measures");
        let lift = fsr_mtf50_lift(base_w, cand_w).expect("lift computes");
        // Shimmer through the real pipeline: TAA settles the 16x16 checker,
        // then each settled frame upscales to 32x32. Both paths must hold the
        // spatial gate on the settled clip.
        let scene = ShimmerScene::checkerboard().expect("shimmer builds");
        let motion = MotionField::zero(scene.width, scene.height).expect("zero builds");
        let params = TaaParams::default();
        let mut history = vec![[0.5, 0.5, 0.5]; 256];
        let mut settled = Vec::new();
        for k in 0..12 {
            let current = &scene.frames[k % 2];
            let resolved = taa_resolve(
                current,
                &history,
                &motion,
                scene.width,
                scene.height,
                &params,
            )
            .expect("settles");
            history = resolved.clone();
            if k >= 8 {
                settled.push(resolved);
            }
        }
        let base_clip: Vec<Vec<[f32; 3]>> = settled
            .iter()
            .map(|f| taa_upsample(f, 16, 16, 32, 32).expect("base settled"))
            .collect();
        let cand_clip: Vec<Vec<[f32; 3]>> = settled
            .iter()
            .map(|f| fsr_upscale(f, 16, 16, 32, 32, FSR_DEFAULT_SHARPNESS).expect("cand settled"))
            .collect();
        let base_var = ShimmerScene::temporal_variance(&base_clip);
        let cand_var = ShimmerScene::temporal_variance(&cand_clip);
        // Present x2 identical: same inputs give bit-identical outputs.
        let again =
            fsr_upscale(&low, src_w, src_h, dst_w, dst_h, FSR_DEFAULT_SHARPNESS).expect("reruns");
        assert_eq!(cand, again, "spatial pixels bit-identical twice");
        eprintln!(
            "FSR 540p->1080p: base_width {base_w:.3}px cand_width {cand_w:.3}px lift {lift:.3} (gate {FSR_MTF50_LIFT_MIN}), base_var {base_var:.6} cand_var {cand_var:.6} (gate {FSR_SHIMMER_GATE}), base_ms {base_ms:.2}ms cand_ms {cand_ms:.2}ms"
        );
        assert!(
            lift >= FSR_MTF50_LIFT_MIN,
            "MTF50 lift {lift} under {FSR_MTF50_LIFT_MIN}"
        );
        assert!(
            cand_var <= FSR_SHIMMER_GATE,
            "shimmer {cand_var} over {FSR_SHIMMER_GATE}"
        );
        assert!(base_ms.is_finite() && cand_ms.is_finite(), "ms must post");
    }

    #[test]
    fn fsr_fails_loudly() {
        let ok = vec![[0.5, 0.5, 0.5]; 16];
        assert!(fsr_easu_con(0, 4, 8, 8).is_err());
        assert!(fsr_easu_upsample(&ok[..15], 4, 4, 8, 8).is_err());
        assert!(fsr_rcas_sharpen(&ok, 4, 4, 2.0).is_err());
        assert!(fsr_upscale(&ok, 4, 4, 4096, 4, FSR_DEFAULT_SHARPNESS).is_err());
        assert!(fsr_edge_width_10_90(&vec![[0.5, 0.5, 0.5]; 16], 4, 4).is_err());
        assert!(fsr_mtf50(0.0 - 1.0).is_err());
        assert!(fsr_mtf50_lift(-1.0, 1.0).is_err());
    }

    #[test]
    fn extract_glam_matches_scalar_bit_exact() {
        // SIMD slice: glam path must reproduce the scalar path
        // bit-identically on a mixed HDR field, plus an A/B timing line
        // for the receipt (wall clock only, never hashed).
        let cfg = BloomConfig::default();
        let mut hdr = vec![[0.1, 0.2, 0.3]; 32 * 32];
        hdr[5] = [8.0, 7.0, 6.0];
        hdr[999] = [0.0, 0.0, 0.0];
        let scalar = bright_extract_scalar(&hdr, cfg.threshold, cfg.knee);
        let dispatched = bright_extract(&hdr, cfg.threshold, cfg.knee);
        assert_eq!(scalar, dispatched, "extract paths agree bit-exactly");
        #[cfg(feature = "v1")]
        {
            let glam = bright_extract_glam(&hdr, cfg.threshold, cfg.knee);
            assert_eq!(scalar, glam, "glam SIMD matches scalar bit-exactly");
            let (scalar_ms, glam_ms) = bright_extract_ab(&hdr, cfg.threshold, cfg.knee);
            assert!(scalar_ms.is_finite() && glam_ms.is_finite());
            std::eprintln!("bright_extract A/B 32x32: scalar={scalar_ms:.3}ms glam={glam_ms:.3}ms");
        }
        // End-to-end bloom still flares through the dispatcher.
        let out = apply_bloom(&hdr, 32, 32, &cfg).expect("bloom runs");
        assert!(out[5][0] >= 8.0, "spark keeps its core");
    }

    #[test]
    fn bloom_fires_on_bright_not_dark() {
        // A lone HDR spark flares its neighbors; a dark field stays dark.
        let mut hdr = vec![[0.1, 0.1, 0.1]; 8 * 8];
        hdr[3 * 8 + 3] = [8.0, 8.0, 8.0];
        let cfg = BloomConfig::default();
        let out = apply_bloom(&hdr, 8, 8, &cfg).expect("bloom runs");
        assert!(out[3 * 8 + 3][0] >= 8.0, "spark keeps its core");
        let neighbor = out[3 * 8 + 4][0];
        assert!(
            neighbor > 0.1,
            "bloom lifts the spark neighbor, got {neighbor}"
        );
        let dark = vec![[0.1, 0.1, 0.1]; 8 * 8];
        let still = apply_bloom(&dark, 8, 8, &cfg).expect("dark blooms");
        let mut max_lift = 0.0f32;
        for (a, b) in dark.iter().zip(still.iter()) {
            max_lift = max_lift.max((b[0] - a[0]).abs());
        }
        assert_eq!(max_lift, 0.0, "sub-threshold field is untouched");
        // Zero intensity adds nothing back anywhere.
        let off = apply_bloom(
            &hdr,
            8,
            8,
            &BloomConfig {
                intensity: 0.0,
                ..cfg
            },
        )
        .expect("off blooms");
        assert_eq!(off, hdr, "zero intensity is the identity");
    }

    #[test]
    fn exposure_aces_grade_are_monotonic_and_bounded() {
        assert!(approx(exposure_mult(1.0).expect("ev"), 2.0, 1e-6));
        assert!(approx(exposure_mult(-1.0).expect("ev"), 0.5, 1e-6));
        assert!(matches!(exposure_mult(9.0), Err(PostError::Invalid(_))));
        // ACES fit pins: 0->0, 1->~0.8, monotone, saturates at 1.
        assert_eq!(aces_fitted(0.0), 0.0);
        assert!((aces_fitted(1.0) - 0.8).abs() < 0.01);
        assert!(aces_fitted(0.5) < aces_fitted(1.0));
        assert!(aces_fitted(1.0) < aces_fitted(4.0));
        assert_eq!(aces_fitted(100.0), 1.0);
        let mid = tonemap_aces([0.5, 0.5, 0.5], 1.0, 11.2).expect("tonemaps");
        let hi = tonemap_aces([2.0, 2.0, 2.0], 1.0, 11.2).expect("tonemaps");
        assert!(hi[0] > mid[0] && hi[0] <= 1.0);
        assert!(matches!(
            tonemap_aces([0.5, 0.5, 0.5], 1.0, 0.0),
            Err(PostError::Invalid(_))
        ));
        // Identity grade keeps pixels (f32 rounding: within 1e-6).
        let ident = GradeConfig::default();
        let c = [0.4, 0.2, 0.1];
        let kept = apply_grade(c, &ident).expect("grades");
        for i in 0..3 {
            assert!(
                (kept[i] - c[i]).abs() < 1e-6,
                "identity grade keeps {c:?}, got {kept:?}"
            );
        }
        let grey = apply_grade(
            c,
            &GradeConfig {
                saturation: 0.0,
                ..ident
            },
        )
        .expect("desats");
        assert!(
            (grey[0] - grey[1]).abs() < 1e-6 && (grey[1] - grey[2]).abs() < 1e-6,
            "zero saturation greys the pixel, got {grey:?}"
        );
    }

    #[test]
    fn chain_reads_rl1_rl2_and_is_deterministic() {
        // The chain consumes the RL1/RL2 receipts of the frame and quotes
        // their ms in the output receipt; same inputs run bit-identical x2.
        let hdr: Vec<[f32; 3]> = (0..256)
            .map(|i| [i as f32 * 0.01 + 0.2, 0.3, 0.15])
            .collect();
        let chain = PostChain::default();
        let run = || run_chain(&hdr, 16, 16, &chain, &lights()).expect("chain runs");
        let (a, b) = (run(), run());
        assert_eq!(a.ldr, b.ldr, "chain pixels bit-identical twice");
        assert_eq!((a.width, a.height), (16, 16));
        assert_eq!(a.ldr.len(), 256);
        for p in &a.ldr {
            assert!(p.iter().all(|c| *c >= 0.0 && *c <= 2.0));
        }
        let r = &a.receipt;
        assert_eq!(
            (r.sun_ms, r.cluster_ms, r.ibl_ms),
            (1.25, 0.5, 2.0),
            "receipt reads the RL1/RL2 pass ms"
        );
        assert!((r.lights_ms() - 3.75).abs() < 1e-9);
        assert!(r.total_ms() >= 0.0);
        for (pass, pixels) in [
            (r.bloom, 256),
            (r.exposure, 256),
            (r.aces, 256),
            (r.grade, 256),
        ] {
            assert_eq!(pass.pixels, pixels, "pass {} quotes pixels", pass.pass);
            assert!(pass.ms >= 0.0, "pass {} quotes ms", pass.pass);
        }
        // The no-device seam names the device-bound submitter instead of
        // pretending success; the live device path is gpu::Rp1Submitter.
        assert!(matches!(chain.submit_gpu(), Err(PostError::Device(_))));
    }

    #[test]
    fn full_frame_names_every_pass_in_ms() {
        // TAA resolve plus TAAU upsample plus the chain: every receipt slot
        // is filled and the output is display-bounded.
        let scene = GhostingScene::moving_bar().expect("scene builds");
        let taa = TaaParams::default();
        let chain = PostChain::default();
        let out = run_frame(
            &scene.current,
            &scene.prev,
            &scene.motion,
            scene.width,
            scene.height,
            32,
            32,
            &taa,
            &chain,
            &lights(),
        )
        .expect("frame runs");
        assert_eq!((out.width, out.height, out.ldr.len()), (32, 32, 1024));
        assert_eq!(out.receipt.taa.pixels, 256);
        assert_eq!(out.receipt.upsample.pixels, 1024);
        assert!(out.receipt.total_ms() >= 0.0);
        assert!((out.receipt.lights_ms() - 3.75).abs() < 1e-9);
        let again = run_frame(
            &scene.current,
            &scene.prev,
            &scene.motion,
            scene.width,
            scene.height,
            32,
            32,
            &taa,
            &chain,
            &lights(),
        )
        .expect("frame runs again");
        assert_eq!(out.ldr, again.ldr, "frame pixels bit-identical twice");
    }

    #[test]
    fn shimmer_converges_under_the_posted_gate() {
        // Raw checkerboard pair flickers; the clamped TAA resolve over the
        // alternating clip converges to a stable mean under SHIMMER_MAX_VAR.
        let scene = ShimmerScene::checkerboard().expect("scene builds");
        let raw_var = ShimmerScene::temporal_variance(&scene.frames);
        eprintln!("RP1 shimmer: raw variance {raw_var:.4}");
        assert!(
            raw_var > SHIMMER_MAX_VAR,
            "scene must actually shimmer, got {raw_var}"
        );
        let motion = MotionField::zero(scene.width, scene.height).expect("zero builds");
        let params = TaaParams::default();
        // Cold start from a cleared mid-grey history (the honest
        // presenter path): the resolve then tracks the alternating pair
        // into its limit cycle instead of dragging frame 0 along.
        let mut history = vec![[0.5, 0.5, 0.5]; scene.width as usize * scene.height as usize];
        let mut clip = Vec::new();
        for k in 0..12 {
            let current = &scene.frames[k % 2];
            let resolved = taa_resolve(
                current,
                &history,
                &motion,
                scene.width,
                scene.height,
                &params,
            )
            .expect("resolves");
            clip.push(resolved.clone());
            history = resolved;
        }
        let settled = ShimmerScene::temporal_variance(&clip[8..]);
        eprintln!("RP1 shimmer: settled variance {settled:.6} (gate {SHIMMER_MAX_VAR})");
        assert!(
            settled <= SHIMMER_MAX_VAR,
            "settled variance {settled} exceeds posted gate {SHIMMER_MAX_VAR}"
        );
        assert!(settled < raw_var, "TAA must calm the shimmer");
    }

    #[test]
    fn invalid_inputs_fail_loudly() {
        let motion = MotionField::zero(4, 4).expect("zero builds");
        let params = TaaParams::default();
        let frame = vec![[0.5, 0.5, 0.5]; 16];
        assert!(matches!(
            taa_resolve(&frame[..15], &frame, &motion, 4, 4, &params),
            Err(PostError::Invalid(_))
        ));
        let mut bad = frame.clone();
        bad[3] = [f32::INFINITY, 0.0, 0.0];
        assert!(matches!(
            taa_resolve(&bad, &frame, &motion, 4, 4, &params),
            Err(PostError::Invalid(_))
        ));
        let wrong = MotionField::zero(8, 8).expect("zero builds");
        assert!(matches!(
            taa_resolve(&frame, &frame, &wrong, 4, 4, &params),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            taa_resolve(
                &frame,
                &frame,
                &motion,
                4,
                4,
                &TaaParams {
                    history_blend: 2.0,
                    ..params
                }
            ),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            taa_upsample(&frame, 4, 4, 0, 4),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            taa_upsample(&frame[..15], 4, 4, 8, 8),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            taa_upsample(&frame, 4, 4, 4096, 4),
            Err(PostError::BudgetExceeded { .. })
        ));
        assert!(matches!(
            PostChain {
                aces_white: 0.0,
                ..PostChain::default()
            }
            .validate(),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            PostChain {
                bloom: BloomConfig {
                    levels: 0,
                    ..BloomConfig::default()
                },
                ..PostChain::default()
            }
            .validate(),
            Err(PostError::Invalid(_))
        ));
        let hdr = vec![[0.5, 0.5, 0.5]; 16];
        assert!(matches!(
            run_chain(&hdr, 4, 4, &PostChain::default(), &lights()).map(|_| ()),
            Ok(())
        ));
        assert!(matches!(
            run_chain(&hdr[..15], 4, 4, &PostChain::default(), &lights()),
            Err(PostError::Invalid(_))
        ));
        // PostError display names the failure without panicking.
        assert!(format!("{}", PostError::Device("needs a device".into())).contains("device"));
        assert!(format!("{}", PostError::Validation("scope hit".into())).contains("validation"));
        assert!(format!(
            "{}",
            PostError::BudgetExceeded {
                requested: 9,
                allowed: 4
            }
        )
        .contains('9'));
    }

    // ------------------------------------------------------------------
    // `v1` parity: jitter equals the R4 sequence, the glam projection
    // hook touches the same terms, ACES equals the `shadow_post`
    // Narkowicz path, and the TAAU path equals the test-only temporal
    // oracle on identical inputs. (The V1 receipt Bevy `ACESFitted` is a
    // different formulation and is not pinned here.)
    // ------------------------------------------------------------------

    #[cfg(feature = "v1")]
    #[test]
    fn jitter_and_aces_match_r4_r1_references() {
        for frame in [0u64, 1, 7, 123] {
            assert_eq!(
                jitter_offset(frame),
                crate::v1_lighting::TaaJitter::offset(frame),
                "frame {frame} pixels reuse the R4 offset"
            );
            assert_eq!(
                jitter_ndc(frame, 1280, 720),
                crate::v1_lighting::TaaJitter::ndc(frame, 1280, 720),
                "frame {frame} NDC reuses the R4 scaling"
            );
            let ndc = jitter_ndc(frame, 1280, 720);
            assert_eq!(
                jitter_projection_glam(glam::Mat4::IDENTITY, ndc),
                crate::v1_lighting::TaaJitter::apply_to_proj(glam::Mat4::IDENTITY, ndc),
                "frame {frame} projection wiring matches R4"
            );
        }
        for x in [0.0, 0.25, 1.0, 4.0] {
            assert_eq!(
                aces_fitted(x),
                crate::shadow_post::aces_fitted(x),
                "aces fit matches the shadow_post path at {x}"
            );
        }
        let src: Vec<[f32; 3]> = (0..64).map(|i| [i as f32 * 0.01, 0.2, 0.05]).collect();
        let ours = taa_upsample(&src, 8, 8, 16, 16).expect("taau runs");
        let theirs = crate::v1_temporal::upscale_bilinear(&src, 8, 8, 16, 16).expect("oracle runs");
        assert_eq!(ours, theirs, "TAAU equals the test-only oracle exactly");
    }

    #[cfg(feature = "v1")]
    #[test]
    fn postlights_reads_rl1_rl2_receipts() {
        // The taps constructor quotes the exact RL1/RL2 pass ms: the chain
        // reads the passes, never guesses them.
        let sun =
            crate::shadow_cluster::SunShadowPass::build(crate::shadow_cluster::SunShadowConfig {
                near_m: 0.5,
                far_m: 60.0,
                half_extents: [4.0, 8.0, 16.0],
                biases: [0.002, 0.004, 0.008],
                size: 128,
            })
            .expect("sun builds");
        let cluster = crate::shadow_cluster::ClusterPass::build(
            crate::shadow_cluster::ClusterConfig {
                width_px: 64,
                height_px: 64,
                tile_px: crate::shadow_cluster::CLUSTER_TILE_PX,
                depth_slices: 4,
                near_m: 0.5,
                far_m: 60.0,
                max_lights: crate::shadow_cluster::MAX_TORCH_LIGHTS,
                max_per_cluster: crate::shadow_cluster::MAX_LIGHTS_PER_CLUSTER,
            },
            &[],
        )
        .expect("cluster builds");
        let lights = crate::shadow_cluster::FrameLightsReceipt {
            sun: sun.receipt(1.25),
            cluster: cluster.receipt(),
        };
        let ibl = crate::ibl::IblReceipt {
            cube_texels: 1,
            lut_texels: 1,
            sh_coeffs: crate::ibl::IBL_SH_COEFFS as u32,
            ms: 2.0,
        };
        let taps = PostLights::from_receipts(&lights, &ibl);
        assert_eq!(
            (taps.sun_ms, taps.cluster_ms, taps.ibl_ms),
            (1.25, cluster.last_ms, 2.0)
        );
    }

    // ------------------------------------------------------------------
    // GPU descriptors behind `window`: pure format/extent checks plus a
    // real history allocation that skips without an adapter (RA1 pattern).
    // ------------------------------------------------------------------

    #[cfg(feature = "window")]
    #[test]
    fn post_descriptors_are_sampler_readable() {
        let hist = gpu::history_desc(128, 72);
        assert_eq!(hist.format, wgpu::TextureFormat::Rgba32Float);
        assert_eq!((hist.size.width, hist.size.height), (128, 72));
        assert!(hist.usage.contains(wgpu::TextureUsages::RENDER_ATTACHMENT));
        assert!(hist.usage.contains(wgpu::TextureUsages::TEXTURE_BINDING));
        let target = gpu::post_target_desc(128, 72);
        assert_eq!(target.format, wgpu::TextureFormat::Rgba16Float);
        assert!(!gpu::TAA_RESOLVE_WGSL.is_empty());
        assert!(!gpu::POST_CHAIN_WGSL.is_empty());
        assert!(matches!(
            gpu::check_history_dims(0, 0),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            gpu::check_history_dims(4096, 64),
            Err(PostError::BudgetExceeded { .. })
        ));
        gpu::check_history_dims(64, 64).expect("valid dims pass");
    }

    #[cfg(feature = "window")]
    #[test]
    fn history_allocates_on_llvmpipe() {
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
                    eprintln!("SKIPPED post gpu test: no wgpu adapter on this machine");
                    return;
                }
            };
        let (device, _queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("rp1-post-history"),
                ..Default::default()
            }))
            .expect("device requests succeed when the adapter exists");
        let tex = gpu::allocate_history(&device, 64, 64).expect("history allocates");
        assert_eq!(tex.width(), 64);
        assert!(matches!(
            gpu::allocate_history(&device, 4096, 64),
            Err(PostError::BudgetExceeded { .. })
        ));
    }

    #[test]
    fn jittered_resolve_is_plain_at_zero_and_shifts_otherwise() {
        // Zero jitter is exactly the plain resolve; a real Halton offset
        // shifts sub-pixel sampling on the bar edge.
        let scene = GhostingScene::moving_bar().expect("scene builds");
        let params = TaaParams::default();
        let plain = taa_resolve(
            &scene.current,
            &scene.prev,
            &scene.motion,
            scene.width,
            scene.height,
            &params,
        )
        .expect("plain resolves");
        let zero = taa_resolve_jittered(
            &scene.current,
            &scene.prev,
            &scene.motion,
            scene.width,
            scene.height,
            &params,
            [0.0, 0.0],
        )
        .expect("zero jitter resolves");
        assert_eq!(plain, zero, "zero jitter is the plain path");
        let j = jitter_offset(1);
        assert!(j != [0.0, 0.0], "frame 1 jitters off the origin");
        let shifted = taa_resolve_jittered(
            &scene.current,
            &scene.prev,
            &scene.motion,
            scene.width,
            scene.height,
            &params,
            j,
        )
        .expect("jittered resolves");
        assert_ne!(shifted, plain, "jitter shifts the bar edge sampling");
        let again = taa_resolve_jittered(
            &scene.current,
            &scene.prev,
            &scene.motion,
            scene.width,
            scene.height,
            &params,
            j,
        )
        .expect("jittered resolves again");
        assert_eq!(shifted, again, "jittered pixels bit-identical twice");
        assert!(matches!(
            taa_resolve_jittered(
                &scene.current,
                &scene.prev,
                &scene.motion,
                scene.width,
                scene.height,
                &params,
                [f32::NAN, 0.0]
            ),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            taa_resolve_jittered(
                &scene.current,
                &scene.prev,
                &scene.motion,
                scene.width,
                scene.height,
                &params,
                [2.0, 0.0]
            ),
            Err(PostError::Invalid(_))
        ));
    }

    #[test]
    fn rp1_graph_declares_six_passes_in_order() {
        // The RA2 declaration names the six RP1 stages in pipeline order;
        // the compiled order pins the same sequence and the CPU executor
        // records one draw per pass.
        let chain = PostChain::default();
        let graph = build_rp1_graph(16, 16, 32, 32, &chain).expect("graph builds");
        assert_eq!(graph.pass_count(), 6);
        assert_eq!(graph.pass(0).map(|p| p.name.as_str()), Some(RP1_PASS_TAA));
        let mut compiled = compile_rp1_for_present(16, 16, 32, 32, &chain).expect("compiles");
        let names: Vec<&str> = compiled
            .order()
            .iter()
            .filter_map(|id| compiled.pass(*id).map(|p| p.name.as_str()))
            .collect();
        assert_eq!(names, rp1_pass_names(), "compiled order is the chain order");
        let receipt = compiled.execute(&crate::graph::FrameInfo {
            frame_index: 7,
            width_px: 32,
            height_px: 32,
        });
        assert_eq!(receipt.draws, 6, "one draw per RP1 pass");
        assert_eq!(receipt.per_pass_cpu_ms.len(), 6);
        assert_eq!(receipt.frame_index, 7);
    }

    #[test]
    fn rp1_graph_rejects_bad_sizes_and_counts_bytes() {
        let chain = PostChain::default();
        assert!(matches!(
            build_rp1_graph(0, 16, 16, 16, &chain),
            Err(PostError::Invalid(_))
        ));
        build_rp1_graph(16, 16, 16, 16, &chain).expect("square present is fine");
        assert!(matches!(
            build_rp1_graph(4096, 16, 16, 16, &chain),
            Err(PostError::BudgetExceeded { .. })
        ));
        assert!(matches!(
            build_rp1_graph(
                16,
                16,
                16,
                16,
                &PostChain {
                    bloom: BloomConfig {
                        levels: 0,
                        ..BloomConfig::default()
                    },
                    ..PostChain::default()
                }
            ),
            Err(PostError::Invalid(_))
        ));
        let small = rp1_transient_bytes(16, 16, 16, 16, 1).expect("bytes count");
        let deep = rp1_transient_bytes(16, 16, 16, 16, 4).expect("bytes count");
        assert!(
            small > 0 && deep > small,
            "more bloom levels need more bytes"
        );
        assert!(matches!(
            rp1_transient_bytes(16, 16, 16, 16, 0),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            rp1_transient_bytes(4096, 16, 16, 16, 4),
            Err(PostError::BudgetExceeded { .. })
        ));
    }

    #[cfg(feature = "window")]
    fn test_device() -> Option<(wgpu::Device, wgpu::Queue)> {
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
                    eprintln!("SKIPPED post gpu test: no wgpu adapter on this machine");
                    return None;
                }
            };
        match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("rp1-test"),
            ..Default::default()
        })) {
            Ok((d, q)) => Some((d, q)),
            Err(e) => panic!("test device requests succeed when the adapter exists: {e}"),
        }
    }

    #[cfg(feature = "window")]
    #[test]
    fn gpu_taa_kills_ghost_and_matches_cpu() {
        // Device TAA over the moving-bar scene: reprojection plus jitter
        // plus box clamp on the device, parity against the CPU jittered
        // oracle, trail under the posted gate, 0 validation errors.
        let (device, queue) = match test_device() {
            Some(d) => d,
            None => return,
        };
        let sub = gpu::Rp1Submitter::new(&device);
        let scene = GhostingScene::moving_bar().expect("scene builds");
        let params = TaaParams::default();
        let jit = jitter_offset(7);
        let cpu = taa_resolve_jittered(
            &scene.current,
            &scene.prev,
            &scene.motion,
            scene.width,
            scene.height,
            &params,
            jit,
        )
        .expect("cpu jittered resolves");
        let g = sub
            .resolve(
                &device,
                &queue,
                &scene.current,
                &scene.prev,
                &scene.motion,
                scene.width,
                scene.height,
                &params,
                jit,
            )
            .expect("gpu resolves");
        assert_eq!(g.validation_errors, 0, "gpu resolve is validation clean");
        let diff = gpu::max_abs_diff(&cpu, &g.pixels);
        let trail = GhostingScene::trail_luma(&g.pixels, scene.width, scene.trail);
        eprintln!(
            "RP1 gpu ghost: trail {trail:.4} (gate {GHOST_MAX_TRAIL}), parity {diff:.6} (tol {})",
            gpu::GPU_PARITY_TOL_TAA
        );
        assert!(
            diff <= gpu::GPU_PARITY_TOL_TAA,
            "gpu/cpu parity {diff} exceeds {}",
            gpu::GPU_PARITY_TOL_TAA
        );
        assert!(
            trail <= GHOST_MAX_TRAIL,
            "gpu trail {trail} exceeds posted gate {GHOST_MAX_TRAIL}"
        );
        // Failure case first: motion/frame mismatch fails before any submit.
        let wrong = MotionField::zero(8, 8).expect("zero builds");
        assert!(matches!(
            sub.resolve(
                &device,
                &queue,
                &scene.current,
                &scene.prev,
                &wrong,
                scene.width,
                scene.height,
                &params,
                jit
            ),
            Err(PostError::Invalid(_))
        ));
    }

    #[cfg(feature = "window")]
    #[test]
    fn gpu_shimmer_settles_and_matches_cpu() {
        // Alternating checkerboard through the device resolve with the
        // per-frame Halton jitter: the settled clip variance stays under
        // the posted gate and every GPU frame matches its CPU twin.
        let (device, queue) = match test_device() {
            Some(d) => d,
            None => return,
        };
        let sub = gpu::Rp1Submitter::new(&device);
        let scene = ShimmerScene::checkerboard().expect("scene builds");
        let motion = MotionField::zero(scene.width, scene.height).expect("zero builds");
        let params = TaaParams::default();
        let mut history = vec![[0.5, 0.5, 0.5]; scene.width as usize * scene.height as usize];
        let mut clip = Vec::new();
        let mut worst_diff = 0f32;
        for k in 0..12 {
            let current = &scene.frames[k % 2];
            let jit = jitter_offset(k as u64);
            let cpu = taa_resolve_jittered(
                current,
                &history,
                &motion,
                scene.width,
                scene.height,
                &params,
                jit,
            )
            .expect("cpu resolves");
            let g = sub
                .resolve(
                    &device,
                    &queue,
                    current,
                    &history,
                    &motion,
                    scene.width,
                    scene.height,
                    &params,
                    jit,
                )
                .expect("gpu resolves");
            assert_eq!(g.validation_errors, 0);
            worst_diff = worst_diff.max(gpu::max_abs_diff(&cpu, &g.pixels));
            clip.push(g.pixels.clone());
            history = g.pixels;
        }
        let settled = ShimmerScene::temporal_variance(&clip[8..]);
        eprintln!(
            "RP1 gpu shimmer: settled {settled:.6} (gate {SHIMMER_MAX_VAR}), worst parity {worst_diff:.6}"
        );
        assert!(
            worst_diff <= gpu::GPU_PARITY_TOL_TAA,
            "gpu/cpu parity {worst_diff} exceeds {}",
            gpu::GPU_PARITY_TOL_TAA
        );
        assert!(
            settled <= SHIMMER_MAX_VAR,
            "gpu settled variance {settled} exceeds posted gate {SHIMMER_MAX_VAR}"
        );
    }

    #[cfg(feature = "window")]
    #[test]
    fn gpu_full_frame_matches_cpu_chain() {
        // Whole RP1 frame on the device in graph order (TAA at 16x16,
        // TAAU to 32x32, bloom plus exposure plus ACES plus grade): pixels
        // match the CPU oracle within tolerance, every receipt slot is
        // filled, 0 validation errors.
        let (device, queue) = match test_device() {
            Some(d) => d,
            None => return,
        };
        let sub = gpu::Rp1Submitter::new(&device);
        let scene = GhostingScene::moving_bar().expect("scene builds");
        let taa = TaaParams::default();
        let chain = PostChain::default();
        let jit = [0.0, 0.0];
        let g = sub
            .run_frame(
                &device,
                &queue,
                &scene.current,
                &scene.prev,
                &scene.motion,
                scene.width,
                scene.height,
                32,
                32,
                &taa,
                &chain,
                &lights(),
                jit,
            )
            .expect("gpu frame runs");
        assert_eq!(g.validation_errors, 0, "gpu frame is validation clean");
        let cpu = run_frame(
            &scene.current,
            &scene.prev,
            &scene.motion,
            scene.width,
            scene.height,
            32,
            32,
            &taa,
            &chain,
            &lights(),
        )
        .expect("cpu frame runs");
        assert_eq!((g.width, g.height, g.ldr.len()), (32, 32, 1024));
        // Bloom-shape sweep at 16x16: same-size (TAAU identity), bloom-off
        // (base path only), single/two-level chains and a full-flood
        // stress every corner of the composite against the CPU oracle.
        let shapes: [(&str, PostChain); 5] = [
            ("same-size", PostChain::default()),
            (
                "bloom-off",
                PostChain {
                    bloom: BloomConfig {
                        intensity: 0.0,
                        ..BloomConfig::default()
                    },
                    ..PostChain::default()
                },
            ),
            (
                "flood",
                PostChain {
                    bloom: BloomConfig {
                        threshold: 0.0,
                        knee: 0.0,
                        intensity: 1.0,
                        ..BloomConfig::default()
                    },
                    ..PostChain::default()
                },
            ),
            (
                "one-level",
                PostChain {
                    bloom: BloomConfig {
                        levels: 1,
                        ..BloomConfig::default()
                    },
                    ..PostChain::default()
                },
            ),
            (
                "two-level",
                PostChain {
                    bloom: BloomConfig {
                        levels: 2,
                        ..BloomConfig::default()
                    },
                    ..PostChain::default()
                },
            ),
        ];
        for (name, cfg) in shapes {
            let gf = sub
                .run_frame(
                    &device,
                    &queue,
                    &scene.current,
                    &scene.prev,
                    &scene.motion,
                    scene.width,
                    scene.height,
                    16,
                    16,
                    &taa,
                    &cfg,
                    &lights(),
                    jit,
                )
                .expect("shape gpu frame runs");
            let cf = run_frame(
                &scene.current,
                &scene.prev,
                &scene.motion,
                scene.width,
                scene.height,
                16,
                16,
                &taa,
                &cfg,
                &lights(),
            )
            .expect("shape cpu frame runs");
            let diff = gpu::max_abs_diff(&cf.ldr, &gf.ldr);
            eprintln!("RP1 gpu frame shape {name}: parity {diff:.6}");
            assert_eq!(gf.validation_errors, 0);
            assert!(
                diff <= gpu::GPU_PARITY_TOL_POST,
                "shape {name} parity {diff} exceeds {}",
                gpu::GPU_PARITY_TOL_POST
            );
        }
        eprintln!(
            "RP1 gpu frame: parity {:.6} (tol {}), submit {:.2}ms (budget {}ms, evidence only)",
            g.parity_max_diff,
            gpu::GPU_PARITY_TOL_POST,
            g.submit_ms,
            gpu::GPU_SUBMIT_BUDGET_MS
        );
        assert!(
            g.parity_max_diff <= gpu::GPU_PARITY_TOL_POST,
            "gpu/cpu frame parity {} exceeds {}",
            g.parity_max_diff,
            gpu::GPU_PARITY_TOL_POST
        );
        assert_eq!(gpu::max_abs_diff(&cpu.ldr, &g.ldr), g.parity_max_diff);
        assert_eq!(
            (g.receipt.taa.pixels, g.receipt.upsample.pixels),
            (256, 1024)
        );
        assert_eq!(g.receipt.taa.pass, "taa");
        assert_eq!(g.receipt.upsample.pass, "taau");
        assert_eq!(g.receipt.bloom.pass, "bloom");
        assert!((g.receipt.lights_ms() - 3.75).abs() < 1e-9);
        // Timing is evidence only (PF2 owns budgets on the reference GPU):
        // debug llvmpipe under pool contention is noisy by construction.
        // Jittered device frame matches the jittered CPU pipeline too.
        let jit2 = jitter_offset(3);
        let g2 = sub
            .run_frame(
                &device,
                &queue,
                &scene.current,
                &scene.prev,
                &scene.motion,
                scene.width,
                scene.height,
                16,
                16,
                &taa,
                &chain,
                &lights(),
                jit2,
            )
            .expect("jittered gpu frame runs");
        let resolved = taa_resolve_jittered(
            &scene.current,
            &scene.prev,
            &scene.motion,
            16,
            16,
            &taa,
            jit2,
        )
        .expect("cpu jittered resolves");
        let up = taa_upsample(&resolved, 16, 16, 16, 16).expect("cpu upscales");
        let oracle = run_chain(&up, 16, 16, &chain, &lights()).expect("cpu chain runs");
        let diff2 = gpu::max_abs_diff(&oracle.ldr, &g2.ldr);
        eprintln!("RP1 gpu jittered frame: parity {diff2:.6}");
        assert!(
            diff2 <= gpu::GPU_PARITY_TOL_POST,
            "jittered gpu/cpu parity {diff2} exceeds {}",
            gpu::GPU_PARITY_TOL_POST
        );
    }

    #[test]
    fn rp5_scale_seam_drives_presented_size() {
        // RP5 production wiring: the post/TAAU seam feeds the live
        // ScaleController into `compile_taa_off_for_scale` so the
        // SMAA-first graph drives the presented size. Under load the
        // hysteresis floors the scale, the floor picks the render size,
        // and the compiled graph is SMAA-first with the RP1 TAAU plus
        // chain after it; clearing the load recovers full rate.
        use crate::aa_scale::{
            taa_off_pass_names, ScaleController, ScalePolicy, SmaaParams, TARGET_60FPS_MS,
        };
        let policy = ScalePolicy {
            over_frames: 2,
            under_frames: 4,
            ..ScalePolicy::default()
        };
        let mut ctl = ScaleController::new(policy).expect("policy builds");
        let chain = PostChain::default();
        // Sustained overload floors the scale through the per-present
        // tick: every present feeds PF2 frame ms through the hysteresis
        // and recompiles the SMAA-first graph at the new scale.
        for _ in 0..40 {
            tick_scale_for_present(&mut ctl, TARGET_60FPS_MS + 9.0, 1920, 1080, &chain)
                .expect("tick compiles");
        }
        assert_eq!(ctl.scale(), policy.min_scale, "load floors the scale");
        // Scale-hold figure: pixel-proportional cost at the floor tail
        // holds 60 fps (fixed 4ms plus 20ms at full rate, quadratic).
        let tail_ms = 4.0 + 20.0 * f64::from(ctl.scale() * ctl.scale());
        eprintln!(
            "RP5 scale under load: scale {:.2}, down {}, up {}, tail {tail_ms:.2}ms (target {TARGET_60FPS_MS})",
            ctl.scale(),
            ctl.steps_down,
            ctl.steps_up
        );
        assert!(
            tail_ms <= TARGET_60FPS_MS,
            "tail cost {tail_ms:.2}ms must hold 60 fps"
        );
        let (compiled, rw, rh) =
            compile_taa_off_for_scale(&ctl, 1920, 1080, &chain).expect("floor compiles");
        assert_eq!((rw, rh), (960, 540));
        assert_eq!(compiled.order().len(), 6);
        assert!(
            compiled.order_names_match(&taa_off_pass_names()),
            "taa-off leads with smaa through the graph seam"
        );
        // CPU oracle at the controller scale runs bit-identical twice and
        // lands at the presented size (small present keeps the debug
        // suite fast; the 1080p floor compile above posts the numbers).
        let smaa = SmaaParams::default();
        let hdr: Vec<[f32; 3]> = (0..160usize * 90usize)
            .map(|i| [i as f32 * 0.00001 + 0.2, 0.3, 0.15])
            .collect();
        let (a, b) = (
            run_taa_off_frame_for_scale(&hdr, &ctl, 320, 180, &smaa, &chain, &lights())
                .expect("scale frame runs"),
            run_taa_off_frame_for_scale(&hdr, &ctl, 320, 180, &smaa, &chain, &lights())
                .expect("scale frame runs again"),
        );
        assert_eq!(a.ldr, b.ldr, "scale frame pixels bit-identical twice");
        assert_eq!((a.width, a.height), (320, 180));
        assert_eq!(a.receipt.smaa.pixels, 160 * 90);
        // Clearing the load climbs back to full rate with the full-size graph.
        for _ in 0..120 {
            tick_scale_for_present(&mut ctl, 8.0, 1920, 1080, &chain).expect("tick compiles");
        }
        assert_eq!(ctl.scale(), 1.0, "cleared load recovers full rate");
        let (full, frw, frh) =
            compile_taa_off_for_scale(&ctl, 1920, 1080, &chain).expect("full compiles");
        assert_eq!((frw, frh), (1920, 1080));
        assert!(full.order_names_match(&taa_off_pass_names()));
        eprintln!("RP5 scale graph: floor {rw}x{rh}, full {frw}x{frh}");
    }

    #[test]
    fn rp5_scale_seam_fails_loudly() {
        // Failure edges: zero and over-bound present sizes, non-finite
        // frame ms, and a non-SMAA-first order all fail loudly (never a
        // silent wrong-size present); error kinds survive the aa_scale
        // mapping.
        use crate::aa_scale::{ScaleController, ScalePolicy, SmaaParams};
        let mut ctl = ScaleController::new(ScalePolicy::default()).expect("policy builds");
        let chain = PostChain::default();
        assert!(matches!(
            compile_taa_off_for_scale(&ctl, 0, 1080, &chain),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            compile_taa_off_for_scale(&ctl, 4096, 1080, &chain),
            Err(PostError::BudgetExceeded { .. })
        ));
        assert!(matches!(
            tick_scale_for_present(&mut ctl, f64::NAN, 1920, 1080, &chain),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            tick_scale_for_present(&mut ctl, -1.0, 1920, 1080, &chain),
            Err(PostError::Invalid(_))
        ));
        // The graph seam itself: RP1 order is not SMAA-first.
        let rp1 = compile_rp1_for_present(16, 16, 32, 32, &chain).expect("rp1 compiles");
        assert!(!rp1.order_names_match(&crate::aa_scale::taa_off_pass_names()));
        assert!(rp1.order_names_match(&rp1_pass_names()));
        // Undersized hdr for the controller render size fails loudly.
        let smaa = SmaaParams::default();
        let short = vec![[0.5, 0.5, 0.5]; 16];
        assert!(matches!(
            run_taa_off_frame_for_scale(&short, &ctl, 32, 32, &smaa, &chain, &lights()),
            Err(PostError::Invalid(_))
        ));
        // Error-kind mapping keeps BudgetExceeded across the seam.
        let mapped = PostError::from(crate::aa_scale::AaScaleError::BudgetExceeded {
            requested: 4096,
            allowed: 2048,
        });
        assert!(matches!(
            mapped,
            PostError::BudgetExceeded {
                requested: 4096,
                allowed: 2048
            }
        ));
    }

    #[test]
    fn post_frame_budget_accessors_are_exact() {
        // Exactness default (no wall clock): the posted full-post bar is
        // 1.5 ms at 1080p on boosted release clocks (PF2 enforces); synthetic
        // receipts pin the flag both ways. Wall-clock enforcement lives in
        // post_chain_wall_budget (--perf only, load-dependent).
        const POST_FRAME_BUDGET_MS: f64 = 1.5;
        let flat = |ms: f64| FramePostReceipt {
            taa: PassReceipt {
                pass: "taa",
                pixels: 256,
                ms,
            },
            upsample: PassReceipt {
                pass: "taau",
                pixels: 1024,
                ms,
            },
            bloom: PassReceipt {
                pass: "bloom",
                pixels: 1024,
                ms,
            },
            exposure: PassReceipt {
                pass: "exposure",
                pixels: 1024,
                ms,
            },
            aces: PassReceipt {
                pass: "aces",
                pixels: 1024,
                ms,
            },
            grade: PassReceipt {
                pass: "grade",
                pixels: 1024,
                ms,
            },
            sun_ms: 1.25,
            cluster_ms: 0.5,
            ibl_ms: 2.0,
        };
        assert!((flat(0.1).total_ms() - 0.6).abs() < 1e-12);
        assert!(flat(0.1).total_ms() <= POST_FRAME_BUDGET_MS);
        assert!(flat(9.9).total_ms() > POST_FRAME_BUDGET_MS);
        assert!((flat(0.1).lights_ms() - 3.75).abs() < 1e-9);
    }

    #[test]
    #[ignore = "timing: post chain wall-clock needs release --perf on boosted clocks (fails under parallel load)"]
    fn post_chain_wall_budget() {
        // Seeded budget regression: the same 16x16 chain the default suite
        // proves exact must clear the 1.5 ms bar on boosted release clocks.
        // Perf harness line keeps --perf medians readable.
        const POST_FRAME_BUDGET_MS: f64 = 1.5;
        let chain = PostChain::default();
        let hdr = vec![[0.35, 0.3, 0.25]; 16 * 16];
        let out = run_chain(&hdr, 16, 16, &chain, &lights()).expect("chain runs");
        eprintln!(
            "post_chain_wall_budget: measured {:.3} ms vs {:.1} ms budget",
            out.receipt.total_ms(),
            POST_FRAME_BUDGET_MS
        );
        assert!(
            out.receipt.total_ms() <= POST_FRAME_BUDGET_MS,
            "seeded chain over budget: {:.3} ms vs {:.1} ms",
            out.receipt.total_ms(),
            POST_FRAME_BUDGET_MS
        );
    }

    // ------------------------------------------------------------------
    // Auto-exposure: histogram, trim, adaptation, fixtures, cost.
    // ------------------------------------------------------------------

    #[test]
    fn auto_histogram_bins_edges_and_validation() {
        // A flat key field lands in one bin with every texel counted.
        let flat = vec![[AUTO_KEY_LUMA, AUTO_KEY_LUMA, AUTO_KEY_LUMA]; 16];
        let hist = histogram_log_luma(&flat, 4, 4).expect("histograms");
        assert_eq!(hist.texels, 16.0);
        let filled: Vec<usize> = hist
            .bins
            .iter()
            .enumerate()
            .filter(|(_, count)| **count > 0.0)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(filled.len(), 1, "flat field fills one bin");
        assert_eq!(hist.bins[filled[0]], 16.0);
        // Determinism: the same frame bins identically twice.
        let again = histogram_log_luma(&flat, 4, 4).expect("histograms again");
        assert_eq!(hist, again);
        // Non-positive luma parks in bin 0 (deep shadow, never NaN).
        let black = vec![[0.0, 0.0, 0.0]; 4];
        let parked = histogram_log_luma(&black, 2, 2).expect("black histograms");
        assert_eq!(parked.bins[0], 4.0);
        // Validation matches the chain inputs.
        assert!(matches!(
            histogram_log_luma(&flat[..15], 4, 4),
            Err(PostError::Invalid(_))
        ));
        let mut nan = flat.clone();
        nan[3] = [f32::NAN, 0.0, 0.0];
        assert!(matches!(
            histogram_log_luma(&nan, 4, 4),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            histogram_log_luma(&flat, 0, 4),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            histogram_log_luma(&flat, 4096, 4),
            Err(PostError::BudgetExceeded { .. })
        ));
    }

    #[test]
    fn auto_trim_and_target_math() {
        // Four grey texels one stop apart: the full-band mean is the middle
        // (bin centers quantize, so the gate is a fifth of a stop).
        let hdr = vec![
            [1.0, 1.0, 1.0],
            [2.0, 2.0, 2.0],
            [4.0, 4.0, 4.0],
            [8.0, 8.0, 8.0],
        ];
        let hist = histogram_log_luma(&hdr, 2, 2).expect("histograms");
        assert_eq!(hist.texels, 4.0);
        let full = trimmed_log_mean(&hist, 0.0, 1.0).expect("means");
        assert!(
            (full - 1.5).abs() <= 0.2,
            "full band means the middle stops, got {full}"
        );
        // Narrow bands read below/above the full mean: trim is order-correct.
        let dark_half = trimmed_log_mean(&hist, 0.0, 0.5).expect("dark means");
        let bright_half = trimmed_log_mean(&hist, 0.5, 1.0).expect("bright means");
        assert!(
            dark_half < full && full < bright_half,
            "bands order {dark_half} < {full} < {bright_half}"
        );
        // Torch-grid discipline: sparse highlights barely move the mean.
        let scene = ExposureScene::day_sweep_torch().expect("scene builds");
        let dark_hist =
            histogram_log_luma(&scene.dark, scene.width, scene.height).expect("dark histograms");
        let torch_hist =
            histogram_log_luma(&scene.torch, scene.width, scene.height).expect("torch histograms");
        let dark_mean =
            trimmed_log_mean(&dark_hist, AUTO_TRIM_LO, AUTO_TRIM_HI).expect("dark means");
        let torch_mean =
            trimmed_log_mean(&torch_hist, AUTO_TRIM_LO, AUTO_TRIM_HI).expect("torch means");
        assert!(
            (torch_mean - dark_mean).abs() < AUTO_FLICKER_MAX_EV,
            "trim keeps torches out: dark {dark_mean} vs torch {torch_mean}"
        );
        // Key math: the mean of key needs no correction, exactly.
        let key_log = AUTO_KEY_LUMA.log2();
        assert_eq!(
            target_ev_for_mean(key_log, AUTO_KEY_LUMA).expect("targets"),
            0.0
        );
        // A mean one stop brighter costs exactly one stop (1e-6 for libm).
        let darker = target_ev_for_mean(key_log + 1.0, AUTO_KEY_LUMA).expect("targets");
        assert!(
            (darker - -1.0).abs() < 1e-6,
            "one stop brighter costs one stop, got {darker}"
        );
    }

    #[test]
    fn auto_exposure_config_validates() {
        AutoExposureConfig::default()
            .validate()
            .expect("default validates");
        assert_eq!(AutoExposure::default().ev(), 0.0);
        AutoExposure::new(AutoExposureConfig::default()).expect("default builds");
        for cfg in [
            AutoExposureConfig {
                trim_lo: -0.1,
                ..AutoExposureConfig::default()
            },
            AutoExposureConfig {
                trim_hi: 1.5,
                ..AutoExposureConfig::default()
            },
            AutoExposureConfig {
                trim_lo: 0.9,
                trim_hi: 0.1,
                ..AutoExposureConfig::default()
            },
            AutoExposureConfig {
                key_luma: 0.0,
                ..AutoExposureConfig::default()
            },
            AutoExposureConfig {
                adapt_ev_per_s: 0.0,
                ..AutoExposureConfig::default()
            },
            AutoExposureConfig {
                min_ev: 2.0,
                max_ev: 1.0,
                ..AutoExposureConfig::default()
            },
            AutoExposureConfig {
                min_ev: -9.0,
                ..AutoExposureConfig::default()
            },
            AutoExposureConfig {
                start_ev: 9.0,
                ..AutoExposureConfig::default()
            },
        ] {
            assert!(
                matches!(AutoExposure::new(cfg), Err(PostError::Invalid(_))),
                "bad config {cfg:?} must fail"
            );
        }
    }

    #[test]
    fn auto_exposure_converges_and_holds() {
        let scene = ExposureScene::day_sweep_torch().expect("scene builds");
        let bright = &scene.sweep[4];
        let hist = histogram_log_luma(bright, scene.width, scene.height).expect("histograms");
        let mean = trimmed_log_mean(&hist, AUTO_TRIM_LO, AUTO_TRIM_HI).expect("means");
        let target = target_ev_for_mean(mean, AUTO_KEY_LUMA).expect("targets");
        assert!(
            target < 0.0,
            "bright sweep wants darker exposure, got {target}"
        );
        // Slew dynamics: 3 stops/s closes the gap, then lands on target.
        let mut state = AutoExposure::default();
        let first = state.tick(bright, 16, 16, 1.0).expect("ticks");
        assert!(
            (-AUTO_ADAPT_EV_PER_S..0.0).contains(&first),
            "first second slews down at most the rate, got {first}"
        );
        for _ in 0..4 {
            state.tick(bright, 16, 16, 1.0).expect("ticks");
        }
        assert!(
            (state.ev() - target).abs() < 1e-5,
            "slew lands on target: got {} want {target}",
            state.ev()
        );
        // Holding: a zero dt never moves, whatever the frame.
        let before = state.ev();
        state.tick(&scene.sweep[0], 16, 16, 0.0).expect("holds");
        assert_eq!(state.ev(), before, "zero dt holds the state");
        // Twin states track bit-identically (replays and captures agree).
        let mut a = AutoExposure::default();
        let mut b = AutoExposure::default();
        for k in 0..6 {
            let frame = &scene.sweep[k % scene.sweep.len()];
            let ea = a.tick(frame, 16, 16, 1.0 / 60.0).expect("ticks");
            let eb = b.tick(frame, 16, 16, 1.0 / 60.0).expect("ticks");
            assert_eq!(ea, eb, "twin states agree on frame {k}");
        }
        // Multiplier matches the stops (1e-6 for the libm path).
        assert!((a.mult() - 2f32.powf(a.ev())).abs() < 1e-6);
    }

    #[test]
    fn auto_exposure_beats_fixed_on_day_sweep() {
        // The accept metric: fixed stops clip the bright sweep end, the
        // converged adapted state cuts the clipped share by 15% or more.
        let scene = ExposureScene::day_sweep_torch().expect("scene builds");
        let bright = &scene.sweep[4];
        let fixed = PostChain::default();
        let fixed_out =
            run_chain(bright, scene.width, scene.height, &fixed, &lights()).expect("chain runs");
        let fixed_clip = clipped_share(&fixed_out.ldr);
        assert!(
            fixed_clip > 0.2,
            "bright sweep must actually clip under fixed exposure, got {fixed_clip}"
        );
        let mut auto_state = AutoExposure::default();
        for _ in 0..3 {
            auto_state
                .tick(bright, scene.width, scene.height, 1.0)
                .expect("ticks");
        }
        assert!(
            auto_state.ev() < 0.0,
            "bright sweep adapts down, got {}",
            auto_state.ev()
        );
        let adapted = fixed.with_exposure_ev(auto_state.ev()).expect("adapts");
        let auto_out = run_chain(bright, scene.width, scene.height, &adapted, &lights())
            .expect("adapted chain runs");
        let auto_clip = clipped_share(&auto_out.ldr);
        let improvement = (fixed_clip - auto_clip) / fixed_clip;
        eprintln!(
            "day sweep: fixed clipped {fixed_clip:.3}, adapted {auto_clip:.3} at {:.2} stops, improvement {improvement:.3} (gate {})",
            auto_state.ev(),
            AUTO_CLIP_IMPROVE_MIN
        );
        assert!(
            improvement >= AUTO_CLIP_IMPROVE_MIN,
            "adapted improvement {improvement} misses {AUTO_CLIP_IMPROVE_MIN}"
        );
    }

    #[test]
    fn auto_exposure_ignores_torch_grid_and_limits_flicker() {
        let scene = ExposureScene::day_sweep_torch().expect("scene builds");
        // Converge on the dark field, then strike the torches: the trimmed
        // mean barely moves, so one 60 Hz step barely follows.
        let mut state = AutoExposure::default();
        for _ in 0..3 {
            state
                .tick(&scene.dark, scene.width, scene.height, 1.0)
                .expect("ticks");
        }
        let dark_ev = state.ev();
        let step = state
            .tick(&scene.torch, scene.width, scene.height, 1.0 / 60.0)
            .expect("ticks");
        assert!(
            (step - dark_ev).abs() <= AUTO_ADAPT_EV_PER_S / 60.0 + 1e-6,
            "torch strike steps at most one slew quantum, got {}",
            (step - dark_ev).abs()
        );
        // Settle fully on the torch grid: steady state stays in the gate.
        for _ in 0..600 {
            state
                .tick(&scene.torch, scene.width, scene.height, 1.0 / 60.0)
                .expect("ticks");
        }
        let torch_ev = state.ev();
        eprintln!(
            "torch grid: dark {dark_ev:.4} stops, torch {torch_ev:.4} stops (gate {AUTO_FLICKER_MAX_EV})"
        );
        assert!(
            (torch_ev - dark_ev).abs() < AUTO_FLICKER_MAX_EV,
            "torches must not pump exposure"
        );
        // Blink war: alternating frames never step past the gate.
        let mut blink = AutoExposure::default();
        let mut worst = 0.0f32;
        let mut prev = blink.ev();
        for k in 0..120 {
            let frame = if k % 2 == 0 {
                &scene.dark
            } else {
                &scene.torch
            };
            blink
                .tick(frame, scene.width, scene.height, 1.0 / 60.0)
                .expect("ticks");
            worst = worst.max((blink.ev() - prev).abs());
            prev = blink.ev();
        }
        eprintln!("torch blink: worst step {worst:.4} stops (gate {AUTO_FLICKER_MAX_EV})");
        assert!(
            worst <= AUTO_FLICKER_MAX_EV,
            "blink step {worst} exceeds the flicker gate"
        );
    }

    #[test]
    fn auto_exposure_costs_no_heap() {
        // The per-frame adapt path (histogram plus tick) never touches the
        // heap: stack bins plus scalar walks, counted by the test allocator.
        let scene = ExposureScene::day_sweep_torch().expect("scene builds");
        let (hist, hist_heap) = engine_core::alloc_probe::measure(|| {
            histogram_log_luma(&scene.sweep[2], scene.width, scene.height)
        });
        hist.expect("histograms");
        assert_eq!(hist_heap.calls, 0, "histogram touches no heap");
        let mut state = AutoExposure::default();
        let (_, tick_heap) = engine_core::alloc_probe::measure(|| {
            state.tick(&scene.sweep[2], scene.width, scene.height, 1.0 / 60.0)
        });
        assert_eq!(tick_heap.calls, 0, "tick touches no heap");
    }

    #[test]
    fn auto_frame_serves_match_clip() {
        // Match-loop integration: 12 frames cycling the sweep at 60 Hz
        // through the auto frame path. Stops rerun bit-identically, steps
        // respect the flicker gate, every receipt slot is filled, and the
        // frame path equals the chain path ticked from the same buffer.
        let scene = ExposureScene::day_sweep_torch().expect("scene builds");
        let motion = MotionField::zero(scene.width, scene.height).expect("zero builds");
        let taa = TaaParams::default();
        let chain = PostChain::default();
        let run_clip = || {
            let mut auto_state = AutoExposure::default();
            let mut evs = Vec::with_capacity(12);
            let mut ldrs = Vec::with_capacity(12);
            for k in 0..12 {
                let frame = &scene.sweep[k % scene.sweep.len()];
                let out = run_frame_auto(
                    frame,
                    frame,
                    &motion,
                    scene.width,
                    scene.height,
                    scene.width,
                    scene.height,
                    &taa,
                    &chain,
                    &mut auto_state,
                    1.0 / 60.0,
                    &lights(),
                )
                .expect("frame runs");
                evs.push(auto_state.ev());
                ldrs.push(out.ldr);
            }
            (evs, ldrs)
        };
        let (evs_a, ldrs_a) = run_clip();
        let (evs_b, ldrs_b) = run_clip();
        assert_eq!(evs_a, evs_b, "clip stops bit-identical twice");
        assert_eq!(ldrs_a, ldrs_b, "clip pixels bit-identical twice");
        let mut worst = 0.0f32;
        for w in evs_a.windows(2) {
            worst = worst.max((w[1] - w[0]).abs());
        }
        eprintln!("match clip: worst step {worst:.4} stops (gate {AUTO_FLICKER_MAX_EV})");
        assert!(
            worst <= AUTO_FLICKER_MAX_EV,
            "clip step {worst} exceeds the flicker gate"
        );
        assert!(
            evs_a.iter().all(|e| (-8.0..=8.0).contains(e)),
            "clip stops stay in range"
        );
        // Frame path equals the chain path ticked from the same buffer.
        let mut s1 = AutoExposure::default();
        let mut s2 = AutoExposure::default();
        let frame = &scene.sweep[4];
        let f = run_frame_auto(
            frame,
            frame,
            &motion,
            16,
            16,
            16,
            16,
            &taa,
            &chain,
            &mut s1,
            1.0 / 60.0,
            &lights(),
        )
        .expect("frame runs");
        let c = run_chain_auto(frame, 16, 16, &chain, &mut s2, 1.0 / 60.0, &lights())
            .expect("chain runs");
        assert_eq!(
            s1.ev(),
            s2.ev(),
            "frame and chain tick from the same buffer"
        );
        assert_eq!(f.ldr, c.ldr, "frame and chain agree pixel for pixel");
        let r = &f.receipt;
        assert_eq!(
            (r.taa.pixels, r.upsample.pixels, r.exposure.pixels),
            (256, 256, 256)
        );
        assert!((r.lights_ms() - 3.75).abs() < 1e-9);
        assert!(r.exposure.ms >= 0.0);
    }

    #[test]
    fn auto_exposure_fails_loudly() {
        let scene = ExposureScene::day_sweep_torch().expect("scene builds");
        let mut state = AutoExposure::default();
        // Bad frame data never adapts.
        let mut bad = scene.sweep[2].clone();
        bad[7] = [f32::NAN, 0.0, 0.0];
        assert!(matches!(
            state.tick(&bad, 16, 16, 1.0 / 60.0),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            state.tick(&scene.sweep[2][..255], 16, 16, 1.0 / 60.0),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            state.tick(&scene.sweep[2], 0, 16, 1.0 / 60.0),
            Err(PostError::Invalid(_))
        ));
        // Bad dts never advance the state.
        let held = state.ev();
        for dt in [f32::NAN, -1.0, f32::INFINITY] {
            assert!(matches!(
                state.tick(&scene.sweep[2], 16, 16, dt),
                Err(PostError::Invalid(_))
            ));
        }
        assert_eq!(state.ev(), held, "failed ticks hold the state");
        // Out-of-range stops never enter the chain.
        assert!(matches!(
            PostChain::default().with_exposure_ev(9.0),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            PostChain::default().with_exposure_ev(f32::NAN),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            run_chain_auto(
                &scene.sweep[2],
                16,
                16,
                &PostChain::default(),
                &mut state,
                -0.5,
                &lights()
            ),
            Err(PostError::Invalid(_))
        ));
        // Empty clips read zero, never NaN.
        assert_eq!(clipped_share(&[]), 0.0);
        // Empty histograms and empty trims fail, never divide by zero.
        assert!(matches!(
            trimmed_log_mean(
                &ExposureHistogram {
                    bins: [0.0f32; AUTO_HIST_BINS],
                    texels: 0.0
                },
                0.1,
                0.95
            ),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            target_ev_for_mean(f32::NAN, 0.18),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            target_ev_for_mean(-2.0, 0.0),
            Err(PostError::Invalid(_))
        ));
        assert!(matches!(
            AutoExposure::new(AutoExposureConfig {
                trim_lo: 0.9,
                trim_hi: 0.1,
                ..AutoExposureConfig::default()
            }),
            Err(PostError::Invalid(_))
        ));
    }

    #[test]
    #[ignore = "timing: auto-exposure overhead needs release --perf on boosted clocks (fails under parallel load)"]
    fn auto_exposure_wall_overhead() {
        // Kill-gate evidence: histogram plus adapt must cost under 5% of
        // the fixed chain on a 64x64 ramp (medians of 21 reps per leg).
        let side = 64usize;
        let total = side * side;
        let hdr: Vec<[f32; 3]> = (0..total)
            .map(|i| {
                let luma = 0.05 + i as f32 / (total - 1) as f32 * 3.0;
                [luma, luma, luma]
            })
            .collect();
        let chain = PostChain::default();
        let mut fixed_ms = Vec::with_capacity(21);
        for _ in 0..21 {
            let started = Instant::now();
            std::hint::black_box(run_chain(&hdr, 64, 64, &chain, &lights()).expect("chain runs"));
            fixed_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        let mut state = AutoExposure::default();
        let mut auto_ms = Vec::with_capacity(21);
        for _ in 0..21 {
            let started = Instant::now();
            std::hint::black_box(
                run_chain_auto(&hdr, 64, 64, &chain, &mut state, 1.0 / 60.0, &lights())
                    .expect("auto chain runs"),
            );
            auto_ms.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        fixed_ms.sort_by(|a, b| a.total_cmp(b));
        auto_ms.sort_by(|a, b| a.total_cmp(b));
        let (fixed_med, auto_med) = (fixed_ms[10], auto_ms[10]);
        let ratio = auto_med / fixed_med.max(1e-9);
        eprintln!(
            "auto-exposure overhead 64x64: fixed {fixed_med:.3}ms auto {auto_med:.3}ms ratio {ratio:.3} (kill above 1.05)"
        );
        assert!(
            ratio <= 1.05,
            "auto chain {auto_med:.3}ms costs over 5% above fixed {fixed_med:.3}ms"
        );
    }

    #[cfg(feature = "window")]
    #[test]
    fn gpu_auto_chain_matches_cpu_chain() {
        // Adapted exposure rides the existing exposure uniform: converge
        // the CPU state on the bright sweep frame, run the adapted chain
        // on the device and against the CPU oracle, parity within the
        // posted tolerance.
        let (device, queue) = match test_device() {
            Some(d) => d,
            None => return,
        };
        let sub = gpu::Rp1Submitter::new(&device);
        let scene = ExposureScene::day_sweep_torch().expect("scene builds");
        let mut auto_state = AutoExposure::default();
        for _ in 0..3 {
            auto_state
                .tick(&scene.sweep[4], scene.width, scene.height, 1.0)
                .expect("ticks");
        }
        let chain = PostChain::default()
            .with_exposure_ev(auto_state.ev())
            .expect("adapts");
        let motion = MotionField::zero(scene.width, scene.height).expect("zero builds");
        let taa = TaaParams::default();
        let g = sub
            .run_frame(
                &device,
                &queue,
                &scene.sweep[4],
                &scene.sweep[3],
                &motion,
                scene.width,
                scene.height,
                32,
                32,
                &taa,
                &chain,
                &lights(),
                [0.0, 0.0],
            )
            .expect("gpu frame runs");
        let cpu = run_frame(
            &scene.sweep[4],
            &scene.sweep[3],
            &motion,
            scene.width,
            scene.height,
            32,
            32,
            &taa,
            &chain,
            &lights(),
        )
        .expect("cpu frame runs");
        eprintln!(
            "auto gpu frame: parity {:.6} (tol {}), adapted stops {:.3}",
            g.parity_max_diff,
            gpu::GPU_PARITY_TOL_POST,
            auto_state.ev()
        );
        assert_eq!(g.validation_errors, 0, "gpu frame is validation clean");
        assert!(
            g.parity_max_diff <= gpu::GPU_PARITY_TOL_POST,
            "gpu/cpu frame parity {} exceeds {}",
            g.parity_max_diff,
            gpu::GPU_PARITY_TOL_POST
        );
        assert_eq!(gpu::max_abs_diff(&cpu.ldr, &g.ldr), g.parity_max_diff);
        assert!(
            auto_state.ev() < 0.0,
            "bright sweep adapts down, got {}",
            auto_state.ev()
        );
    }
}
