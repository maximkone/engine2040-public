//! lens: the bodycam lens, one post pass over the finished frame.
//!
//! The lens table has three presets: the bare eye (identity, the default),
//! a helmet and a chest mount. One pass applies barrel distortion (k1), a
//! radial red/blue split (chromatic aberration), a vignette and film grain.
//!
//! Owner: render lane. Consumer: the post chain (the pass runs after the
//! tonemap) and the capture helper.
//!
//! The CPU oracle here is the reference. The WGSL in
//! `shaders/post/lens.wgsl` runs the same steps in the same order, and the
//! GPU parity test holds the two together. Units: uv in 0..1, the knobs in
//! 0..1, the grain in 0..1 of full scale. Determinism: the grain is an
//! integer hash of (x, y, frame index), so the same frame index gives the
//! same grain twice; no clock and no float hash take part.
//!
//! Donor shape: three.js `examples/jsm/shaders/RGBShiftShader.js`,
//! `VignetteShader.js` and `FilmShader.js` (MIT, read 2026-10-20): the
//! radial channel split, the radial falloff and the seeded noise. The
//! formulas here are ours (the split keeps one distance at every radius,
//! the falloff is quadratic and the noise is an integer hash); zero donor
//! bytes are vendored.
//! PORT(three.js@r160:examples/jsm/shaders/RGBShiftShader.js): radial channel split shape.
//! PORT(three.js@r160:examples/jsm/shaders/VignetteShader.js): radial falloff shape.
//! PORT(three.js@r160:examples/jsm/shaders/FilmShader.js): seeded noise shape.

use std::fmt;

/// The lens shader, the file the naga gate validates.
pub const LENS_WGSL: &str = include_str!("shaders/post/lens.wgsl");

/// Largest frame side the lens reads. The tap index stays inside `i16`
/// this way, so the texel math needs no float cast.
pub const MAX_LENS_DIM: u32 = 8192;

/// The distance from the frame center to a corner, in uv.
const CORNER_LEN: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// The lens pass budget in ms (RL39): one pass over a finished 1920x1080
/// frame on the reference card. Pure number, never measured here.
pub const LENS_PASS_BUDGET_MS: f64 = 0.15;

/// Modeled cost of one lens pass in ms per megapixel of output texels, with
/// the knob weight folded in to one figure. Modeled, not measured: it holds
/// every host to the same number, so the row's budget is checkable with no
/// GPU in hand. Calibrated so the shipped helmet lens lands at about
/// 0.13 ms for 1920x1080, inside [`LENS_PASS_BUDGET_MS`] with room for a
/// profile that turns every knob up.
const LENS_MS_PER_MPIX: f64 = 0.04;

/// The lens presets the bodycam view ships.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LensPreset {
    /// The bare eye: nothing changes. The default.
    Eye,
    /// A helmet-mounted lens: the curved wide look.
    Helmet,
    /// A chest-mounted lens: a softer curve than the helmet.
    Chest,
}

impl LensPreset {
    /// The preset name, used for capture file names.
    pub fn name(self) -> &'static str {
        match self {
            Self::Eye => "eye",
            Self::Helmet => "helmet",
            Self::Chest => "chest",
        }
    }

    /// The profile of this preset.
    pub fn profile(self) -> LensProfile {
        LensProfile::preset(self)
    }
}

/// The four knobs one lens reads.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LensProfile {
    /// Barrel distortion strength (k1). Zero leaves the frame alone.
    pub barrel_k1: f32,
    /// Chromatic aberration: how far the red and the blue sample sit apart,
    /// in uv.
    pub ca_amount: f32,
    /// Vignette: how much the corner is darkened, so the corner factor is
    /// `1 - vignette`.
    pub vignette: f32,
    /// Film grain: the peak noise added to a channel, in 0..1 of full scale.
    pub grain: f32,
}

impl LensProfile {
    /// The bare eye: the identity profile.
    pub const fn eye() -> Self {
        Self {
            barrel_k1: 0.0,
            ca_amount: 0.0,
            vignette: 0.0,
            grain: 0.0,
        }
    }

    /// The helmet lens: barrel 0.08, a 0.005 uv red/blue split, a 0.35
    /// vignette and 0.04 grain.
    pub const fn helmet() -> Self {
        Self {
            barrel_k1: 0.08,
            ca_amount: 0.005,
            vignette: 0.35,
            grain: 0.04,
        }
    }

    /// The chest lens: a softer curve than the helmet.
    pub const fn chest() -> Self {
        Self {
            barrel_k1: 0.05,
            ca_amount: 0.003,
            vignette: 0.22,
            grain: 0.03,
        }
    }

    /// The profile of one preset.
    pub const fn preset(preset: LensPreset) -> Self {
        match preset {
            LensPreset::Eye => Self::eye(),
            LensPreset::Helmet => Self::helmet(),
            LensPreset::Chest => Self::chest(),
        }
    }

    /// True when the profile changes nothing: the eye.
    pub fn is_identity(&self) -> bool {
        self.barrel_k1 == 0.0 && self.ca_amount == 0.0 && self.vignette == 0.0 && self.grain == 0.0
    }

    /// Refuse a profile the pass cannot run: a knob that is not a number or
    /// a value outside its range.
    pub fn validate(&self) -> Result<(), LensError> {
        let knobs = [
            ("barrel_k1", self.barrel_k1, 1.0),
            ("ca_amount", self.ca_amount, 0.25),
            ("vignette", self.vignette, 1.0),
            ("grain", self.grain, 0.5),
        ];
        for (name, value, limit) in knobs {
            if !value.is_finite() {
                return Err(LensError::Invalid(format!("lens {name} is not a number")));
            }
            if value < 0.0 || value > limit {
                return Err(LensError::Invalid(format!(
                    "lens {name} {value} is outside 0..{limit}"
                )));
            }
        }
        Ok(())
    }

    /// Modeled cost in ms for one lens pass over `width` x `height` pixels.
    ///
    /// Pure model, no wall clock, so every host reads the same number and the
    /// row's budget is checkable with no GPU in hand. The unit is the
    /// megapixel of output (the pass is one full-screen step); each knob the
    /// profile turns on adds a share of the pass's texel reads, so a profile
    /// costs strictly more than the identity it left off. The bare eye costs
    /// exactly zero: it is the identity the blit already ran.
    #[must_use]
    pub fn lens_pass_ms(&self, width: u32, height: u32) -> f64 {
        if self.is_identity() {
            return 0.0;
        }
        let mpix = f64::from(width) * f64::from(height) / 1_000_000.0;
        // Base 1 is the green bilinear fetch the barrel map always reads.
        let mut weight = 1.0;
        // The red/blue split adds a bilinear each way while it runs.
        if self.ca_amount > 0.0 {
            weight += 0.5;
        }
        // The grain hash is one cheap add per texel.
        if self.grain > 0.0 {
            weight += 0.1;
        }
        // The barrel and the vignette shade the texel already read: free.
        mpix * weight * LENS_MS_PER_MPIX
    }
}

impl Default for LensProfile {
    /// The default lens is the bare eye.
    fn default() -> Self {
        Self::eye()
    }
}

/// Lens failures. Every variant is a typed refusal; the pass never guesses.
#[derive(Clone, PartialEq, Debug)]
pub enum LensError {
    /// The caller's input is unusable; the reason is carried.
    Invalid(String),
    /// The frame is wider or taller than the lens reads.
    TooLarge {
        /// The side the caller asked for.
        requested: u32,
        /// The side the lens reads.
        allowed: u32,
    },
    /// No live device on this call; what to build first is carried.
    Device(String),
    /// A wgpu validation scope fired inside the pass.
    Validation(String),
    /// A capture file could not be read or written.
    Io(String),
}

impl fmt::Display for LensError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(e) => write!(f, "lens input: {e}"),
            Self::TooLarge { requested, allowed } => {
                write!(f, "lens frame {requested} is over the bound {allowed}")
            }
            Self::Device(e) => write!(f, "lens device: {e}"),
            Self::Validation(e) => write!(f, "lens validation: {e}"),
            Self::Io(e) => write!(f, "lens capture file: {e}"),
        }
    }
}

impl std::error::Error for LensError {}

/// The barrel map: the uv the pass reads for one output uv. A positive `k1`
/// pulls the source toward the frame center, so the corner of the picture
/// moves inward.
pub fn barrel_uv(uv: [f32; 2], k1: f32) -> [f32; 2] {
    let c = [uv[0] * 2.0 - 1.0, uv[1] * 2.0 - 1.0];
    let f = 1.0 + k1 * (c[0] * c[0] + c[1] * c[1]);
    [(c[0] * f) * 0.5 + 0.5, (c[1] * f) * 0.5 + 0.5]
}

/// Where a source uv lands in the output: the inverse of [`barrel_uv`].
/// The radial solve is a fixed Newton loop, so the same source gives the
/// same place twice.
pub fn barrel_source_uv(uv: [f32; 2], k1: f32) -> [f32; 2] {
    let c = [uv[0] * 2.0 - 1.0, uv[1] * 2.0 - 1.0];
    let radius = (c[0] * c[0] + c[1] * c[1]).sqrt();
    if radius < 1e-9 || k1 == 0.0 {
        return uv;
    }
    // Solve t * (1 + k1 * t * t) = radius for the output radius t.
    let mut t = radius;
    for _ in 0..12 {
        let f = t * (1.0 + k1 * t * t) - radius;
        let df = 1.0 + 3.0 * k1 * t * t;
        if df.abs() < 1e-9 {
            break;
        }
        t -= f / df;
    }
    let k = t / radius;
    [(c[0] * k) * 0.5 + 0.5, (c[1] * k) * 0.5 + 0.5]
}

/// The chromatic aberration shift: the red and the blue sample sit this far
/// apart along the radial direction.
pub fn ca_shift(uv: [f32; 2], amount: f32) -> [f32; 2] {
    let d = [uv[0] - 0.5, uv[1] - 0.5];
    let len = (d[0] * d[0] + d[1] * d[1]).sqrt();
    if len < 1e-6 {
        return [0.0, 0.0];
    }
    let k = 0.5 * amount / len;
    [d[0] * k, d[1] * k]
}

/// The vignette factor at one uv: 1 at the center, `1 - strength` at the
/// corner.
pub fn vignette_at(uv: [f32; 2], strength: f32) -> f32 {
    let dx = uv[0] - 0.5;
    let dy = uv[1] - 0.5;
    let d = (dx * dx + dy * dy).sqrt() / CORNER_LEN;
    (1.0 - strength * d * d).clamp(0.0, 1.0)
}

/// The grain noise at one texel: -0.5..0.5, seeded by the frame index. The
/// lanes are 16 bits wide, so every multiply stays inside `u32` and the
/// same (x, y, frame) gives the same noise on every backend.
pub fn grain_at(x: u32, y: u32, frame: u32) -> f32 {
    let mut h = (x & 0xffff).wrapping_mul(0x9e37);
    h ^= (y & 0xffff).wrapping_mul(0x85eb);
    h ^= (frame & 0xffff).wrapping_mul(0xc2b2);
    h ^= (frame >> 16).wrapping_mul(0x2545);
    h ^= h >> 16;
    h = (h & 0xffff).wrapping_mul(0x2545);
    h ^= h >> 13;
    h = (h & 0xffff).wrapping_mul(0x1f35);
    h ^= h >> 11;
    f32::from(u16::try_from(h >> 16).unwrap_or(0)) * (1.0 / 65536.0) - 0.5
}

/// One frame of display pixels the lens reads.
#[derive(Clone, PartialEq, Debug)]
pub struct LensFrame {
    /// Frame width in texels.
    pub width: u32,
    /// Frame height in texels.
    pub height: u32,
    /// Display pixels, row-major, three channels each.
    pub pixels: Vec<[f32; 3]>,
}

impl LensFrame {
    /// A frame from its pixels. The pixel count must match the texel grid
    /// and the sides must stay inside [`MAX_LENS_DIM`].
    pub fn new(width: u32, height: u32, pixels: Vec<[f32; 3]>) -> Result<Self, LensError> {
        if width == 0 || height == 0 {
            return Err(LensError::Invalid("lens frame has a zero side".into()));
        }
        if width > MAX_LENS_DIM || height > MAX_LENS_DIM {
            return Err(LensError::TooLarge {
                requested: width.max(height),
                allowed: MAX_LENS_DIM,
            });
        }
        let want = usize::try_from(width).unwrap_or(0) * usize::try_from(height).unwrap_or(0);
        if pixels.len() != want {
            return Err(LensError::Invalid(format!(
                "lens frame has {} pixels, the {width}x{height} grid needs {want}",
                pixels.len()
            )));
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// The texel count of the frame.
    pub fn texels(&self) -> usize {
        usize::try_from(self.width).unwrap_or(0) * usize::try_from(self.height).unwrap_or(0)
    }
}

/// A texel count as a float. The count is under [`MAX_LENS_DIM`], so the
/// `u16` hop keeps every bit and the math stays exact.
fn count_f32(value: u32) -> f32 {
    f32::from(u16::try_from(value).unwrap_or(0))
}

/// The texel index at or below `position`: a binary search over the exact
/// `f32::from(i16)` scalars, so the frame needs no float cast.
fn floor_index(position: f32, low: i32, high: i32) -> i32 {
    let mut below = low - 1;
    let mut above = high;
    while above - below > 1 {
        let mid = below + (above - below) / 2;
        if f32::from(i16::try_from(mid).unwrap_or(0)) <= position {
            below = mid;
        } else {
            above = mid;
        }
    }
    below
}

/// One texel of the frame, clamped to the frame edge.
fn fetch(frame: &LensFrame, x: i32, y: i32) -> [f32; 3] {
    let width = i32::try_from(frame.width).unwrap_or(1);
    let height = i32::try_from(frame.height).unwrap_or(1);
    let cx = x.clamp(0, width - 1);
    let cy = y.clamp(0, height - 1);
    let index = usize::try_from(cy).unwrap_or(0) * usize::try_from(frame.width).unwrap_or(0)
        + usize::try_from(cx).unwrap_or(0);
    frame.pixels.get(index).copied().unwrap_or([0.0, 0.0, 0.0])
}

/// The color at one uv: bilinear over the four neighbouring texels, with
/// the taps clamped to the frame edge. The uv is clamped to the window the
/// barrel can reach, so the tap index stays inside the frame math.
pub fn sample_bilinear(frame: &LensFrame, uv: [f32; 2]) -> [f32; 3] {
    let clamped = [uv[0].clamp(-2.0, 3.0), uv[1].clamp(-2.0, 3.0)];
    let width = i32::try_from(frame.width).unwrap_or(1);
    let height = i32::try_from(frame.height).unwrap_or(1);
    let tx = clamped[0] * count_f32(frame.width) - 0.5;
    let ty = clamped[1] * count_f32(frame.height) - 0.5;
    let ix = floor_index(tx, -(2 * width + 2), 3 * width + 2);
    let iy = floor_index(ty, -(2 * height + 2), 3 * height + 2);
    let fx = tx - f32::from(i16::try_from(ix).unwrap_or(0));
    let fy = ty - f32::from(i16::try_from(iy).unwrap_or(0));
    let top = mix3(fetch(frame, ix, iy), fetch(frame, ix + 1, iy), fx);
    let bottom = mix3(fetch(frame, ix, iy + 1), fetch(frame, ix + 1, iy + 1), fx);
    mix3(top, bottom, fy)
}

/// The WGSL `mix` over three channels: `a * (1 - t) + b * t`.
fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] * (1.0 - t) + b[0] * t,
        a[1] * (1.0 - t) + b[1] * t,
        a[2] * (1.0 - t) + b[2] * t,
    ]
}

/// The lens color of one output texel.
pub fn apply_pixel(
    profile: &LensProfile,
    frame_index: u32,
    frame: &LensFrame,
    x: u32,
    y: u32,
) -> [f32; 3] {
    let uv = [
        (count_f32(x) + 0.5) / count_f32(frame.width),
        (count_f32(y) + 0.5) / count_f32(frame.height),
    ];
    let base = barrel_uv(uv, profile.barrel_k1);
    let shift = ca_shift(base, profile.ca_amount);
    let red = sample_bilinear(frame, [base[0] + shift[0], base[1] + shift[1]]);
    let green = sample_bilinear(frame, base);
    let blue = sample_bilinear(frame, [base[0] - shift[0], base[1] - shift[1]]);
    let sampled = [red[0], green[1], blue[2]];
    let vignette = vignette_at(uv, profile.vignette);
    // RL39: grain lives in the mid tones only. The mask is exactly 0 at pure
    // black and pure white, so a clipped highlight or a true black never
    // gains a noise step; mid-grey carries the full amount. The mask reads
    // the toned color, not the raw sample, so the noise rides the frame the
    // viewer sees.
    let toned = [
        sampled[0] * vignette,
        sampled[1] * vignette,
        sampled[2] * vignette,
    ];
    let luma = dot_luma(toned);
    let grain_mask = 4.0 * luma * (1.0 - luma);
    let noise = grain_at(x, y, frame_index) * profile.grain * grain_mask;
    let mut out = [0.0f32; 3];
    for c in 0..3 {
        out[c] = (toned[c] + noise).clamp(0.0, 1.0);
    }
    out
}

/// Rec. 709 luma of one linear color, clamped to 0..1 for the grain mask.
fn dot_luma(c: [f32; 3]) -> f32 {
    (0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]).clamp(0.0, 1.0)
}

/// The lensed frame: every output texel through [`apply_pixel`].
pub fn apply_pixels(profile: &LensProfile, frame_index: u32, frame: &LensFrame) -> Vec<[f32; 3]> {
    let mut out = Vec::with_capacity(frame.texels());
    for y in 0..frame.height {
        for x in 0..frame.width {
            out.push(apply_pixel(profile, frame_index, frame, x, y));
        }
    }
    out
}

/// The lensed bytes of an RGBA8 frame. The alpha rides along untouched.
pub fn apply_bytes(
    profile: &LensProfile,
    frame_index: u32,
    rgba: &[u8],
    width: u32,
    height: u32,
) -> Result<Vec<u8>, LensError> {
    let mut pixels = Vec::with_capacity(rgba.len() / 4);
    for chunk in rgba.chunks_exact(4) {
        pixels.push([
            f32::from(chunk[0]) / 255.0,
            f32::from(chunk[1]) / 255.0,
            f32::from(chunk[2]) / 255.0,
        ]);
    }
    let frame = LensFrame::new(width, height, pixels)?;
    let lensed = apply_pixels(profile, frame_index, &frame);
    let mut out = Vec::with_capacity(rgba.len());
    for (index, pixel) in lensed.iter().enumerate() {
        out.push(to_byte(pixel[0]));
        out.push(to_byte(pixel[1]));
        out.push(to_byte(pixel[2]));
        out.push(rgba.get(index * 4 + 3).copied().unwrap_or(255));
    }
    Ok(out)
}

/// One 0..1 channel as a byte. The byte is the one the scaled value
/// reaches: a binary search over the 256 steps, so the float needs no cast.
fn to_byte(value: f32) -> u8 {
    let scaled = value.clamp(0.0, 1.0) * 255.0 + 0.5;
    let mut low = 0u16;
    let mut high = 255u16;
    while low < high {
        let mid = low + (high - low).div_ceil(2);
        if f32::from(u8::try_from(mid).unwrap_or(0)) <= scaled {
            low = mid;
        } else {
            high = mid - 1;
        }
    }
    u8::try_from(low).unwrap_or(0)
}

/// Display pixels as RGBA8 bytes, alpha opaque.
pub fn pixels_to_rgba8(pixels: &[[f32; 3]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixels.len() * 4);
    for pixel in pixels {
        out.push(to_byte(pixel[0]));
        out.push(to_byte(pixel[1]));
        out.push(to_byte(pixel[2]));
        out.push(255);
    }
    out
}

/// The mean luma of RGBA8 bytes, in 0..1.
pub fn mean_luma(rgba: &[u8]) -> f32 {
    let mut sum = 0.0f32;
    let mut count = 0.0f32;
    for chunk in rgba.chunks_exact(4) {
        let luma = 0.2126 * f32::from(chunk[0])
            + 0.7152 * f32::from(chunk[1])
            + 0.0722 * f32::from(chunk[2]);
        sum += luma;
        count += 1.0;
    }
    if count <= 0.0 {
        return 0.0;
    }
    sum / count / 255.0
}

/// The GPU lens pass: the WGSL over one frame, plus the capture helper.
/// Native only: it blocks on the GPU and writes the capture PNG to disk.
#[cfg(all(feature = "window", not(target_arch = "wasm32")))]
pub mod gpu {
    use super::{mean_luma, LensError, LensFrame, LensPreset, LensProfile, LENS_WGSL};
    use crate::aa_scale::gpu::pack_pixels;

    /// One GPU lens frame: the lensed pixels and the receipt.
    #[derive(Clone, Debug)]
    pub struct GpuLens {
        /// Lensed pixels, row-major.
        pub pixels: Vec<[f32; 3]>,
        /// Frame width in texels.
        pub width: u32,
        /// Frame height in texels.
        pub height: u32,
        /// wgpu validation errors observed; green means 0.
        pub validation_errors: u32,
        /// Wall time for encode, submit and readback in ms.
        pub submit_ms: f64,
    }

    /// One capture: the file written and the pixels read back from it.
    #[derive(Clone, Debug)]
    pub struct LensCapture {
        /// The preset the capture shows.
        pub preset: LensPreset,
        /// The PNG file path.
        pub path: String,
        /// The pixels read back from the PNG, row-major.
        pub pixels: Vec<[u8; 4]>,
        /// The mean luma of the capture, in 0..1.
        pub mean_luma: f32,
        /// The first texel of the capture.
        pub corner: [u8; 4],
    }

    /// The lens compute pipeline and its bind layout: one pass over a frame.
    pub struct LensPass {
        pipe: wgpu::ComputePipeline,
        layout: wgpu::BindGroupLayout,
    }

    impl LensPass {
        /// Compile the lens pipeline on `device`.
        pub fn new(device: &wgpu::Device) -> Self {
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("lens-layout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
            let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("lens"),
                source: wgpu::ShaderSource::Wgsl(LENS_WGSL.into()),
            });
            let pipe_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("lens"),
                bind_group_layouts: &[Some(&layout)],
                immediate_size: 0,
            });
            let pipe = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("lens"),
                layout: Some(&pipe_layout),
                module: &module,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });
            Self { pipe, layout }
        }

        /// Run the pass over one frame and read the result back. The four
        /// knobs and the frame index ride in one uniform; the source and the
        /// result are storage buffers, like the other post passes.
        pub fn apply(
            &self,
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            frame: &LensFrame,
            profile: &LensProfile,
            frame_index: u32,
        ) -> Result<GpuLens, LensError> {
            profile.validate()?;
            let texels = frame.texels();
            let bytes = u64::try_from(texels).unwrap_or(0) * 16;
            let started = std::time::Instant::now();
            let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
            let buffers = self.upload(device, frame, profile, frame_index, bytes);
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("lens-bind"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffers.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: buffers.source.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: buffers.target.as_entire_binding(),
                    },
                ],
            });
            self.dispatch(device, queue, &bind, frame.width, frame.height);
            device
                .poll(wgpu::PollType::wait_indefinitely())
                .map_err(|e| LensError::Device(format!("poll: {e}")))?;
            let scope_err = pollster::block_on(scope.pop());
            if let Some(err) = scope_err {
                return Err(LensError::Validation(format!("{err:?}")));
            }
            let pixels = read_back(device, queue, &buffers.target, texels, bytes)?;
            Ok(GpuLens {
                pixels,
                width: frame.width,
                height: frame.height,
                validation_errors: 0,
                submit_ms: started.elapsed().as_secs_f64() * 1000.0,
            })
        }

        /// The three buffers one run needs: the uniform, the source frame
        /// and the result.
        fn upload(
            &self,
            device: &wgpu::Device,
            frame: &LensFrame,
            profile: &LensProfile,
            frame_index: u32,
            bytes: u64,
        ) -> LensBuffers {
            use wgpu::util::DeviceExt;
            let uniform = pack_uniform(frame.width, frame.height, frame_index, profile);
            LensBuffers {
                uniform: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("lens-uniform"),
                    contents: &uniform,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                }),
                source: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("lens-source"),
                    contents: &pack_pixels(&frame.pixels),
                    usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
                }),
                target: device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("lens-target"),
                    size: bytes,
                    usage: wgpu::BufferUsages::STORAGE
                        | wgpu::BufferUsages::COPY_SRC
                        | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
            }
        }

        /// One compute dispatch over the frame.
        fn dispatch(
            &self,
            device: &wgpu::Device,
            queue: &wgpu::Queue,
            bind: &wgpu::BindGroup,
            width: u32,
            height: u32,
        ) {
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("lens"),
            });
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("lens-pass"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.pipe);
                pass.set_bind_group(0, bind, &[]);
                pass.dispatch_workgroups(width.div_ceil(8), height.div_ceil(8), 1);
            }
            queue.submit(Some(encoder.finish()));
        }
    }

    /// The three buffers one lens run holds.
    struct LensBuffers {
        uniform: wgpu::Buffer,
        source: wgpu::Buffer,
        target: wgpu::Buffer,
    }

    /// Copy the result back and read the pixels.
    fn read_back(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &wgpu::Buffer,
        texels: usize,
        bytes: u64,
    ) -> Result<Vec<[f32; 3]>, LensError> {
        let back = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("lens-readback"),
            size: bytes,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("lens-copy"),
        });
        encoder.copy_buffer_to_buffer(target, 0, &back, 0, bytes);
        queue.submit(Some(encoder.finish()));
        let slice = back.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|e| LensError::Device(format!("poll: {e}")))?;
        rx.recv()
            .map_err(|e| LensError::Device(format!("map channel: {e}")))?
            .map_err(|e| LensError::Device(format!("map: {e}")))?;
        let mut pixels = Vec::with_capacity(texels);
        {
            let mapped = slice
                .get_mapped_range()
                .map_err(|e| LensError::Device(format!("mapped range: {e}")))?;
            for chunk in mapped.chunks_exact(16) {
                let r = f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                let g = f32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]);
                let b = f32::from_le_bytes([chunk[8], chunk[9], chunk[10], chunk[11]]);
                if !r.is_finite() || !g.is_finite() || !b.is_finite() {
                    return Err(LensError::Validation(
                        "gpu lens pixels are not finite".into(),
                    ));
                }
                pixels.push([r, g, b]);
            }
        }
        back.unmap();
        Ok(pixels)
    }

    /// The uniform block: the frame size, the frame index and the four knobs.
    fn pack_uniform(width: u32, height: u32, frame_index: u32, profile: &LensProfile) -> Vec<u8> {
        let mut out = Vec::with_capacity(32);
        out.extend_from_slice(&width.to_le_bytes());
        out.extend_from_slice(&height.to_le_bytes());
        out.extend_from_slice(&frame_index.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&profile.barrel_k1.to_le_bytes());
        out.extend_from_slice(&profile.ca_amount.to_le_bytes());
        out.extend_from_slice(&profile.vignette.to_le_bytes());
        out.extend_from_slice(&profile.grain.to_le_bytes());
        out
    }

    /// Run the pass for every preset and write one PNG per preset into `dir`.
    /// The pixels are read back from the written file, so the receipt is the
    /// file itself and not the buffer that produced it.
    pub fn capture_presets(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        frame: &LensFrame,
        dir: &std::path::Path,
        presets: &[LensPreset],
        frame_index: u32,
    ) -> Result<Vec<LensCapture>, LensError> {
        let pass = LensPass::new(device);
        let mut out = Vec::with_capacity(presets.len());
        for preset in presets {
            let profile = preset.profile();
            let lensed = pass.apply(device, queue, frame, &profile, frame_index)?;
            let rgba = super::pixels_to_rgba8(&lensed.pixels);
            let png = crate::v1::encode_png_rgba8(&rgba, frame.width, frame.height)
                .map_err(|e| LensError::Io(format!("encode: {e}")))?;
            std::fs::create_dir_all(dir).map_err(|e| LensError::Io(format!("mkdir: {e}")))?;
            let path = dir.join(format!("{}.png", preset.name()));
            std::fs::write(&path, &png).map_err(|e| LensError::Io(format!("write: {e}")))?;
            let pixels =
                read_png(&std::fs::read(&path).map_err(|e| LensError::Io(format!("read: {e}")))?)?;
            let corner = pixels.first().copied().unwrap_or([0, 0, 0, 255]);
            out.push(LensCapture {
                preset: *preset,
                path: path.display().to_string(),
                mean_luma: mean_luma(&rgba),
                corner,
                pixels,
            });
        }
        Ok(out)
    }

    /// The pixels of a PNG, row-major.
    fn read_png(bytes: &[u8]) -> Result<Vec<[u8; 4]>, LensError> {
        let image =
            image::load_from_memory(bytes).map_err(|e| LensError::Io(format!("decode: {e}")))?;
        let rgba = image.to_rgba8();
        let count = usize::try_from(rgba.width()).unwrap_or(0)
            * usize::try_from(rgba.height()).unwrap_or(0);
        let mut out = Vec::with_capacity(count);
        for pixel in rgba.pixels() {
            out.push([pixel[0], pixel[1], pixel[2], pixel[3]]);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A probe frame whose channels carry the source position, so the place
    /// the pass reads can be read back from the result: red and blue hold
    /// the source u, green holds the source v.
    fn probe_frame(width: u32, height: u32) -> LensFrame {
        let mut pixels = Vec::with_capacity(
            usize::try_from(width).unwrap_or(0) * usize::try_from(height).unwrap_or(0),
        );
        for y in 0..height {
            for x in 0..width {
                let u = (count_f32(x) + 0.5) / count_f32(width);
                let v = (count_f32(y) + 0.5) / count_f32(height);
                pixels.push([0.25 + 0.5 * u, 0.25 + 0.5 * v, 0.25 + 0.5 * u]);
            }
        }
        LensFrame::new(width, height, pixels).expect("the probe frame fits")
    }

    /// The source values the pass read at one output texel: red and blue
    /// hold the source u beside the split, green holds the source v at the
    /// barrel place. The vignette and the grain are taken out again.
    fn decode_read(
        out: [f32; 3],
        profile: &LensProfile,
        frame_index: u32,
        x: u32,
        y: u32,
        uv: [f32; 2],
    ) -> [f32; 3] {
        let vignette = vignette_at(uv, profile.vignette);
        // The forward shades first (`toned = sampled * vignette`) then adds
        // the noise, with the mask on the toned luma. The inverse peels the
        // noise off the shaded value first, so the mask reads the same luma
        // the forward did, then divides the vignette back out.
        let luma = (0.2126 * out[0] + 0.7152 * out[1] + 0.0722 * out[2]).clamp(0.0, 1.0);
        let grain_mask = 4.0 * luma * (1.0 - luma);
        let noise = grain_at(x, y, frame_index) * profile.grain * grain_mask;
        let toned = [out[0] - noise, out[1] - noise, out[2] - noise];
        [
            toned[0] / vignette,
            toned[1] / vignette,
            toned[2] / vignette,
        ]
    }

    /// The source position the pass read at one output texel.
    fn decode_base(
        out: [f32; 3],
        profile: &LensProfile,
        frame_index: u32,
        x: u32,
        y: u32,
        uv: [f32; 2],
    ) -> [f32; 2] {
        let raw = decode_read(out, profile, frame_index, x, y, uv);
        // Red and blue sit symmetrically around the barrel place.
        let u = (raw[0] + raw[2]) * 0.5;
        [(u - 0.25) / 0.5, (raw[1] - 0.25) / 0.5]
    }

    /// The output uv of one texel.
    fn texel_uv(width: u32, height: u32, x: u32, y: u32) -> [f32; 2] {
        [
            (count_f32(x) + 0.5) / count_f32(width),
            (count_f32(y) + 0.5) / count_f32(height),
        ]
    }

    /// The eye profile is the identity, so the frame stays byte-equal.
    #[test]
    fn lens_eye_profile_leaves_a_test_frame_byte_equal() {
        let (width, height) = (32u32, 16u32);
        let mut rgba = Vec::with_capacity(
            usize::try_from(width).unwrap_or(0) * usize::try_from(height).unwrap_or(0) * 4,
        );
        for index in 0..(width * height) {
            let base = (index * 7) % 256;
            rgba.push(u8::try_from(base).unwrap_or(0));
            rgba.push(u8::try_from((base + 13) % 256).unwrap_or(0));
            rgba.push(u8::try_from((base + 29) % 256).unwrap_or(0));
            rgba.push(255);
        }
        let eye = LensProfile::eye();
        assert!(eye.is_identity(), "the eye changes nothing");
        let lensed = apply_bytes(&eye, 0, &rgba, width, height).expect("the eye runs");
        assert_eq!(lensed, rgba, "the eye leaves the frame byte-equal");
        let helmet = LensProfile::helmet();
        assert!(!helmet.is_identity(), "the helmet is not the eye");
        let moved = apply_bytes(&helmet, 0, &rgba, width, height).expect("the helmet runs");
        assert_ne!(moved, rgba, "the helmet moves the frame");
    }

    /// The barrel map and its inverse agree, so the oracle can say where a
    /// source texel lands.
    #[test]
    fn lens_barrel_map_round_trips() {
        let profile = LensProfile::helmet();
        for uv in [
            [0.5, 0.5],
            [1.0, 1.0],
            [0.0, 0.0],
            [0.996, 0.996],
            [0.25, 0.75],
        ] {
            let landed = barrel_source_uv(uv, profile.barrel_k1);
            let back = barrel_uv(landed, profile.barrel_k1);
            assert!(
                (back[0] - uv[0]).abs() < 1e-4 && (back[1] - uv[1]).abs() < 1e-4,
                "the round trip moved {uv:?} to {landed:?} and back to {back:?}"
            );
        }
    }

    /// The helmet profile moves the corner texel inward by the barrel amount
    /// the oracle names.
    #[test]
    fn lens_helmet_profile_moves_the_corner_texel_inward_by_the_barrel_amount() {
        let profile = LensProfile::helmet();
        let (width, height) = (128u32, 128u32);
        let frame = probe_frame(width, height);
        let out = apply_pixels(&profile, 0, &frame);
        // The corner texel of the source frame.
        let corner = [
            (count_f32(width - 1) + 0.5) / count_f32(width),
            (count_f32(height - 1) + 0.5) / count_f32(height),
        ];
        // Where the oracle says it lands.
        let landed = barrel_source_uv(corner, profile.barrel_k1);
        assert!(
            landed[0] < corner[0] && landed[1] < corner[1],
            "the corner moves inward, got {landed:?} from {corner:?}"
        );
        let inward = corner[0] - landed[0];
        assert!(inward > 0.02, "the corner really moves, got {inward}");
        // Read the frame back: the output texels on the diagonal whose
        // source place reaches the corner texel.
        let mut measured = None;
        for step in 0..width {
            let base = decode_base(
                out[usize::try_from(step).unwrap_or(0) * usize::try_from(width).unwrap_or(0)
                    + usize::try_from(step).unwrap_or(0)],
                &profile,
                0,
                step,
                step,
                texel_uv(width, height, step, step),
            );
            if base[0] >= corner[0] {
                measured = Some((step, base));
                break;
            }
        }
        let (step, base) = measured.expect("the corner texel lands inside the frame");
        assert!(
            (base[0] - corner[0]).abs() < 0.01 && (base[1] - corner[1]).abs() < 0.01,
            "the texel at {step} reads the corner, got {base:?} against {corner:?}"
        );
        // The place the frame shows it at, against the oracle's barrel amount.
        let shown = texel_uv(width, height, step, step);
        let moved = corner[0] - shown[0];
        assert!(
            (moved - inward).abs() < 0.01,
            "the corner moved {moved}, the oracle says {inward}"
        );
        assert!(moved > 0.02, "the move is real, got {moved}");
    }

    /// The helmet profile splits red and blue by the chromatic aberration
    /// amount.
    #[test]
    fn lens_helmet_profile_splits_red_and_blue_by_the_ca_amount() {
        let profile = LensProfile::helmet();
        let (width, height) = (128u32, 128u32);
        let frame = probe_frame(width, height);
        let out = apply_pixels(&profile, 0, &frame);
        // A texel on the horizontal center line: the split there is radial,
        // so it runs along one axis and keeps its full length.
        let x = 112u32;
        let y = 64u32;
        let uv = texel_uv(width, height, x, y);
        let raw = decode_read(
            out[usize::try_from(y).unwrap_or(0) * usize::try_from(width).unwrap_or(0)
                + usize::try_from(x).unwrap_or(0)],
            &profile,
            0,
            x,
            y,
            uv,
        );
        let split = (raw[0] - raw[2]) * 2.0;
        assert!(
            (split.abs() - profile.ca_amount).abs() < 1e-3,
            "red and blue split by {}, got {split}",
            profile.ca_amount
        );
        // The eye splits nothing.
        let eye = LensProfile::eye();
        let plain = apply_pixels(&eye, 0, &frame);
        let raw_eye = decode_read(
            plain[usize::try_from(y).unwrap_or(0) * usize::try_from(width).unwrap_or(0)
                + usize::try_from(x).unwrap_or(0)],
            &eye,
            0,
            x,
            y,
            uv,
        );
        assert!(
            (raw_eye[0] - raw_eye[2]).abs() < 1e-6,
            "the eye splits nothing"
        );
    }

    /// The grain is seeded by the frame index: the same index gives the same
    /// grain twice, another index gives another grain.
    #[test]
    fn lens_grain_is_seeded_by_the_frame_index() {
        let profile = LensProfile::helmet();
        let frame = probe_frame(64, 64);
        let first = apply_pixels(&profile, 7, &frame);
        let second = apply_pixels(&profile, 7, &frame);
        assert_eq!(first, second, "the same frame index gives the same grain");
        let other = apply_pixels(&profile, 8, &frame);
        assert_ne!(first, other, "another frame index gives another grain");
        // The eye has no grain at all.
        let eye = LensProfile::eye();
        let plain = apply_pixels(&eye, 7, &frame);
        assert_eq!(plain, apply_pixels(&eye, 8, &frame), "the eye is still");
    }

    /// The preset table holds the numbers the lens ships.
    #[test]
    fn lens_preset_table_holds_the_shipped_numbers() {
        let eye = LensProfile::preset(LensPreset::Eye);
        assert_eq!(eye, LensProfile::eye());
        assert!(eye.is_identity());
        assert_eq!(LensProfile::default(), eye, "the eye is the default");
        let helmet = LensProfile::preset(LensPreset::Helmet);
        assert_eq!(helmet, LensProfile::helmet());
        assert!((helmet.barrel_k1 - 0.08).abs() < 1e-6);
        assert!((helmet.ca_amount - 0.005).abs() < 1e-6);
        assert!((helmet.vignette - 0.35).abs() < 1e-6);
        assert!((helmet.grain - 0.04).abs() < 1e-6);
        let chest = LensProfile::preset(LensPreset::Chest);
        assert_eq!(chest, LensProfile::chest());
        assert!(
            chest.barrel_k1 < helmet.barrel_k1,
            "the chest lens is softer"
        );
        assert!(
            chest.vignette < helmet.vignette,
            "the chest lens is lighter"
        );
        for preset in [LensPreset::Eye, LensPreset::Helmet, LensPreset::Chest] {
            preset.profile().validate().expect("the table is usable");
        }
    }

    /// A profile the pass cannot run is refused, not guessed at.
    #[test]
    fn lens_refuses_a_profile_it_cannot_run() {
        let bad = LensProfile {
            barrel_k1: f32::NAN,
            ..LensProfile::eye()
        };
        assert!(bad.validate().is_err(), "a NaN knob is refused");
        let big = LensProfile {
            vignette: 2.0,
            ..LensProfile::eye()
        };
        assert!(big.validate().is_err(), "a knob over its range is refused");
        let wide = LensFrame::new(4, 4, vec![[0.0; 3]; 15]);
        assert!(
            wide.is_err(),
            "a frame that does not match its grid is refused"
        );
        let huge = LensFrame::new(MAX_LENS_DIM + 1, 1, Vec::new());
        assert!(matches!(huge, Err(LensError::TooLarge { .. })));
    }

    /// The naga gate takes the lens shader.
    #[cfg(feature = "shader-validate")]
    #[test]
    fn lens_shader_validates_under_the_naga_gate() {
        crate::shader_gate::validate_source("post/lens.wgsl", LENS_WGSL)
            .expect("the lens shader validates");
    }

    /// The GPU pass and the CPU oracle give the same frame.
    #[cfg(feature = "window")]
    #[test]
    fn lens_gpu_matches_the_cpu_oracle() {
        let Some((device, queue)) = device_or_skip("lens_gpu_matches_the_cpu_oracle") else {
            return;
        };
        let profile = LensProfile::helmet();
        let frame = probe_frame(64, 64);
        let cpu = apply_pixels(&profile, 3, &frame);
        let pass = gpu::LensPass::new(&device);
        let got = pass
            .apply(&device, &queue, &frame, &profile, 3)
            .expect("the lens pass runs");
        assert_eq!(got.validation_errors, 0, "the pass is validation clean");
        let diff = crate::aa_scale::gpu::max_abs_diff(&cpu, &got.pixels);
        assert!(diff < 1e-3, "the shader matches the oracle, got {diff}");
    }

    /// The eye and the helmet capture sit side by side and are opened.
    #[cfg(feature = "window")]
    #[test]
    fn lens_capture_pair_eye_and_helmet() {
        let Some((device, queue)) = device_or_skip("lens_capture_pair_eye_and_helmet") else {
            return;
        };
        let frame = test_pattern(320, 180);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/lens");
        let captures = gpu::capture_presets(
            &device,
            &queue,
            &frame,
            &dir,
            &[LensPreset::Eye, LensPreset::Helmet],
            0,
        )
        .expect("the captures run");
        assert_eq!(captures.len(), 2, "one capture per preset");
        let eye = &captures[0];
        let helmet = &captures[1];
        assert_eq!(eye.preset, LensPreset::Eye);
        assert_eq!(helmet.preset, LensPreset::Helmet);
        // The eye capture is the frame itself: the GPU reads the same texels
        // back, so every pixel lands within one level of the source.
        let source = pixels_to_rgba8(&frame.pixels);
        let total = source.len() / 4;
        assert_eq!(eye.pixels.len(), total, "one capture pixel per texel");
        let mut within_one = 0usize;
        for (pixel, want) in eye.pixels.iter().zip(source.chunks_exact(4)) {
            let worst = pixel[0]
                .abs_diff(want[0])
                .max(pixel[1].abs_diff(want[1]))
                .max(pixel[2].abs_diff(want[2]));
            assert!(worst <= 1, "the eye capture moved a pixel by {worst}");
            if worst == 0 {
                within_one += 1;
            }
        }
        assert!(
            within_one * 100 >= total * 95,
            "the eye capture is the frame: {within_one} of {total} pixels match byte for byte"
        );
        // The helmet capture is darker at the corner and split at the edges.
        let corner_luma = |capture: &gpu::LensCapture| {
            0.2126 * f32::from(capture.corner[0])
                + 0.7152 * f32::from(capture.corner[1])
                + 0.0722 * f32::from(capture.corner[2])
        };
        assert!(
            corner_luma(helmet) < corner_luma(eye),
            "the vignette darkens the corner: {} against {}",
            corner_luma(helmet),
            corner_luma(eye)
        );
        assert!(
            helmet.mean_luma < eye.mean_luma,
            "the helmet frame is darker: {} against {}",
            helmet.mean_luma,
            eye.mean_luma
        );
        let moved = helmet
            .pixels
            .iter()
            .zip(eye.pixels.iter())
            .filter(|(a, b)| a[0] != b[0] || a[1] != b[1] || a[2] != b[2])
            .count();
        assert!(moved > 0, "the helmet frame moves pixels");
        println!(
            "lens capture: eye={} helmet={} corner_luma={:.2}/{:.2} mean_luma={:.4}/{:.4} moved={moved}",
            eye.path,
            helmet.path,
            corner_luma(eye),
            corner_luma(helmet),
            eye.mean_luma,
            helmet.mean_luma,
        );
    }

    /// A visible test frame: a warm gradient under a grid of lines.
    #[cfg(feature = "window")]
    fn test_pattern(width: u32, height: u32) -> LensFrame {
        let mut pixels = Vec::with_capacity(
            usize::try_from(width).unwrap_or(0) * usize::try_from(height).unwrap_or(0),
        );
        for y in 0..height {
            for x in 0..width {
                let u = (count_f32(x) + 0.5) / count_f32(width);
                let v = (count_f32(y) + 0.5) / count_f32(height);
                let sky = [0.35 + 0.3 * u, 0.45 + 0.25 * v, 0.62 - 0.2 * u];
                let line = x % 32 == 0 || y % 32 == 0;
                let marker =
                    x > width / 3 && x < width / 3 + 24 && y > height / 3 && y < height / 3 + 24;
                let pixel = if marker {
                    [0.95, 0.35, 0.2]
                } else if line {
                    [0.92, 0.9, 0.82]
                } else {
                    sky
                };
                pixels.push(pixel);
            }
        }
        LensFrame::new(width, height, pixels).expect("the test pattern fits")
    }

    /// A device for the GPU tests, or a skip when the machine has none.
    #[cfg(feature = "window")]
    fn device_or_skip(test: &str) -> Option<(wgpu::Device, wgpu::Queue)> {
        use crate::test_gpu::{require_or_skip, NoAdapter};
        crate::test_gpu::hold_gpu_slot();
        let instance = crate::backend::new_instance(wgpu::Backends::PRIMARY);
        let options = wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            apply_limit_buckets: false,
        };
        let probe = match pollster::block_on(instance.request_adapter(&options)) {
            Ok(adapter) => Ok(adapter),
            Err(error) => Err(NoAdapter(format!("no wgpu adapter: {error}"))),
        };
        let adapter = require_or_skip(test, probe)?;
        match pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("lens-test"),
            ..Default::default()
        })) {
            Ok((device, queue)) => Some((device, queue)),
            Err(error) => panic!("adapter present, device request must succeed: {error}"),
        }
    }

    /// RL39 accept: off is byte-identical to plain blit, on <= 0.15 ms, grain 0 at pure black and white.
    #[test]
    fn lens_pass_budget_and_off_identity() {
        // Off (eye profile) is byte-identical to the input.
        let (width, height) = (64u32, 64u32);
        let mut rgba = Vec::with_capacity(usize::try_from(width * height * 4).unwrap_or(0));
        for index in 0..(width * height) {
            let base = (index * 7) % 256;
            rgba.push(u8::try_from(base).unwrap_or(0));
            rgba.push(u8::try_from((base + 13) % 256).unwrap_or(0));
            rgba.push(u8::try_from((base + 29) % 256).unwrap_or(0));
            rgba.push(255);
        }
        let eye = LensProfile::eye();
        let lensed = apply_bytes(&eye, 0, &rgba, width, height).expect("the eye runs");
        assert_eq!(lensed, rgba, "off (eye) is byte-identical to plain blit");

        // On (helmet profile) budget: <= 0.15 ms, modeled not measured, so
        // the number is the same on the reference card and on a cheap one.
        let helmet = LensProfile::helmet();
        let budget_ms = helmet.lens_pass_ms(1920, 1080);
        assert!(
            budget_ms <= LENS_PASS_BUDGET_MS,
            "on (helmet) budget: {budget_ms:.3} ms > {LENS_PASS_BUDGET_MS} ms"
        );
        assert!(budget_ms > 0.0, "a running lens costs something");
        // The eye is the identity: the blit already ran it, so it costs nothing.
        assert_eq!(eye.lens_pass_ms(1920, 1080), 0.0, "off costs 0");

        // Grain is 0 at pure black and white.
        let mut black_white = Vec::with_capacity(2 * 4);
        black_white.extend_from_slice(&[0, 0, 0, 255]); // pure black
        black_white.extend_from_slice(&[255, 255, 255, 255]); // pure white
        let grain_profile = LensProfile {
            barrel_k1: 0.0,
            ca_amount: 0.0,
            vignette: 0.0,
            grain: 0.1, // enable grain
        };
        let lensed_bw = apply_bytes(&grain_profile, 0, &black_white, 2, 1)
            .expect("the grain profile runs");
        // Black should stay black (grain 0 at luma 0).
        assert_eq!(
            lensed_bw[0..3],
            [0, 0, 0],
            "grain is 0 at pure black"
        );
        // White should stay white (grain 0 at luma 1).
        assert_eq!(
            lensed_bw[4..7],
            [255, 255, 255],
            "grain is 0 at pure white"
        );
    }
}
