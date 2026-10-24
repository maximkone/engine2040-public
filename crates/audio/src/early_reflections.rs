//! SW112: early reflections sized from the room, not a preset.
//!
//! A blow's first return comes off the room's wall before the late tail blooms.
//! [`taps`] derives four tap delays from the listener-to-wall distance alone
//! (`r_a = d / 343.3 m/s`, `r0 = 2/3 r_a`, `r1 = 4/3 r_a`,
//! `T_i = (2^(i/7) - 1) (r1 - r0)`), and each tap carries the reflection gain of
//! the wall's own [`occlude`] absorption row, so a hard stone wall returns
//! louder than soft sand. [`EarlyLines`] is four integer delay lines built once;
//! [`EarlyLines::add`] sums them into an existing wet bus with no allocation,
//! no clock and no float index, so the audio thread never allocates here.
//!
//! Units: centimetres in, whole microseconds out, Q16 gain (`GAIN_ONE` is
//! unity), samples at the caller's rate. Determinism: same distance plus same
//! material gives the same taps twice.

use crate::occlude::{absorption_q16, q16_to_gain, Material, GAIN_ONE};

/// Speed of sound in air, centimetres per second (343.3 m/s).
pub const SPEED_OF_SOUND_CM_S: u32 = 34_330;
/// Taps a room returns: one per early-reflection lane.
pub const TAP_COUNT: usize = 4;
/// `(2^(i/7) - 1)` in ten-thousandths, one lane per tap: the spacing grows by
/// the same ratio a semitone ladder does, so the first returns stay distinct.
const TAP_LADDER: [u64; TAP_COUNT] = [0, 1_041, 2_190, 3_459];
/// Nearest distance a wall is asked about, centimetres: closer than this the
/// return is the listener's own body, not a wall.
pub const MIN_DIST_CM: u32 = 1;
/// Furthest wall a room is sized for, centimetres (a 200 m hall).
pub const MAX_DIST_CM: u32 = 20_000;

/// What refused a tap query or a delay line. Refusals change nothing.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum EarlyError {
    /// A distance outside `MIN_DIST_CM..=MAX_DIST_CM`; culprit carried.
    BadDistance(u32),
    /// A sample rate outside the reverb module's band; culprit carried.
    BadRate(u32),
}

impl std::fmt::Display for EarlyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BadDistance(cm) => write!(
                f,
                "early_reflections: distance {cm} cm is outside {MIN_DIST_CM}..={MAX_DIST_CM}"
            ),
            Self::BadRate(hz) => write!(
                f,
                "early_reflections: sample rate {hz} is outside {}..={}",
                crate::reverb::MIN_SAMPLE_RATE_HZ,
                crate::reverb::MAX_SAMPLE_RATE_HZ
            ),
        }
    }
}

impl std::error::Error for EarlyError {}

/// One early return: when it lands and how loud the wall hands it back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tap {
    /// Flight time past the direct path, whole microseconds.
    pub delay_us: u32,
    /// Reflection gain off the wall, Q16 (unity is [`GAIN_ONE`]).
    pub gain_q16: i32,
}

impl Tap {
    /// Delay in milliseconds, for receipts and tests.
    pub fn delay_ms(&self) -> f64 {
        f64::from(self.delay_us) / 1000.0
    }

    /// Gain as a float in `0.0..=1.0` for the mixer call.
    pub fn gain(&self) -> f32 {
        q16_to_gain(self.gain_q16)
    }
}

/// Reflection gain off a wall of `material`, Q16: the wall's own absorption
/// row scaled onto the return, so a hard wall returns more than soft ground
/// and a new material needs no table here. Stone reads 2.9 dB above sand.
pub fn reflection_q16(material: Material) -> i32 {
    GAIN_ONE / 5 + absorption_q16(material) / 4
}

/// The four early returns of a wall `dist_cm` away, in arrival order.
///
/// The first tap is `r0 = 2/3 d/c` (the near wall) and the rest climb the
/// semitone ladder to `r1 = 4/3 d/c`; a wall 20 m away reads 39/43/47/52 ms.
/// Integer microseconds throughout: the same distance always lands the same tap.
pub fn taps(dist_cm: u32, material: Material) -> Result<[Tap; TAP_COUNT], EarlyError> {
    if !(MIN_DIST_CM..=MAX_DIST_CM).contains(&dist_cm) {
        return Err(EarlyError::BadDistance(dist_cm));
    }
    // r0 == r1 - r0 == 2/3 d/c: one round trip of the wall the row sizes.
    let span_us = u64::from(dist_cm) * 2_000_000 / (3 * u64::from(SPEED_OF_SOUND_CM_S));
    let gain_q16 = reflection_q16(material);
    let mut out = [Tap {
        delay_us: 0,
        gain_q16,
    }; TAP_COUNT];
    for (index, tap) in out.iter_mut().enumerate() {
        let offset_us = span_us * TAP_LADDER[index] / 10_000;
        // A 200 m wall stays far under a second, so the wide sum fits u32.
        tap.delay_us = u32::try_from(span_us + offset_us).unwrap_or(u32::MAX);
    }
    Ok(out)
}

/// Whole samples a tap's flight time covers at `sample_rate_hz`: integer math,
/// so the same rate always lands the tap on the same sample.
pub fn delay_samples(delay_us: u32, sample_rate_hz: u32) -> u64 {
    u64::from(delay_us) * u64::from(sample_rate_hz) / 1_000_000
}

/// Four integer delay lines, one per early tap, summed into the wet bus.
///
/// [`EarlyLines::new`] allocates on the game thread; [`EarlyLines::add`] then
/// reads and writes plain slices, so the audio thread allocates nothing.
#[derive(Debug)]
pub struct EarlyLines {
    lines: [Vec<f32>; TAP_COUNT],
    pos: [usize; TAP_COUNT],
    gain: [f32; TAP_COUNT],
}

impl EarlyLines {
    /// Build the lines for `taps` at `sample_rate_hz`. Allocates: game thread.
    pub fn new(taps: &[Tap; TAP_COUNT], sample_rate_hz: u32) -> Result<Self, EarlyError> {
        if !(crate::reverb::MIN_SAMPLE_RATE_HZ..=crate::reverb::MAX_SAMPLE_RATE_HZ)
            .contains(&sample_rate_hz)
        {
            return Err(EarlyError::BadRate(sample_rate_hz));
        }
        let mut lines: [Vec<f32>; TAP_COUNT] = Default::default();
        let mut gain = [0.0f32; TAP_COUNT];
        for (index, tap) in taps.iter().enumerate() {
            // One line holds exactly its own flight time, so reading before
            // writing hands back the sample from that many frames ago.
            let len = delay_samples(tap.delay_us, sample_rate_hz).max(1);
            lines[index] = vec![0.0; usize::try_from(len).unwrap_or(1)];
            gain[index] = tap.gain();
        }
        Ok(Self {
            lines,
            pos: [0; TAP_COUNT],
            gain,
        })
    }

    /// Gains this bus sums with, in tap order (float in `0.0..=1.0`).
    pub fn gains(&self) -> [f32; TAP_COUNT] {
        self.gain
    }

    /// Samples each line holds, in tap order.
    pub fn lens(&self) -> [usize; TAP_COUNT] {
        let mut out = [0usize; TAP_COUNT];
        for (slot, line) in out.iter_mut().zip(self.lines.iter()) {
            *slot = line.len();
        }
        out
    }

    /// Clear every line to silence without allocating; call on zone switch.
    pub fn reset(&mut self) {
        for (line, pos) in self.lines.iter_mut().zip(self.pos.iter_mut()) {
            for sample in line.iter_mut() {
                *sample = 0.0;
            }
            *pos = 0;
        }
    }

    /// Delay `input` through the four lines and sum each tap's gain into `wet`.
    ///
    /// Reads and writes only the samples both slices share, so it never panics
    /// on a short block and never allocates.
    pub fn add(&mut self, input: &[f32], wet: &mut [f32]) {
        let frames = input.len().min(wet.len());
        for (index, (line, pos)) in self.lines.iter_mut().zip(self.pos.iter_mut()).enumerate() {
            let gain = self.gain[index];
            for frame in 0..frames {
                let delayed = line[*pos];
                line[*pos] = input[frame];
                *pos = (*pos + 1) % line.len();
                wet[frame] += delayed * gain;
            }
        }
    }
}

#[cfg(test)]
#[path = "early_reflections_tests.rs"]
mod tests;
