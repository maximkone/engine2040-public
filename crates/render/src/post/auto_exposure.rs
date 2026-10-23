use super::*;

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
pub(super) const AUTO_BIN_WIDTH_EV: f32 = (AUTO_LOG_MAX_EV - AUTO_LOG_MIN_EV) / 64.0;
/// Halving steps locating one of [`AUTO_HIST_BINS`] bins (2 to the 6th
/// spans 64 bins).
pub(super) const AUTO_HIST_STEPS: usize = 6;
/// Widest histogram stride in texels: large frames sample one texel per
/// 4x4 block, small frames sample every texel (see [`hist_stride`]).
pub(super) const AUTO_HIST_STRIDE_MAX: u32 = 4;

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
/// The GPU histogram kernel samples the same texels the CPU oracle reads.
pub(crate) fn hist_stride(width: u32, height: u32) -> u32 {
    (width.min(height) / 16).clamp(1, AUTO_HIST_STRIDE_MAX)
}

/// Bin width in stops of the GPU kernel: the same 0.3125 the CPU oracle
/// divides by, so one sample lands in the same bin on both sides.
pub const AUTO_BIN_WIDTH_EV_F32: f32 = 0.3125;

/// One integer count as an exact f32: the count splits into two 16-bit
/// halves, each exact in f32, and the high half rides a power of two, so
/// counts below 2 to the 24th convert without rounding. No numeric cast,
/// no heap.
fn exact_f32(value: u32) -> f32 {
    let low = u16::try_from(value & 0xFFFF).unwrap_or(0);
    let high = u16::try_from(value >> 16).unwrap_or(0);
    f32::from(low) + f32::from(high) * 65_536.0
}

impl ExposureHistogram {
    /// Build a histogram from the GPU kernel's per-bin counts (one atomic
    /// add per sampled texel). The counts are exact below 2 to the 24th
    /// samples, so the trim math sees the same numbers the CPU oracle
    /// produced. An all-zero count set is an empty frame and fails loudly.
    pub fn from_gpu_counts(counts: &[u32; AUTO_HIST_BINS]) -> Result<Self, PostError> {
        let mut bins = [0.0f32; AUTO_HIST_BINS];
        let mut texels = 0.0f32;
        for (slot, count) in counts.iter().enumerate() {
            let value = exact_f32(*count);
            bins[slot] = value;
            texels += value;
        }
        if texels <= 0.0 {
            return Err(PostError::Invalid(
                "auto-exposure gpu histogram is empty".into(),
            ));
        }
        Ok(Self { bins, texels })
    }
}

/// Mean linear luma of an HDR frame: the plain average over the frame,
/// the number a reader compares against the key. Empty frames read 0.0.
pub fn mean_linear_luma(hdr: &[[f32; 3]]) -> f32 {
    if hdr.is_empty() {
        return 0.0;
    }
    let mut sum = 0.0f32;
    for p in hdr {
        sum += luminance(*p);
    }
    let count = u32::try_from(hdr.len()).unwrap_or(u32::MAX);
    sum / exact_f32(count)
}

/// Bin index for a scaled log-luma in `[0, 64]`: six halving steps, most
/// significant bit first, so no float-to-int conversion appears on the
/// hot path. Exact for finite inputs; 64.0 still lands on bin 63.
pub(super) fn bin_for_scaled(scaled: f32) -> usize {
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
        self.tick_hist(&hist, dt_s)
    }

    /// Tick from a histogram already built (the GPU histogram path): the
    /// same trim, aim and slew as [`Self::tick`], with no pixel walk. A
    /// `dt_s` of 0 holds; negative or non-finite dts fail loudly.
    pub fn tick_hist(&mut self, hist: &ExposureHistogram, dt_s: f32) -> Result<f32, PostError> {
        if !dt_s.is_finite() || dt_s < 0.0 {
            return Err(PostError::Invalid(
                "auto-exposure dt_s is not finite non-negative".into(),
            ));
        }
        let mean = trimmed_log_mean(hist, self.config.trim_lo, self.config.trim_hi)?;
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
    let mut out = run_chain_core(hdr, width, height, &adapted, None, lights)?;
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
// Fixtures: a grey ramp for the convergence accept and a night arena lit
// by torches for the clip accept. Deterministic data, no wall clock.
// ---------------------------------------------------------------------------

/// A grey ramp frame: the linear luma runs from `low` to `high` across the
/// width, one value per column, every row the same. The trimmed mean sits
/// inside the band, and the histogram spreads over several bins, so the
/// meter resolves small steps instead of jumping a whole bin.
pub fn luma_ramp(width: u32, height: u32, low: f32, high: f32) -> Result<Vec<[f32; 3]>, PostError> {
    if width == 0 || height == 0 {
        return Err(PostError::Invalid("exposure ramp size is zero".into()));
    }
    if !low.is_finite() || !high.is_finite() || low <= 0.0 || high < low {
        return Err(PostError::Invalid(
            "exposure ramp band must be positive finite and ordered".into(),
        ));
    }
    let last = f32::from(u16::try_from(width - 1).unwrap_or(1));
    let mut frame = Vec::with_capacity(
        usize::try_from(width).unwrap_or(0) * usize::try_from(height).unwrap_or(0),
    );
    for _ in 0..height {
        for x in 0..width {
            let t = if width > 1 {
                f32::from(u16::try_from(x).unwrap_or(0)) / last
            } else {
                0.0
            };
            let luma = low + (high - low) * t;
            frame.push([luma, luma, luma]);
        }
    }
    check_image(&frame, width, height, "exposure ramp")?;
    Ok(frame)
}

/// A night arena lit by torches: a sand ramp plus a lattice of warm torch
/// cores. The sand carries the scene level, the cores are the highlights
/// the percentile trim keeps out of the mean, and under a fixed exposure
/// the cores clip while the adapted exposure keeps the frame clean.
#[derive(Clone, PartialEq, Debug)]
pub struct NightArena {
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// The HDR frame, row-major.
    pub frame: Vec<[f32; 3]>,
    /// Torch core pixels in the frame.
    pub torch_pixels: u32,
}

impl NightArena {
    /// Build the night arena with `torches` cores on a fixed lattice over
    /// a sand ramp. The sand runs from half a stop under one to half a
    /// stop over one in linear luma, the cores sit eight stops over the
    /// mid sand value.
    pub fn with_torches(torches: u32) -> Result<Self, PostError> {
        let (width, height) = (128u32, 128u32);
        if torches == 0 {
            return Err(PostError::Invalid(
                "night arena needs at least one torch".into(),
            ));
        }
        let mut frame = luma_ramp(width, height, 0.5, 1.5)?;
        let mut torch_pixels = 0u32;
        let mut index = 0u32;
        for y in 0..height {
            for x in 0..width {
                if x % 8 == 0 && y % 8 == 0 && index < torches {
                    let at = usize::try_from(y).unwrap_or(0) * usize::try_from(width).unwrap_or(0)
                        + usize::try_from(x).unwrap_or(0);
                    if let Some(pixel) = frame.get_mut(at) {
                        *pixel = [8.6, 7.9, 7.0];
                    }
                    torch_pixels += 1;
                    index += 1;
                }
            }
        }
        Ok(Self {
            width,
            height,
            frame,
            torch_pixels,
        })
    }

    /// The frame scaled by `mult`, the exposure pass applied to the scene.
    pub fn exposed(&self, mult: f32) -> Vec<[f32; 3]> {
        self.frame
            .iter()
            .map(|p| [p[0] * mult, p[1] * mult, p[2] * mult])
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests: the ramp convergence accept, the night capture clip accept, the
// byte-equal rerun and the GPU count readback.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Light receipt for the chain runs: no light pass ms, the chain only
    /// reads the numbers.
    fn lights() -> PostLights {
        PostLights {
            sun_ms: 0.0,
            cluster_ms: 0.0,
            ibl_ms: 0.0,
        }
    }

    /// The metered mean luma of a frame: the trimmed log mean the meter
    /// itself reads, turned back into linear luma. This is the number the
    /// exposure aims at the key.
    fn metered_luma(frame: &[[f32; 3]], width: u32, height: u32) -> f32 {
        let hist = histogram_log_luma(frame, width, height).expect("histograms");
        let mean = trimmed_log_mean(&hist, AUTO_TRIM_LO, AUTO_TRIM_HI).expect("means");
        2f32.powf(mean)
    }

    /// The ramp fixture at `ev_offset` stops from the key: the reference
    /// ramp's metered mean is the key itself, so the case sits exactly
    /// that many stops away.
    fn ramp_case(ev_offset: f32) -> (Vec<[f32; 3]>, u32, u32) {
        let mult = 2f32.powf(ev_offset);
        let frame = luma_ramp(32, 32, 0.09 * mult, 0.27 * mult).expect("ramp builds");
        (frame, 32, 32)
    }

    /// The step length the accept needs: the shipped slew is three stops
    /// a second, so thirty steps of a sixth of a second cover fifteen
    /// stops, while thirty sixtieths would cover one and a half and could
    /// never cross the four stops the case names.
    const STEP_DT_S: f32 = 1.0 / 6.0;

    /// The convergence accept: a ramp four stops under and four stops
    /// over the key reaches it inside thirty steps and holds it for the
    /// next ninety. The meter reads the pre-exposure scene, the way the
    /// device path does: the histogram runs over the HDR target before
    /// the exposure lands, and the frame the reader sees is the scene
    /// scaled by the adapted stops.
    #[test]
    fn auto_exposure_ramp_reaches_the_key_within_thirty_steps_and_holds() {
        for offset in [-4.0f32, 4.0f32] {
            let (scene, width, height) = ramp_case(offset);
            let mut state = AutoExposure::default();
            let mut reached: Option<usize> = None;
            let mut worst_hold = 0.0f32;
            let mut metered = 0.0f32;
            for step in 0..(30 + 90) {
                state.tick(&scene, width, height, STEP_DT_S).expect("ticks");
                let mult = state.mult();
                let exposed: Vec<[f32; 3]> = scene
                    .iter()
                    .map(|p| [p[0] * mult, p[1] * mult, p[2] * mult])
                    .collect();
                metered = metered_luma(&exposed, width, height);
                let error = (metered - AUTO_KEY_LUMA).abs();
                if error <= 0.02 && reached.is_none() {
                    reached = Some(step);
                }
                if let Some(first) = reached {
                    if step > first && step <= first + 90 {
                        worst_hold = worst_hold.max(error);
                    }
                }
            }
            let first = reached.unwrap_or(usize::MAX);
            eprintln!(
                "ramp {offset:+.1} EV: key reached at step {first}, metered {metered:.5}, plain {:.5}, worst hold error {worst_hold:.5}",
                mean_linear_luma(&scene) * state.mult()
            );
            assert!(
                first < 30,
                "ramp {offset:+.1} EV misses the key in thirty steps (reached {first})"
            );
            assert!(
                worst_hold <= 0.01,
                "ramp {offset:+.1} EV drifts {worst_hold} off the key after reaching it"
            );
        }
    }

    /// The night capture accept: a night arena lit by 250 torches clips
    /// its torch cores under a fixed exposure and stays clean under the
    /// adapted one, with the frame aimed at the key.
    #[test]
    fn auto_exposure_night_arena_torches_do_not_clip() {
        let arena = NightArena::with_torches(250).expect("arena builds");
        assert_eq!(arena.torch_pixels, 250, "every torch sits on the frame");
        let chain = PostChain::default();
        let fixed = run_chain(&arena.frame, arena.width, arena.height, &chain, &lights())
            .expect("chain runs");
        let fixed_clip = clipped_share(&fixed.ldr);
        assert!(
            fixed_clip > 0.0,
            "the fixed exposure must clip the torch cores of the night arena"
        );
        let mut state = AutoExposure::default();
        for _ in 0..120 {
            state
                .tick(&arena.frame, arena.width, arena.height, 1.0 / 60.0)
                .expect("ticks");
        }
        let ev = state.ev();
        let adapted = chain.with_exposure_ev(ev).expect("adapts");
        let auto = run_chain(&arena.frame, arena.width, arena.height, &adapted, &lights())
            .expect("adapted chain runs");
        let auto_clip = clipped_share(&auto.ldr);
        let metered = metered_luma(&arena.exposed(state.mult()), arena.width, arena.height);
        eprintln!(
            "night arena: fixed clip {fixed_clip:.5}, adapted clip {auto_clip:.5} at {ev:.3} stops, metered {metered:.5}"
        );
        assert_eq!(
            auto_clip, 0.0,
            "the adapted night capture still clips {auto_clip} of the frame"
        );
        assert!(
            (metered - AUTO_KEY_LUMA).abs() <= 0.02,
            "the night capture lands at {metered}, not on the key"
        );
    }

    /// The same ramp twice gives the same stops and the same pixels: the
    /// meter never reads a clock.
    #[test]
    fn auto_exposure_two_runs_are_byte_equal() {
        let run = || {
            let (scene, width, height) = ramp_case(-4.0);
            let mut state = AutoExposure::default();
            let mut stops = Vec::new();
            let mut frames = Vec::new();
            for _ in 0..12 {
                stops.push(state.tick(&scene, width, height, STEP_DT_S).expect("ticks"));
                let mult = state.mult();
                frames.push(
                    scene
                        .iter()
                        .map(|p| [p[0] * mult, p[1] * mult, p[2] * mult])
                        .collect::<Vec<[f32; 3]>>(),
                );
            }
            (stops, frames)
        };
        let (stops_a, frames_a) = run();
        let (stops_b, frames_b) = run();
        assert_eq!(stops_a, stops_b, "two runs give the same stops");
        assert_eq!(frames_a, frames_b, "two runs give the same pixels");
    }

    /// The GPU kernel hands over u32 counts: they read back as the same
    /// histogram the CPU oracle wrote, so both sides trim alike.
    #[test]
    fn auto_exposure_gpu_counts_read_back_as_the_same_histogram() {
        let arena = NightArena::with_torches(250).expect("arena builds");
        let cpu = histogram_log_luma(&arena.frame, arena.width, arena.height).expect("histograms");
        let mut counts = [0u32; AUTO_HIST_BINS];
        for (slot, count) in cpu.bins.iter().enumerate() {
            counts[slot] = *count as u32;
        }
        let gpu = ExposureHistogram::from_gpu_counts(&counts).expect("counts read back");
        assert_eq!(gpu.bins, cpu.bins, "the counts read back as the same bins");
        assert_eq!(gpu.texels, cpu.texels, "the same samples were binned");
        let empty = ExposureHistogram::from_gpu_counts(&[0u32; AUTO_HIST_BINS]);
        assert!(
            matches!(empty, Err(PostError::Invalid(_))),
            "an empty count set fails loudly"
        );
    }
}
