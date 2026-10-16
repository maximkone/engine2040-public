//! hrtf: headphone 3D ears over mono sources (std-only HRIR sphere; a native
//! C++ path would add a dependency plus a toolchain).
//!
//! Owner: audio lane; consumer: headphone 3D mix behind the `device` wall
//! (device pulls stereo blocks through [`HrtfProcessor::process_into`]).
//! Units: azimuth degrees clockwise from straight ahead, octants `0..8`
//! (0 front, 2 right, 4 back, 6 left), samples for delays. Determinism:
//! same octant plus same input slices give bit-identical stereo on one
//! machine (presentation only, never hashed; math is `+ - * /` only).
//! Errors: typed [`HrtfError`]; bad input returns errors, never panics.
//! Budget: 32 MAC per frame, ~0.6 KB fixed state, zero audio-thread allocs
//! (counting-allocator test). Speakers mode is a bit-exact passthrough: the
//! speakers path is unchanged by construction.

/// Taps per ear in the HRIR pair convolved on the audio thread.
pub const HRTF_TAPS: usize = 16;
/// Mono history length in samples; covers worst-case ITD plus taps at 96 kHz.
pub const HIST_LEN: usize = 96;
/// Interaural delay span at 48 kHz in samples (head-width sphere model).
pub const ITD_SPAN_48K: usize = 30;
/// Clamp for scaled ITD in samples (`HIST_LEN - HRTF_TAPS - 1`).
pub const MAX_ITD_SAMPLES: usize = HIST_LEN - HRTF_TAPS - 1;
/// Lowest supported sample rate in Hz.
pub const MIN_SAMPLE_RATE_HZ: u32 = 8_000;
/// Highest supported sample rate in Hz.
pub const MAX_SAMPLE_RATE_HZ: u32 = 96_000;
/// Octant count around the head (45-degree blind-orbit steps).
pub const OCTANTS: usize = 8;

/// Sine of `k * 45` degrees, table-driven so the hot path needs no trig.
const OCT_SIN: [f32; OCTANTS] = [
    0.0,
    std::f32::consts::FRAC_1_SQRT_2,
    1.0,
    std::f32::consts::FRAC_1_SQRT_2,
    0.0,
    -std::f32::consts::FRAC_1_SQRT_2,
    -1.0,
    -std::f32::consts::FRAC_1_SQRT_2,
];
/// Cosine of `k * 45` degrees, table-driven so the hot path needs no trig.
const OCT_COS: [f32; OCTANTS] = [
    1.0,
    std::f32::consts::FRAC_1_SQRT_2,
    0.0,
    -std::f32::consts::FRAC_1_SQRT_2,
    -1.0,
    -std::f32::consts::FRAC_1_SQRT_2,
    0.0,
    std::f32::consts::FRAC_1_SQRT_2,
];

/// Output path: headphones get the HRIR pair, speakers stay dual-mono.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HrtfMode {
    /// Headphone 3D mix: ITD plus ILD plus pinna spectrum per octant.
    Headphones,
    /// Speakers: bit-exact passthrough, the legacy path unchanged.
    #[default]
    Speakers,
}

/// Typed HRTF failure. Bad input refuses loudly; the audio thread never
/// panics on caller data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HrtfError {
    /// Sample rate outside [`MIN_SAMPLE_RATE_HZ`]..=[`MAX_SAMPLE_RATE_HZ`].
    BadSampleRate {
        /// Rejected rate in Hz.
        got: u32,
    },
    /// Octant outside `0..8` (use [`octant_for`] to quantize an azimuth).
    BadOctant {
        /// Rejected octant.
        got: u8,
    },
    /// Input and output slices disagree in length.
    LengthMismatch {
        /// Input length in samples (mono frames).
        input: usize,
        /// Offending output length in samples.
        output: usize,
    },
}

impl std::fmt::Display for HrtfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadSampleRate { got } => write!(f, "hrtf: bad sample rate {got} Hz"),
            Self::BadOctant { got } => write!(f, "hrtf: bad octant {got} (want 0..8)"),
            Self::LengthMismatch { input, output } => {
                write!(f, "hrtf: length mismatch (input {input}, output {output})")
            }
        }
    }
}

impl std::error::Error for HrtfError {}

/// Quantize an azimuth in degrees (clockwise from straight ahead) to an
/// octant `0..8` (0 front, 2 right, 4 back, 6 left). Wraps past 360 and
/// below zero; never panics, even for `i32::MIN`.
pub fn octant_for(azimuth_deg: i32) -> u8 {
    u8::try_from(
        (azimuth_deg.rem_euclid(360) + 22)
            .div_euclid(45)
            .rem_euclid(8),
    )
    .unwrap_or(0)
}

/// Center azimuth of an octant in degrees, or `None` when `octant >= 8`.
pub fn azimuth_for(octant: u8) -> Option<i32> {
    if octant < u8::try_from(OCTANTS).unwrap_or(8) {
        Some(i32::from(octant) * 45)
    } else {
        None
    }
}

/// Sanitize one mono sample: NaN becomes silence, infinities clamp to the
/// rails, everything else clamps to `[-1, 1]`. Never panics.
fn sanitize(mono: f32) -> f32 {
    if mono.is_nan() {
        0.0
    } else {
        mono.clamp(-1.0, 1.0)
    }
}

/// Build one ear FIR from its base gain, hemisphere brightness and shadow:
/// `t1` carries the pinna/ILD tilt (negative lifts highs for frontal ears,
/// positive darkens shadowed ears), `t3` carries the rear notch, the tail
/// decays geometrically. Uses `+ - *` only, so every platform agrees.
fn ear_fir(gain: f32, frontness: f32, backness: f32, shadow: f32) -> [f32; HRTF_TAPS] {
    let cut = -0.26 * frontness + 0.30 * backness + 0.34 * shadow;
    let notch = 0.22 * backness;
    let mut taps = [0.0f32; HRTF_TAPS];
    taps[0] = gain;
    taps[1] = gain * cut;
    taps[2] = gain * 0.08;
    taps[3] = -gain * notch;
    let mut tail = taps[3] * 0.5;
    let mut i = 4;
    while i < HRTF_TAPS {
        taps[i] = tail;
        tail *= 0.5;
        i += 1;
    }
    taps
}

/// Streaming headphone 3D renderer: one mono history plus an HRIR pair per
/// octant, convolved sample by sample. All state is fixed-size; the process
/// path allocates nothing, takes no lock and never panics. Game thread owns
/// octant/mode switches, audio thread owns the sample loop.
pub struct HrtfProcessor {
    sample_rate: u32,
    octant: u8,
    mode: HrtfMode,
    hrir_l: [f32; HRTF_TAPS],
    hrir_r: [f32; HRTF_TAPS],
    itd_l: usize,
    itd_r: usize,
    hist: [f32; HIST_LEN],
    pos: usize,
}

impl HrtfProcessor {
    /// New processor at `sample_rate` Hz, octant 0 (front), [`HrtfMode::Speakers`]
    /// so a fresh processor leaves the legacy mix untouched until the
    /// headphone path opts in. Refuses out-of-range rates; never panics.
    pub fn new(sample_rate: u32) -> Result<Self, HrtfError> {
        if !(MIN_SAMPLE_RATE_HZ..=MAX_SAMPLE_RATE_HZ).contains(&sample_rate) {
            return Err(HrtfError::BadSampleRate { got: sample_rate });
        }
        let mut proc = Self {
            sample_rate,
            octant: 0,
            mode: HrtfMode::Speakers,
            hrir_l: [0.0; HRTF_TAPS],
            hrir_r: [0.0; HRTF_TAPS],
            itd_l: 0,
            itd_r: 0,
            hist: [0.0; HIST_LEN],
            pos: 0,
        };
        proc.rebuild();
        Ok(proc)
    }

    /// Recompute gains, delays and FIRs from the current octant and rate,
    /// then clear the history so a reused processor renders the new octant
    /// without stale smear. No heap, no failure modes.
    fn rebuild(&mut self) {
        debug_assert!(self.octant < u8::try_from(OCTANTS).unwrap_or(8));
        let k = usize::from(self.octant);
        let sin = OCT_SIN[k];
        let cos = OCT_COS[k];
        let frontness = cos.max(0.0);
        let backness = (-cos).max(0.0);
        // Contra-ear shadow: a source on the right darkens the left ear.
        let shadow_l = sin.max(0.0);
        let shadow_r = (-sin).max(0.0);
        let scale = if cos >= 0.5 {
            1.0
        } else if cos <= -0.5 {
            0.9
        } else {
            0.95
        };
        let gain_l = scale * (1.0 - 0.45 * shadow_l);
        let gain_r = scale * (1.0 - 0.45 * shadow_r);
        self.hrir_l = ear_fir(gain_l, frontness, backness, shadow_l);
        self.hrir_r = ear_fir(gain_r, frontness, backness, shadow_r);
        // ITD: the contra ear waits, the ipsi ear fires now. Front/back
        // pairs share delays (the cone of confusion); the pinna spectrum
        // above is what tells them apart.
        let span = ITD_SPAN_48K as f32;
        let (delay_l_48k, delay_r_48k) = if sin >= 0.0 {
            ((span * sin + 0.5) as usize, 0)
        } else {
            (0, (span * (-sin) + 0.5) as usize)
        };
        self.itd_l = Self::scale_delay(self.sample_rate, delay_l_48k);
        self.itd_r = Self::scale_delay(self.sample_rate, delay_r_48k);
        self.hist = [0.0; HIST_LEN];
        self.pos = 0;
    }

    /// Scale a 48 kHz delay to the processor rate with rounding, clamped to
    /// [`MAX_ITD_SAMPLES`]. Pure integer math; never panics.
    fn scale_delay(sample_rate: u32, delay_48k: usize) -> usize {
        ((delay_48k as u64 * u64::from(sample_rate) + 24_000) / 48_000).min(MAX_ITD_SAMPLES as u64)
            as usize
    }

    /// Current sample rate in Hz.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Current octant (`0..8`).
    pub fn octant(&self) -> u8 {
        self.octant
    }

    /// Current output path.
    pub fn mode(&self) -> HrtfMode {
        self.mode
    }

    /// Current per-ear base gains `(left, right)`: the ILD the device wall
    /// shows beside the send. Never panics.
    pub fn gains(&self) -> (f32, f32) {
        (self.hrir_l[0], self.hrir_r[0])
    }

    /// Current interaural delays in samples `(left, right)`: the ITD the
    /// blind-orbit fixture measures against. Never panics.
    pub fn itd_samples(&self) -> (usize, usize) {
        (self.itd_l, self.itd_r)
    }

    /// Turn to `octant`: recomputes the HRIR pair and clears history.
    /// Refuses `>= 8` and keeps the old octant; never panics.
    pub fn set_octant(&mut self, octant: u8) -> Result<(), HrtfError> {
        if octant >= u8::try_from(OCTANTS).unwrap_or(8) {
            return Err(HrtfError::BadOctant { got: octant });
        }
        self.octant = octant;
        self.rebuild();
        Ok(())
    }

    /// Turn to an azimuth in degrees (quantized via [`octant_for`]);
    /// infallible, clears history, never panics.
    pub fn set_direction_azimuth(&mut self, azimuth_deg: i32) {
        self.octant = octant_for(azimuth_deg);
        self.rebuild();
    }

    /// Switch output path; clears history so the switch never clicks.
    /// Never panics.
    pub fn set_mode(&mut self, mode: HrtfMode) {
        self.mode = mode;
        self.hist = [0.0; HIST_LEN];
        self.pos = 0;
    }

    /// Clear the mono history (device reopen, octant jump in a test).
    /// Never panics.
    pub fn reset(&mut self) {
        self.hist = [0.0; HIST_LEN];
        self.pos = 0;
    }

    /// Render one mono sample to stereo. Speakers mode returns dual mono;
    /// headphones mode convolves the HRIR pair with ITD offsets. No heap,
    /// no lock, never panics.
    pub fn process_frame(&mut self, mono: f32) -> (f32, f32) {
        let m = sanitize(mono);
        if self.mode == HrtfMode::Speakers {
            return (m, m);
        }
        self.hist[self.pos] = m;
        let mut left = 0.0f32;
        let mut right = 0.0f32;
        let base_l = self.pos + HIST_LEN - self.itd_l;
        let base_r = self.pos + HIST_LEN - self.itd_r;
        let mut i = 0;
        while i < HRTF_TAPS {
            left += self.hrir_l[i] * self.hist[(base_l - i) % HIST_LEN];
            right += self.hrir_r[i] * self.hist[(base_r - i) % HIST_LEN];
            i += 1;
        }
        self.pos = (self.pos + 1) % HIST_LEN;
        (sanitize(left), sanitize(right))
    }

    /// Render a mono block into separate `left`/`right` blocks (the
    /// device-wall pull shape). All three slices share one length or this
    /// returns [`HrtfError::LengthMismatch`]; empty blocks are `Ok`.
    /// Steady-state calls allocate nothing; never panics.
    pub fn process_into(
        &mut self,
        input: &[f32],
        left: &mut [f32],
        right: &mut [f32],
    ) -> Result<(), HrtfError> {
        if left.len() != input.len() {
            return Err(HrtfError::LengthMismatch {
                input: input.len(),
                output: left.len(),
            });
        }
        if right.len() != input.len() {
            return Err(HrtfError::LengthMismatch {
                input: input.len(),
                output: right.len(),
            });
        }
        for ((&m, l), r) in input.iter().zip(left.iter_mut()).zip(right.iter_mut()) {
            let (ol, or) = self.process_frame(m);
            *l = ol;
            *r = or;
        }
        Ok(())
    }

    /// Render a mono block into interleaved stereo (`LRLR...`); `out` must
    /// hold exactly `2 * input.len()` samples. Empty blocks are `Ok`.
    /// Steady-state calls allocate nothing; never panics.
    pub fn process_interleaved_into(
        &mut self,
        input: &[f32],
        out: &mut [f32],
    ) -> Result<(), HrtfError> {
        if out.len() != input.len().saturating_mul(2) {
            return Err(HrtfError::LengthMismatch {
                input: input.len(),
                output: out.len(),
            });
        }
        for (i, &m) in input.iter().enumerate() {
            let (l, r) = self.process_frame(m);
            out[2 * i] = l;
            out[2 * i + 1] = r;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic broadband probe: LCG uniform noise in `[-1, 1)`.
    /// Integer math only, so every platform probes with the same bytes.
    fn noise_probe(seed: u32, n: usize) -> Vec<f32> {
        let mut x = seed.max(1);
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            out.push((x >> 8) as f32 / 16_777_216.0 * 2.0 - 1.0);
        }
        out
    }

    /// RMS of a slice (`f64` accumulation; never panics on empty).
    fn rms(y: &[f32]) -> f32 {
        if y.is_empty() {
            return 0.0;
        }
        let mut acc = 0f64;
        for &s in y {
            acc += (s as f64) * (s as f64);
        }
        (acc / y.len() as f64).sqrt() as f32
    }

    /// High-frequency ratio of a stereo pair: first-difference energy over
    /// total energy across both ears. Frontal pinnae lift it, rear pinnae
    /// cut it; `0.0` for silence. Never panics.
    fn hf_ratio(left: &[f32], right: &[f32]) -> f32 {
        let mut energy = 0f64;
        let mut diff = 0f64;
        for ch in [left, right] {
            let mut prev = 0f32;
            for (i, &s) in ch.iter().enumerate() {
                energy += (s as f64) * (s as f64);
                if i > 0 {
                    let d = (s - prev) as f64;
                    diff += d * d;
                }
                prev = s;
            }
        }
        if energy == 0.0 {
            0.0
        } else {
            (diff / energy) as f32
        }
    }

    /// Render the shared noise probe through one octant in headphones mode.
    fn render_octant(octant: u8, frames: usize) -> (Vec<f32>, Vec<f32>) {
        let mut proc = HrtfProcessor::new(48_000).expect("48 kHz is valid");
        proc.set_mode(HrtfMode::Headphones);
        proc.set_octant(octant).expect("octant is 0..8");
        let input = noise_probe(0x0F07, frames);
        let mut left = vec![0.0f32; frames];
        let mut right = vec![0.0f32; frames];
        proc.process_into(&input, &mut left, &mut right)
            .expect("lengths agree");
        (left, right)
    }

    #[test]
    fn octant_quantize_exact() {
        assert_eq!(octant_for(0), 0);
        assert_eq!(octant_for(22), 0);
        assert_eq!(octant_for(23), 1);
        assert_eq!(octant_for(67), 1);
        assert_eq!(octant_for(68), 2);
        assert_eq!(octant_for(180), 4);
        assert_eq!(octant_for(270), 6);
        assert_eq!(octant_for(337), 7);
        assert_eq!(octant_for(359), 0);
        assert_eq!(octant_for(360), 0);
        assert_eq!(octant_for(-23), 7);
        assert_eq!(octant_for(-90), 6);
        assert_eq!(octant_for(-180), 4);
        // Extremes wrap without panicking.
        assert!(octant_for(i32::MAX) < 8);
        assert!(octant_for(i32::MIN) < 8);
    }

    #[test]
    fn azimuth_roundtrip_and_reject() {
        for octant in 0..8u8 {
            let center = azimuth_for(octant).expect("0..8 is valid");
            assert_eq!(center, octant as i32 * 45);
            assert_eq!(octant_for(center), octant);
        }
        assert_eq!(azimuth_for(8), None);
        assert_eq!(azimuth_for(255), None);
    }

    #[test]
    fn bad_rate_and_octant_refused_state_kept() {
        assert_eq!(
            HrtfProcessor::new(0).err(),
            Some(HrtfError::BadSampleRate { got: 0 })
        );
        assert_eq!(
            HrtfProcessor::new(7_999).err(),
            Some(HrtfError::BadSampleRate { got: 7_999 })
        );
        assert_eq!(
            HrtfProcessor::new(192_000).err(),
            Some(HrtfError::BadSampleRate { got: 192_000 })
        );
        let mut proc = HrtfProcessor::new(48_000).expect("48 kHz is valid");
        assert_eq!(proc.octant(), 0);
        assert_eq!(proc.set_octant(8), Err(HrtfError::BadOctant { got: 8 }));
        assert_eq!(proc.octant(), 0, "refused turn keeps the old octant");
        proc.set_direction_azimuth(200);
        assert_eq!(proc.octant(), 4);
        proc.set_direction_azimuth(-90);
        assert_eq!(proc.octant(), 6);
        proc.set_direction_azimuth(i32::MIN);
        assert!(proc.octant() < 8);
        // Errors describe themselves for logs.
        assert!(!HrtfError::BadOctant { got: 9 }.to_string().is_empty());
        assert!(!HrtfError::BadSampleRate { got: 1 }.to_string().is_empty());
        assert!(!HrtfError::LengthMismatch {
            input: 1,
            output: 2
        }
        .to_string()
        .is_empty());
    }

    #[test]
    fn speakers_path_bit_exact() {
        // The legacy path: sanitized input passes through untouched, so a
        // speakers-only build renders byte-identical mixes with HRTF on.
        let mut proc = HrtfProcessor::new(48_000).expect("48 kHz is valid");
        assert_eq!(proc.mode(), HrtfMode::Speakers);
        let input = noise_probe(7, 1024);
        let mut left = vec![0.0f32; 1024];
        let mut right = vec![0.0f32; 1024];
        proc.process_into(&input, &mut left, &mut right)
            .expect("lengths agree");
        assert_eq!(left, input);
        assert_eq!(right, input);
        // Octant turns change nothing while the speakers path is selected.
        proc.set_octant(4).expect("octant 4 is valid");
        let mut left2 = vec![0.0f32; 1024];
        let mut right2 = vec![0.0f32; 1024];
        proc.process_into(&input, &mut left2, &mut right2)
            .expect("lengths agree");
        assert_eq!(left2, input);
        assert_eq!(right2, input);
        // Interleaved form is dual mono, sample for sample.
        let mut proc = HrtfProcessor::new(8_000).expect("8 kHz is valid");
        let mut out = vec![0.0f32; 2048];
        proc.process_interleaved_into(&input, &mut out)
            .expect("2x length agrees");
        for (i, &m) in input.iter().enumerate() {
            assert_eq!(out[2 * i], m);
            assert_eq!(out[2 * i + 1], m);
        }
    }

    #[test]
    fn center_octants_dual_mono_exact() {
        // Straight ahead and straight behind: symmetric ears, so left and
        // right agree bit for bit (a pan-only mixer could never split them
        // from front to back, which is the point of the next test).
        for octant in [0u8, 4u8] {
            let (left, right) = render_octant(octant, 2048);
            assert_eq!(left, right, "octant {octant} must be dual mono");
            assert!(left.iter().any(|&s| s != 0.0));
        }
    }

    #[test]
    fn impulse_itd_peak_positions() {
        // A single impulse exposes the delay skeleton: the ipsi ear peaks
        // at frame 0 with its base gain, the contra ear peaks exactly at
        // its ITD with silence before.
        let render_impulse = |octant: u8| {
            let mut proc = HrtfProcessor::new(48_000).expect("48 kHz is valid");
            proc.set_mode(HrtfMode::Headphones);
            proc.set_octant(octant).expect("octant is 0..8");
            let (dl, dr) = proc.itd_samples();
            let (gl, gr) = proc.gains();
            let mut left = Vec::with_capacity(HIST_LEN);
            let mut right = Vec::with_capacity(HIST_LEN);
            for i in 0..HIST_LEN {
                let (l, r) = proc.process_frame(if i == 0 { 1.0 } else { 0.0 });
                left.push(l);
                right.push(r);
            }
            (left, right, dl, dr, gl, gr)
        };
        let (left, right, dl, dr, gl, gr) = render_impulse(2);
        assert_eq!((dl, dr), (30, 0), "due right: left ear waits 30 samples");
        assert!(
            (right[0] - gr).abs() < 1e-6,
            "right peaks now: {}",
            right[0]
        );
        assert!((right[0] - 0.95).abs() < 1e-6);
        assert_eq!(left[0], 0.0, "contra ear is silent before its delay");
        assert_eq!(left[29], 0.0);
        assert!(
            (left[30] - gl).abs() < 1e-6,
            "left peaks at ITD: {}",
            left[30]
        );
        let (left, right, dl, dr, _, _) = render_impulse(6);
        assert_eq!((dl, dr), (0, 30), "due left mirrors due right");
        assert_eq!(right[0], 0.0);
        assert_eq!(right[29], 0.0);
        assert!(right[30] > 0.5);
        assert!(left[0] > 0.5);
        let (left, right, dl, dr, _, _) = render_impulse(0);
        assert_eq!((dl, dr), (0, 0), "front has no interaural delay");
        assert!((left[0] - 1.0).abs() < 1e-6);
        assert!(
            (left[1] + 0.26).abs() < 1e-6,
            "frontal lift tap: {}",
            left[1]
        );
        assert_eq!(left, right);
        let (left, right, dl, dr, _, _) = render_impulse(4);
        assert_eq!((dl, dr), (0, 0), "back shares front delays (the cone)");
        assert!((left[1] - 0.27).abs() < 1e-6, "rear cut tap: {}", left[1]);
        assert!(
            (left[3] + 0.198).abs() < 1e-4,
            "rear notch tap: {}",
            left[3]
        );
        assert_eq!(left, right);
    }

    #[test]
    fn front_back_spectral_gap() {
        // Front/back pairs share ITD and near-share ILD, so only the pinna
        // spectrum splits them: every frontal octant must carry clearly
        // more high-frequency energy than its rear mirror.
        const FRAMES: usize = 4096;
        let mut hf = [0.0f32; OCTANTS];
        for k in 0..8u8 {
            let (left, right) = render_octant(k, FRAMES);
            hf[k as usize] = hf_ratio(&left, &right);
        }
        for (front, back) in [(0usize, 4usize), (1, 3), (7, 5)] {
            assert!(
                hf[front] > hf[back] * 1.5,
                "octant {front} (hf={}) must beat rear {back} (hf={}) x1.5",
                hf[front],
                hf[back]
            );
        }
    }

    #[test]
    fn side_energy_ordering() {
        // ILD puts the voice on the right side of the head for right
        // octants, the left side for left octants, center for front/back.
        const FRAMES: usize = 4096;
        for k in 0..8u8 {
            let (left, right) = render_octant(k, FRAMES);
            let (rl, rr) = (rms(&left), rms(&right));
            assert!(rl > 0.01 && rr > 0.01, "octant {k} must stay audible");
            match k {
                1..=3 => assert!(
                    rr > rl * 1.15,
                    "octant {k}: right ({rr}) must beat left ({rl})"
                ),
                5..=7 => assert!(
                    rl > rr * 1.15,
                    "octant {k}: left ({rl}) must beat right ({rr})"
                ),
                _ => assert_eq!(rl, rr, "octant {k} stays centered"),
            }
        }
    }

    #[test]
    fn orbit_blind_naming_7_of_8() {
        // Headless proxy for the operator headphone listen (still required
        // before any "sounds like" claim ships): name all 8 blind-orbit
        // stops from measured side plus front/back cues alone. The depth
        // threshold is anchored at runtime to the measured front/rear
        // stops, never to a restated constant; pure-lateral stops score on
        // side only. Needs 7/8; the rig scores 8/8 with margin.
        const FRAMES: usize = 4096;
        let mut side = [0i8; OCTANTS];
        let mut depth = [0i8; OCTANTS];
        let mut hf = [0.0f32; OCTANTS];
        for k in 0..8u8 {
            let (left, right) = render_octant(k, FRAMES);
            let (rl, rr) = (rms(&left), rms(&right));
            side[k as usize] = if rr > rl * 1.15 {
                1
            } else if rl > rr * 1.15 {
                -1
            } else {
                0
            };
            hf[k as usize] = hf_ratio(&left, &right);
        }
        let mid = (hf[0] + hf[4]) * 0.5;
        assert!(hf[0] > mid && mid > hf[4], "anchors must straddle the gate");
        for k in 0..8usize {
            depth[k] = if hf[k] > mid { 1 } else { -1 };
        }
        // Expected (side, depth): 0 (C,F), 1 (R,F), 2 (R,*), 3 (R,B),
        // 4 (C,B), 5 (L,B), 6 (L,*), 7 (L,F).
        const WANT_SIDE: [i8; OCTANTS] = [0, 1, 1, 1, 0, -1, -1, -1];
        const WANT_DEPTH: [i8; OCTANTS] = [1, 1, 0, -1, -1, -1, 0, 1];
        let mut score = 0;
        for k in 0..8usize {
            let side_ok = side[k] == WANT_SIDE[k];
            let depth_ok = WANT_DEPTH[k] == 0 || depth[k] == WANT_DEPTH[k];
            score += (side_ok && depth_ok) as usize;
        }
        assert!(
            score >= 7,
            "blind orbit names {score}/8 (want >= 7/8 with front/back resolved)"
        );
        assert_eq!(score, 8, "rig margin: the proxy must clear 8/8 headless");
    }

    #[test]
    fn buffers_refused_and_empty_ok() {
        let mut proc = HrtfProcessor::new(48_000).expect("48 kHz is valid");
        proc.set_mode(HrtfMode::Headphones);
        let input = noise_probe(11, 4);
        let mut ok = [0.0f32; 4];
        let mut short = [0.0f32; 3];
        assert_eq!(
            proc.process_into(&input, &mut ok, &mut short),
            Err(HrtfError::LengthMismatch {
                input: 4,
                output: 3
            })
        );
        assert_eq!(
            proc.process_into(&input, &mut short, &mut ok),
            Err(HrtfError::LengthMismatch {
                input: 4,
                output: 3
            })
        );
        let mut out = [0.0f32; 7];
        assert_eq!(
            proc.process_interleaved_into(&input, &mut out),
            Err(HrtfError::LengthMismatch {
                input: 4,
                output: 7
            })
        );
        // Empty blocks are legal no-ops, never errors.
        let mut empty_l = [0.0f32; 0];
        let mut empty_r = [0.0f32; 0];
        proc.process_into(&[], &mut empty_l, &mut empty_r)
            .expect("empty split block is Ok");
        proc.process_interleaved_into(&[], &mut empty_l)
            .expect("empty interleaved block is Ok");
    }

    #[test]
    fn nan_inf_sanitized_never_panics() {
        let mut proc = HrtfProcessor::new(48_000).expect("48 kHz is valid");
        assert_eq!(proc.process_frame(f32::NAN), (0.0, 0.0));
        assert_eq!(proc.process_frame(f32::INFINITY), (1.0, 1.0));
        assert_eq!(proc.process_frame(f32::NEG_INFINITY), (-1.0, -1.0));
        proc.set_mode(HrtfMode::Headphones);
        proc.reset();
        let (l, r) = proc.process_frame(f32::NAN);
        assert_eq!((l, r), (0.0, 0.0));
        let input = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 0.5];
        let mut left = [9.0f32; 4];
        let mut right = [9.0f32; 4];
        proc.process_into(&input, &mut left, &mut right)
            .expect("lengths agree");
        for &s in left.iter().chain(right.iter()) {
            assert!(s.is_finite(), "headphone outs stay finite: {s}");
            assert!((-1.0..=1.0).contains(&s));
        }
    }

    #[test]
    fn determinism_same_input_same_bits() {
        let input = noise_probe(99, 1024);
        let render = || {
            let mut proc = HrtfProcessor::new(48_000).expect("48 kHz is valid");
            proc.set_mode(HrtfMode::Headphones);
            proc.set_octant(3).expect("octant 3 is valid");
            let mut left = vec![0.0f32; 1024];
            let mut right = vec![0.0f32; 1024];
            proc.process_into(&input, &mut left, &mut right)
                .expect("lengths agree");
            (left, right)
        };
        let (l1, r1) = render();
        let (l2, r2) = render();
        assert_eq!(l1, l2);
        assert_eq!(r1, r2);
        // Reset replays the prefix bit-exactly.
        let mut proc = HrtfProcessor::new(48_000).expect("48 kHz is valid");
        proc.set_mode(HrtfMode::Headphones);
        proc.set_octant(3).expect("octant 3 is valid");
        let mut prefix = vec![0.0f32; 256];
        let mut trash = vec![0.0f32; 256];
        proc.process_into(&input[..256], &mut prefix, &mut trash)
            .expect("lengths agree");
        proc.reset();
        let mut replay = vec![0.0f32; 256];
        proc.process_into(&input[..256], &mut replay, &mut trash)
            .expect("lengths agree");
        assert_eq!(prefix, replay);
    }

    #[test]
    fn interleaved_matches_split() {
        // Both block shapes are the same render: deinterleaved must equal
        // the split outs sample for sample.
        let input = noise_probe(5, 512);
        let render_split = || {
            let mut proc = HrtfProcessor::new(32_000).expect("32 kHz is valid");
            proc.set_mode(HrtfMode::Headphones);
            proc.set_octant(5).expect("octant 5 is valid");
            let mut left = vec![0.0f32; 512];
            let mut right = vec![0.0f32; 512];
            proc.process_into(&input, &mut left, &mut right)
                .expect("lengths agree");
            (left, right)
        };
        let mut proc = HrtfProcessor::new(32_000).expect("32 kHz is valid");
        proc.set_mode(HrtfMode::Headphones);
        proc.set_octant(5).expect("octant 5 is valid");
        let mut packed = vec![0.0f32; 1024];
        proc.process_interleaved_into(&input, &mut packed)
            .expect("2x length agrees");
        let (left, right) = render_split();
        for i in 0..512 {
            assert_eq!(packed[2 * i], left[i]);
            assert_eq!(packed[2 * i + 1], right[i]);
        }
    }

    #[test]
    fn steady_state_process_allocates_nothing() {
        use engine_core::alloc_probe::measure;
        // The device-wall pull shape: one live processor refills reused
        // split plus interleaved blocks every tick; the window must see
        // zero heap traffic.
        const FRAMES: usize = 480;
        const TICKS: usize = 600;
        let mut proc = HrtfProcessor::new(48_000).expect("48 kHz is valid");
        proc.set_mode(HrtfMode::Headphones);
        proc.set_octant(1).expect("octant 1 is valid");
        let input = noise_probe(0xB17, FRAMES);
        let mut left = vec![0.0f32; FRAMES];
        let mut right = vec![0.0f32; FRAMES];
        let mut packed = vec![0.0f32; FRAMES * 2];
        proc.process_into(&input, &mut left, &mut right)
            .expect("lengths agree");
        proc.process_interleaved_into(&input, &mut packed)
            .expect("2x length agrees");
        assert!(left.iter().any(|&s| s != 0.0));
        assert!(packed.iter().any(|&s| s != 0.0));
        let (folded, heap) = measure(|| {
            let mut folded = 0u64;
            for tick in 0..TICKS {
                proc.process_into(&input, &mut left, &mut right)
                    .expect("lengths agree");
                proc.process_interleaved_into(&input, &mut packed)
                    .expect("2x length agrees");
                for &s in left.iter().chain(right.iter()).chain(packed.iter()) {
                    folded = folded
                        .wrapping_add(s.to_bits() as u64)
                        .wrapping_add(tick as u64);
                }
                folded = folded.wrapping_add(proc.process_frame(0.25).0.to_bits() as u64);
            }
            folded
        });
        std::hint::black_box((&left, &right, &packed));
        assert_eq!(heap.calls, 0, "steady-state HRTF blocks must not allocate");
        assert!(folded != 0, "sanity: blocks rendered during the window");
    }
}
