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
pub(super) fn hist_stride(width: u32, height: u32) -> u32 {
    (width.min(height) / 16).clamp(1, AUTO_HIST_STRIDE_MAX)
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
    pub fn tick_hist(
        &mut self,
        hist: &ExposureHistogram,
        dt_s: f32,
    ) -> Result<f32, PostError> {
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
