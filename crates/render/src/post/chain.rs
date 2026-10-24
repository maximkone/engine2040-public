use super::*;

use engine_core::Fnv64;

// ---------------------------------------------------------------------------
// Post chain: bloom, exposure, ACES, grading.
// ---------------------------------------------------------------------------

/// Bloom configuration: Kino soft-knee threshold in linear HDR, pyramid
/// radius plus mip cap, and add-back strength.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BloomConfig {
    /// Knee center in linear HDR; brighter texels bloom.
    pub threshold: f32,
    /// Soft-knee fraction in `0..=1` (donor `softKnee`): absolute knee is
    /// `threshold * knee + 1e-5`.
    pub knee: f32,
    /// Pyramid radius in `0..=8` (donor `radius`, default 2.5): larger radii
    /// deepen the mip chain via [`kino_iterations`].
    pub radius: f32,
    /// Add-back strength, `0..=2`.
    pub intensity: f32,
    /// Mip cap in the blur chain, `1..=MAX_BLOOM_LEVELS`: the radius-driven
    /// depth clamps to this cap.
    pub levels: u32,
}

impl Default for BloomConfig {
    fn default() -> Self {
        Self {
            threshold: 1.0,
            knee: 0.5,
            radius: 2.5,
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
        if !self.knee.is_finite() || !(0.0..=1.0).contains(&self.knee) {
            return Err(PostError::Invalid(format!(
                "bloom knee {} outside 0..=1 soft-knee fraction",
                self.knee
            )));
        }
        if !self.radius.is_finite() || !(0.0..=8.0).contains(&self.radius) {
            return Err(PostError::Invalid(format!(
                "bloom radius {} outside 0..=8",
                self.radius
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

/// Off intensity for the bloom add-back: zero adds nothing, so the CPU
/// chain is the identity (CPU mirror only, never a live-frame switch).
pub const BLOOM_OFF_INTENSITY: f32 = 0.0;

/// On-vs-off bloom lift in R at two texels: `on` runs [`apply_bloom`]
/// with `cfg`, `off` runs it with intensity 0 and the same threshold,
/// knee, radius and levels. Returns `(halo_lift, dark_lift)` where each lift is
/// `on minus off` at the texel. Validates the frame plus the config plus
/// both coords; out-of-range coords fail loudly. Pure, std-only,
/// caller-owned buffers, sizes nothing.
/// Caller: bloom health tests in this file (the per-frame chain no longer
/// runs the 8x8 oracle; see `run_chain_core`).
pub fn bloom_on_off_lift(
    hdr: &[[f32; 3]],
    width: u32,
    height: u32,
    cfg: &BloomConfig,
    halo_xy: (u32, u32),
    dark_xy: (u32, u32),
) -> Result<(f32, f32), PostError> {
    check_image(hdr, width, height, "bloom hdr")?;
    cfg.validate()?;
    if halo_xy.0 >= width || halo_xy.1 >= height {
        return Err(PostError::Invalid(format!(
            "bloom halo ({}, {}) outside {width}x{height}",
            halo_xy.0, halo_xy.1
        )));
    }
    if dark_xy.0 >= width || dark_xy.1 >= height {
        return Err(PostError::Invalid(format!(
            "bloom dark ({}, {}) outside {width}x{height}",
            dark_xy.0, dark_xy.1
        )));
    }
    let on = apply_bloom(hdr, width, height, cfg)?;
    let off_cfg = BloomConfig {
        intensity: BLOOM_OFF_INTENSITY,
        ..*cfg
    };
    let off = apply_bloom(hdr, width, height, &off_cfg)?;
    let stride = usize::try_from(width)
        .map_err(|_| PostError::Invalid("bloom width does not fit".into()))?;
    let halo_x = usize::try_from(halo_xy.0)
        .map_err(|_| PostError::Invalid("bloom halo x does not fit".into()))?;
    let halo_y = usize::try_from(halo_xy.1)
        .map_err(|_| PostError::Invalid("bloom halo y does not fit".into()))?;
    let dark_x = usize::try_from(dark_xy.0)
        .map_err(|_| PostError::Invalid("bloom dark x does not fit".into()))?;
    let dark_y = usize::try_from(dark_xy.1)
        .map_err(|_| PostError::Invalid("bloom dark y does not fit".into()))?;
    let halo_idx = halo_y * stride + halo_x;
    let dark_idx = dark_y * stride + dark_x;
    let halo_lift = on
        .get(halo_idx)
        .ok_or_else(|| PostError::Invalid("bloom halo index out of range".into()))?[0]
        - off
            .get(halo_idx)
            .ok_or_else(|| PostError::Invalid("bloom halo index out of range".into()))?[0];
    let dark_lift = on
        .get(dark_idx)
        .ok_or_else(|| PostError::Invalid("bloom dark index out of range".into()))?[0]
        - off
            .get(dark_idx)
            .ok_or_else(|| PostError::Invalid("bloom dark index out of range".into()))?[0];
    Ok((halo_lift, dark_lift))
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

/// Default RCAS sharpness for the chain finish: off, so the default frame
/// keeps its exact bytes and the GPU submit's parity frame stays
/// byte-comparable. A look raises it through [`PostChain::sharpen`].
pub const DEFAULT_CHAIN_SHARPEN: f32 = 0.0;

/// Full post chain configuration: bloom plus exposure plus ACES white plus
/// grading, then the display-space sharpen finish. Validated as a unit
/// before `run`.
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
    /// RCAS sharpen strength on the graded display image, `0..=1`
    /// ([`FSR_SHARPEN_MAX`]); 0 is the identity.
    pub sharpen: f32,
}

impl Default for PostChain {
    fn default() -> Self {
        Self {
            bloom: BloomConfig::default(),
            exposure: ExposureConfig::default(),
            aces_white: 11.2,
            grade: GradeConfig::default(),
            sharpen: DEFAULT_CHAIN_SHARPEN,
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
        if !self.sharpen.is_finite() || !(FSR_SHARPEN_MIN..=FSR_SHARPEN_MAX).contains(&self.sharpen)
        {
            return Err(PostError::Invalid(format!(
                "chain sharpen {} outside 0..={FSR_SHARPEN_MAX}",
                self.sharpen
            )));
        }
        Ok(())
    }

    /// Chain with the sharpen finish set: validates the strength through
    /// [`Self::validate`] so the returned chain always passes. Bloom,
    /// exposure, white and grade stay untouched.
    pub fn with_sharpen(&self, sharpen: f32) -> Result<PostChain, PostError> {
        let next = PostChain { sharpen, ..*self };
        next.validate()?;
        Ok(next)
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

/// Chain plus the look: the config a look preset writes and the frame path
/// runs through [`run_chain_look`]. Deliberately a wrapper, not a field on
/// [`PostChain`]: the knob block stays `Copy`, so every existing caller
/// (`aa_scale`, the presenter, `run_frame_auto`) keeps its signature and the
/// LUT-free path keeps its bytes. `lut` is the loaded table; `lut_path` is
/// the preset's file name, filled in by whoever loads it (this module does
/// no IO). Both default to `None`, so a default config is today's chain.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct ChainConfig {
    /// Bloom, exposure, ACES white and grading.
    pub chain: PostChain,
    /// Loaded `.cube` table, sampled after grading and before the display
    /// write; `None` skips the stage and keeps the LUT-free bytes.
    pub lut: Option<lut::Lut3d>,
    /// Look file the active preset names, for the loader; `None` when the
    /// look has no LUT.
    pub lut_path: Option<String>,
}

impl ChainConfig {
    /// Config over one chain, no table loaded.
    pub fn new(chain: PostChain) -> Self {
        Self {
            chain,
            lut: None,
            lut_path: None,
        }
    }

    /// Hand the loaded table over (the loader seam): the path stays as the
    /// preset wrote it, the table is what the chain samples.
    pub fn set_lut(&mut self, table: lut::Lut3d) {
        self.lut = Some(table);
    }

    /// Drop the table and its path: the chain returns to the LUT-free bytes.
    pub fn clear_lut(&mut self) {
        self.lut = None;
        self.lut_path = None;
    }

    /// Digest over every chain knob, the look path and the loaded table
    /// (through [`lut::Lut3d::digest`]), in a fixed order: two configs that
    /// render the same frame digest alike, and a changed knob or table moves
    /// it. `ms` receipts stay out; they are evidence, not config.
    pub fn digest(&self) -> u64 {
        let mut hasher = Fnv64::new();
        let chain = &self.chain;
        hasher.write_f32(chain.bloom.threshold);
        hasher.write_f32(chain.bloom.knee);
        hasher.write_f32(chain.bloom.radius);
        hasher.write_f32(chain.bloom.intensity);
        hasher.write_u32(chain.bloom.levels);
        hasher.write_f32(chain.exposure.exposure_ev);
        hasher.write_f32(chain.aces_white);
        hasher.write_f32(chain.grade.contrast);
        hasher.write_f32(chain.grade.saturation);
        for value in chain.grade.lift.iter().chain(chain.grade.gain.iter()) {
            hasher.write_f32(*value);
        }
        hasher.write_f32(chain.sharpen);
        match &self.lut_path {
            Some(path) => {
                hasher.write_bool(true);
                hasher.write_len32(path.len());
                hasher.write_str(path);
            }
            None => hasher.write_bool(false),
        }
        match &self.lut {
            Some(table) => {
                hasher.write_bool(true);
                hasher.write_u64(table.digest());
            }
            None => hasher.write_bool(false),
        }
        hasher.finish()
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

pub(crate) fn luminance(p: [f32; 3]) -> f32 {
    0.2126 * p[0] + 0.7152 * p[1] + 0.0722 * p[2]
}

/// Kino curve triple from threshold and soft knee.
///
/// `knee = threshold * soft_knee + 1e-5`; curve is
/// `(threshold - knee, 2 * knee, 0.25 / knee)`. Linear HDR units, pure,
/// no clock. Presentation only, never the tick hash.
/// PORT(keijiro/KinoBloom@0a04c43:Assets/Kino/Bloom/Bloom.cs): shape only,
/// reimplemented, zero donor bytes.
pub(super) fn kino_curve(threshold: f32, soft_knee: f32) -> [f32; 3] {
    let knee = threshold * soft_knee + 1e-5;
    [threshold - knee, knee * 2.0, 0.25 / knee]
}

/// Kino prefilter weight for one brightness value.
///
/// Quadratic under-threshold part plus the over-threshold line, folded
/// into one response: `max(rq, br - threshold) / max(br, 1e-5)` where
/// `rq = curve[2] * clamp(br - curve[0], 0, curve[1])^2`. Linear HDR,
/// pure. Presentation only, never the tick hash.
/// PORT(keijiro/KinoBloom@0a04c43:Assets/Kino/Bloom/Shader/Bloom.cginc):
/// shape only, reimplemented, zero donor bytes.
pub(super) fn kino_prefilter_weight(brightness: f32, threshold: f32, curve: [f32; 3]) -> f32 {
    if !brightness.is_finite() {
        return 0.0;
    }
    if brightness <= curve[0] {
        return 0.0;
    }
    if brightness >= curve[0] + curve[1] {
        return 1.0;
    }
    let rq = (brightness - curve[0]).clamp(0.0, curve[1]);
    let rq = curve[2] * rq * rq;
    let over = brightness - threshold;
    rq.max(over) / brightness.max(1e-5)
}

/// Kino pyramid depth from frame height and radius.
///
/// `depth = clamp(floor(log2(h)) + floor(radius) - 8, 1, max_levels)`.
/// Height only, so wide and tall frames with the same height share the
/// depth. Pure, no clock. Presentation only, never the tick hash.
/// PORT(keijiro/KinoBloom@0a04c43:Assets/Kino/Bloom/Bloom.cs): shape only,
/// reimplemented, zero donor bytes.
pub(super) fn kino_iterations(height: u32, radius: f32, max_levels: u32) -> u32 {
    let logh: i32 = height
        .checked_ilog2()
        .map_or(0, |v| i32::try_from(v).map_or(0, |x| x));
    let rf = radius.floor();
    let r_int: i32 = if rf >= 8.0 {
        8
    } else if rf >= 7.0 {
        7
    } else if rf >= 6.0 {
        6
    } else if rf >= 5.0 {
        5
    } else if rf >= 4.0 {
        4
    } else if rf >= 3.0 {
        3
    } else if rf >= 2.0 {
        2
    } else {
        i32::from(rf >= 1.0)
    };
    let cap: i32 = i32::try_from(max_levels).unwrap_or(6);
    let want = logh.saturating_add(r_int).saturating_sub(8);
    let clamped = want.clamp(1, cap.max(1));
    u32::try_from(clamped).unwrap_or(1)
}

/// Karis first-mip blend weight for one tap (anti-flicker option).
///
/// `w = 1 / (brightness + 1)`, normalized over the four taps by the
/// caller. Bright fireflies weigh less, so one hot texel cannot own the
/// first mip. Pure. Presentation only, never the tick hash.
/// PORT(keijiro/KinoBloom@0a04c43:Assets/Kino/Bloom/Shader/Bloom.cginc):
/// shape only, reimplemented, zero donor bytes.
pub(super) fn kino_karis_weight(brightness: f32) -> f32 {
    1.0 / (brightness + 1.0)
}

/// The Kino soft-knee weight for one texel luma, `0..=1`: [`kino_curve`]
/// plus [`kino_prefilter_weight`]. Below the knee foot nothing passes, far
/// above everything passes, smooth between. Linear HDR only. One owner for
/// the curve: the CPU pyramid ([`apply_bloom`]) and the live dual-filter
/// chain ([`crate::bloom_dual`]) both call it.
pub fn bloom_weight(luma: f32, threshold: f32, soft_knee: f32) -> f32 {
    let curve = kino_curve(threshold, soft_knee);
    kino_prefilter_weight(luma, threshold, curve)
}

/// Frame max brightness in linear HDR: max channel (Kino peak, the max
/// of R, G, B) over the frame, 0.0 when empty. Pure, no allocation, no
/// device. Same inputs give the same output twice. Presentation only,
/// never the tick hash.
/// Caller: `apply_bloom` in this file (the live gate oracle skips the pyramid
/// when no halo can form).
#[must_use]
pub fn frame_max_luma(hdr: &[[f32; 3]]) -> f32 {
    let mut max = 0.0f32;
    for p in hdr {
        let b = p[0].max(p[1]).max(p[2]);
        if b > max {
            max = b;
        }
    }
    max
}

/// Whether the bloom halo can form for one frame's max luma. Pure, no
/// allocation, no device, no readback. Fail-closed on bad input; sub-knee
/// skips matching `bloom_weight` nothing-below-knee rule (foot is
/// `threshold - (threshold * knee + 1e-5)`).
/// Caller: `apply_bloom` in this file (early-out keeps dark frames
/// bit-identical); the future live bloom pass in
/// `present/gpu_present/live.rs` mirrors this rule through
/// `live_bloom_should_run`.
#[must_use]
pub fn bloom_should_run(max_luma: f32, threshold: f32, knee: f32) -> bool {
    if !max_luma.is_finite() {
        return false;
    }
    if !threshold.is_finite() || threshold < 0.0 {
        return false;
    }
    if !knee.is_finite() || !(0.0..=1.0).contains(&knee) {
        return false;
    }
    let foot = threshold - (threshold * knee + 1e-5);
    if max_luma <= foot {
        return false;
    }
    true
}

/// Byte offset of the bloom enable inside the blit fog uniform
/// (`FogU::_pad0`). Units bytes. The neutral uniform is zero there, so
/// `0.0` keeps today's frame bit-identical. Presentation only.
/// Caller: `crates/render/src/present/gpu_present/submit.rs` live encode.
pub const BLOOM_PASS_FLAG_OFFSET: u64 = 20;

/// GPU-side bloom pass plan: extract, blur, add descriptors reusing the
/// CPU thresholds. No CPU curve change: threshold, knee and intensity
/// mirror [`BloomConfig`], depth mirrors [`kino_iterations`] clamped to
/// the config cap. Presentation only, never the tick hash.
/// Caller: `crates/render/src/present/gpu_present/presenter.rs` bloom state.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct BloomPassPlan {
    /// Knee center in linear HDR; brighter texels bloom.
    pub threshold: f32,
    /// Soft-knee fraction in `0..=1`.
    pub knee: f32,
    /// Add-back strength, `0..=2`.
    pub intensity: f32,
    /// Pyramid depth from height and radius, clamped to levels.
    pub depth: u32,
    /// Mip cap from the config, `1..=MAX_BLOOM_LEVELS`.
    pub levels: u32,
}

impl BloomPassPlan {
    /// Build from frame height and config: validates the config, derives
    /// the radius-driven depth. Fails loudly on bad height or config.
    pub fn for_frame(height: u32, cfg: &BloomConfig) -> Result<Self, PostError> {
        cfg.validate()?;
        if height == 0 {
            return Err(PostError::Invalid("bloom pass height is zero".into()));
        }
        if height > MAX_POST_DIM {
            return Err(PostError::BudgetExceeded {
                requested: height,
                allowed: MAX_POST_DIM,
            });
        }
        let kino = kino_iterations(height, cfg.radius, MAX_BLOOM_LEVELS);
        let depth = kino.min(cfg.levels).max(1);
        Ok(Self {
            threshold: cfg.threshold,
            knee: cfg.knee,
            intensity: cfg.intensity,
            depth,
            levels: cfg.levels,
        })
    }

    /// Whether the halo can form for `max_luma`: pure delegate to
    /// [`bloom_should_run`], fail-closed. No allocation, no device.
    #[must_use]
    pub fn should_run(&self, max_luma: f32) -> bool {
        bloom_should_run(max_luma, self.threshold, self.knee)
    }

    /// Flag for the blit uniform: `1.0` when enabled and the gate runs,
    /// else `0.0` (bit-identical off). No allocation, no device.
    #[must_use]
    pub fn flag(&self, enabled: bool, max_luma: f32) -> f32 {
        if enabled && self.should_run(max_luma) {
            1.0
        } else {
            0.0
        }
    }
}

/// SIMD slice: bright-extract over one HDR frame. The scalar loop is
/// canonical (std-only, runs everywhere); the `v1` glam loop is the SIMD
/// path (lane-wise `Vec3` multiply, same op order, bit-identical pixels).
/// Simulation code is untouched (presentation pixels only, never the tick
/// hash). Each change is A/B-measured by `bright_extract_ab` below.
pub(super) fn bright_extract_scalar(hdr: &[[f32; 3]], threshold: f32, knee: f32) -> Vec<[f32; 3]> {
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
pub(super) fn bright_extract_glam(hdr: &[[f32; 3]], threshold: f32, knee: f32) -> Vec<[f32; 3]> {
    let mut bright = Vec::with_capacity(hdr.len());
    for p in hdr {
        let w = bloom_weight(luminance(*p), threshold, knee);
        let lit = glam::Vec3::from_array(*p) * w;
        bright.push(lit.to_array());
    }
    bright
}

/// Dispatcher: glam SIMD under `v1`, scalar fallback otherwise.
pub(super) fn bright_extract(hdr: &[[f32; 3]], threshold: f32, knee: f32) -> Vec<[f32; 3]> {
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
pub(super) fn downsample_box(src: &[[f32; 3]], w: u32, h: u32) -> (Vec<[f32; 3]>, u32, u32) {
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

/// Karis-weighted first downsample: each of the up to four taps weighs
/// `1 / (luma + 1)`, normalized over the taps present, so a single hot
/// texel cannot own the first mip (anti-flicker). Later mips stay plain
/// box averages. Same ceil-halve sizes as [`downsample_box`]; fixed tap
/// order, bit-identical twice. Linear HDR only.
pub(super) fn downsample_karis(src: &[[f32; 3]], w: u32, h: u32) -> (Vec<[f32; 3]>, u32, u32) {
    let dw = w.div_ceil(2).max(1);
    let dh = h.div_ceil(2).max(1);
    let total = dw.saturating_mul(dh);
    let cap = usize::try_from(total).unwrap_or(0);
    let mut out = Vec::with_capacity(cap);
    for y in 0..dh {
        for x in 0..dw {
            let mut acc = [0.0f32; 3];
            let mut wsum = 0.0f32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let sx = x.saturating_mul(2).saturating_add(dx);
                    let sy = y.saturating_mul(2).saturating_add(dy);
                    if sx < w && sy < h {
                        let idx_u32 = sy.saturating_mul(w).saturating_add(sx);
                        let idx = usize::try_from(idx_u32).unwrap_or(0);
                        let p = src[idx];
                        let wt = kino_karis_weight(luminance(p));
                        acc[0] += p[0] * wt;
                        acc[1] += p[1] * wt;
                        acc[2] += p[2] * wt;
                        wsum += wt;
                    }
                }
            }
            let inv = if wsum > 0.0 { 1.0 / wsum } else { 0.0 };
            out.push([acc[0] * inv, acc[1] * inv, acc[2] * inv]);
        }
    }
    (out, dw, dh)
}

/// Apply bloom: Kino soft-knee extract, Karis-weighted first mip plus
/// box-blur pyramid, bilinear halo add-back of mips larger than 2x2 (the
/// 2x2 plus 1x1 veil would lift the whole frame). Returns the flared HDR
/// frame. Linear HDR only.
pub fn apply_bloom(
    hdr: &[[f32; 3]],
    width: u32,
    height: u32,
    cfg: &BloomConfig,
) -> Result<Vec<[f32; 3]>, PostError> {
    cfg.validate()?;
    check_image(hdr, width, height, "bloom hdr")?;
    if !bloom_should_run(frame_max_luma(hdr), cfg.threshold, cfg.knee) {
        return Ok(hdr.to_vec());
    }
    let bright = bright_extract(hdr, cfg.threshold, cfg.knee);
    // Pyramid depth: radius-driven Kino count clamped to the config cap.
    // Donor derives depth from height only, so wide and tall frames with
    // the same height share the depth.
    let kino_depth = kino_iterations(height, cfg.radius, MAX_BLOOM_LEVELS);
    let depth = kino_depth.min(cfg.levels).max(1);
    // Blur chain: the first downsample is Karis-weighted (one hot texel
    // cannot own the mip), later levels are plain box averages. Each level
    // halves the previous bright mip. No per-level clone: each mip moves
    // into the chain and the next level reads it from there, so the chain
    // holds one copy only.
    let mut chain: Vec<(Vec<[f32; 3]>, u32, u32)> = Vec::new();
    let mut bright_opt: Option<Vec<[f32; 3]>> = Some(bright);
    let (mut lw, mut lh) = (width, height);
    for i in 0..depth {
        let (next, nw, nh) = if i == 0 {
            let src = bright_opt
                .as_ref()
                .ok_or_else(|| PostError::Invalid("bloom bright held for first mip".into()))?;
            downsample_karis(src, lw, lh)
        } else {
            let (prev, _, _) = chain
                .last()
                .ok_or_else(|| PostError::Invalid("bloom chain holds prior mip".into()))?;
            downsample_box(prev, lw, lh)
        };
        if i == 0 {
            bright_opt.take();
        }
        lw = nw;
        lh = nh;
        chain.push((next, lw, lh));
        if lw == 1 && lh == 1 {
            break;
        }
    }
    // Halo add-back excludes the global veil: mips at or below 2x2 cover
    // the whole frame, so compositing them lifts far darks (the 8x8 torch
    // at (3, 3) leaked 0.0625 into (0, 0) via the 2x2 plus 1x1). Only
    // larger mips scatter locally, so the neighbor lifts while the far
    // corner stays bit-identical. Small frames with only veil mips stay
    // the identity (no halo mips, no add-back).
    // Caller: crates/render/src/present/gpu_present/live.rs plus submit.rs
    // (presenter encode between forward and encode_blit; GPU veil follow-up
    // still open, CPU oracle only here).
    let halo_count = chain
        .iter()
        .filter(|(_, mw, mh)| *mw > 2 || *mh > 2)
        .count();
    // Add halo mips back, spread evenly over the intensity. The CPU oracle
    // samples with bilinear; the GPU upsample follow-up uses the donor 9-tap
    // tent (3x3: center 4, edges 2, corners 1, over 16) on the same mips.
    let halo_levels = u16::try_from(halo_count)
        .map_err(|_| PostError::Invalid("bloom halo levels do not fit".into()))?;
    let per = if halo_count == 0 {
        0.0
    } else {
        cfg.intensity / f32::from(halo_levels)
    };
    let mut out: Vec<[f32; 3]> = hdr.to_vec();
    for (mip, mw, mh) in &chain {
        if *mw <= 2 && *mh <= 2 {
            continue;
        }
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

/// The per-channel fit of the one source: `crate::tonemap::fit_one` is the
/// CPU twin of the WGSL `tonemap.wgsl` chunk, so this path and every shader
/// consumer share one curve. Re-exported here because the post chain is the
/// crate's parity oracle and the shadow path imports it from this module.
pub(super) use crate::tonemap::fit_one as neutral_fit_one;

#[expect(
    non_upper_case_globals,
    reason = "compat alias keeps the shadow path until later slices"
)]
/// Compatibility alias for the shadow path until later slices route it to
/// the one source. This is a pointer to the one source's fit, not a second
/// curve: the WGSL `tonemap.wgsl` chunk owns the curve and every consumer
/// calls it. Units: linear HDR in, display `0..=1` out. Errors: none.
/// Determinism: pure; same input gives same output twice.
pub const aces_fitted: fn(f32) -> f32 = neutral_fit_one;

/// ACES tonemap with white normalization: exposure-scaled hue-keeping
/// display over the shared Neutral fit, output `0..=1` with ratios kept.
/// This is the crate's one tone map: the WGSL `tonemap.wgsl` chunk and
/// `crate::tonemap::tone_map` hold the same curve, and the shader
/// consumers (the live blit, the offscreen capture, the post chain and its
/// compute twin) all call it.
/// Units: linear HDR in, display `0..=1` out. Errors: typed [`PostError`]
/// on non-finite or non-positive exposure, white, or color.
/// Determinism: pure; same inputs give same outputs twice.
/// Caller: `run_chain_core` in this file (presenter frame oracle health).
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
    // The exposure rides the colour, then the tone map sees the exposed
    // value with exposure 1.0: the same order the WGSL post chain runs
    // (`post_chain.wgsl`), so the CPU oracle and the shader agree bit for
    // bit. One scale for every channel, so the hue survives (the per
    // channel fit this replaced shifted a torch flame's hue by +32.3
    // degrees).
    let exposed = [c[0] * exposure, c[1] * exposure, c[2] * exposure];
    Ok(crate::tonemap::tone_map(exposed, 1.0, white))
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
/// complete inputs give bit-identical pixels; only `ms` varies. This is the
/// LUT-free entry: [`run_chain_look`] runs the same chain and samples the
/// config's table after grading.
pub fn run_chain(
    hdr: &[[f32; 3]],
    width: u32,
    height: u32,
    chain: &PostChain,
    lights: &PostLights,
) -> Result<PostOutput, PostError> {
    chain.validate()?;
    check_image(hdr, width, height, "post hdr")?;
    run_chain_core(hdr, width, height, chain, None, lights)
}

/// Run the chain with the look: identical to [`run_chain`] plus, when
/// `cfg.lut` holds a table, one trilinear sample per pixel after
/// `apply_grade` and before the display write. With `cfg.lut` at `None` the
/// pixels and the replay digest are exactly `run_chain`'s. Same complete
/// inputs give bit-identical pixels; only `ms` varies.
pub fn run_chain_look(
    hdr: &[[f32; 3]],
    width: u32,
    height: u32,
    cfg: &ChainConfig,
    lights: &PostLights,
) -> Result<PostOutput, PostError> {
    cfg.chain.validate()?;
    check_image(hdr, width, height, "post hdr")?;
    run_chain_core(hdr, width, height, &cfg.chain, cfg.lut.as_ref(), lights)
}

/// Validated chain core: bloom, exposure, ACES, grading over inputs the
/// caller already checked. Exists so the auto path validates once (the
/// tick checks the frame, the core never rescans it); behavior past the
/// checks is exactly `run_chain`. `look` is the optional table sampled after
/// grading, so the LUT stage sits on display values and before the write.
pub(super) fn run_chain_core(
    hdr: &[[f32; 3]],
    width: u32,
    height: u32,
    chain: &PostChain,
    look: Option<&lut::Lut3d>,
    lights: &PostLights,
) -> Result<PostOutput, PostError> {
    let pixels = u64::from(width) * u64::from(height);

    let started = Instant::now();
    let flared = apply_bloom(hdr, width, height, &chain.bloom)?;
    let bloom_ms = started.elapsed().as_secs_f64() * 1000.0;

    // Fused exposure plus ACES plus grade in one pass over the flared
    // frame: `tonemap_aces(p, mult, white)` repeats the old `p * mult`
    // then `tonemap_aces(exposed, 1.0)` bit-identically (times 1.0 is
    // exact), then grades. One full-frame buffer instead of three; the
    // fused ms rides the exposure slot while aces/grade quote 0 so the
    // total stays honest. The look samples the graded value in the same
    // pass: no second buffer, and a `None` look leaves the arithmetic
    // exactly as it was.
    let started = Instant::now();
    let mult = chain.exposure.mult()?;
    let mut ldr = Vec::with_capacity(flared.len());
    for p in &flared {
        let toned = tonemap_aces(*p, mult, chain.aces_white)?;
        let graded = apply_grade(toned, &chain.grade)?;
        ldr.push(match look {
            Some(table) => table.apply(graded),
            None => graded,
        });
    }
    let exposure_ms = started.elapsed().as_secs_f64() * 1000.0;
    let (aces_ms, grade_ms) = (0.0, 0.0);

    // Sharpen finish: RCAS over the graded display frame, the last stage
    // after the TAA resolve and the upsample. Its lobe limiter clamps every
    // channel to the four neighbour taps, so a sharpened edge cannot
    // invent a level past the input image's own range. Sharpness 0 keeps
    // the pixel vector as it is (no copy, no pass).
    let started = engine_clock::now();
    let ldr = if chain.sharpen > FSR_SHARPEN_MIN {
        fsr_rcas_sharpen(&ldr, width, height, chain.sharpen)?
    } else {
        ldr
    };
    let sharpen_ms = engine_clock::since(started).as_secs_f64() * 1000.0;

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
            // The display pass receipt carries grade plus the RCAS finish:
            // they run back to back on the same buffer and the frame
            // receipt has one slot for it.
            grade: receipt("grade", pixels, grade_ms + sharpen_ms),
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
pub(super) fn resolve_and_upsample(
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Light receipt with no light pass ms: the chain only reads the numbers.
    fn lights() -> PostLights {
        PostLights {
            sun_ms: 0.0,
            cluster_ms: 0.0,
            ibl_ms: 0.0,
        }
    }

    /// Digest over display pixels: the replay proof for one chain run.
    fn pixels_digest(px: &[[f32; 3]]) -> u64 {
        let mut hasher = Fnv64::new();
        for pixel in px {
            for channel in pixel {
                hasher.write_f32(*channel);
            }
        }
        hasher.finish()
    }

    /// `.cube` text of `size^3` rows in blue-fastest order.
    fn cube_text<F: Fn(f32, f32, f32) -> [f32; 3]>(size: usize, map: F) -> String {
        let last = (size - 1) as f32;
        let mut text = format!("LUT_3D_SIZE {size}\n");
        for r in 0..size {
            for g in 0..size {
                for b in 0..size {
                    let out = map(r as f32 / last, g as f32 / last, b as f32 / last);
                    text.push_str(&format!("{} {} {}\n", out[0], out[1], out[2]));
                }
            }
        }
        text
    }

    /// A grey ramp frame: 64 texels from deep shadow to a bright highlight.
    fn ramp() -> (Vec<[f32; 3]>, u32, u32) {
        let (width, height) = (16u32, 4u32);
        let mut frame = Vec::with_capacity(64);
        for i in 0..64 {
            let luma = 0.02 + 3.8 * (i as f32 / 63.0);
            frame.push([luma, luma, luma]);
        }
        (frame, width, height)
    }

    /// Mean of one channel over the ramp output.
    fn mean_channel(px: &[[f32; 3]], channel: usize) -> f32 {
        let mut sum = 0.0f32;
        for pixel in px {
            sum += pixel[channel];
        }
        sum / 64.0
    }

    /// The LUT-free look path is the old chain, byte for byte.
    #[test]
    fn chain_look_none_matches_run_chain_bytes() {
        let (frame, width, height) = ramp();
        let chain = PostChain::default();
        let base = run_chain(&frame, width, height, &chain, &lights()).expect("base runs");
        let cfg = ChainConfig::new(chain);
        assert!(cfg.lut.is_none(), "a fresh config carries no table");
        let look = run_chain_look(&frame, width, height, &cfg, &lights()).expect("look runs");
        assert_eq!(base.ldr, look.ldr, "the LUT-free look changes pixels");
        assert_eq!(
            pixels_digest(&base.ldr),
            pixels_digest(&look.ldr),
            "the LUT-free look changes the replay digest"
        );
        assert_eq!(base.width, look.width, "width stays");
        assert_eq!(base.height, look.height, "height stays");
    }

    /// An identity 2x2x2 table leaves every graded pixel within one 8-bit
    /// step, so wiring a no-op look is invisible on the ramp.
    #[test]
    fn chain_look_identity_keeps_the_ramp_within_one_step() {
        let (frame, width, height) = ramp();
        let mut cfg = ChainConfig::default();
        cfg.set_lut(lut::parse_cube(&cube_text(2, |r, g, b| [r, g, b])).expect("identity parses"));
        let base = run_chain(&frame, width, height, &cfg.chain, &lights()).expect("base runs");
        let look = run_chain_look(&frame, width, height, &cfg, &lights()).expect("look runs");
        let step = 1.0 / 255.0;
        for (before, after) in base.ldr.iter().zip(look.ldr.iter()) {
            for channel in 0..3 {
                assert!(
                    (before[channel] - after[channel]).abs() <= step,
                    "identity look moves {before:?} to {after:?}"
                );
            }
        }
    }

    /// A warm 3x3x3 table raises red and lowers blue over the same ramp.
    #[test]
    fn chain_look_warm_shift_moves_red_up_and_blue_down() {
        let (frame, width, height) = ramp();
        let mut cfg = ChainConfig::default();
        cfg.set_lut(
            lut::parse_cube(&cube_text(3, |r, g, b| [r * 0.9 + 0.1, g, b * 0.9]))
                .expect("warm cube parses"),
        );
        let base = run_chain(&frame, width, height, &cfg.chain, &lights()).expect("base runs");
        let warm = run_chain_look(&frame, width, height, &cfg, &lights()).expect("look runs");
        let red_before = mean_channel(&base.ldr, 0);
        let red_after = mean_channel(&warm.ldr, 0);
        let blue_before = mean_channel(&base.ldr, 2);
        let blue_after = mean_channel(&warm.ldr, 2);
        eprintln!(
            "warm look: red {red_before:.4} -> {red_after:.4}, blue {blue_before:.4} -> {blue_after:.4}"
        );
        assert!(
            red_after > red_before,
            "red must rise, went {red_before} -> {red_after}"
        );
        assert!(
            blue_after < blue_before,
            "blue must fall, went {blue_before} -> {blue_after}"
        );
        assert!(
            (mean_channel(&base.ldr, 1) - mean_channel(&warm.ldr, 1)).abs() <= 1e-4,
            "green stays put on the warm table"
        );
    }

    /// Clearing the look returns the exact bytes and the exact config digest
    /// of the LUT-free chain.
    #[test]
    fn chain_look_cleared_restores_the_exact_bytes() {
        let (frame, width, height) = ramp();
        let mut cfg = ChainConfig::default();
        let clean_digest = cfg.digest();
        let clean = run_chain_look(&frame, width, height, &cfg, &lights()).expect("clean runs");
        cfg.set_lut(
            lut::parse_cube(&cube_text(3, |r, g, b| [r * 0.9 + 0.1, g, b * 0.9]))
                .expect("warm cube parses"),
        );
        assert_ne!(
            cfg.digest(),
            clean_digest,
            "the loaded table moves the digest"
        );
        let warm = run_chain_look(&frame, width, height, &cfg, &lights()).expect("warm runs");
        assert_ne!(
            pixels_digest(&warm.ldr),
            pixels_digest(&clean.ldr),
            "the warm look must move pixels"
        );
        cfg.clear_lut();
        assert_eq!(
            cfg.digest(),
            clean_digest,
            "clearing restores the config digest"
        );
        let back = run_chain_look(&frame, width, height, &cfg, &lights()).expect("back runs");
        assert_eq!(
            back.ldr, clean.ldr,
            "clearing the look must restore every byte"
        );
    }

    /// The config digest covers the look path and the table, not the receipt ms.
    #[test]
    fn chain_config_digest_tracks_the_look() {
        let none = ChainConfig::default().digest();
        let mut cfg = ChainConfig {
            lut_path: Some("content/looks/dusk.cube".into()),
            ..ChainConfig::default()
        };
        let path = cfg.digest();
        assert_ne!(path, none, "the look path moves the digest");
        cfg.set_lut(lut::parse_cube(&cube_text(2, |r, g, b| [r, g, b])).expect("identity parses"));
        let table = cfg.digest();
        assert_ne!(table, path, "the loaded table moves the digest");
        let again = {
            let mut other = ChainConfig {
                lut_path: cfg.lut_path.clone(),
                ..ChainConfig::default()
            };
            other.set_lut(lut::parse_cube(&cube_text(2, |r, g, b| [r, g, b])).expect("parses"));
            other.digest()
        };
        assert_eq!(again, table, "two configs that render alike digest alike");
    }

    /// LF5d fixture: a soft vertical edge, one intermediate column between
    /// deep sand and bright sand, bloom off so the add-back cannot blur it.
    fn soft_edge_luma(x: u32) -> f32 {
        match x {
            0..=6 => 0.05,
            7 => 0.35,
            _ => 1.0,
        }
    }

    fn soft_edge() -> (Vec<[f32; 3]>, u32, u32) {
        let (width, height) = (16u32, 8u32);
        let mut frame = Vec::with_capacity((width * height) as usize);
        for _ in 0..height {
            for x in 0..width {
                let luma = soft_edge_luma(x);
                frame.push([luma, luma, luma]);
            }
        }
        (frame, width, height)
    }

    /// Largest adjacent luma step on one middle row: the edge's contrast.
    fn peak_gradient(px: &[[f32; 3]], width: u32) -> f32 {
        let y = 3usize;
        let w = width as usize;
        let mut peak = 0.0f32;
        for x in 0..w - 1 {
            let step = (luminance(px[y * w + x + 1]) - luminance(px[y * w + x])).abs();
            peak = peak.max(step);
        }
        peak
    }

    /// LF5d Accept: the RCAS finish after the resolve raises the fixture
    /// edge's contrast, and no output channel lands past the input image's
    /// own max or below its min (the lobe limiter cannot invent a level).
    #[test]
    fn rcas_finish_raises_edge_contrast_without_new_peaks() {
        let (frame, width, height) = soft_edge();
        let chain = PostChain {
            bloom: BloomConfig {
                intensity: 0.0,
                ..BloomConfig::default()
            },
            ..PostChain::default()
        };
        let flat = run_chain(&frame, width, height, &chain, &lights()).expect("flat chain runs");
        let sharp = run_chain(
            &frame,
            width,
            height,
            &chain
                .with_sharpen(FSR_DEFAULT_SHARPNESS)
                .expect("sharpen knob validates"),
            &lights(),
        )
        .expect("sharp chain runs");

        let flat_peak = peak_gradient(&flat.ldr, width);
        let sharp_peak = peak_gradient(&sharp.ldr, width);
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for pixel in &flat.ldr {
            for channel in pixel {
                lo = lo.min(*channel);
                hi = hi.max(*channel);
            }
        }
        // The round-trip through the lobe clamp keeps every texel inside
        // the input's own range; one f32 ulp of slack for the clamp math.
        let slack = 1e-6;
        for (before, after) in flat.ldr.iter().zip(sharp.ldr.iter()) {
            for channel in 0..3 {
                assert!(
                    after[channel] >= lo - slack && after[channel] <= hi + slack,
                    "sharpen left the input range [{lo}, {hi}]: {before:?} -> {after:?}"
                );
            }
        }
        eprintln!(
            "rcas finish: edge contrast {flat_peak:.6} -> {sharp_peak:.6} \
             ({:.1}%), input range [{lo:.4}, {hi:.4}]",
            100.0 * (sharp_peak / flat_peak - 1.0)
        );
        assert!(
            sharp_peak > flat_peak * 1.02,
            "the finish must raise edge contrast: {flat_peak:.6} -> {sharp_peak:.6}"
        );
        assert_ne!(
            pixels_digest(&flat.ldr),
            pixels_digest(&sharp.ldr),
            "the finish must move pixels"
        );
        assert!(
            flat.receipt.grade.ms <= sharp.receipt.grade.ms,
            "the display receipt carries the finish's ms"
        );
    }

    /// Sharpen 0 and the default chain are the same frame, byte for byte:
    /// the finish is off unless a look asks for it.
    #[test]
    fn chain_sharpen_zero_keeps_the_default_bytes() {
        let (frame, width, height) = soft_edge();
        let base =
            run_chain(&frame, width, height, &PostChain::default(), &lights()).expect("base runs");
        let cfg = ChainConfig::default();
        assert!(cfg.chain.sharpen.abs() <= f32::EPSILON, "off by default");
        let zero = run_chain(
            &frame,
            width,
            height,
            &cfg.chain
                .with_sharpen(FSR_SHARPEN_MIN)
                .expect("zero validates"),
            &lights(),
        )
        .expect("zero runs");
        assert_eq!(zero.ldr, base.ldr, "sharpness 0 keeps every byte");
    }

    /// The sharpen knob is part of the chain's validated range and digest.
    #[test]
    fn chain_sharpen_out_of_range_fails_loudly() {
        let chain = PostChain::default();
        for bad in [-0.1, 1.1, f32::NAN] {
            assert!(
                matches!(
                    PostChain {
                        sharpen: bad,
                        ..chain
                    }
                    .validate(),
                    Err(PostError::Invalid(_))
                ),
                "sharpen {bad} must fail validation"
            );
            assert!(chain.with_sharpen(bad).is_err(), "with_sharpen {bad} fails");
        }
        let mut cfg = ChainConfig::new(chain);
        let off = cfg.digest();
        cfg.chain.sharpen = FSR_DEFAULT_SHARPNESS;
        assert_ne!(cfg.digest(), off, "the sharpen knob moves the digest");
    }
}
