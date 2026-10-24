//! frame_tolerance: when does a frame match a golden.
//!
//! Owner: render lane. One rule in one place. A frame matches a golden only if
//! BOTH hold: the mean absolute RGB channel error is at most
//! `GOLDEN_MEAN_ABS_MILLI` (1.5 levels on the 0..255 scale) AND at most
//! `GOLDEN_OVER_PPM` of the pixels (0.5 percent) are off by more than
//! `GOLDEN_OVER_LEVEL` levels on any channel. Each number hides what the
//! other shows: a mean alone lets a few pixels be wrong by a lot (a lost
//! highlight), a share alone lets the whole frame drift a little (a gain nudge).
//! `frames_match` returns a `FrameVerdict` with both numbers, so a failure
//! says which rule broke and by how much.
//! The gate proves itself: the tests below nudge one post coefficient by 0.02
//! and one patch of pixels by a visible amount, and the rule must FAIL; a
//! nudge a tenth the size must PASS. A rule that passes both has no teeth.
//! Units: RGBA bytes row-major, levels on 0..255; alpha is ignored. Integer
//! math only (milli-levels, parts per million), so the same bytes give the
//! same verdict on every machine. Errors: `FrameError` names ragged input.
//! Budget: one pass over both frames, no allocation.
//! Consumers: the GPU-skinned mesh parity test in `present/gpu_present/presenter.rs`
//! (the CPU-skinned frame is its golden); new golden or parity tests call
//! `frames_match` instead of a local threshold.

use std::error::Error;
use std::fmt;

/// Golden mean absolute channel error limit, in milli-levels (1500 = 1.5).
pub const GOLDEN_MEAN_ABS_MILLI: u32 = 1500;
/// A pixel counts as far off when any channel differs by more than this.
pub const GOLDEN_OVER_LEVEL: u8 = 12;
/// Golden share limit of far-off pixels, parts per million (5000 = 0.5 percent).
pub const GOLDEN_OVER_PPM: u32 = 5_000;

const PPM: u64 = 1_000_000;

/// The two numbers a frame must stay inside.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FrameTolerance {
    /// Mean absolute RGB channel error limit, milli-levels.
    pub mean_abs_milli_max: u32,
    /// A pixel is far off when a channel differs by more than this many levels.
    pub over_level: u8,
    /// Limit on the share of far-off pixels, parts per million.
    pub over_ppm_max: u32,
}

impl FrameTolerance {
    /// The golden rule: mean 1.5 levels and 0.5 percent of pixels over 12.
    /// A parity test that held a tighter far-off level keeps it with struct
    /// update: `FrameTolerance { over_level: 3, ..FrameTolerance::GOLDEN }`.
    pub const GOLDEN: Self = Self {
        mean_abs_milli_max: GOLDEN_MEAN_ABS_MILLI,
        over_level: GOLDEN_OVER_LEVEL,
        over_ppm_max: GOLDEN_OVER_PPM,
    };
}

/// Both numbers of one comparison and the verdict on each.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FrameVerdict {
    /// The rule this verdict was judged against.
    pub tolerance: FrameTolerance,
    /// Pixels compared.
    pub pixels: usize,
    /// Mean absolute RGB channel error, milli-levels, rounded up.
    pub mean_abs_milli: u32,
    /// Pixels with a channel off by more than `tolerance.over_level`.
    pub over_pixels: usize,
    /// `over_pixels` as parts per million of `pixels`, rounded up.
    pub over_ppm: u32,
    /// Largest single channel difference, levels.
    pub worst: u8,
    /// The mean number is inside its limit.
    pub mean_ok: bool,
    /// The far-off share is inside its limit.
    pub over_ok: bool,
}

impl FrameVerdict {
    /// True only when both numbers are inside their limits.
    #[must_use]
    pub fn matches(&self) -> bool {
        self.mean_ok && self.over_ok
    }

    /// One line with both numbers, their limits and the verdict, for an
    /// assertion message: `mean_abs=2.531 (max 1.500) over12=0.0000% (max
    /// 0.5000%) worst=7 FAIL(mean)`.
    #[must_use]
    pub fn describe(&self) -> String {
        let verdict = match (self.mean_ok, self.over_ok) {
            (true, true) => "PASS",
            (false, true) => "FAIL(mean)",
            (true, false) => "FAIL(over)",
            (false, false) => "FAIL(mean+over)",
        };
        format!(
            "mean_abs={} (max {}) over{}={} (max {}) worst={} {verdict}",
            milli_text(self.mean_abs_milli),
            milli_text(self.tolerance.mean_abs_milli_max),
            self.tolerance.over_level,
            percent_text(self.over_ppm),
            percent_text(self.tolerance.over_ppm_max),
            self.worst,
        )
    }
}

/// Ragged frames; a short or empty capture never reads as a match.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FrameError {
    /// Both frames are empty.
    Empty,
    /// The byte length is not a whole number of RGBA pixels.
    NotRgba {
        /// The offending length.
        len: usize,
    },
    /// The frames differ in size.
    LengthMismatch {
        /// Golden length in bytes.
        golden: usize,
        /// Frame length in bytes.
        frame: usize,
    },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "frame compare needs pixels, got none"),
            Self::NotRgba { len } => write!(f, "frame length {len} is not whole RGBA pixels"),
            Self::LengthMismatch { golden, frame } => {
                write!(f, "frame is {frame} bytes but the golden is {golden}")
            }
        }
    }
}

impl Error for FrameError {}

/// Judge `frame` against `golden` (RGBA bytes, same size) under `tolerance`.
/// Integer only; the same bytes give the same verdict twice. Alpha is ignored.
pub fn frames_match(
    golden: &[u8],
    frame: &[u8],
    tolerance: &FrameTolerance,
) -> Result<FrameVerdict, FrameError> {
    if golden.len() != frame.len() {
        return Err(FrameError::LengthMismatch {
            golden: golden.len(),
            frame: frame.len(),
        });
    }
    if golden.is_empty() {
        return Err(FrameError::Empty);
    }
    if !golden.len().is_multiple_of(4) {
        return Err(FrameError::NotRgba { len: golden.len() });
    }
    let mut sum = 0u64;
    let mut over_pixels = 0usize;
    let mut worst = 0u8;
    for (want, got) in golden.chunks_exact(4).zip(frame.chunks_exact(4)) {
        let mut far = false;
        for (a, b) in want.iter().zip(got).take(3) {
            let diff = a.abs_diff(*b);
            sum += u64::from(diff);
            worst = worst.max(diff);
            far |= diff > tolerance.over_level;
        }
        over_pixels += usize::from(far);
    }
    let pixels = golden.len() / 4;
    let wide_pixels = u64::try_from(pixels).unwrap_or(u64::MAX);
    let wide_over = u64::try_from(over_pixels).unwrap_or(u64::MAX);
    let channels = wide_pixels.saturating_mul(3);
    let mean_abs_milli = narrow(sum.saturating_mul(1000).div_ceil(channels));
    let over_ppm = narrow(wide_over.saturating_mul(PPM).div_ceil(wide_pixels));
    Ok(FrameVerdict {
        tolerance: *tolerance,
        pixels,
        mean_abs_milli,
        over_pixels,
        over_ppm,
        worst,
        mean_ok: mean_abs_milli <= tolerance.mean_abs_milli_max,
        over_ok: over_ppm <= tolerance.over_ppm_max,
    })
}

/// `u64` to `u32`, saturating (a mean is at most 255000, a share at most 1e6).
fn narrow(value: u64) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// `1500` as `1.500`.
fn milli_text(milli: u32) -> String {
    format!("{}.{:03}", milli / 1000, milli % 1000)
}

/// `5000` parts per million as `0.5000%`.
fn percent_text(ppm: u32) -> String {
    format!("{}.{:04}%", ppm / 10_000, ppm % 10_000)
}

#[cfg(test)]
mod tests;
