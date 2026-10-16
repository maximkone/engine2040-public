//! 3-band crossover occlusion + soft-clip (integration).
//!
//! Attribution: inspired by `zem-invictus/bevy_sonus` (MIT, 8 stars): an
//! `OcclusionChain` with a 500 Hz / 4 kHz crossover and a soft clipper.
//! Fresh Rust write against our own dsp API (`engine_audio::dsp::db_to_linear`
//! for gains, `engine_audio::dsp::rms` for level probes); no donor code is
//! copied. Owner: audio lane; consumer: the game arena mix (wall-muffled
//! voices feed per-band gains, the master bus feeds the clipper). Units: Hz
//! for cutoffs, dB for gains, f32 samples nominally in `[-1, 1]`.
//! Determinism: same samples in give bit-identical samples out (pinned).
//! Cost: O(n) per buffer with two f32s of filter state; test-only code.

use engine_audio::dsp::{db_to_linear, rms};
use std::f32::consts::TAU;

/// Sample rate every probe runs at.
const SAMPLE_RATE: u32 = 48_000;
/// Probe length in frames (0.1 s: filter transients settle in ~100 frames).
const N_FRAMES: usize = 4_800;
/// Transient skip before measuring: steady state only.
const SKIP: usize = 1_000;
/// Low/high crossover cutoffs (Hz): lows pass under 500, highs over 4000.
const XOVER_LOW_HZ: f32 = 500.0;
/// High crossover cutoff (Hz).
const XOVER_HIGH_HZ: f32 = 4_000.0;
/// Occluded wall-muffle preset: lows pass, mids -12 dB, highs -18 dB, so the
/// high band always clears the gate below with 6 dB of margin.
const OCCLUDED_LOW_DB: f32 = 0.0;
/// Occluded mid-band gain (dB).
const OCCLUDED_MID_DB: f32 = -12.0;
/// Occluded high-band gain (dB).
const OCCLUDED_HIGH_DB: f32 = -18.0;
/// Gate under test: an occluded high band sits at or below -12 dB.
const OCCLUDED_HIGH_MAX_DB: f32 = -12.0;
/// Low band through the same wall must lose less than 3 dB (selectivity).
const LOW_MAX_LOSS_DB: f32 = -3.0;

/// Soft-clip one sample: tanh knee, transparent near zero, bounded to
/// `(-1, 1)` for hot signals; non-finite input becomes silence. Never panics.
fn soft_clip(x: f32) -> f32 {
    if !x.is_finite() {
        return 0.0;
    }
    x.tanh()
}

/// One-pole lowpass (bake helper for the chain below, deterministic).
struct LowPass {
    a: f32,
    y: f32,
}

impl LowPass {
    fn new(cutoff_hz: f32, sample_rate: u32) -> Self {
        let a = if sample_rate == 0 {
            0.0
        } else {
            let dt = 1.0 / sample_rate as f32;
            let rc = 1.0 / (TAU * cutoff_hz);
            dt / (rc + dt)
        };
        Self { a, y: 0.0 }
    }

    fn step(&mut self, x: f32) -> f32 {
        self.y += self.a * (x - self.y);
        self.y
    }
}

/// One-pole highpass (bake helper for the chain below, deterministic).
struct HighPass {
    a: f32,
    y: f32,
    px: f32,
}

impl HighPass {
    fn new(cutoff_hz: f32, sample_rate: u32) -> Self {
        let a = if sample_rate == 0 {
            0.0
        } else {
            let dt = 1.0 / sample_rate as f32;
            let rc = 1.0 / (TAU * cutoff_hz);
            rc / (rc + dt)
        };
        Self { a, y: 0.0, px: 0.0 }
    }

    fn step(&mut self, x: f32) -> f32 {
        self.y = self.a * (self.y + x - self.px);
        self.px = x;
        self.y
    }
}

/// 3-band occlusion chain: lowpass at 500 Hz, highpass at 4 kHz, the mid
/// band is the residual, per-band gains, then [`soft_clip`]. Bypass skips
/// filters and clipper and returns the input bit-exact. Fresh state per
/// chain; processing the same input twice gives identical output.
struct OcclusionChain {
    low_gain: f32,
    mid_gain: f32,
    high_gain: f32,
    bypass: bool,
    lp: LowPass,
    hp: HighPass,
}

impl OcclusionChain {
    fn new(sample_rate: u32, low_gain: f32, mid_gain: f32, high_gain: f32, bypass: bool) -> Self {
        Self {
            low_gain,
            mid_gain,
            high_gain,
            bypass,
            lp: LowPass::new(XOVER_LOW_HZ, sample_rate),
            hp: HighPass::new(XOVER_HIGH_HZ, sample_rate),
        }
    }

    /// Wall-muffled preset: lows pass, mids -12 dB, highs -18 dB.
    fn occluded(sample_rate: u32) -> Self {
        Self::new(
            sample_rate,
            db_to_linear(OCCLUDED_LOW_DB),
            db_to_linear(OCCLUDED_MID_DB),
            db_to_linear(OCCLUDED_HIGH_DB),
            false,
        )
    }

    /// Unity gains through filters plus clipper (transparency probe).
    fn open(sample_rate: u32) -> Self {
        Self::new(sample_rate, 1.0, 1.0, 1.0, false)
    }

    /// Bypass: no filtering, no clipping, bit-exact copy.
    fn bypassed(sample_rate: u32) -> Self {
        Self::new(sample_rate, 1.0, 1.0, 1.0, true)
    }

    /// Split into (low, mid, high) bands; mid is the residual so the three
    /// sum to the input up to float rounding.
    fn split(&mut self, input: &[f32]) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
        let mut lo = Vec::with_capacity(input.len());
        let mut mid = Vec::with_capacity(input.len());
        let mut hi = Vec::with_capacity(input.len());
        for &x in input {
            let l = self.lp.step(x);
            let h = self.hp.step(x);
            let m = x - l - h;
            lo.push(l);
            mid.push(m);
            hi.push(h);
        }
        (lo, mid, hi)
    }

    /// Run the full chain: split, per-band gains, sum, soft-clip. Bypass
    /// returns the input bit-exact. Never panics.
    fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if self.bypass {
            return input.to_vec();
        }
        let mut out = Vec::with_capacity(input.len());
        for &x in input {
            let l = self.lp.step(x);
            let h = self.hp.step(x);
            let m = x - l - h;
            out.push(soft_clip(
                l * self.low_gain + m * self.mid_gain + h * self.high_gain,
            ));
        }
        out
    }
}

/// Level ratio in dB between two RMS values (both must be positive).
fn ratio_db(out_rms: f32, in_rms: f32) -> f32 {
    20.0 * (out_rms / in_rms).log10()
}

/// Deterministic 100 Hz sine at `amp` (10 whole cycles in `N_FRAMES`).
fn sine_100hz(amp: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| {
            let t = i as f32 / SAMPLE_RATE as f32;
            (TAU * 100.0 * t).sin() * amp
        })
        .collect()
}

/// Deterministic Nyquist probe: alternates `+-amp`, exact in f32, lives
/// almost entirely in the high band (24 kHz at 48 kHz).
fn nyquist(amp: f32, n: usize) -> Vec<f32> {
    (0..n)
        .map(|i| if i % 2 == 0 { amp } else { -amp })
        .collect()
}

/// Steady-state RMS over the tail (past the filter transient).
fn tail_rms(samples: &[f32]) -> f32 {
    rms(&samples[SKIP..])
}

#[test]
fn high_band_occluded_at_least_12db_end_to_end() {
    // Nyquist probe lives in the high band; the occluded chain must knock
    // it down by at least 12 dB. A flat broadband mute would pass this too,
    // so `low_band_passes_through_wall` pins the selectivity separately.
    let probe = nyquist(0.5, N_FRAMES);
    let in_rms = tail_rms(&probe);
    let mut chain = OcclusionChain::occluded(SAMPLE_RATE);
    let out = chain.process(&probe);
    assert_eq!(out.len(), probe.len());
    let loss_db = ratio_db(tail_rms(&out), in_rms);
    assert!(
        loss_db <= OCCLUDED_HIGH_MAX_DB,
        "high band loss {loss_db:.2} dB must clear {OCCLUDED_HIGH_MAX_DB} dB"
    );
    // The wall muffles: occluded output is far below the bypass copy.
    let mut bypass = OcclusionChain::bypassed(SAMPLE_RATE);
    let ref_out = bypass.process(&probe);
    assert_eq!(ref_out, probe, "bypass is the 0 dB reference");
    assert!(
        tail_rms(&out) < tail_rms(&ref_out),
        "occlusion must muffle, not boost"
    );
}

#[test]
fn high_band_gain_path_pins_margin() {
    // Direct pin: the chain high gain is -18 dB, 6 dB past the gate, so
    // float rounding near the filter skirts cannot flake the end-to-end
    // test. Would fail if the preset ever drifted up to the gate.
    let hg = db_to_linear(OCCLUDED_HIGH_DB);
    let measured_db = 20.0 * hg.log10();
    assert!(
        (measured_db - OCCLUDED_HIGH_DB).abs() < 0.5,
        "high gain {measured_db:.2} dB must match the preset"
    );
    assert!(
        measured_db <= OCCLUDED_HIGH_MAX_DB,
        "high gain {measured_db:.2} dB must clear {OCCLUDED_HIGH_MAX_DB} dB"
    );
    // Wiring pin: the Nyquist probe really does live in the high band, so
    // the end-to-end test measures the high path, not leakage.
    let mut chain = OcclusionChain::occluded(SAMPLE_RATE);
    let probe = nyquist(0.5, N_FRAMES);
    let (lo, _, hi) = chain.split(&probe);
    let (lo_rms, hi_rms) = (tail_rms(&lo), tail_rms(&hi));
    assert!(
        hi_rms > lo_rms * 2.0,
        "probe must sit in the high band (hi {hi_rms:.4}, lo {lo_rms:.4})"
    );
}

#[test]
fn low_band_passes_through_wall() {
    // 100 Hz sine lives in the low band (gain 0 dB): the same occluded
    // chain that buries highs must keep lows within 3 dB. Catches swapped
    // band gains (a low gain of -18 dB would read about -18 here).
    let probe = sine_100hz(0.5, N_FRAMES);
    let in_rms = tail_rms(&probe);
    let mut chain = OcclusionChain::occluded(SAMPLE_RATE);
    let out = chain.process(&probe);
    let loss_db = ratio_db(tail_rms(&out), in_rms);
    assert!(
        loss_db >= LOW_MAX_LOSS_DB,
        "low band loss {loss_db:.2} dB must stay above {LOW_MAX_LOSS_DB} dB"
    );
}

#[test]
fn unity_chain_stays_transparent_within_1db() {
    // Open chain (unity gains): only the soft-clip knee touches normal
    // levels, so a mixed probe stays within 1 dB. Catches wrong defaults.
    let mut probe = sine_100hz(0.4, N_FRAMES);
    let fast = nyquist(0.1, N_FRAMES);
    for (s, f) in probe.iter_mut().zip(fast.iter()) {
        *s += f;
    }
    let in_rms = tail_rms(&probe);
    let mut chain = OcclusionChain::open(SAMPLE_RATE);
    let out = chain.process(&probe);
    let delta_db = ratio_db(tail_rms(&out), in_rms).abs();
    assert!(
        delta_db <= 1.0,
        "unity chain moved {delta_db:.2} dB, want <= 1 dB"
    );
}

#[test]
fn bypass_is_bit_exact_and_ignores_gains() {
    // Bypass returns the input bit-exact, twice in a row, even with gains
    // that would silence the signal if they leaked through.
    let probe = sine_100hz(0.5, N_FRAMES);
    let mut bypass = OcclusionChain::bypassed(SAMPLE_RATE);
    let (first, second) = (bypass.process(&probe), bypass.process(&probe));
    assert_eq!(first, probe, "bypass must copy the input exactly");
    assert_eq!(second, probe, "bypass must stay exact on repeat");
    let mut sneaky = OcclusionChain::new(SAMPLE_RATE, 0.0, 0.0, 0.0, true);
    assert_eq!(
        sneaky.process(&probe),
        probe,
        "bypass must ignore gains entirely"
    );
    // Non-bypass with zero gains goes silent instead (control case).
    let mut muted = OcclusionChain::new(SAMPLE_RATE, 0.0, 0.0, 0.0, false);
    let silent = muted.process(&probe);
    assert!(
        silent.iter().all(|&s| s == 0.0),
        "zero gains with no bypass must silence"
    );
}

#[test]
fn occluded_runs_are_deterministic() {
    // Same input through two fresh chains gives bit-identical output:
    // no hidden state, no wall clock, no allocation-order effects.
    let mut probe = sine_100hz(0.4, N_FRAMES);
    let fast = nyquist(0.1, N_FRAMES);
    for (s, f) in probe.iter_mut().zip(fast.iter()) {
        *s += f;
    }
    let a = OcclusionChain::occluded(SAMPLE_RATE).process(&probe);
    let b = OcclusionChain::occluded(SAMPLE_RATE).process(&probe);
    assert_eq!(a, b, "same input must give identical samples");
}

#[test]
fn soft_clip_knee_and_bounds() {
    // Transparent near zero (would fail on a hard quantizer), strictly
    // below 1.0 past the knee (fails on a hard clipper, which parks at
    // exactly 1.0), bounded and odd-symmetric, non-finite becomes silence.
    assert_eq!(soft_clip(0.0), 0.0);
    assert!(
        (soft_clip(0.25) - 0.25).abs() < 0.01,
        "small signals pass nearly untouched"
    );
    let hot = soft_clip(1.5);
    assert!(
        hot < 1.0 && hot > 0.8,
        "soft knee compresses past 1 without parking (got {hot})"
    );
    assert!(
        soft_clip(2.0) > soft_clip(1.0) && soft_clip(1.0) > soft_clip(0.5),
        "clipper stays monotonic"
    );
    assert!(soft_clip(3.0).abs() <= 1.0, "hot signals stay bounded");
    assert_eq!(soft_clip(-1.5), -soft_clip(1.5), "clipper is odd-symmetric");
    assert_eq!(soft_clip(f32::NAN), 0.0, "NaN becomes silence");
    assert!(
        soft_clip(f32::INFINITY).abs() <= 1.0,
        "infinity stays bounded"
    );
}

#[test]
fn crossover_cutoffs_are_placed() {
    // Pins 500 Hz / 4 kHz against a swap: the lowpass keeps 100 Hz and
    // kills Nyquist, the highpass keeps Nyquist and kills 100 Hz. A swapped
    // pair (lowpass 4k, highpass 500) would leak ~0.2 through both kills.
    let low_tone = sine_100hz(0.5, N_FRAMES);
    let fast = nyquist(0.5, N_FRAMES);
    let mut lp = LowPass::new(XOVER_LOW_HZ, SAMPLE_RATE);
    let mut hp = HighPass::new(XOVER_HIGH_HZ, SAMPLE_RATE);
    let lp_keep: Vec<f32> = low_tone.iter().map(|&x| lp.step(x)).collect();
    let mut lp2 = LowPass::new(XOVER_LOW_HZ, SAMPLE_RATE);
    let lp_kill: Vec<f32> = fast.iter().map(|&x| lp2.step(x)).collect();
    let mut hp2 = HighPass::new(XOVER_HIGH_HZ, SAMPLE_RATE);
    let hp_kill: Vec<f32> = low_tone.iter().map(|&x| hp2.step(x)).collect();
    let hp_keep: Vec<f32> = fast.iter().map(|&x| hp.step(x)).collect();
    assert!(
        tail_rms(&lp_keep) >= 0.9 * tail_rms(&low_tone),
        "lowpass must keep 100 Hz"
    );
    assert!(
        tail_rms(&lp_kill) <= 0.1 * tail_rms(&fast),
        "lowpass must kill Nyquist"
    );
    assert!(
        tail_rms(&hp_kill) <= 0.1 * tail_rms(&low_tone),
        "highpass must kill 100 Hz"
    );
    assert!(
        tail_rms(&hp_keep) >= 0.5 * tail_rms(&fast),
        "highpass must keep Nyquist"
    );
}

#[test]
fn empty_and_degenerate_inputs_stay_silent() {
    // Empty in, empty out on every path; a zero rate never panics and
    // stays bounded (failure case: divide-by-zero in the coefficients).
    let empty: Vec<f32> = Vec::new();
    assert!(OcclusionChain::occluded(SAMPLE_RATE)
        .process(&empty)
        .is_empty());
    assert!(OcclusionChain::bypassed(SAMPLE_RATE)
        .process(&empty)
        .is_empty());
    let (lo, mid, hi) = OcclusionChain::occluded(SAMPLE_RATE).split(&empty);
    assert!(lo.is_empty() && mid.is_empty() && hi.is_empty());
    let probe = sine_100hz(0.5, 64);
    let out = OcclusionChain::occluded(0).process(&probe);
    assert_eq!(out.len(), probe.len(), "rate 0 keeps the frame count");
    assert!(
        out.iter().all(|s| s.is_finite() && s.abs() <= 1.0),
        "rate 0 stays bounded"
    );
}
