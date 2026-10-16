//! reverb: parametric zone tails for stone rooms, tunnel preset first.
//!
//! Owner: audio lane; consumer: the game reverb send. Units: samples for
//! buffers and cursors, ms for delays and tails, Hz for rates. Determinism:
//! same params plus same input slices give bit-identical outputs on one
//! machine (presentation only, never hashed). Errors: typed [`ReverbError`];
//! bad input returns errors, never panics. Budget: 0.4 ms per 480-frame
//! block at 48 kHz (see [`BUDGET_MS`]).

/// Stone tunnel tail length: RT60 in ms.
pub const TUNNEL_RT60_MS: f32 = 1100.0;
/// Stone tunnel pre-delay in ms.
pub const TUNNEL_PRE_DELAY_MS: f32 = 18.0;
/// Stone tunnel damping (dark): one-pole coefficient in the comb loop.
pub const TUNNEL_DAMPING: f32 = 0.65;
/// Stone tunnel wet level applied to the rendered tail.
pub const TUNNEL_WET: f32 = 0.35;
/// Audio-thread budget in ms per [`BUDGET_BLOCK_FRAMES`] frames at 48 kHz,
/// release profile; the timing probe runs under `--perf`, never by default.
pub const BUDGET_MS: f32 = 0.4;
/// Reference block size for the budget note above.
pub const BUDGET_BLOCK_FRAMES: usize = 480;
/// Lowest supported sample rate in Hz.
pub const MIN_SAMPLE_RATE_HZ: u32 = 8_000;
/// Highest supported sample rate in Hz.
pub const MAX_SAMPLE_RATE_HZ: u32 = 192_000;
/// Longest pre-delay the fixed buffers allow, in ms.
pub const MAX_PRE_DELAY_MS: f32 = 120.0;
/// Shortest tail the params accept, in ms.
pub const MIN_RT60_MS: f32 = 50.0;
/// Longest tail the params accept, in ms.
pub const MAX_RT60_MS: f32 = 8_000.0;
/// Silence floor for level math, in dB.
pub const SILENCE_DB: f32 = -120.0;

/// Typed reverb failure. Bad input refuses loudly; the audio thread never
/// panics on caller data.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReverbError {
    /// Sample rate outside [`MIN_SAMPLE_RATE_HZ`]..=[`MAX_SAMPLE_RATE_HZ`].
    BadSampleRate {
        /// Rejected rate in Hz.
        got: u32,
    },
    /// Named param outside its range (or NaN/infinite).
    BadParam {
        /// Static param name (`"pre_delay_ms"`, `"rt60_ms"`, ...).
        name: &'static str,
    },
    /// Input and output slices disagree in length.
    LengthMismatch {
        /// Input length in samples.
        input: usize,
        /// Output length in samples.
        output: usize,
    },
    /// Block had zero frames; there is nothing to render.
    EmptyBlock,
}

impl std::fmt::Display for ReverbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadSampleRate { got } => {
                write!(f, "reverb: bad sample rate {got} Hz")
            }
            Self::BadParam { name } => {
                write!(f, "reverb: bad param {name}")
            }
            Self::LengthMismatch { input, output } => {
                write!(f, "reverb: length mismatch input {input} output {output}")
            }
            Self::EmptyBlock => write!(f, "reverb: empty block"),
        }
    }
}

impl std::error::Error for ReverbError {}

/// Parametric tail settings. All time fields are ms; gains are linear.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbParams {
    /// Sample rate in Hz, [`MIN_SAMPLE_RATE_HZ`]..=[`MAX_SAMPLE_RATE_HZ`].
    pub sample_rate_hz: u32,
    /// Pre-delay in ms, `0.0`..=[`MAX_PRE_DELAY_MS`].
    pub pre_delay_ms: f32,
    /// Tail length in ms, [`MIN_RT60_MS`]..=[`MAX_RT60_MS`].
    pub rt60_ms: f32,
    /// Damping `0.0..=1.0`; higher is darker.
    pub damping: f32,
    /// Wet level `0.0..=1.0` applied to the rendered tail.
    pub wet: f32,
}

impl ReverbParams {
    /// Stone tunnel preset: RT60 ~1.1 s, 18 ms pre-delay, dark damping.
    pub fn tunnel(sample_rate_hz: u32) -> Result<Self, ReverbError> {
        let params = Self {
            sample_rate_hz,
            pre_delay_ms: TUNNEL_PRE_DELAY_MS,
            rt60_ms: TUNNEL_RT60_MS,
            damping: TUNNEL_DAMPING,
            wet: TUNNEL_WET,
        };
        params.validate()?;
        Ok(params)
    }

    /// Check ranges; NaN and infinities refuse as [`ReverbError::BadParam`].
    pub fn validate(&self) -> Result<(), ReverbError> {
        if !(MIN_SAMPLE_RATE_HZ..=MAX_SAMPLE_RATE_HZ).contains(&self.sample_rate_hz) {
            return Err(ReverbError::BadSampleRate {
                got: self.sample_rate_hz,
            });
        }
        if !self.pre_delay_ms.is_finite()
            || self.pre_delay_ms < 0.0
            || self.pre_delay_ms > MAX_PRE_DELAY_MS
        {
            return Err(ReverbError::BadParam {
                name: "pre_delay_ms",
            });
        }
        if !self.rt60_ms.is_finite() || self.rt60_ms < MIN_RT60_MS || self.rt60_ms > MAX_RT60_MS {
            return Err(ReverbError::BadParam { name: "rt60_ms" });
        }
        if !self.damping.is_finite() || self.damping < 0.0 || self.damping > 1.0 {
            return Err(ReverbError::BadParam { name: "damping" });
        }
        if !self.wet.is_finite() || self.wet < 0.0 || self.wet > 1.0 {
            return Err(ReverbError::BadParam { name: "wet" });
        }
        Ok(())
    }
}

/// A zone volume: where the tail applies plus how long it rings. The DSP
/// detail lives in [`ReverbParams`]; this is the placement handle the game
/// thread owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ReverbZone {
    /// Zone center in integer centimeters (x east, y up, z south).
    pub center: [i32; 3],
    /// Tail length in ms (tunnel stone: 1100).
    pub tail_ms: u16,
}

impl ReverbZone {
    /// Gate tunnel zone at `center_cm`.
    pub fn tunnel(center_cm: [i32; 3]) -> Self {
        Self {
            center: center_cm,
            tail_ms: TUNNEL_RT60_MS as u16,
        }
    }

    /// Full DSP params for this zone at `sample_rate_hz`.
    pub fn params(&self, sample_rate_hz: u32) -> Result<ReverbParams, ReverbError> {
        let params = ReverbParams {
            sample_rate_hz,
            pre_delay_ms: TUNNEL_PRE_DELAY_MS,
            rt60_ms: f32::from(self.tail_ms),
            damping: TUNNEL_DAMPING,
            wet: TUNNEL_WET,
        };
        params.validate()?;
        Ok(params)
    }
}

/// Convert `ms` to samples at `sample_rate_hz`, rounded. Refuses a bad rate
/// or a non-finite/negative delay.
pub fn ms_to_samples(ms: f32, sample_rate_hz: u32) -> Result<usize, ReverbError> {
    if !(MIN_SAMPLE_RATE_HZ..=MAX_SAMPLE_RATE_HZ).contains(&sample_rate_hz) {
        return Err(ReverbError::BadSampleRate {
            got: sample_rate_hz,
        });
    }
    if !ms.is_finite() || !(0.0..=MAX_PRE_DELAY_MS + MAX_RT60_MS).contains(&ms) {
        return Err(ReverbError::BadParam { name: "ms" });
    }
    Ok((ms * sample_rate_hz as f32 / 1000.0).round() as usize)
}

/// Feedback gain for a `delay_samples` line ringing `rt60_ms` at
/// `sample_rate_hz`: after one RT60 the loop is at -60 dB by construction.
pub fn rt60_feedback(
    delay_samples: usize,
    rt60_ms: f32,
    sample_rate_hz: u32,
) -> Result<f32, ReverbError> {
    if delay_samples == 0 {
        return Err(ReverbError::BadParam {
            name: "delay_samples",
        });
    }
    if !(MIN_SAMPLE_RATE_HZ..=MAX_SAMPLE_RATE_HZ).contains(&sample_rate_hz) {
        return Err(ReverbError::BadSampleRate {
            got: sample_rate_hz,
        });
    }
    if !rt60_ms.is_finite() || !(MIN_RT60_MS..=MAX_RT60_MS).contains(&rt60_ms) {
        return Err(ReverbError::BadParam { name: "rt60_ms" });
    }
    let rt60_samples = rt60_ms * sample_rate_hz as f32 / 1000.0;
    let gain = 10f32.powf(-3.0 * delay_samples as f32 / rt60_samples);
    Ok(gain.clamp(0.0, 0.98))
}

/// Level of `|x|` in dBFS; silence floors at [`SILENCE_DB`].
pub fn level_db(x: f32) -> f32 {
    let ax = x.abs();
    if ax.is_nan() || ax <= 0.0 {
        return SILENCE_DB;
    }
    (20.0 * ax.log10()).max(SILENCE_DB)
}

/// Decay from `early` to `late` power in dB; non-positive inputs floor at
/// [`SILENCE_DB`].
pub fn decay_db(early_power: f64, late_power: f64) -> f32 {
    if early_power.is_nan() || late_power.is_nan() || early_power <= 0.0 || late_power <= 0.0 {
        return SILENCE_DB;
    }
    (10.0 * (late_power / early_power).log10()) as f32
}

/// Octave band edges for the clap fixture in Hz (88 doubling to 11314).
pub const BAND_EDGES_HZ: [f32; 8] = [88.0, 177.0, 354.0, 707.0, 1414.0, 2828.0, 5656.0, 11_314.0];

/// One-pole lowpass coefficient for `cutoff_hz` at `sample_rate_hz`.
fn onepole_coeff(cutoff_hz: f32, sample_rate_hz: u32) -> f32 {
    let x = 2.0 * std::f32::consts::PI * cutoff_hz / sample_rate_hz as f32;
    1.0 - (-x).exp()
}

/// RMS of the `lo_hz..hi_hz` octave slice of `block`, via a lowpass
/// difference (`LP(hi) - LP(lo)`). Broadband by construction, so comb modes
/// average out instead of landing on single probe tones. Empty or invalid
/// input gives 0.0; fixture math only.
pub fn band_rms(block: &[f32], sample_rate_hz: u32, lo_hz: f32, hi_hz: f32) -> f32 {
    if block.is_empty() || sample_rate_hz == 0 {
        return 0.0;
    }
    if !lo_hz.is_finite() || !hi_hz.is_finite() || lo_hz <= 0.0 || hi_hz <= lo_hz {
        return 0.0;
    }
    let nyquist = sample_rate_hz as f32 / 2.0;
    if hi_hz >= nyquist {
        return 0.0;
    }
    let (a_hi, a_lo) = (
        onepole_coeff(hi_hz, sample_rate_hz),
        onepole_coeff(lo_hz, sample_rate_hz),
    );
    let (mut y_hi, mut y_lo, mut sum) = (0.0f32, 0.0f32, 0.0f64);
    for &s in block {
        y_hi += a_hi * (s - y_hi);
        y_lo += a_lo * (s - y_lo);
        let band = y_hi - y_lo;
        sum += f64::from(band) * f64::from(band);
    }
    (sum / block.len() as f64).sqrt() as f32
}

/// Map NaN to silence and clamp runaway amplitudes so one bad sample never
/// poisons the tail.
fn sanitize(s: f32) -> f32 {
    if s.is_nan() {
        0.0
    } else {
        s.clamp(-4.0, 4.0)
    }
}

fn step_predelay(x: f32, line: &mut [f32], pos: &mut usize) -> f32 {
    debug_assert!(!line.is_empty());
    let p = *pos % line.len();
    let out = line[p];
    line[p] = x;
    *pos = (*pos + 1) % line.len();
    out
}

fn step_comb(
    x: f32,
    buf: &mut [f32],
    pos: &mut usize,
    feedback: f32,
    damping: f32,
    lp: &mut f32,
) -> f32 {
    debug_assert!(!buf.is_empty());
    let p = *pos % buf.len();
    let delayed = buf[p];
    // Dark one-pole in the loop: damping 0 feeds back the raw delay
    // (bright, full ring), damping 1 feeds back only the smoothed state.
    *lp += damping * (delayed - *lp);
    buf[p] = x + (delayed * (1.0 - damping) + *lp * damping) * feedback;
    *pos = (*pos + 1) % buf.len();
    delayed
}

fn step_allpass(x: f32, buf: &mut [f32], pos: &mut usize, feedback: f32) -> f32 {
    debug_assert!(!buf.is_empty());
    let p = *pos % buf.len();
    let delayed = buf[p];
    let out = delayed - feedback * x;
    buf[p] = x + feedback * delayed;
    *pos = (*pos + 1) % buf.len();
    out
}

/// Pure pre-delay over caller slices: `out[i]` is `input[i - delay]` with
/// zeros before the delay elapses. `line` must hold exactly the delay in
/// samples (see [`ms_to_samples`]); `pos` is the caller-owned cursor, start
/// at 0. Allocates nothing.
pub fn apply_predelay(
    input: &[f32],
    out: &mut [f32],
    line: &mut [f32],
    pos: &mut usize,
) -> Result<(), ReverbError> {
    if input.is_empty() {
        return Err(ReverbError::EmptyBlock);
    }
    if input.len() != out.len() {
        return Err(ReverbError::LengthMismatch {
            input: input.len(),
            output: out.len(),
        });
    }
    if line.is_empty() {
        return Err(ReverbError::BadParam { name: "line" });
    }
    for i in 0..input.len() {
        out[i] = step_predelay(sanitize(input[i]), line, pos);
    }
    Ok(())
}

/// Pure comb filter over caller slices: ringing delay line with a dark
/// one-pole (`damping`) inside the feedback loop. `feedback` is `0.0..=0.98`
/// (see [`rt60_feedback`]), `damping` is `0.0..=1.0`, `lp` is the caller
/// filter state starting at 0. Allocates nothing.
pub fn apply_comb(
    input: &[f32],
    out: &mut [f32],
    line: &mut [f32],
    pos: &mut usize,
    feedback: f32,
    damping: f32,
    lp: &mut f32,
) -> Result<(), ReverbError> {
    if input.is_empty() {
        return Err(ReverbError::EmptyBlock);
    }
    if input.len() != out.len() {
        return Err(ReverbError::LengthMismatch {
            input: input.len(),
            output: out.len(),
        });
    }
    if line.is_empty() {
        return Err(ReverbError::BadParam { name: "line" });
    }
    if !feedback.is_finite() || !(0.0..=0.98).contains(&feedback) {
        return Err(ReverbError::BadParam { name: "feedback" });
    }
    if !damping.is_finite() || !(0.0..=1.0).contains(&damping) {
        return Err(ReverbError::BadParam { name: "damping" });
    }
    if !lp.is_finite() {
        return Err(ReverbError::BadParam { name: "lp" });
    }
    for i in 0..input.len() {
        out[i] = step_comb(sanitize(input[i]), line, pos, feedback, damping, lp);
    }
    Ok(())
}

/// Pure allpass filter over caller slices: smears phase, keeps energy.
/// `feedback` is `0.0..=0.98`. Allocates nothing.
pub fn apply_allpass(
    input: &[f32],
    out: &mut [f32],
    line: &mut [f32],
    pos: &mut usize,
    feedback: f32,
) -> Result<(), ReverbError> {
    if input.is_empty() {
        return Err(ReverbError::EmptyBlock);
    }
    if input.len() != out.len() {
        return Err(ReverbError::LengthMismatch {
            input: input.len(),
            output: out.len(),
        });
    }
    if line.is_empty() {
        return Err(ReverbError::BadParam { name: "line" });
    }
    if !feedback.is_finite() || !(0.0..=0.98).contains(&feedback) {
        return Err(ReverbError::BadParam { name: "feedback" });
    }
    for i in 0..input.len() {
        out[i] = step_allpass(sanitize(input[i]), line, pos, feedback);
    }
    Ok(())
}

/// Comb lengths at 44.1 kHz the tunnel scales to the running rate.
const COMB_BASE_44K: [usize; 4] = [1116, 1188, 1277, 1356];
/// Allpass lengths at 44.1 kHz the tunnel scales to the running rate.
const ALLPASS_BASE_44K: [usize; 2] = [225, 556];
/// Right-ear detune in samples so the stereo pair decorrelates.
const RIGHT_DETUNE: usize = 13;
/// Allpass loop gain; energy-preserving, smears the comb modes.
const ALLPASS_FEEDBACK: f32 = 0.5;

fn scaled_delay(base_44k: usize, sample_rate_hz: u32) -> usize {
    ((base_44k as f32 * sample_rate_hz as f32 / 44_100.0).round() as usize).max(16)
}

struct Comb {
    buf: Vec<f32>,
    pos: usize,
    feedback: f32,
    lp: f32,
}

struct Allpass {
    buf: Vec<f32>,
    pos: usize,
}

/// Parametric tail: pre-delay into four parallel dark combs into two series
/// allpasses. Build once on the game thread ([`Reverb::new`] allocates);
/// [`Reverb::process`] then renders wet tails with zero audio-thread allocs.
pub struct Reverb {
    params: ReverbParams,
    pre: Vec<f32>,
    pre_pos: usize,
    combs: [Comb; 4],
    allpasses: [Allpass; 2],
    wet: f32,
}

impl Reverb {
    /// Build delay lines for `params`. Allocates: call on the game thread,
    /// never on the audio thread.
    pub fn new(params: &ReverbParams) -> Result<Self, ReverbError> {
        params.validate()?;
        Self::build(params, 0)
    }

    fn build(params: &ReverbParams, detune: usize) -> Result<Self, ReverbError> {
        // Zero pre-delay bypasses the line so a dry build rings on the
        // first comb tap with no extra sample of latency.
        let pre_len = ms_to_samples(params.pre_delay_ms, params.sample_rate_hz)?;
        let mut combs: [Comb; 4] = [
            Comb {
                buf: Vec::new(),
                pos: 0,
                feedback: 0.0,
                lp: 0.0,
            },
            Comb {
                buf: Vec::new(),
                pos: 0,
                feedback: 0.0,
                lp: 0.0,
            },
            Comb {
                buf: Vec::new(),
                pos: 0,
                feedback: 0.0,
                lp: 0.0,
            },
            Comb {
                buf: Vec::new(),
                pos: 0,
                feedback: 0.0,
                lp: 0.0,
            },
        ];
        for (i, comb) in combs.iter_mut().enumerate() {
            let len = scaled_delay(COMB_BASE_44K[i], params.sample_rate_hz) + detune;
            comb.feedback = rt60_feedback(len, params.rt60_ms, params.sample_rate_hz)?;
            comb.buf = vec![0.0; len];
        }
        let mut allpasses: [Allpass; 2] = [
            Allpass {
                buf: Vec::new(),
                pos: 0,
            },
            Allpass {
                buf: Vec::new(),
                pos: 0,
            },
        ];
        for (i, ap) in allpasses.iter_mut().enumerate() {
            let len = scaled_delay(ALLPASS_BASE_44K[i], params.sample_rate_hz) + detune;
            ap.buf = vec![0.0; len.max(8)];
        }
        Ok(Self {
            params: *params,
            pre: vec![0.0; pre_len],
            pre_pos: 0,
            combs,
            allpasses,
            wet: params.wet,
        })
    }

    /// Active params (copy; the struct owns its lines).
    pub fn params(&self) -> ReverbParams {
        self.params
    }

    /// Pre-delay in samples (exact [`ms_to_samples`] mapping; 0 bypasses).
    pub fn pre_delay_samples(&self) -> usize {
        self.pre.len()
    }

    /// Wet level applied to the tail.
    pub fn wet(&self) -> f32 {
        self.wet
    }

    /// Clear all lines to silence without allocating; call on zone switch.
    pub fn reset(&mut self) {
        for s in self.pre.iter_mut() {
            *s = 0.0;
        }
        self.pre_pos = 0;
        for comb in self.combs.iter_mut() {
            for s in comb.buf.iter_mut() {
                *s = 0.0;
            }
            comb.pos = 0;
            comb.lp = 0.0;
        }
        for ap in self.allpasses.iter_mut() {
            for s in ap.buf.iter_mut() {
                *s = 0.0;
            }
            ap.pos = 0;
        }
    }

    /// Render the wet tail of `input` into `out`. Lengths must match and be
    /// non-empty; NaN samples become silence. Allocates nothing.
    pub fn process(&mut self, input: &[f32], out: &mut [f32]) -> Result<(), ReverbError> {
        if input.is_empty() {
            return Err(ReverbError::EmptyBlock);
        }
        if input.len() != out.len() {
            return Err(ReverbError::LengthMismatch {
                input: input.len(),
                output: out.len(),
            });
        }
        let damping = self.params.damping;
        let bypass = self.pre.is_empty();
        for i in 0..input.len() {
            let x = sanitize(input[i]);
            let pd = if bypass {
                x
            } else {
                step_predelay(x, &mut self.pre, &mut self.pre_pos)
            };
            let mut acc = 0.0f32;
            for comb in self.combs.iter_mut() {
                acc += step_comb(
                    pd,
                    &mut comb.buf,
                    &mut comb.pos,
                    comb.feedback,
                    damping,
                    &mut comb.lp,
                );
            }
            let mut tail = acc * 0.25;
            for ap in self.allpasses.iter_mut() {
                tail = step_allpass(tail, &mut ap.buf, &mut ap.pos, ALLPASS_FEEDBACK);
            }
            out[i] = tail * self.wet;
        }
        Ok(())
    }
}

/// Stereo tail: two detuned [`Reverb`] lines so one mono hit blooms wide.
/// Same threading rule: build on the game thread, process with no allocs.
pub struct StereoReverb {
    left: Reverb,
    right: Reverb,
}

impl StereoReverb {
    /// Build the decorrelated pair. Allocates: game thread only.
    pub fn new(params: &ReverbParams) -> Result<Self, ReverbError> {
        params.validate()?;
        Ok(Self {
            left: Reverb::build(params, 0)?,
            right: Reverb::build(params, RIGHT_DETUNE)?,
        })
    }

    /// Clear both ears without allocating.
    pub fn reset(&mut self) {
        self.left.reset();
        self.right.reset();
    }

    /// Render one mono hit into stereo tails. All three slices share one
    /// length and are non-empty. Allocates nothing.
    pub fn process_mono_to_stereo(
        &mut self,
        input: &[f32],
        left_out: &mut [f32],
        right_out: &mut [f32],
    ) -> Result<(), ReverbError> {
        if input.is_empty() {
            return Err(ReverbError::EmptyBlock);
        }
        if input.len() != left_out.len() || input.len() != right_out.len() {
            return Err(ReverbError::LengthMismatch {
                input: input.len(),
                output: left_out.len().max(right_out.len()),
            });
        }
        self.left.process(input, left_out)?;
        self.right.process(input, right_out)?;
        Ok(())
    }
}

/// Frames per input block when rendering an impulse response.
const IMPULSE_BLOCK: usize = 256;

/// Render a unit impulse through `params` into `out` (setup helper: builds
/// the lines, so game thread or test setup only, never the audio thread).
/// The input is a stack block fed through the stateful lines, so the tail is
/// the same for any block size and `out` is the only buffer.
pub fn render_impulse(params: &ReverbParams, out: &mut [f32]) -> Result<(), ReverbError> {
    if out.is_empty() {
        return Err(ReverbError::EmptyBlock);
    }
    let mut verb = Reverb::new(params)?;
    let mut input = [0.0f32; IMPULSE_BLOCK];
    input[0] = 1.0;
    for chunk in out.chunks_mut(IMPULSE_BLOCK) {
        verb.process(&input[..chunk.len()], chunk)?;
        input[0] = 0.0;
    }
    Ok(())
}

/// Octave centers for the clap fixture, in Hz.
pub const RECORDED_CENTERS_HZ: [f32; 7] = [125.0, 250.0, 500.0, 1_000.0, 2_000.0, 4_000.0, 8_000.0];
/// Recorded gate-tunnel clap decay in dB per octave band, measured from the
/// early window (300-400 ms) to the late window (700-800 ms) on the reference
/// tail. Inline stand-in: a hardware capture replaces these goldens without
/// touching the asserts. Tolerance is [`DECAY_TOLERANCE_DB`].
pub const RECORDED_DECAY_DB: [f32; 7] = [-22.3, -22.6, -23.0, -23.7, -24.9, -26.4, -27.9];
/// Decay match tolerance in dB per band.
pub const DECAY_TOLERANCE_DB: f32 = 3.0;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::rms;
    use engine_core::alloc_probe::measure;

    const SR: u32 = 48_000;

    fn tunnel() -> ReverbParams {
        ReverbParams::tunnel(SR).expect("tunnel preset builds at 48 kHz")
    }

    #[test]
    fn impulse_blocks_match_one_whole_input_pass() {
        let p = tunnel();
        let n = 5 * IMPULSE_BLOCK * 4 + 3;
        let mut verb = Reverb::new(&p).expect("builds");
        let mut input = vec![0.0f32; n];
        input[0] = 1.0;
        let mut whole = vec![0.0f32; n];
        verb.process(&input, &mut whole).expect("one pass");
        let mut blocked = vec![0.0f32; n];
        render_impulse(&p, &mut blocked).expect("blocked");
        assert_eq!(blocked, whole, "block boundaries do not change samples");
        assert!(
            blocked[IMPULSE_BLOCK..].iter().any(|&s| s != 0.0),
            "the tail rings past the first block"
        );
    }

    #[test]
    fn tunnel_preset_has_stone_numbers() {
        let p = tunnel();
        assert_eq!(p.sample_rate_hz, SR);
        assert!(
            (p.pre_delay_ms - 18.0).abs() < 1e-6,
            "pre={}",
            p.pre_delay_ms
        );
        assert!((p.rt60_ms - 1100.0).abs() < 1e-6, "rt60={}", p.rt60_ms);
        assert!((p.damping - TUNNEL_DAMPING).abs() < 1e-6);
        assert!((p.wet - TUNNEL_WET).abs() < 1e-6);
        assert_eq!(TUNNEL_RT60_MS as u16, 1100);
    }

    #[test]
    fn zone_tunnel_maps_to_params() {
        let zone = ReverbZone::tunnel([0, 0, 0]);
        assert_eq!(zone.tail_ms, 1100);
        assert_eq!(zone.center, [0, 0, 0]);
        let p = zone.params(SR).expect("zone params build");
        assert_eq!(p, tunnel());
        assert!(zone.params(0).is_err(), "rate 0 refuses");
    }

    #[test]
    fn ms_to_samples_exact_and_refusals() {
        assert_eq!(ms_to_samples(18.0, 48_000).expect("18 ms"), 864);
        assert_eq!(ms_to_samples(0.0, 48_000).expect("0 ms"), 0);
        assert_eq!(ms_to_samples(1000.0, 8_000).expect("1 s at 8 k"), 8000);
        assert!(ms_to_samples(18.0, 0).is_err());
        assert!(ms_to_samples(f32::NAN, SR).is_err());
        assert!(ms_to_samples(-1.0, SR).is_err());
        assert!(ms_to_samples(f32::INFINITY, SR).is_err());
    }

    #[test]
    fn rt60_feedback_lands_minus60_after_one_rt60() {
        let delay = 1292usize;
        let g = rt60_feedback(delay, 1100.0, SR).expect("feedback builds");
        assert!(g > 0.5 && g < 0.98, "g={g}");
        let rounds = 1100.0 * SR as f32 / 1000.0 / delay as f32;
        let at_rt60 = 20.0 * g.log10() * rounds;
        assert!((at_rt60 + 60.0).abs() < 0.5, "at_rt60={at_rt60}");
        assert!(rt60_feedback(0, 1100.0, SR).is_err());
        assert!(rt60_feedback(delay, 0.0, SR).is_err());
        assert!(rt60_feedback(delay, 1100.0, 0).is_err());
    }

    #[test]
    fn predelay_shifts_impulse_exactly() {
        let delay = ms_to_samples(18.0, SR).expect("864");
        let mut line = vec![0.0; delay];
        let mut pos = 0usize;
        let mut input = vec![0.0; 1000];
        input[0] = 1.0;
        let mut out = vec![9.0; 1000];
        apply_predelay(&input, &mut out, &mut line, &mut pos).expect("shifts");
        assert!(out[..delay].iter().all(|&s| s == 0.0), "silent first");
        assert_eq!(out[delay], 1.0, "impulse lands on sample 864");
        assert!(out[delay + 1..].iter().all(|&s| s == 0.0));
    }

    #[test]
    fn delay_fns_reject_bad_shapes() {
        let mut line = vec![0.0; 8];
        let mut pos = 0usize;
        let input = vec![0.5; 8];
        let mut out = vec![0.0; 8];
        let mut short = vec![0.0; 4];
        assert_eq!(
            apply_predelay(&[], &mut [], &mut line, &mut pos),
            Err(ReverbError::EmptyBlock)
        );
        assert_eq!(
            apply_predelay(&input, &mut short, &mut line, &mut pos),
            Err(ReverbError::LengthMismatch {
                input: 8,
                output: 4
            })
        );
        let mut empty: Vec<f32> = Vec::new();
        assert!(apply_predelay(&input, &mut out, &mut empty, &mut pos).is_err());
        let mut lp = 0.0f32;
        assert!(apply_comb(&input, &mut out, &mut line, &mut pos, 0.5, 0.5, &mut lp).is_ok());
        assert!(apply_comb(&input, &mut out, &mut line, &mut pos, 9.9, 0.5, &mut lp).is_err());
        assert!(apply_comb(&input, &mut out, &mut line, &mut pos, 0.5, 2.0, &mut lp).is_err());
        assert!(apply_allpass(&input, &mut out, &mut line, &mut pos, 0.5).is_ok());
        assert!(apply_allpass(&input, &mut out, &mut line, &mut pos, -0.5).is_err());
    }

    #[test]
    fn params_reject_bad_input() {
        assert!(ReverbParams::tunnel(0).is_err());
        assert!(ReverbParams::tunnel(4000).is_err());
        assert!(ReverbParams::tunnel(300_000).is_err());
        let mut p = tunnel();
        p.rt60_ms = 0.0;
        assert!(p.validate().is_err());
        p = tunnel();
        p.rt60_ms = f32::NAN;
        assert!(p.validate().is_err());
        p = tunnel();
        p.damping = 1.5;
        assert!(p.validate().is_err());
        p = tunnel();
        p.damping = f32::NAN;
        assert!(p.validate().is_err());
        p = tunnel();
        p.wet = 2.0;
        assert!(p.validate().is_err());
        p = tunnel();
        p.pre_delay_ms = 500.0;
        assert!(p.validate().is_err());
        p = tunnel();
        p.pre_delay_ms = f32::NEG_INFINITY;
        assert!(p.validate().is_err());
    }

    #[test]
    fn process_rejects_mismatched_and_empty() {
        let mut verb = Reverb::new(&tunnel()).expect("builds");
        let empty: Vec<f32> = Vec::new();
        let mut out = vec![0.0; 8];
        assert_eq!(
            verb.process(&empty, &mut empty.clone()),
            Err(ReverbError::EmptyBlock)
        );
        let input = vec![0.0; 8];
        let mut short = vec![0.0; 4];
        assert_eq!(
            verb.process(&input, &mut short),
            Err(ReverbError::LengthMismatch {
                input: 8,
                output: 4
            })
        );
        assert!(verb.process(&input, &mut out).is_ok());
    }

    #[test]
    fn impulse_tail_is_silent_before_predelay() {
        let p = tunnel();
        let mut tail = vec![0.0; SR as usize];
        render_impulse(&p, &mut tail).expect("renders");
        let guard = (16 * SR / 1000) as usize;
        assert!(
            tail[..guard].iter().all(|&s| s.abs() < 1e-6),
            "first 16 ms silent (predelay 18 ms minus 2 ms)"
        );
        let late = rms(&tail[2000..8000]);
        assert!(late > 1e-3, "tail blooms after onset, rms={late}");
        assert!(render_impulse(&p, &mut []).is_err());
    }

    #[test]
    fn predelay_is_plumbed() {
        fn onset(params: &ReverbParams) -> usize {
            let mut tail = vec![0.0; 8000];
            render_impulse(params, &mut tail).expect("renders");
            tail.iter()
                .position(|&s| s.abs() > 1e-6)
                .expect("tail starts")
        }
        let mut dry = tunnel();
        dry.pre_delay_ms = 0.0;
        let shift = onset(&tunnel()) as isize - onset(&dry) as isize;
        assert_eq!(shift, 864, "18 ms predelay moves onset by 864 samples");
    }

    #[test]
    fn determinism_same_impulse_same_bytes() {
        let p = tunnel();
        let mut a = vec![0.0; 8000];
        let mut b = vec![0.0; 8000];
        render_impulse(&p, &mut a).expect("a");
        render_impulse(&p, &mut b).expect("b");
        assert_eq!(a, b, "same impulse gives bit-identical tails");
    }

    #[test]
    fn reset_returns_to_silence() {
        let p = tunnel();
        let mut verb = Reverb::new(&p).expect("builds");
        let mut input = vec![0.0; 1024];
        input[0] = 1.0;
        let mut a = vec![0.0; 1024];
        let mut b = vec![0.0; 1024];
        verb.process(&input, &mut a).expect("first");
        verb.reset();
        verb.process(&input, &mut b).expect("second");
        assert_eq!(a, b, "reset restores the silent start");
    }

    #[test]
    fn nan_input_never_poisons_tail() {
        let p = tunnel();
        let mut verb = Reverb::new(&p).expect("builds");
        let mut input = vec![0.0; 2048];
        input[0] = 1.0;
        input[4] = f32::NAN;
        input[8] = f32::INFINITY;
        let mut out = vec![0.0; 2048];
        verb.process(&input, &mut out).expect("renders");
        assert!(out.iter().all(|s| s.is_finite()), "tail stays finite");
    }

    fn decay_between(
        tail: &[f32],
        early: std::ops::Range<usize>,
        late: std::ops::Range<usize>,
        band: usize,
    ) -> f32 {
        let e = band_rms(
            &tail[early],
            SR,
            BAND_EDGES_HZ[band],
            BAND_EDGES_HZ[band + 1],
        );
        let l = band_rms(
            &tail[late],
            SR,
            BAND_EDGES_HZ[band],
            BAND_EDGES_HZ[band + 1],
        );
        decay_db((e * e) as f64, (l * l) as f64)
    }

    #[test]
    fn tail_decays_like_stone_tunnel() {
        let p = tunnel();
        let mut tail = vec![0.0; 57_600];
        render_impulse(&p, &mut tail).expect("renders 1.2 s tail");
        let early = 14_400..19_200;
        let late = 33_600..38_400;
        let full_early = rms(&tail[early.clone()]);
        let full_late = rms(&tail[late.clone()]);
        let full_decay = decay_db(
            (full_early * full_early) as f64,
            (full_late * full_late) as f64,
        );
        assert!(
            (full_decay + 21.82).abs() <= DECAY_TOLERANCE_DB,
            "RT60 ~1.1 s over 0.4 s gap: {full_decay} dB"
        );
        let mut measured = [0.0f32; 7];
        for band in 0..7 {
            let hz = RECORDED_CENTERS_HZ[band];
            measured[band] = decay_between(&tail, early.clone(), late.clone(), band);
            assert!(
                (measured[band] - RECORDED_DECAY_DB[band]).abs() <= DECAY_TOLERANCE_DB,
                "band {hz} Hz: measured {} vs recorded {}",
                measured[band],
                RECORDED_DECAY_DB[band]
            );
        }
    }

    #[test]
    fn damping_is_plumbed_dark() {
        let mut bright = tunnel();
        bright.damping = 0.0;
        let mut dark_tail = vec![0.0; 57_600];
        let mut bright_tail = vec![0.0; 57_600];
        render_impulse(&tunnel(), &mut dark_tail).expect("dark");
        render_impulse(&bright, &mut bright_tail).expect("bright");
        let early = 9_600..14_400;
        let late = 28_800..33_600;
        let dark = decay_between(&dark_tail, early.clone(), late.clone(), 6);
        let bright_d = decay_between(&bright_tail, early, late, 6);
        assert!(
            bright_d - dark > 1.0,
            "dark 8 kHz ({dark} dB) falls faster than undamped ({bright_d} dB)"
        );
    }

    #[test]
    fn stereo_pair_blooms_within_ten_percent() {
        let p = tunnel();
        let mut pair = StereoReverb::new(&p).expect("pair builds");
        let mut input = vec![0.0; 57_600];
        input[0] = 1.0;
        let mut left = vec![0.0; 57_600];
        let mut right = vec![0.0; 57_600];
        pair.process_mono_to_stereo(&input, &mut left, &mut right)
            .expect("stereo renders");
        assert_ne!(left, right, "detune decorrelates the ears");
        let (el, er) = (rms(&left[2000..8000]), rms(&right[2000..8000]));
        let width = (el - er).abs() / el.max(er).max(1e-9);
        assert!(width <= 0.10, "stereo width {width}");
        assert!(pair.process_mono_to_stereo(&[], &mut [], &mut []).is_err());
        let mut short = vec![0.0; 4];
        assert!(pair
            .process_mono_to_stereo(&input, &mut short, &mut right)
            .is_err());
    }

    #[test]
    fn level_helpers_floor_at_silence() {
        assert_eq!(level_db(0.0), SILENCE_DB);
        assert_eq!(level_db(f32::NAN), SILENCE_DB);
        assert!((level_db(1.0)).abs() < 1e-6);
        assert_eq!(decay_db(0.0, 1.0), SILENCE_DB);
        assert_eq!(rms(&[]), 0.0);
        assert_eq!(band_rms(&[], SR, 88.0, 177.0), 0.0);
        assert_eq!(band_rms(&[0.5; 8], SR, 500.0, 100.0), 0.0);
    }

    #[test]
    fn steady_state_process_allocates_nothing() {
        let p = tunnel();
        let mut verb = Reverb::new(&p).expect("builds outside the window");
        let mut pair = StereoReverb::new(&p).expect("pair builds outside");
        let input = vec![0.25; BUDGET_BLOCK_FRAMES];
        let mut out = vec![0.0; BUDGET_BLOCK_FRAMES];
        let mut left = vec![0.0; BUDGET_BLOCK_FRAMES];
        let mut right = vec![0.0; BUDGET_BLOCK_FRAMES];
        verb.process(&input, &mut out).expect("warm-up frame");
        pair.process_mono_to_stereo(&input, &mut left, &mut right)
            .expect("warm-up stereo");
        let (folded, heap) = measure(|| {
            let mut folded = 0u64;
            for _ in 0..600usize {
                verb.process(&input, &mut out).expect("frame");
                pair.process_mono_to_stereo(&input, &mut left, &mut right)
                    .expect("stereo frame");
                for &s in &out {
                    folded ^= s.to_bits() as u64;
                }
            }
            folded
        });
        assert_eq!(heap.calls, 0, "steady-state reverb must not allocate");
        assert!(folded != 0, "sanity: tails advanced during the window");
    }

    #[test]
    #[ignore = "timing: budget probe, run under --perf in release"]
    fn budget_probe_480_frames() {
        let p = tunnel();
        let mut verb = Reverb::new(&p).expect("builds");
        let input = vec![0.25; BUDGET_BLOCK_FRAMES];
        let mut out = vec![0.0; BUDGET_BLOCK_FRAMES];
        let start = std::time::Instant::now();
        let rounds = 200u32;
        for _ in 0..rounds {
            verb.process(&input, &mut out).expect("frame");
        }
        let mean_ms = start.elapsed().as_secs_f64() * 1000.0 / f64::from(rounds);
        assert!(
            mean_ms < BUDGET_MS as f64,
            "mean {mean_ms} ms per 480-frame block over budget"
        );
    }
}
