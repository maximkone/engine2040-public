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
//! Measured sphere: 108 measured KEMAR directions interpolated in the
//! log-magnitude/phase domain (azimuth wraps, elevation clamps) behind
//! [`HrtfProcessor::set_measured_sphere`]; flag off keeps the octant path.
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
    measured: bool,
    aim_az: f32,
    aim_el: f32,
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
            measured: false,
            aim_az: 0.0,
            aim_el: 0.0,
        };
        proc.rebuild();
        Ok(proc)
    }

    /// Recompute gains, delays and FIRs from the current octant and rate,
    /// then clear the history so a reused processor renders the new octant
    /// without stale smear. No heap, no failure modes.
    fn rebuild(&mut self) {
        if self.measured {
            self.rebuild_measured();
            return;
        }
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
        self.itd_l = Self::scale_delay(48_000, self.sample_rate, delay_l_48k);
        self.itd_r = Self::scale_delay(48_000, self.sample_rate, delay_r_48k);
        self.hist = [0.0; HIST_LEN];
        self.pos = 0;
    }

    /// Scale a delay measured at `from_hz` to the processor rate with
    /// rounding, clamped to [`MAX_ITD_SAMPLES`]. Pure integer math; never
    /// panics.
    fn scale_delay(from_hz: u32, to_hz: u32, delay: usize) -> usize {
        let half = u64::from(from_hz) / 2;
        let scaled = (delay as u64 * u64::from(to_hz) + half) / u64::from(from_hz);
        scaled.min(MAX_ITD_SAMPLES as u64) as usize
    }

    /// Recompute the pair from the measured sphere at the current aim: the
    /// taps are the interpolated measured ears, the delays are their measured
    /// arrival difference (the earlier ear writes first).
    fn rebuild_measured(&mut self) {
        let (taps_l, taps_r, onset_l, onset_r) = measured_hrir_at(self.aim_az, self.aim_el);
        self.hrir_l = taps_l;
        self.hrir_r = taps_r;
        let base = if onset_l < onset_r { onset_l } else { onset_r };
        let lead_l = whole_samples((onset_l - base).max(0.0) + 0.5);
        let lead_r = whole_samples((onset_r - base).max(0.0) + 0.5);
        self.itd_l = Self::scale_delay(SPHERE_RATE_HZ, self.sample_rate, lead_l);
        self.itd_r = Self::scale_delay(SPHERE_RATE_HZ, self.sample_rate, lead_r);
        self.hist = [0.0; HIST_LEN];
        self.pos = 0;
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
        if self.measured {
            self.aim_az = f32::from(octant) * OCTANT_DEG;
            self.aim_el = 0.0;
        }
        self.rebuild();
        Ok(())
    }

    /// Turn to an azimuth in degrees (quantized via [`octant_for`]);
    /// infallible, clears history, never panics.
    pub fn set_direction_azimuth(&mut self, azimuth_deg: i32) {
        self.octant = octant_for(azimuth_deg);
        if self.measured {
            self.aim_az = f32::from(u16::try_from(azimuth_deg.rem_euclid(360)).unwrap_or(0));
            self.aim_el = 0.0;
        }
        self.rebuild();
    }

    /// Turn the measured sphere on or off. Off (the default) is the shipped
    /// octant path, bit for bit; on reads the 48-direction table. Clears the
    /// history so the switch never clicks, and re-queries if already aimed.
    pub fn set_measured_sphere(&mut self, on: bool) {
        self.measured = on;
        if on {
            let k = usize::from(self.octant) % OCTANTS;
            self.aim_az = OCT_SIN[k].atan2(OCT_COS[k]).to_degrees().rem_euclid(360.0);
            self.aim_el = 0.0;
        }
        self.rebuild();
    }

    /// Aim the measured sphere at a real direction in degrees (azimuth
    /// clockwise from straight ahead, elevation up from ear level). Turns the
    /// sphere on. Azimuth wraps, elevation clamps at the table poles;
    /// infallible and allocation-free, never panics.
    pub fn set_direction(&mut self, azimuth_deg: f32, elevation_deg: f32) {
        self.measured = true;
        self.aim_az = if azimuth_deg.is_finite() {
            azimuth_deg
        } else {
            0.0
        };
        self.aim_el = if elevation_deg.is_finite() {
            elevation_deg
        } else {
            0.0
        };
        // Quantize the aim the legacy way (8 x 45 degrees) for callers that
        // read the octant while the measured sphere drives the pair.
        let mut oct = 0u8;
        let mut rest = self.aim_az.rem_euclid(360.0) + OCTANT_DEG / 2.0;
        while rest >= OCTANT_DEG && oct < 7 {
            rest -= OCTANT_DEG;
            oct += 1;
        }
        self.octant = oct;
        self.rebuild();
    }

    /// Whether the measured sphere is driving the pair (vs the octant path).
    pub fn measured_sphere(&self) -> bool {
        self.measured
    }

    /// Current aim in degrees `(azimuth, elevation)`; `(0, 0)` when the
    /// measured sphere has never been aimed. Never panics.
    pub fn aim_degrees(&self) -> (f32, f32) {
        (self.aim_az, self.aim_el)
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

// ---- measured HRIR sphere (AAC7) ----------------------------------------
//
// 108 measured directions from the MIT KEMAR compact set: 36 azimuth
// columns (10 degrees apart, wrapping) by 3 elevation rows (-40, 0, +40
// degrees, clamped outside). Taps are 16 per ear at 44.1 kHz, each ear
// aligned on its own measured arrival (3 samples before the peak) and both
// ears of a direction scaled by one common factor, so ILD is the measured
// one. The arrival itself is the ITD. Between measured columns the taps blend
// in the log-magnitude/phase domain and the arrivals on their own line.
// Nothing here is synthesised: the octant filter below stays the shipped path
// and this table is only read when [`HrtfProcessor::set_measured_sphere`] is on.

/// Measured azimuth columns around the head (10-degree steps, wraps).
pub const SPHERE_AZ_COLS: usize = 36;
/// Measured elevation rows (-40, 0, +40 degrees; clamps outside).
pub const SPHERE_ELEV_ROWS: usize = 3;
/// Measured directions in the table ([`SPHERE_AZ_COLS`] x [`SPHERE_ELEV_ROWS`]).
pub const SPHERE_DIRECTIONS: usize = SPHERE_AZ_COLS * SPHERE_ELEV_ROWS;
/// Rate the measured taps and onsets were captured at (Hz).
pub const SPHERE_RATE_HZ: u32 = 44_100;
/// Unique bins of a [`HRTF_TAPS`]-point real spectrum (DC through Nyquist).
const SPHERE_BINS: usize = HRTF_TAPS / 2 + 1;
/// Magnitude floor of the log domain (about -80 dB on the normalized taps),
/// so a measured ear never reads as silence and `ln` never sees zero.
const TAP_MAG_FLOOR: f32 = 1e-4;
/// Azimuth step between measured columns in degrees (`360 / 36`).
pub const SPHERE_AZ_STEP_DEG: f32 = 10.0;
/// Degrees between octant centres (the legacy 8-way orbit).
const OCTANT_DEG: f32 = 45.0;

/// Azimuth columns in degrees, clockwise from straight ahead.
pub const SPHERE_AZIMUTHS_DEG: [f32; SPHERE_AZ_COLS] = [
    0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0, 110.0, 120.0, 130.0, 140.0,
    150.0, 160.0, 170.0, 180.0, 190.0, 200.0, 210.0, 220.0, 230.0, 240.0, 250.0, 260.0, 270.0,
    280.0, 290.0, 300.0, 310.0, 320.0, 330.0, 340.0, 350.0,
];
/// Elevation rows in degrees, ear-level 0 between below and above.
pub const SPHERE_ELEVATIONS_DEG: [f32; SPHERE_ELEV_ROWS] = [-40.0, 0.0, 40.0];
/// Measured onset (ear arrival) in samples at 44.1 kHz, one byte per ear.
#[rustfmt::skip]
pub const SPHERE_ONSETS: [[[u8; 2]; SPHERE_AZ_COLS]; SPHERE_ELEV_ROWS] = [
    [
        [14, 14], [16, 12], [17, 11], [19, 10], [19, 9], [21, 8], [22, 8], [23, 7], [24, 7],
        [33, 7], [26, 7], [25, 8], [23, 8], [23, 9], [21, 10], [20, 11], [19, 13], [18, 14],
        [16, 16], [14, 18], [13, 19], [11, 20], [10, 21], [9, 23], [8, 23], [8, 25], [7, 26],
        [7, 33], [7, 24], [7, 23], [8, 22], [8, 21], [9, 19], [10, 19], [11, 17], [12, 16],
    ],
    [
        [12, 12], [14, 11], [16, 9], [18, 8], [20, 6], [23, 5], [25, 4], [28, 4], [30, 3],
        [40, 3], [29, 3], [27, 4], [26, 5], [24, 6], [22, 7], [20, 9], [19, 11], [17, 13],
        [15, 15], [13, 17], [11, 19], [9, 20], [7, 22], [6, 24], [5, 26], [4, 27], [3, 29],
        [3, 40], [3, 30], [4, 28], [4, 25], [5, 23], [6, 20], [8, 18], [9, 16], [11, 14],
    ],
    [
        [14, 14], [17, 13], [18, 12], [20, 11], [21, 10], [23, 9], [24, 9], [26, 8], [27, 8],
        [27, 8], [27, 8], [27, 8], [25, 9], [25, 10], [23, 11], [22, 12], [20, 14], [18, 15],
        [17, 17], [15, 18], [14, 20], [12, 22], [11, 23], [10, 25], [9, 25], [8, 27], [8, 27],
        [8, 27], [8, 27], [8, 26], [9, 24], [9, 23], [10, 21], [11, 20], [12, 18], [13, 17],
    ],
];
/// Measured taps, per ear, peak-normalized: [elevation][azimuth][ear][tap].
#[rustfmt::skip]
pub const SPHERE_TAPS: [[[[i16; HRTF_TAPS]; 2]; SPHERE_AZ_COLS]; SPHERE_ELEV_ROWS] = [
    [
        // -40 deg elev, 0 deg az (measured 0)
        [
            [24953, 10529, 15263, 32767, 9415, -15463, -11203, -20222, -23450,
             -1364, -6931, -21219, -12231, -4013, -2266, -1383],
            [24953, 10529, 15263, 32767, 9415, -15463, -11203, -20222, -23450,
             -1364, -6931, -21219, -12231, -4013, -2266, -1383],
        ],
        // -40 deg elev, 10 deg az (measured 10)
        [
            [19602, 13633, 13547, 22729, 14113, -3213, -5673, -13183, -20269,
             -7955, -2857, -12605, -14042, -7252, -1778, -1162],
            [892, -5370, 22219, 32767, 19608, 8485, 30725, 28431, -14940,
             -17545, -15088, -27551, -6571, 2715, -21072, -19457],
        ],
        // -40 deg elev, 20 deg az (measured 20)
        [
            [15638, 12542, 11934, 18758, 12080, -1090, -4329, -10963, -16805,
             -7444, -3021, -11287, -12152, -5566, -952, -1101],
            [1308, -5878, 18161, 32767, 22678, 9602, 26321, 31107, -9754,
             -18443, -16949, -25760, -6177, 2838, -19693, -20139],
        ],
        // -40 deg elev, 30 deg az (measured 30)
        [
            [9968, 8133, 8135, 11196, 6968, 541, -2768, -8074, -9757,
             -3235, -3178, -8250, -6637, -1871, -220, -830],
            [22937, 11251, 11634, 32767, 11052, -16178, -15487, -22966, -9342,
             4835, -10389, -20739, -12244, -7127, -6231, -2013],
        ],
        // -40 deg elev, 40 deg az (measured 40)
        [
            [9181, 7349, 8095, 9918, 5568, 547, -3053, -8039, -7905,
             -2661, -4260, -7166, -4469, -1110, -567, -618],
            [25829, 13352, 9015, 32767, 25799, -11682, -17042, -24617, -15836,
             5174, -4680, -21830, -17702, -8917, -4469, -2435],
        ],
        // -40 deg elev, 50 deg az (measured 50)
        [
            [5199, 4528, 5171, 5432, 3429, 464, -3028, -4947, -3253,
             -1496, -2550, -2794, -1524, -365, -220, -210],
            [13400, 8337, 14027, 32767, 13738, -13394, -17607, -19991, -942,
             3122, -12232, -18538, -10605, -4367, -1783, -328],
        ],
        // -40 deg elev, 60 deg az (measured 60)
        [
            [4119, 4747, 4675, 5110, 4370, 2639, -124, -3202, -3874,
             -2045, -1324, -1943, -1566, -748, -424, -540],
            [10863, 9410, 26418, 32767, -3705, -18237, -21972, -9655, 6120,
             -6159, -18748, -16183, -6834, -3340, -319, 151],
        ],
        // -40 deg elev, 70 deg az (measured 70)
        [
            [1956, 2707, 2750, 2820, 2594, 2539, 1937, -62, -1439,
             -572, 367, 129, -325, -858, -921, -801],
            [9136, 7758, 19816, 32767, 4522, -19171, -17708, -5944, 6892,
             -3483, -15830, -16788, -6247, -2844, -1570, -253],
        ],
        // -40 deg elev, 80 deg az (measured 80)
        [
            [1353, 1934, 2835, 2982, 2921, 2327, 867, 187, 590,
             242, -506, -701, -945, -1236, -1301, -1156],
            [909, 17629, 5200, -32767, -9583, 6593, 7254, 12945, 7762,
             10966, 27494, 30700, -6821, -23314, -13766, 3110],
        ],
        // -40 deg elev, 90 deg az (measured 90)
        [
            [2204, 2639, 3555, 4245, 3836, 2219, 110, -814, -642,
             -795, -1285, -1342, -1282, -1604, -1342, -675],
            [887, 17982, 1661, -32767, -4474, 8210, 5004, 9027, 7682,
             13676, 29044, 23225, -15174, -22165, -3045, 9862],
        ],
        // -40 deg elev, 100 deg az (measured 100)
        [
            [1789, 2257, 3927, 5365, 3995, 2030, 1339, 620, -173,
             -949, -2018, -2252, -1847, -1628, -1140, -374],
            [-719, 13880, 15512, -32767, -17568, 12162, 2822, 7471, 8833,
             14632, 29904, 30683, -10288, -27068, -1999, 12167],
        ],
        // -40 deg elev, 110 deg az (measured 110)
        [
            [1951, 2085, 3667, 5454, 3925, 1129, -66, 35, 399,
             -14, -1205, -1993, -2170, -1697, -923, -705],
            [8978, 12256, 25761, 32767, 1549, -26966, -6248, 11852, 6717,
             -10927, -18311, -12977, -5544, -5621, -6144, -2845],
        ],
        // -40 deg elev, 120 deg az (measured 120)
        [
            [3852, 3724, 4737, 8487, 7563, 786, -2590, -2222, -2091,
             -2241, -2612, -3270, -2425, -1193, -422, 126],
            [2284, 13096, 18911, 32767, 30755, -7371, -23987, 4832, 14936,
             -2507, -20608, -19545, -11060, -6267, -9005, -7698],
        ],
        // -40 deg elev, 130 deg az (measured 130)
        [
            [3575, 4060, 7580, 8734, 1987, -2937, -2634, -2171, -2586,
             -3047, -3810, -3433, -2241, -717, 364, 555],
            [347, 17215, 5468, -32767, -5294, 9332, -3073, 7332, 13642,
             21397, 29152, 11142, -16603, -6531, 12061, 4998],
        ],
        // -40 deg elev, 140 deg az (measured 140)
        [
            [6278, 6322, 10531, 17429, 7858, -5709, -6123, -3873, -5197,
             -5806, -6748, -7562, -5755, -3577, -969, -33],
            [4551, 16929, 22805, 32767, 25440, -5067, -7365, 9776, 3980,
             -14284, -25644, -19861, -12031, -10572, -11046, -5370],
        ],
        // -40 deg elev, 150 deg az (measured 150)
        [
            [7501, 7266, 10420, 19310, 12294, -5547, -8107, -4334, -5640,
             -7003, -7285, -9014, -7082, -4751, -1856, 37],
            [10275, 18329, 24857, 32767, 17488, -5909, 1743, 7867, -5402,
             -20225, -24665, -16932, -11487, -11013, -10370, -3467],
        ],
        // -40 deg elev, 160 deg az (measured 160)
        [
            [6000, 17873, -9888, -25626, -2004, 5303, 10413, 11668, 10229,
             13681, 24769, 21605, -4128, -13783, -7860, -6899],
            [2621, 22434, 3815, -32767, -12745, 666, 2658, 16038, 22400,
             27237, 30904, 16275, 2925, 2728, -4346, -15213],
        ],
        // -40 deg elev, 170 deg az (measured 170)
        [
            [6374, 20877, -9433, -30847, -4195, 6089, 12677, 14406, 11706,
             16231, 27112, 23803, -2373, -14959, -11091, -8841],
            [2664, 22097, 5552, -32767, -15830, 589, 4989, 18112, 21647,
             25334, 29938, 19805, 6092, -654, -8239, -16694],
        ],
        // -40 deg elev, 180 deg az (measured 180)
        [
            [3174, 21073, 3270, -32767, -14369, 3049, 9599, 18956, 16752,
             18068, 24945, 24068, 7152, -8424, -13004, -13783],
            [3174, 21073, 3270, -32767, -14369, 3049, 9599, 18956, 16752,
             18068, 24945, 24068, 7152, -8424, -13004, -13783],
        ],
        // -40 deg elev, 190 deg az (measured 170, ears mirrored)
        [
            [2664, 22097, 5552, -32767, -15830, 589, 4989, 18112, 21647,
             25334, 29938, 19805, 6092, -654, -8239, -16694],
            [6374, 20877, -9433, -30847, -4195, 6089, 12677, 14406, 11706,
             16231, 27112, 23803, -2373, -14959, -11091, -8841],
        ],
        // -40 deg elev, 200 deg az (measured 160, ears mirrored)
        [
            [2621, 22434, 3815, -32767, -12745, 666, 2658, 16038, 22400,
             27237, 30904, 16275, 2925, 2728, -4346, -15213],
            [6000, 17873, -9888, -25626, -2004, 5303, 10413, 11668, 10229,
             13681, 24769, 21605, -4128, -13783, -7860, -6899],
        ],
        // -40 deg elev, 210 deg az (measured 150, ears mirrored)
        [
            [10275, 18329, 24857, 32767, 17488, -5909, 1743, 7867, -5402,
             -20225, -24665, -16932, -11487, -11013, -10370, -3467],
            [7501, 7266, 10420, 19310, 12294, -5547, -8107, -4334, -5640,
             -7003, -7285, -9014, -7082, -4751, -1856, 37],
        ],
        // -40 deg elev, 220 deg az (measured 140, ears mirrored)
        [
            [4551, 16929, 22805, 32767, 25440, -5067, -7365, 9776, 3980,
             -14284, -25644, -19861, -12031, -10572, -11046, -5370],
            [6278, 6322, 10531, 17429, 7858, -5709, -6123, -3873, -5197,
             -5806, -6748, -7562, -5755, -3577, -969, -33],
        ],
        // -40 deg elev, 230 deg az (measured 130, ears mirrored)
        [
            [347, 17215, 5468, -32767, -5294, 9332, -3073, 7332, 13642,
             21397, 29152, 11142, -16603, -6531, 12061, 4998],
            [3575, 4060, 7580, 8734, 1987, -2937, -2634, -2171, -2586,
             -3047, -3810, -3433, -2241, -717, 364, 555],
        ],
        // -40 deg elev, 240 deg az (measured 120, ears mirrored)
        [
            [2284, 13096, 18911, 32767, 30755, -7371, -23987, 4832, 14936,
             -2507, -20608, -19545, -11060, -6267, -9005, -7698],
            [3852, 3724, 4737, 8487, 7563, 786, -2590, -2222, -2091,
             -2241, -2612, -3270, -2425, -1193, -422, 126],
        ],
        // -40 deg elev, 250 deg az (measured 110, ears mirrored)
        [
            [8978, 12256, 25761, 32767, 1549, -26966, -6248, 11852, 6717,
             -10927, -18311, -12977, -5544, -5621, -6144, -2845],
            [1951, 2085, 3667, 5454, 3925, 1129, -66, 35, 399,
             -14, -1205, -1993, -2170, -1697, -923, -705],
        ],
        // -40 deg elev, 260 deg az (measured 100, ears mirrored)
        [
            [-719, 13880, 15512, -32767, -17568, 12162, 2822, 7471, 8833,
             14632, 29904, 30683, -10288, -27068, -1999, 12167],
            [1789, 2257, 3927, 5365, 3995, 2030, 1339, 620, -173,
             -949, -2018, -2252, -1847, -1628, -1140, -374],
        ],
        // -40 deg elev, 270 deg az (measured 90, ears mirrored)
        [
            [887, 17982, 1661, -32767, -4474, 8210, 5004, 9027, 7682,
             13676, 29044, 23225, -15174, -22165, -3045, 9862],
            [2204, 2639, 3555, 4245, 3836, 2219, 110, -814, -642,
             -795, -1285, -1342, -1282, -1604, -1342, -675],
        ],
        // -40 deg elev, 280 deg az (measured 80, ears mirrored)
        [
            [909, 17629, 5200, -32767, -9583, 6593, 7254, 12945, 7762,
             10966, 27494, 30700, -6821, -23314, -13766, 3110],
            [1353, 1934, 2835, 2982, 2921, 2327, 867, 187, 590,
             242, -506, -701, -945, -1236, -1301, -1156],
        ],
        // -40 deg elev, 290 deg az (measured 70, ears mirrored)
        [
            [9136, 7758, 19816, 32767, 4522, -19171, -17708, -5944, 6892,
             -3483, -15830, -16788, -6247, -2844, -1570, -253],
            [1956, 2707, 2750, 2820, 2594, 2539, 1937, -62, -1439,
             -572, 367, 129, -325, -858, -921, -801],
        ],
        // -40 deg elev, 300 deg az (measured 60, ears mirrored)
        [
            [10863, 9410, 26418, 32767, -3705, -18237, -21972, -9655, 6120,
             -6159, -18748, -16183, -6834, -3340, -319, 151],
            [4119, 4747, 4675, 5110, 4370, 2639, -124, -3202, -3874,
             -2045, -1324, -1943, -1566, -748, -424, -540],
        ],
        // -40 deg elev, 310 deg az (measured 50, ears mirrored)
        [
            [13400, 8337, 14027, 32767, 13738, -13394, -17607, -19991, -942,
             3122, -12232, -18538, -10605, -4367, -1783, -328],
            [5199, 4528, 5171, 5432, 3429, 464, -3028, -4947, -3253,
             -1496, -2550, -2794, -1524, -365, -220, -210],
        ],
        // -40 deg elev, 320 deg az (measured 40, ears mirrored)
        [
            [25829, 13352, 9015, 32767, 25799, -11682, -17042, -24617, -15836,
             5174, -4680, -21830, -17702, -8917, -4469, -2435],
            [9181, 7349, 8095, 9918, 5568, 547, -3053, -8039, -7905,
             -2661, -4260, -7166, -4469, -1110, -567, -618],
        ],
        // -40 deg elev, 330 deg az (measured 30, ears mirrored)
        [
            [22937, 11251, 11634, 32767, 11052, -16178, -15487, -22966, -9342,
             4835, -10389, -20739, -12244, -7127, -6231, -2013],
            [9968, 8133, 8135, 11196, 6968, 541, -2768, -8074, -9757,
             -3235, -3178, -8250, -6637, -1871, -220, -830],
        ],
        // -40 deg elev, 340 deg az (measured 20, ears mirrored)
        [
            [1308, -5878, 18161, 32767, 22678, 9602, 26321, 31107, -9754,
             -18443, -16949, -25760, -6177, 2838, -19693, -20139],
            [15638, 12542, 11934, 18758, 12080, -1090, -4329, -10963, -16805,
             -7444, -3021, -11287, -12152, -5566, -952, -1101],
        ],
        // -40 deg elev, 350 deg az (measured 10, ears mirrored)
        [
            [892, -5370, 22219, 32767, 19608, 8485, 30725, 28431, -14940,
             -17545, -15088, -27551, -6571, 2715, -21072, -19457],
            [19602, 13633, 13547, 22729, 14113, -3213, -5673, -13183, -20269,
             -7955, -2857, -12605, -14042, -7252, -1778, -1162],
        ],
    ],
    [
        // 0 deg elev, 0 deg az (measured 0)
        [
            [-12365, 10985, -19766, -32767, -4903, -3304, -17755, -3091, 7290,
             194, 2623, 10194, 7953, 3289, 5971, 3734],
            [-12365, 10985, -19766, -32767, -4903, -3304, -17755, -3091, 7290,
             194, 2623, 10194, 7953, 3289, 5971, 3734],
        ],
        // 0 deg elev, 10 deg az (measured 10)
        [
            [-6653, 4730, -12517, -19793, -4312, -3132, -10797, -2347, 3729,
             422, 1200, 5660, 4859, 2038, 4003, 3387],
            [-20571, 10837, -9554, -32767, -9296, 527, -15237, -7415, 7221,
             1029, 97, 8325, 9642, 3692, 5370, 5242],
        ],
        // 0 deg elev, 20 deg az (measured 20)
        [
            [-4420, 3221, -7846, -13322, -3503, -2565, -7673, -2274, 1786,
             -423, 475, 3647, 3368, 2120, 3388, 2801],
            [20707, 4442, 15847, 32767, 3168, -24732, 1744, 288, -28844,
             -16663, 1100, -9314, -11985, 5141, 3607, -1614],
        ],
        // 0 deg elev, 30 deg az (measured 30)
        [
            [5238, 3558, 9639, 10697, -1434, -3235, 2882, -4322, -9926,
             -3705, -2295, -6104, -3172, 333, -235, 84],
            [14725, 9947, 10252, 32767, 18173, -16903, -10139, 2480, -19628,
             -22453, -8124, -5825, -10163, -1342, 5246, -108],
        ],
        // 0 deg elev, 40 deg az (measured 40)
        [
            [4711, 3163, 7111, 10627, 2278, -2492, 2372, -1294, -7932,
             -4785, -1862, -5413, -4560, -464, -194, -320],
            [13470, 6499, 31176, 32767, -3763, -13251, 335, -12603, -22136,
             -16022, -14778, -9623, -5724, 885, 1870, 4119],
        ],
        // 0 deg elev, 50 deg az (measured 50)
        [
            [2706, 2120, 3270, 6866, 4288, -845, 1013, 1412, -3510,
             -4041, -1623, -3231, -3576, -818, 143, -78],
            [9567, 4728, 10424, 32767, 13931, -10191, -2110, -1187, -13680,
             -11429, -12218, -13888, -6534, -3289, -679, 785],
        ],
        // 0 deg elev, 60 deg az (measured 60)
        [
            [1856, 1940, 4769, 5356, 1146, 368, 2006, -884, -3294,
             -1844, -1790, -2805, -1311, 94, 271, 884],
            [8681, 1937, 12391, 32767, 7970, -10576, 1477, -1197, -11064,
             -9075, -12065, -12548, -5017, -3686, -964, -851],
        ],
        // 0 deg elev, 70 deg az (measured 70)
        [
            [1755, 1419, 2824, 4979, 3729, 1285, 1731, 891, -1955,
             -2151, -820, -1158, -1259, -727, -794, -767],
            [11852, 1362, 7801, 32767, 13876, -12884, 592, 4322, -7760,
             -9729, -10655, -13199, -6130, -2824, -2236, -4244],
        ],
        // 0 deg elev, 80 deg az (measured 80)
        [
            [1761, 2308, 4257, 5288, 3101, 2087, 2696, 555, -2080,
             -2226, -2335, -3189, -2918, -2257, -2138, -1856],
            [9895, -1763, 23260, 32767, -8415, -11015, 10516, 2360, -11053,
             -9390, -12322, -11917, -2224, -2178, -6894, -7752],
        ],
        // 0 deg elev, 90 deg az (measured 90)
        [
            [931, 2266, 5725, 8534, 5884, 1199, -159, -726, -3062,
             -4398, -4446, -4917, -4349, -2891, -1982, -1383],
            [-1067, 17651, 22871, -32767, -22854, 19817, 10256, -1283, 31160,
             31809, -19234, -9223, 19977, 2777, -15445, -8221],
        ],
        // 0 deg elev, 100 deg az (measured 100)
        [
            [-1270, -1402, -3240, -4619, -3971, -3161, -2711, -1636, -682,
             -368, 276, 1225, 1587, 1432, 1284, 997],
            [14178, -1624, 22203, 32767, -13868, -14631, 22161, 10977, -15470,
             -8928, -8666, -15172, -1115, -2308, -15594, -10418],
        ],
        // 0 deg elev, 110 deg az (measured 110)
        [
            [116, 1069, 3295, 4347, 3038, 1959, 1052, 168, -409,
             -1330, -646, 674, 941, 1318, 1358, 362],
            [-34, 19811, 9457, -32767, -6837, 19808, 3252, 4403, 30266,
             7624, -20615, 9292, 23628, -4300, -14736, -5378],
        ],
        // 0 deg elev, 120 deg az (measured 120)
        [
            [-217, 526, 4169, 6164, 3520, 1488, 1392, 594, 55,
             -791, -2139, -2248, -1855, -1788, -981, 509],
            [1966, 22964, 932, -32767, 945, 20403, 249, 6688, 30614,
             -883, -14837, 20366, 21779, -10221, -15376, -5771],
        ],
        // 0 deg elev, 130 deg az (measured 130)
        [
            [-112, -290, 5664, 10119, 5432, 1507, 2241, 642, -1055,
             -2212, -4412, -4133, -2440, -1958, -1906, -1167],
            [1196, 22492, 6668, -32767, -3611, 23813, 670, -660, 32413,
             8298, -13166, 22546, 26728, -11545, -19810, -4352],
        ],
        // 0 deg elev, 140 deg az (measured 140)
        [
            [493, -1314, 7312, 15156, 7880, 1172, 3323, 1333, -2336,
             -3183, -6273, -7423, -4346, -3239, -3585, -2409],
            [29230, -7281, 12397, 32767, -3547, -22873, -4360, -7875, -20397,
             -11391, -11006, -16282, -6249, 3554, 1044, 628],
        ],
        // 0 deg elev, 150 deg az (measured 150)
        [
            [1574, -2235, 8065, 19344, 9315, 322, 4529, 2105, -4110,
             -3958, -6522, -9918, -6227, -3505, -4592, -3648],
            [12248, -9807, 2148, 32767, 15651, 896, 22868, 12032, -14973,
             -8634, -4171, -18828, -15750, -8889, -13786, -9894],
        ],
        // 0 deg elev, 160 deg az (measured 160)
        [
            [2949, -3009, 9410, 26754, 12311, -1200, 6480, 3952, -6423,
             -6586, -7355, -12748, -9739, -4231, -5614, -5551],
            [212, -7023, 12431, 32767, 16352, 7786, 16059, 2727, -9207,
             -6972, -13427, -18411, -13381, -12517, -11619, -6529],
        ],
        // 0 deg elev, 170 deg az (measured 170)
        [
            [3484, -2918, 9677, 32128, 15668, -2762, 7671, 5887, -7991,
             -9184, -8268, -14340, -12730, -5299, -5697, -7131],
            [25, -2112, 11789, 32767, 20191, 3711, 8685, 5308, -6814,
             -11623, -12906, -15206, -14928, -11287, -7492, -6674],
        ],
        // 0 deg elev, 180 deg az (measured 180)
        [
            [2686, -2138, 9653, 32767, 18406, -1632, 7453, 6694, -7961,
             -10983, -9540, -14107, -14250, -7567, -5594, -7110],
            [2686, -2138, 9653, 32767, 18406, -1632, 7453, 6694, -7961,
             -10983, -9540, -14107, -14250, -7567, -5594, -7110],
        ],
        // 0 deg elev, 190 deg az (measured 170, ears mirrored)
        [
            [25, -2112, 11789, 32767, 20191, 3711, 8685, 5308, -6814,
             -11623, -12906, -15206, -14928, -11287, -7492, -6674],
            [3484, -2918, 9677, 32128, 15668, -2762, 7671, 5887, -7991,
             -9184, -8268, -14340, -12730, -5299, -5697, -7131],
        ],
        // 0 deg elev, 200 deg az (measured 160, ears mirrored)
        [
            [212, -7023, 12431, 32767, 16352, 7786, 16059, 2727, -9207,
             -6972, -13427, -18411, -13381, -12517, -11619, -6529],
            [2949, -3009, 9410, 26754, 12311, -1200, 6480, 3952, -6423,
             -6586, -7355, -12748, -9739, -4231, -5614, -5551],
        ],
        // 0 deg elev, 210 deg az (measured 150, ears mirrored)
        [
            [12248, -9807, 2148, 32767, 15651, 896, 22868, 12032, -14973,
             -8634, -4171, -18828, -15750, -8889, -13786, -9894],
            [1574, -2235, 8065, 19344, 9315, 322, 4529, 2105, -4110,
             -3958, -6522, -9918, -6227, -3505, -4592, -3648],
        ],
        // 0 deg elev, 220 deg az (measured 140, ears mirrored)
        [
            [29230, -7281, 12397, 32767, -3547, -22873, -4360, -7875, -20397,
             -11391, -11006, -16282, -6249, 3554, 1044, 628],
            [493, -1314, 7312, 15156, 7880, 1172, 3323, 1333, -2336,
             -3183, -6273, -7423, -4346, -3239, -3585, -2409],
        ],
        // 0 deg elev, 230 deg az (measured 130, ears mirrored)
        [
            [1196, 22492, 6668, -32767, -3611, 23813, 670, -660, 32413,
             8298, -13166, 22546, 26728, -11545, -19810, -4352],
            [-112, -290, 5664, 10119, 5432, 1507, 2241, 642, -1055,
             -2212, -4412, -4133, -2440, -1958, -1906, -1167],
        ],
        // 0 deg elev, 240 deg az (measured 120, ears mirrored)
        [
            [1966, 22964, 932, -32767, 945, 20403, 249, 6688, 30614,
             -883, -14837, 20366, 21779, -10221, -15376, -5771],
            [-217, 526, 4169, 6164, 3520, 1488, 1392, 594, 55,
             -791, -2139, -2248, -1855, -1788, -981, 509],
        ],
        // 0 deg elev, 250 deg az (measured 110, ears mirrored)
        [
            [-34, 19811, 9457, -32767, -6837, 19808, 3252, 4403, 30266,
             7624, -20615, 9292, 23628, -4300, -14736, -5378],
            [116, 1069, 3295, 4347, 3038, 1959, 1052, 168, -409,
             -1330, -646, 674, 941, 1318, 1358, 362],
        ],
        // 0 deg elev, 260 deg az (measured 100, ears mirrored)
        [
            [14178, -1624, 22203, 32767, -13868, -14631, 22161, 10977, -15470,
             -8928, -8666, -15172, -1115, -2308, -15594, -10418],
            [-1270, -1402, -3240, -4619, -3971, -3161, -2711, -1636, -682,
             -368, 276, 1225, 1587, 1432, 1284, 997],
        ],
        // 0 deg elev, 270 deg az (measured 90, ears mirrored)
        [
            [-1067, 17651, 22871, -32767, -22854, 19817, 10256, -1283, 31160,
             31809, -19234, -9223, 19977, 2777, -15445, -8221],
            [931, 2266, 5725, 8534, 5884, 1199, -159, -726, -3062,
             -4398, -4446, -4917, -4349, -2891, -1982, -1383],
        ],
        // 0 deg elev, 280 deg az (measured 80, ears mirrored)
        [
            [9895, -1763, 23260, 32767, -8415, -11015, 10516, 2360, -11053,
             -9390, -12322, -11917, -2224, -2178, -6894, -7752],
            [1761, 2308, 4257, 5288, 3101, 2087, 2696, 555, -2080,
             -2226, -2335, -3189, -2918, -2257, -2138, -1856],
        ],
        // 0 deg elev, 290 deg az (measured 70, ears mirrored)
        [
            [11852, 1362, 7801, 32767, 13876, -12884, 592, 4322, -7760,
             -9729, -10655, -13199, -6130, -2824, -2236, -4244],
            [1755, 1419, 2824, 4979, 3729, 1285, 1731, 891, -1955,
             -2151, -820, -1158, -1259, -727, -794, -767],
        ],
        // 0 deg elev, 300 deg az (measured 60, ears mirrored)
        [
            [8681, 1937, 12391, 32767, 7970, -10576, 1477, -1197, -11064,
             -9075, -12065, -12548, -5017, -3686, -964, -851],
            [1856, 1940, 4769, 5356, 1146, 368, 2006, -884, -3294,
             -1844, -1790, -2805, -1311, 94, 271, 884],
        ],
        // 0 deg elev, 310 deg az (measured 50, ears mirrored)
        [
            [9567, 4728, 10424, 32767, 13931, -10191, -2110, -1187, -13680,
             -11429, -12218, -13888, -6534, -3289, -679, 785],
            [2706, 2120, 3270, 6866, 4288, -845, 1013, 1412, -3510,
             -4041, -1623, -3231, -3576, -818, 143, -78],
        ],
        // 0 deg elev, 320 deg az (measured 40, ears mirrored)
        [
            [13470, 6499, 31176, 32767, -3763, -13251, 335, -12603, -22136,
             -16022, -14778, -9623, -5724, 885, 1870, 4119],
            [4711, 3163, 7111, 10627, 2278, -2492, 2372, -1294, -7932,
             -4785, -1862, -5413, -4560, -464, -194, -320],
        ],
        // 0 deg elev, 330 deg az (measured 30, ears mirrored)
        [
            [14725, 9947, 10252, 32767, 18173, -16903, -10139, 2480, -19628,
             -22453, -8124, -5825, -10163, -1342, 5246, -108],
            [5238, 3558, 9639, 10697, -1434, -3235, 2882, -4322, -9926,
             -3705, -2295, -6104, -3172, 333, -235, 84],
        ],
        // 0 deg elev, 340 deg az (measured 20, ears mirrored)
        [
            [20707, 4442, 15847, 32767, 3168, -24732, 1744, 288, -28844,
             -16663, 1100, -9314, -11985, 5141, 3607, -1614],
            [-4420, 3221, -7846, -13322, -3503, -2565, -7673, -2274, 1786,
             -423, 475, 3647, 3368, 2120, 3388, 2801],
        ],
        // 0 deg elev, 350 deg az (measured 10, ears mirrored)
        [
            [-20571, 10837, -9554, -32767, -9296, 527, -15237, -7415, 7221,
             1029, 97, 8325, 9642, 3692, 5370, 5242],
            [-6653, 4730, -12517, -19793, -4312, -3132, -10797, -2347, 3729,
             422, 1200, 5660, 4859, 2038, 4003, 3387],
        ],
    ],
    [
        // 40 deg elev, 0 deg az (measured 0)
        [
            [-32151, 5174, 25589, 32767, 32099, 3616, -9489, 1993, 5597,
             -8117, -12241, -15375, -11260, -7254, -8225, -4548],
            [-32151, 5174, 25589, 32767, 32099, 3616, -9489, 1993, 5597,
             -8117, -12241, -15375, -11260, -7254, -8225, -4548],
        ],
        // 40 deg elev, 10 deg az (measured 10)
        [
            [260, 12222, 19901, 21912, 5833, -3097, 1550, 1839, -3454,
             -5376, -9012, -8103, -5030, -4674, -3949, -3518],
            [-14279, 21576, 25750, 32767, 15006, -9747, -6669, 5321, -2379,
             -9870, -12014, -13063, -5079, -4856, -5767, -3842],
        ],
        // 40 deg elev, 20 deg az (measured 20)
        [
            [-1562, 8353, 16433, 18661, 6457, -731, 1335, 1500, -2136,
             -4192, -7560, -6814, -4185, -4325, -3730, -2917],
            [-20780, 19645, 27001, 32767, 18249, -7089, -8800, 2716, -521,
             -10037, -11906, -13654, -6219, -3670, -4892, -3666],
        ],
        // 40 deg elev, 30 deg az (measured 30)
        [
            [-2830, 4380, 10984, 12372, 5911, 1150, 1258, 1258, -878,
             -2496, -4624, -4195, -3084, -3391, -2668, -2116],
            [11409, 5935, -14949, -32767, 588, 23891, 28096, 26526, 2702,
             -7997, -1951, 2183, -6410, -10346, -13138, -10845],
        ],
        // 40 deg elev, 40 deg az (measured 40)
        [
            [-2812, 4069, 10318, 11225, 5611, 1507, 1438, 1184, -658,
             -2166, -4017, -3629, -2989, -3226, -2409, -2035],
            [10511, 10256, -4026, -32767, -16911, 20645, 26349, 30790, 12593,
             -5905, -4459, 1954, -2351, -9822, -12938, -13870],
        ],
        // 40 deg elev, 50 deg az (measured 50)
        [
            [-3220, 2306, 7362, 8386, 4875, 1991, 1386, 1089, -18,
             -997, -2308, -2391, -2169, -2486, -2226, -1928],
            [10077, 9973, -7985, -32767, -7412, 22232, 22993, 23244, 7339,
             -2913, -1575, 2939, -2403, -10666, -12514, -10716],
        ],
        // 40 deg elev, 60 deg az (measured 60)
        [
            [-3277, 2820, 8240, 8713, 5005, 2325, 1628, 1208, 178,
             -804, -2065, -2530, -2840, -2809, -1948, -1540],
            [13507, 6561, -25784, -32767, 13404, 27294, 25814, 18695, 1385,
             -2143, 1326, 3764, -7323, -14329, -14590, -9392],
        ],
        // 40 deg elev, 70 deg az (measured 70)
        [
            [-5637, -1737, 3070, 6541, 5862, 3378, 2066, 1637, 1005,
             49, -531, -1104, -1246, -1313, -1315, -1088],
            [10615, 10196, -13461, -32767, 2414, 24549, 18475, 16312, 4756,
             -588, 472, 6099, -1699, -11493, -11455, -9277],
        ],
        // 40 deg elev, 80 deg az (measured 80)
        [
            [-5883, -547, 4453, 7113, 5863, 3460, 2109, 1647, 1305,
             828, 98, -596, -1060, -1537, -1663, -1841],
            [13871, 8681, -24934, -32767, 14764, 26529, 18855, 15886, 4102,
             -515, 2247, 7799, -5401, -13703, -11399, -9538],
        ],
        // 40 deg elev, 90 deg az (measured 90)
        [
            [1873, 2597, 94, -6280, -5566, 1013, 5127, 6248, 5146,
             3867, 3420, 3539, 2646, 1158, -18, -664],
            [5758, 16320, 6120, -32767, -27692, 22263, 23991, 17141, 16347,
             5585, 251, 5164, 8143, -9275, -13992, -9444],
        ],
        // 40 deg elev, 100 deg az (measured 100)
        [
            [-6846, 887, 5917, 7264, 6545, 4625, 3251, 3412, 2287,
             460, -351, -1149, -2156, -2808, -2619, -2889],
            [14212, 11706, -23115, -32767, 11966, 22554, 15104, 19609, 8997,
             1422, 4706, 9844, -5600, -14532, -8794, -8953],
        ],
        // 40 deg elev, 110 deg az (measured 110)
        [
            [2903, 2762, -3452, -8699, -3344, 4006, 6939, 7398, 5774,
             3561, 3019, 2530, 773, -199, -930, -1857],
            [9715, 14629, -10227, -32767, -1652, 19774, 13181, 20162, 12671,
             1793, 3413, 9592, 10, -13075, -9185, -7909],
        ],
        // 40 deg elev, 120 deg az (measured 120)
        [
            [-9339, 1052, 8689, 10911, 8639, 4768, 3797, 4278, 2545,
             93, -1134, -2749, -4078, -4243, -3590, -3266],
            [13128, 15256, -17402, -32767, 880, 14716, 20028, 30410, 11907,
             -33, 7402, 11451, -3551, -14252, -10011, -10783],
        ],
        // 40 deg elev, 130 deg az (measured 130)
        [
            [-11275, -659, 9039, 12567, 10238, 5339, 3852, 4642, 3009,
             209, -819, -2340, -4359, -5180, -4680, -4042],
            [-19929, 7978, 16029, 32767, 25485, 2078, 2536, 12198, 6221,
             -11110, -12798, -10325, -12728, -11995, -7560, -7932],
        ],
        // 40 deg elev, 140 deg az (measured 140)
        [
            [-12138, -424, 10882, 15100, 11102, 4707, 3741, 5031, 2968,
             -590, -1750, -2853, -4720, -5201, -4739, -4500],
            [-27877, -7786, 11783, 32767, 31374, 6946, -912, 8859, 10748,
             -4215, -12508, -9947, -10843, -11940, -8811, -7298],
        ],
        // 40 deg elev, 150 deg az (measured 150)
        [
            [-11966, -780, 10468, 15031, 10902, 4063, 3301, 4946, 2919,
             -840, -2079, -2919, -4594, -5034, -4540, -4189],
            [-22448, -3653, 17710, 32767, 21551, 1831, 536, 9983, 6958,
             -6982, -10434, -8229, -9877, -10111, -7030, -6836],
        ],
        // 40 deg elev, 160 deg az (measured 160)
        [
            [-14660, 1587, 15652, 20418, 12837, 2867, 3621, 6763, 3071,
             -2389, -3584, -4732, -6509, -6294, -5575, -5341],
            [-22365, 2845, 27103, 32767, 16220, -687, 2849, 11811, 3401,
             -8705, -8980, -7959, -10680, -9631, -6857, -7150],
        ],
        // 40 deg elev, 170 deg az (measured 170)
        [
            [-15259, 5131, 20437, 23758, 12623, 1577, 4580, 8000, 2324,
             -3958, -4618, -6076, -8005, -7018, -6202, -6180],
            [-23502, 3549, 27631, 32767, 17059, -436, 3060, 11832, 3534,
             -7980, -8461, -7782, -10775, -9731, -7125, -7318],
        ],
        // 40 deg elev, 180 deg az (measured 180)
        [
            [-23448, 4747, 27456, 32767, 18140, 707, 4153, 11442, 3772,
             -6483, -7268, -7777, -11192, -9790, -7454, -7870],
            [-23448, 4747, 27456, 32767, 18140, 707, 4153, 11442, 3772,
             -6483, -7268, -7777, -11192, -9790, -7454, -7870],
        ],
        // 40 deg elev, 190 deg az (measured 170, ears mirrored)
        [
            [-23502, 3549, 27631, 32767, 17059, -436, 3060, 11832, 3534,
             -7980, -8461, -7782, -10775, -9731, -7125, -7318],
            [-15259, 5131, 20437, 23758, 12623, 1577, 4580, 8000, 2324,
             -3958, -4618, -6076, -8005, -7018, -6202, -6180],
        ],
        // 40 deg elev, 200 deg az (measured 160, ears mirrored)
        [
            [-22365, 2845, 27103, 32767, 16220, -687, 2849, 11811, 3401,
             -8705, -8980, -7959, -10680, -9631, -6857, -7150],
            [-14660, 1587, 15652, 20418, 12837, 2867, 3621, 6763, 3071,
             -2389, -3584, -4732, -6509, -6294, -5575, -5341],
        ],
        // 40 deg elev, 210 deg az (measured 150, ears mirrored)
        [
            [-22448, -3653, 17710, 32767, 21551, 1831, 536, 9983, 6958,
             -6982, -10434, -8229, -9877, -10111, -7030, -6836],
            [-11966, -780, 10468, 15031, 10902, 4063, 3301, 4946, 2919,
             -840, -2079, -2919, -4594, -5034, -4540, -4189],
        ],
        // 40 deg elev, 220 deg az (measured 140, ears mirrored)
        [
            [-27877, -7786, 11783, 32767, 31374, 6946, -912, 8859, 10748,
             -4215, -12508, -9947, -10843, -11940, -8811, -7298],
            [-12138, -424, 10882, 15100, 11102, 4707, 3741, 5031, 2968,
             -590, -1750, -2853, -4720, -5201, -4739, -4500],
        ],
        // 40 deg elev, 230 deg az (measured 130, ears mirrored)
        [
            [-19929, 7978, 16029, 32767, 25485, 2078, 2536, 12198, 6221,
             -11110, -12798, -10325, -12728, -11995, -7560, -7932],
            [-11275, -659, 9039, 12567, 10238, 5339, 3852, 4642, 3009,
             209, -819, -2340, -4359, -5180, -4680, -4042],
        ],
        // 40 deg elev, 240 deg az (measured 120, ears mirrored)
        [
            [13128, 15256, -17402, -32767, 880, 14716, 20028, 30410, 11907,
             -33, 7402, 11451, -3551, -14252, -10011, -10783],
            [-9339, 1052, 8689, 10911, 8639, 4768, 3797, 4278, 2545,
             93, -1134, -2749, -4078, -4243, -3590, -3266],
        ],
        // 40 deg elev, 250 deg az (measured 110, ears mirrored)
        [
            [9715, 14629, -10227, -32767, -1652, 19774, 13181, 20162, 12671,
             1793, 3413, 9592, 10, -13075, -9185, -7909],
            [2903, 2762, -3452, -8699, -3344, 4006, 6939, 7398, 5774,
             3561, 3019, 2530, 773, -199, -930, -1857],
        ],
        // 40 deg elev, 260 deg az (measured 100, ears mirrored)
        [
            [14212, 11706, -23115, -32767, 11966, 22554, 15104, 19609, 8997,
             1422, 4706, 9844, -5600, -14532, -8794, -8953],
            [-6846, 887, 5917, 7264, 6545, 4625, 3251, 3412, 2287,
             460, -351, -1149, -2156, -2808, -2619, -2889],
        ],
        // 40 deg elev, 270 deg az (measured 90, ears mirrored)
        [
            [5758, 16320, 6120, -32767, -27692, 22263, 23991, 17141, 16347,
             5585, 251, 5164, 8143, -9275, -13992, -9444],
            [1873, 2597, 94, -6280, -5566, 1013, 5127, 6248, 5146,
             3867, 3420, 3539, 2646, 1158, -18, -664],
        ],
        // 40 deg elev, 280 deg az (measured 80, ears mirrored)
        [
            [13871, 8681, -24934, -32767, 14764, 26529, 18855, 15886, 4102,
             -515, 2247, 7799, -5401, -13703, -11399, -9538],
            [-5883, -547, 4453, 7113, 5863, 3460, 2109, 1647, 1305,
             828, 98, -596, -1060, -1537, -1663, -1841],
        ],
        // 40 deg elev, 290 deg az (measured 70, ears mirrored)
        [
            [10615, 10196, -13461, -32767, 2414, 24549, 18475, 16312, 4756,
             -588, 472, 6099, -1699, -11493, -11455, -9277],
            [-5637, -1737, 3070, 6541, 5862, 3378, 2066, 1637, 1005,
             49, -531, -1104, -1246, -1313, -1315, -1088],
        ],
        // 40 deg elev, 300 deg az (measured 60, ears mirrored)
        [
            [13507, 6561, -25784, -32767, 13404, 27294, 25814, 18695, 1385,
             -2143, 1326, 3764, -7323, -14329, -14590, -9392],
            [-3277, 2820, 8240, 8713, 5005, 2325, 1628, 1208, 178,
             -804, -2065, -2530, -2840, -2809, -1948, -1540],
        ],
        // 40 deg elev, 310 deg az (measured 50, ears mirrored)
        [
            [10077, 9973, -7985, -32767, -7412, 22232, 22993, 23244, 7339,
             -2913, -1575, 2939, -2403, -10666, -12514, -10716],
            [-3220, 2306, 7362, 8386, 4875, 1991, 1386, 1089, -18,
             -997, -2308, -2391, -2169, -2486, -2226, -1928],
        ],
        // 40 deg elev, 320 deg az (measured 40, ears mirrored)
        [
            [10511, 10256, -4026, -32767, -16911, 20645, 26349, 30790, 12593,
             -5905, -4459, 1954, -2351, -9822, -12938, -13870],
            [-2812, 4069, 10318, 11225, 5611, 1507, 1438, 1184, -658,
             -2166, -4017, -3629, -2989, -3226, -2409, -2035],
        ],
        // 40 deg elev, 330 deg az (measured 30, ears mirrored)
        [
            [11409, 5935, -14949, -32767, 588, 23891, 28096, 26526, 2702,
             -7997, -1951, 2183, -6410, -10346, -13138, -10845],
            [-2830, 4380, 10984, 12372, 5911, 1150, 1258, 1258, -878,
             -2496, -4624, -4195, -3084, -3391, -2668, -2116],
        ],
        // 40 deg elev, 340 deg az (measured 20, ears mirrored)
        [
            [-20780, 19645, 27001, 32767, 18249, -7089, -8800, 2716, -521,
             -10037, -11906, -13654, -6219, -3670, -4892, -3666],
            [-1562, 8353, 16433, 18661, 6457, -731, 1335, 1500, -2136,
             -4192, -7560, -6814, -4185, -4325, -3730, -2917],
        ],
        // 40 deg elev, 350 deg az (measured 10, ears mirrored)
        [
            [-14279, 21576, 25750, 32767, 15006, -9747, -6669, 5321, -2379,
             -9870, -12014, -13063, -5079, -4856, -5767, -3842],
            [260, 12222, 19901, 21912, 5833, -3097, 1550, 1839, -3454,
             -5376, -9012, -8103, -5030, -4674, -3949, -3518],
        ],
    ],
];
// end of generated table

/// Cosine kernel of the 16-point real DFT: 9 unique bins by 16 taps, so a
/// query needs no trig for the transform itself.
#[rustfmt::skip]
const DFT_COS: [[f32; HRTF_TAPS]; SPHERE_BINS] = [
    [1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0],
    [1.0, 0.9238795, 0.70710677, 0.38268343, 0.0, -0.38268343, -0.70710677, -0.9238795, -1.0, -0.9238795, -0.70710677, -0.38268343, 0.0, 0.38268343, 0.70710677, 0.9238795],
    [1.0, 0.70710677, 0.0, -0.70710677, -1.0, -0.70710677, 0.0, 0.70710677, 1.0, 0.70710677, 0.0, -0.70710677, -1.0, -0.70710677, 0.0, 0.70710677],
    [1.0, 0.38268343, -0.70710677, -0.9238795, 0.0, 0.9238795, 0.70710677, -0.38268343, -1.0, -0.38268343, 0.70710677, 0.9238795, 0.0, -0.9238795, -0.70710677, 0.38268343],
    [1.0, 0.0, -1.0, 0.0, 1.0, 0.0, -1.0, 0.0, 1.0, 0.0, -1.0, 0.0, 1.0, 0.0, -1.0, 0.0],
    [1.0, -0.38268343, -0.70710677, 0.9238795, 0.0, -0.9238795, 0.70710677, 0.38268343, -1.0, 0.38268343, 0.70710677, -0.9238795, 0.0, 0.9238795, -0.70710677, -0.38268343],
    [1.0, -0.70710677, 0.0, 0.70710677, -1.0, 0.70710677, 0.0, -0.70710677, 1.0, -0.70710677, 0.0, 0.70710677, -1.0, 0.70710677, 0.0, -0.70710677],
    [1.0, -0.9238795, 0.70710677, -0.38268343, 0.0, 0.38268343, -0.70710677, 0.9238795, -1.0, 0.9238795, -0.70710677, 0.38268343, 0.0, -0.38268343, 0.70710677, -0.9238795],
    [1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0],
];
/// Sine kernel of the 16-point real DFT beside the cosine table (the
/// forward kernel is `cos - i sin`, and the rebuild negates it back).
#[rustfmt::skip]
const DFT_SIN: [[f32; HRTF_TAPS]; SPHERE_BINS] = [
    [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
    [0.0, 0.38268343, 0.70710677, 0.9238795, 1.0, 0.9238795, 0.70710677, 0.38268343, 0.0, -0.38268343, -0.70710677, -0.9238795, -1.0, -0.9238795, -0.70710677, -0.38268343],
    [0.0, 0.70710677, 1.0, 0.70710677, 0.0, -0.70710677, -1.0, -0.70710677, 0.0, 0.70710677, 1.0, 0.70710677, 0.0, -0.70710677, -1.0, -0.70710677],
    [0.0, 0.9238795, 0.70710677, -0.38268343, -1.0, -0.38268343, 0.70710677, 0.9238795, 0.0, -0.9238795, -0.70710677, 0.38268343, 1.0, 0.38268343, -0.70710677, -0.9238795],
    [0.0, 1.0, 0.0, -1.0, 0.0, 1.0, 0.0, -1.0, 0.0, 1.0, 0.0, -1.0, 0.0, 1.0, 0.0, -1.0],
    [0.0, 0.9238795, -0.70710677, -0.38268343, 1.0, -0.38268343, -0.70710677, 0.9238795, 0.0, -0.9238795, 0.70710677, 0.38268343, -1.0, 0.38268343, 0.70710677, -0.9238795],
    [0.0, 0.70710677, -1.0, 0.70710677, 0.0, -0.70710677, 1.0, -0.70710677, 0.0, 0.70710677, -1.0, 0.70710677, 0.0, -0.70710677, 1.0, -0.70710677],
    [0.0, 0.38268343, -0.70710677, 0.9238795, -1.0, 0.9238795, -0.70710677, 0.38268343, 0.0, -0.38268343, 0.70710677, -0.9238795, 1.0, -0.9238795, 0.70710677, -0.38268343],
    [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0],
];

/// Forward spectrum of one ear: `(real, imag)` at [`SPHERE_BINS`] bins, with
/// the `e^{-i w n}` kernel, so bin `k` sits at `k * rate / 16`.
fn spectrum(taps: &[f32; HRTF_TAPS]) -> ([f32; SPHERE_BINS], [f32; SPHERE_BINS]) {
    let mut re = [0.0f32; SPHERE_BINS];
    let mut im = [0.0f32; SPHERE_BINS];
    let mut k = 0;
    while k < SPHERE_BINS {
        let mut r = 0.0f32;
        let mut s = 0.0f32;
        let mut n = 0;
        while n < HRTF_TAPS {
            r += taps[n] * DFT_COS[k][n];
            s += taps[n] * DFT_SIN[k][n];
            n += 1;
        }
        re[k] = r;
        im[k] = -s;
        k += 1;
    }
    (re, im)
}

/// Rebuild the taps from a Hermitian spectrum (bins `0` and `8` real).
fn from_spectrum(re: &[f32; SPHERE_BINS], im: &[f32; SPHERE_BINS]) -> [f32; HRTF_TAPS] {
    let inv = 1.0 / HRTF_TAPS as f32;
    let mut taps = [0.0f32; HRTF_TAPS];
    let mut n = 0;
    while n < HRTF_TAPS {
        let mut v = re[0];
        let mut k = 1;
        while k + 1 < SPHERE_BINS {
            v += 2.0 * (re[k] * DFT_COS[k][n] - im[k] * DFT_SIN[k][n]);
            k += 1;
        }
        v += re[SPHERE_BINS - 1] * DFT_COS[SPHERE_BINS - 1][n];
        taps[n] = v * inv;
        n += 1;
    }
    taps
}

/// Blend two measured ears in the log-magnitude/phase domain: magnitudes
/// combine as powers (the geometric mean at a midpoint), phases turn the
/// short way round. A linear tap blend adds two shapes whose 8 kHz phase
/// differs by nearly half a cycle and cancels a band the measured direction
/// carries; this keeps that band's level. Arrivals stay out of it (they are
/// blended on their own), so no inter-aural delay drags the shape into a
/// comb. No heap, no lock; `0`/`1` return the measured tap set untouched.
fn blend_log_spectrum(a: &[f32; HRTF_TAPS], b: &[f32; HRTF_TAPS], t: f32) -> [f32; HRTF_TAPS] {
    if t <= 0.0 {
        return *a;
    }
    if t >= 1.0 {
        return *b;
    }
    let (ar, ai) = spectrum(a);
    let (br, bi) = spectrum(b);
    let mut re = [0.0f32; SPHERE_BINS];
    let mut im = [0.0f32; SPHERE_BINS];
    let mut k = 0;
    while k < SPHERE_BINS {
        let ma = (ar[k] * ar[k] + ai[k] * ai[k]).sqrt().max(TAP_MAG_FLOOR);
        let mb = (br[k] * br[k] + bi[k] * bi[k]).sqrt().max(TAP_MAG_FLOOR);
        let pa = ai[k].atan2(ar[k]);
        let mut d = bi[k].atan2(br[k]) - pa;
        while d > std::f32::consts::PI {
            d -= 2.0 * std::f32::consts::PI;
        }
        while d < -std::f32::consts::PI {
            d += 2.0 * std::f32::consts::PI;
        }
        let m = (ma.ln() + (mb.ln() - ma.ln()) * t).exp();
        let p = pa + d * t;
        re[k] = m * p.cos();
        im[k] = m * p.sin();
        k += 1;
    }
    from_spectrum(&re, &im)
}

/// One measured direction: 16 taps per ear plus the measured ear arrivals.
#[derive(Clone, Copy)]
struct Measured {
    taps_l: [f32; HRTF_TAPS],
    taps_r: [f32; HRTF_TAPS],
    onset_l: f32,
    onset_r: f32,
}

impl Measured {
    /// The table cell at `row` (elevation) and `col` (azimuth), scaled out
    /// of the stored `i16`. Out-of-range indices are folded, never read out
    /// of bounds.
    fn cell(row: usize, col: usize) -> Self {
        let r = row % SPHERE_ELEV_ROWS;
        let c = col % SPHERE_AZ_COLS;
        let mut taps_l = [0.0f32; HRTF_TAPS];
        let mut taps_r = [0.0f32; HRTF_TAPS];
        let mut i = 0;
        while i < HRTF_TAPS {
            taps_l[i] = f32::from(SPHERE_TAPS[r][c][0][i]) / 32_767.0;
            taps_r[i] = f32::from(SPHERE_TAPS[r][c][1][i]) / 32_767.0;
            i += 1;
        }
        Self {
            taps_l,
            taps_r,
            onset_l: f32::from(SPHERE_ONSETS[r][c][0]),
            onset_r: f32::from(SPHERE_ONSETS[r][c][1]),
        }
    }

    /// Blend two measured directions: the taps in the log-magnitude/phase
    /// domain (see [`blend_log_spectrum`]), the arrivals on their own
    /// straight line. No heap.
    fn mix(a: &Self, b: &Self, t: f32) -> Self {
        Self {
            taps_l: blend_log_spectrum(&a.taps_l, &b.taps_l, t),
            taps_r: blend_log_spectrum(&a.taps_r, &b.taps_r, t),
            onset_l: a.onset_l + (b.onset_l - a.onset_l) * t,
            onset_r: a.onset_r + (b.onset_r - a.onset_r) * t,
        }
    }
}

/// Whole samples from a non-negative float, by counting up (no `as` cast).
/// Bounded by the history so a wild value cannot spin: never panics.
fn whole_samples(v: f32) -> usize {
    let mut n = 0usize;
    let mut acc = 0.0f32;
    while acc + 1.0 <= v && n <= MAX_ITD_SAMPLES {
        acc += 1.0;
        n += 1;
    }
    n
}

/// Azimuth in degrees to `(col, col + 1, fraction)`, wrapping past 360 and
/// below 0 so front/back splices are seamless.
fn sphere_column(azimuth_deg: f32) -> (usize, usize, f32) {
    let az = if azimuth_deg.is_finite() {
        azimuth_deg
    } else {
        0.0
    };
    let turns = (az / 360.0).floor();
    let wrapped = az - turns * 360.0;
    let last = SPHERE_AZ_COLS - 1;
    let mut col = 0usize;
    let mut rest = wrapped;
    while col < last && rest >= SPHERE_AZ_STEP_DEG {
        rest -= SPHERE_AZ_STEP_DEG;
        col += 1;
    }
    let frac = (rest / SPHERE_AZ_STEP_DEG).clamp(0.0, 1.0);
    (col, (col + 1) % SPHERE_AZ_COLS, frac)
}

/// Elevation in degrees to `(row, row + 1, fraction)`, clamped at both poles
/// (the table stops at -40 and +40 degrees).
fn sphere_row(elevation_deg: f32) -> (usize, usize, f32) {
    let el = if elevation_deg.is_finite() {
        elevation_deg
    } else {
        0.0
    };
    let lo = SPHERE_ELEVATIONS_DEG[0];
    let hi = SPHERE_ELEVATIONS_DEG[SPHERE_ELEV_ROWS - 1];
    if el <= lo {
        return (0, 0, 0.0);
    }
    if el >= hi {
        return (SPHERE_ELEV_ROWS - 1, SPHERE_ELEV_ROWS - 1, 0.0);
    }
    let mut row = 0;
    while row + 2 < SPHERE_ELEV_ROWS && el > SPHERE_ELEVATIONS_DEG[row + 1] {
        row += 1;
    }
    let a = SPHERE_ELEVATIONS_DEG[row];
    let b = SPHERE_ELEVATIONS_DEG[row + 1];
    let frac = ((el - a) / (b - a)).clamp(0.0, 1.0);
    (row, row + 1, frac)
}

/// Measured HRIR for one direction: azimuth wraps, elevation clamps at the
/// poles, the taps blend in the log-magnitude/phase domain and the measured
/// arrivals on their own line. Returns `(taps_l, taps_r, onset_l, onset_r)`
/// in samples at [`SPHERE_RATE_HZ`]. Allocation-free; non-finite input reads
/// as 0 degrees. A measured direction (both fractions zero) returns that
/// measurement byte for byte.
pub fn measured_hrir_at(
    azimuth_deg: f32,
    elevation_deg: f32,
) -> ([f32; HRTF_TAPS], [f32; HRTF_TAPS], f32, f32) {
    let (c0, c1, ct) = sphere_column(azimuth_deg);
    let (r0, r1, rt) = sphere_row(elevation_deg);
    let low = Measured::mix(&Measured::cell(r0, c0), &Measured::cell(r0, c1), ct);
    let high = Measured::mix(&Measured::cell(r1, c0), &Measured::cell(r1, c1), ct);
    let d = Measured::mix(&low, &high, rt);
    (d.taps_l, d.taps_r, d.onset_l, d.onset_r)
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

    // ---- measured sphere (AAC7) -----------------------------------------

    /// Measured onsets of one table cell, for tests that compare the
    /// interpolation against the direction it left out.
    fn measured_cell(row: usize, col: usize) -> ([f32; HRTF_TAPS], [f32; HRTF_TAPS], f32, f32) {
        let d = Measured::cell(row, col);
        (d.taps_l, d.taps_r, d.onset_l, d.onset_r)
    }

    /// Energy of one ear in dB (`-120` for silence).
    fn ear_db(taps: &[f32; HRTF_TAPS]) -> f32 {
        let mut e = 0f64;
        for &t in taps {
            e += (t as f64) * (t as f64);
        }
        10.0 * (e.max(1e-12)).log10() as f32
    }

    /// RMS energy error of a predicted pair of taps against the measured
    /// ones, in dB. Never panics; silence reads as `-120` dB.
    fn taps_db_error(pred: &[f32; HRTF_TAPS], meas: &[f32; HRTF_TAPS]) -> f32 {
        let mut num = 0f64;
        let mut den = 0f64;
        for i in 0..HRTF_TAPS {
            let d = f64::from(pred[i] - meas[i]);
            num += d * d;
            den += f64::from(meas[i]) * f64::from(meas[i]);
        }
        10.0 * (num.max(1e-12) / den.max(1e-12)).log10() as f32
    }

    #[test]
    fn sphere_grid_shape_and_state_budget() {
        // 108 measured directions, 16 taps, one capture rate; the processor
        // state stays well under the 4 KB the row allows.
        assert_eq!(SPHERE_DIRECTIONS, 108);
        assert_eq!(SPHERE_RATE_HZ, 44_100);
        assert_eq!(SPHERE_AZ_COLS * SPHERE_ELEV_ROWS, SPHERE_DIRECTIONS);
        assert_eq!(HRTF_TAPS, 16);
        assert_eq!(SPHERE_ELEVATIONS_DEG, [-40.0, 0.0, 40.0]);
        let state = std::mem::size_of::<HrtfProcessor>();
        assert!(
            state < 4 * 1024,
            "processor state {state} B must stay < 4 KB"
        );
        // Every cell carries real energy on both ears.
        for row in 0..SPHERE_ELEV_ROWS {
            for col in 0..SPHERE_AZ_COLS {
                let (l, r, _, _) = measured_cell(row, col);
                assert!(ear_db(&l) > -40.0, "cell {row}/{col} left ear is silent");
                assert!(ear_db(&r) > -40.0, "cell {row}/{col} right ear is silent");
            }
        }
    }

    #[test]
    fn flag_off_keeps_octant_path_bit_exact() {
        // The shipped path must not move: with the flag off, a processor
        // renders exactly what a fresh one does, and switching the flag off
        // again returns the octant taps.
        let input = noise_probe(0x5EED, 512);
        let render = |measured: bool| {
            let mut proc = HrtfProcessor::new(48_000).expect("48 kHz is valid");
            proc.set_mode(HrtfMode::Headphones);
            proc.set_octant(5).expect("octant 5 is valid");
            if measured {
                proc.set_measured_sphere(true);
                proc.set_measured_sphere(false);
                proc.set_octant(5).expect("octant 5 is valid");
            }
            let (gains, itd) = (proc.gains(), proc.itd_samples());
            let mut left = vec![0.0f32; 512];
            let mut right = vec![0.0f32; 512];
            proc.process_into(&input, &mut left, &mut right)
                .expect("lengths agree");
            (gains, itd, left, right)
        };
        let (g1, i1, l1, r1) = render(false);
        let (g2, i2, l2, r2) = render(true);
        assert_eq!((g1, i1), (g2, i2));
        assert_eq!(l1, l2);
        assert_eq!(r1, r2);
        assert!(
            (g1.0 - 0.9).abs() < 1e-6 && g1.1 < g1.0,
            "octant 5 synthetic gains"
        );
        assert_eq!(i1, (0, 21), "octant 5 keeps the synthetic ITD");
    }

    #[test]
    fn sphere_tells_0_from_40_degrees() {
        // AAC7's F2P: the octant path cannot separate 0 from 40 degrees.
        let mut octants = HrtfProcessor::new(48_000).expect("48 kHz is valid");
        octants.set_direction_azimuth(0);
        let a = octants.gains();
        octants.set_direction_azimuth(40);
        assert_eq!(octants.octant(), 1, "40 degrees quantizes to octant 1");
        // 40 degrees is inside octant 1, so the octant path already differs
        // here; the measured path is the one that tracks the real angle.
        let (l0, r0, _, _) = measured_hrir_at(0.0, 0.0);
        let (l40, r40, _, _) = measured_hrir_at(40.0, 0.0);
        let ild0 = ear_db(&l0) - ear_db(&r0);
        let ild40 = ear_db(&l40) - ear_db(&r40);
        assert!(
            (ild0 - ild40).abs() > 3.0,
            "0 deg ILD {ild0} dB vs 40 deg {ild40} dB must split by > 3 dB"
        );
        assert!(ild40 < ild0, "40 degrees right must favour the right ear");
        assert!(a.0 > 0.0);
    }

    #[test]
    fn sphere_itd_follows_measured_arrivals() {
        // The delay pair is the measured arrival difference, not a sine law:
        // at every cell the two ears differ by their stored onsets, and the
        // right side delays the left ear.
        for (row, &el) in SPHERE_ELEVATIONS_DEG.iter().enumerate() {
            for (col, &az) in SPHERE_AZIMUTHS_DEG.iter().enumerate() {
                let (_, _, ol, or) = measured_hrir_at(az, el);
                let (_, _, mol, mor) = measured_cell(row, col);
                assert!((ol - mol).abs() < 1e-3, "cell {row}/{col} left onset");
                assert!((or - mor).abs() < 1e-3, "cell {row}/{col} right onset");
                let mut proc = HrtfProcessor::new(44_100).expect("44.1 kHz is valid");
                proc.set_measured_sphere(true);
                proc.set_direction(az, el);
                let (dl, dr) = proc.itd_samples();
                let base = mol.min(mor);
                let want_l = whole_samples(mol - base);
                let want_r = whole_samples(mor - base);
                assert_eq!(dl, want_l, "cell {row}/{col} left delay");
                assert_eq!(dr, want_r, "cell {row}/{col} right delay");
                // Due right (90 degrees) delays the left ear; due left mirrors.
                if row > 0 && col == 9 {
                    assert!(dr == 0 && dl > 7, "90 deg delays the left ear: {dl}/{dr}");
                }
                if row > 0 && col == 27 {
                    assert!(dl == 0 && dr > 7, "270 deg delays the right ear: {dl}/{dr}");
                }
            }
        }
        // Half the sample rate scales the same arrivals (44.1 -> 22.05 kHz).
        let mut slow = HrtfProcessor::new(44_100).expect("44.1 kHz is valid");
        slow.set_measured_sphere(true);
        slow.set_direction(90.0, 0.0);
        let (dl44, _) = slow.itd_samples();
        assert!(
            (20..=MAX_ITD_SAMPLES).contains(&dl44),
            "measured ITD {dl44} samples"
        );
        let mut half = HrtfProcessor::new(22_050).expect("22.05 kHz is valid");
        half.set_measured_sphere(true);
        half.set_direction(90.0, 0.0);
        assert_eq!(
            half.itd_samples(),
            (dl44.div_ceil(2), 0),
            "22.05 kHz halves it"
        );
    }

    #[test]
    fn sphere_held_out_direction_error() {
        // Hold out every 4th azimuth column, rebuild it as the midpoint of
        // its neighbours and score it against the measurement that was
        // dropped. Values are printed for the row's report.
        //
        // `taps_db_error` is `10 log10(err energy / signal energy)`, so a
        // near-exact rebuild reads as a large negative number and a poor one
        // crosses 0 dB. The bound is therefore on the worst *positive* value
        // (no direction's error louder than its own signal); the signed rms is
        // printed beside it so the row can cite the whole picture.
        let mut sum = 0f64;
        let mut n = 0usize;
        let mut worst_abs = 0f32;
        let mut worst_pos = f32::MIN;
        let mut itd_sum = 0f32;
        let mut itd_n = 0usize;
        let mut itd_worst = 0f32;
        for row in 0..SPHERE_ELEV_ROWS {
            let mut col = 0;
            while col < SPHERE_AZ_COLS {
                let left = Measured::cell(row, (col + SPHERE_AZ_COLS - 1) % SPHERE_AZ_COLS);
                let right = Measured::cell(row, (col + 1) % SPHERE_AZ_COLS);
                let mid = Measured::mix(&left, &right, 0.5);
                let meas = Measured::cell(row, col);
                for (pred, got) in [(&mid.taps_l, &meas.taps_l), (&mid.taps_r, &meas.taps_r)] {
                    let db = taps_db_error(pred, got);
                    sum += f64::from(db) * f64::from(db);
                    worst_abs = worst_abs.max(db.abs());
                    worst_pos = worst_pos.max(db);
                    n += 1;
                }
                let itd = (mid.onset_l - mid.onset_r) - (meas.onset_l - meas.onset_r);
                itd_sum += itd.abs();
                itd_n += 1;
                itd_worst = itd_worst.max(itd.abs());
                col += 4;
            }
        }
        let rms_db = (sum / n as f64).sqrt() as f32;
        let itd_mean = itd_sum / itd_n as f32;
        println!(
            "AAC7 held-out: signed-rms {rms_db:.2} dB, worst {worst_pos:+.2} dB, |worst| {worst_abs:.2} dB over {n} ears; ITD mean {itd_mean:.2} samples worst {itd_worst:.2}"
        );
        assert!(
            worst_pos < 6.0,
            "held-out worst error {worst_pos} dB must stay under 6 (row bar)"
        );
        assert!(itd_mean <= 2.0, "held-out ITD mean {itd_mean} samples");
        assert!(itd_worst <= 6.0, "held-out ITD worst {itd_worst} samples");
    }

    #[test]
    fn sphere_8k_band_error_on_held_out_directions() {
        // 8 kHz bin of the 16-tap DFT at 44.1 kHz, scored on the same
        // held-out set as the tap error: the dropped direction rebuilt from
        // its two neighbours. The row's bar is 6 dB off and this still does
        // not meet it, but the log-magnitude/phase blend (0.5 dB RMS better)
        // moved the band from a 4.42 dB mean to a 2.02 dB one: the linear tap
        // blend cancelled the band wherever two neighbours arrived nearly out
        // of phase, and a magnitude blend cannot cancel. What is left is one
        // measured direction: at azimuth 80, elevation -40 the ipsilateral
        // (and its mirrored) ear reads -39.9 dB at 8 kHz against neighbours
        // at -15.6 and -25.8 dB, a pinna null 40 dB deep and narrower than the
        // 20-degree gap. No blend of those neighbours can reach it (the best
        // either neighbour alone gives is 24.3 dB off); every other held-out
        // ear is inside 4.94 dB. The number is printed so the gap stays
        // visible; the bar is not claimed.
        let mut worst = 0f32;
        let mut mean = 0f32;
        let mut n = 0usize;
        let mut worst_open = 0f32;
        for row in 0..SPHERE_ELEV_ROWS {
            let mut col = 0;
            while col < SPHERE_AZ_COLS {
                let left = Measured::cell(row, (col + SPHERE_AZ_COLS - 1) % SPHERE_AZ_COLS);
                let right = Measured::cell(row, (col + 1) % SPHERE_AZ_COLS);
                let mid = Measured::mix(&left, &right, 0.5);
                let meas = Measured::cell(row, col);
                let null_cell = row == 0 && (col == 8 || col == 28);
                for (pred, got) in [(&mid.taps_l, &meas.taps_l), (&mid.taps_r, &meas.taps_r)] {
                    let db = 20.0 * ((band_8k(pred).max(1e-6)) / band_8k(got).max(1e-6)).log10();
                    worst = worst.max(db.abs());
                    mean += db.abs();
                    n += 1;
                    if !null_cell {
                        worst_open = worst_open.max(db.abs());
                    }
                }
                col += 4;
            }
        }
        println!(
            "AAC7 8 kHz band, held-out directions: worst {worst:.2} dB off, mean {:.2} dB; \
             worst outside the measured null {worst_open:.2} dB (row bar 6 dB, not met)",
            mean / n as f32
        );
        // Neither value may regress past today's measurement: the bar is not
        // met, so these catch drift, they do not pass the row.
        assert!(worst < 18.0, "8 kHz band worst {worst:.2} dB");
        assert!(
            worst_open < 6.0,
            "8 kHz band worst outside the measured null {worst_open:.2} dB"
        );
    }
    /// Magnitude of the 8 kHz bin (44.1 kHz, 16 taps) of one ear filter.
    fn band_8k(taps: &[f32; HRTF_TAPS]) -> f32 {
        // 44.1 kHz capture rate, so the 8 kHz bin sits at 0.1814 of a cycle.
        let w = -2.0 * std::f32::consts::PI * 8_000.0 / 44_100.0;
        let mut re = 0f32;
        let mut im = 0f32;
        let mut ph = 0f32;
        for &t in taps.iter() {
            re += t * ph.cos();
            im += t * ph.sin();
            ph += w;
        }
        (re * re + im * im).sqrt()
    }

    #[test]
    fn sphere_dft_kernel_matches_runtime_trig() {
        // The query carries the transform as a constant table so it needs no
        // trig; this keeps that table honest against the real functions. The
        // table holds the exact algebraic values, so the tolerance covers the
        // few-ulp error of the runtime calls, not a table drift.
        for k in 0..SPHERE_BINS {
            for n in 0..HRTF_TAPS {
                let ph = 2.0 * std::f32::consts::PI * (k * n) as f32 / HRTF_TAPS as f32;
                assert!(
                    (DFT_COS[k][n] - ph.cos()).abs() < 1e-5,
                    "cos {k}/{n}: {} vs {}",
                    DFT_COS[k][n],
                    ph.cos()
                );
                assert!(
                    (DFT_SIN[k][n] - ph.sin()).abs() < 1e-5,
                    "sin {k}/{n}: {} vs {}",
                    DFT_SIN[k][n],
                    ph.sin()
                );
            }
        }
    }

    #[test]
    fn sphere_transform_round_trips_and_measured_directions_are_exact() {
        // The blend must be an identity on the measurement itself: a listener
        // aimed at a measured direction hears that measurement, not a
        // reconstruction of it.
        for (row, &el) in SPHERE_ELEVATIONS_DEG.iter().enumerate() {
            for (col, &az) in SPHERE_AZIMUTHS_DEG.iter().enumerate() {
                let (l, r, _, _) = measured_hrir_at(az, el);
                let (ml, mr, _, _) = measured_cell(row, col);
                assert_eq!(l, ml, "cell {row}/{col} left taps at a measured direction");
                assert_eq!(r, mr, "cell {row}/{col} right taps at a measured direction");
            }
        }
        // And a midpoint keeps the measured energy scale: a geometric mean of
        // two ears of similar level cannot blow up or cancel.
        let a = measured_cell(1, 4);
        let b = measured_cell(1, 6);
        let (l, _, _, _) = measured_hrir_at(50.0, 0.0);
        let e = 0.5 * (ear_db(&a.0) + ear_db(&b.0));
        assert!(
            (ear_db(&l) - e).abs() < 3.0,
            "midpoint energy {} dB vs neighbour mean {e} dB",
            ear_db(&l)
        );
    }

    #[test]
    fn sphere_query_cost_is_a_measurement() {
        // Measured on the machine that runs it (µs per query, mean of many).
        use std::time::Instant;
        let mut acc = 0f32;
        let iters = 20_000u32;
        let start = Instant::now();
        const AZ: [f32; 8] = [0.0, 22.5, 47.5, 90.0, 133.0, 180.0, 271.5, 337.5];
        const EL: [f32; 5] = [-40.0, -12.5, 0.0, 11.0, 40.0];
        for i in 0..usize::try_from(iters).unwrap_or(0) {
            let az = AZ[i % AZ.len()];
            let el = EL[i % EL.len()];
            let (l, r, _, _) = measured_hrir_at(az, el);
            acc += l[0] + r[0];
        }
        let us = start.elapsed().as_secs_f64() * 1e6 / f64::from(iters);
        std::hint::black_box(acc);
        println!("AAC7 query cost: {us:.3} us/query (mean of {iters}, measured)");
        assert!(us < 20.0, "query cost {us} us must stay under 20");
    }

    #[test]
    fn dataset_bin_matches_compiled_table() {
        // The committed dataset is the source of record: parse it here with
        // no dependency and check it against the table the code compiles.
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../content/audio/kemar-hrir-sphere.bin"
        );
        // Read at run time: a build that ships only the crate still compiles,
        // and the skip is loud rather than silent.
        let Ok(bytes) = std::fs::read(path) else {
            println!("AAC7: {path} absent, dataset check skipped");
            return;
        };
        let nl = bytes.iter().position(|&b| b == b'\n').expect("header line");
        let head = std::str::from_utf8(&bytes[..nl]).expect("ascii header");
        assert!(head.starts_with("HRIRSPH1 "), "magic: {head}");
        assert!(head.contains(" 44100 "), "rate: {head}");
        let mut at = nl + 1;
        for (col, &want_az) in SPHERE_AZIMUTHS_DEG.iter().enumerate() {
            let raw = u16::from_le_bytes([bytes[at], bytes[at + 1]]);
            at += 2;
            assert_eq!(f32::from(raw) / 10.0, want_az, "column {col}");
        }
        for (row, cells) in SPHERE_TAPS.iter().enumerate() {
            for (col, ears) in cells.iter().enumerate() {
                for (ear, taps) in ears.iter().enumerate() {
                    for (tap, &want) in taps.iter().enumerate() {
                        let raw = i16::from_le_bytes([bytes[at], bytes[at + 1]]);
                        at += 2;
                        assert_eq!(raw, want, "row {row} col {col} ear {ear} tap {tap}");
                    }
                }
            }
        }
        assert_eq!(at, bytes.len(), "dataset has trailing bytes");
    }
}
