//! SSIMULACRA2 v2.1: the look gate's second metric, beside SSIM.
//!
//! Consumer: the `view.compare` door verb (`engine-endpoint`, the
//! `ssimulacra2_x10000` field) and the look judge, which reports it next to
//! the SSIM score. Data owner: the caller owns the pixels; this module reads
//! two decoded [`Frame`]s and never owns a file.
//!
//! Donor: `ssimulacra2` 0.5.1 (BSD-2-Clause, LICENSE read live 2026-10-21,
//! <https://github.com/rust-av/ssimulacra2>), the Rust port of the
//! cloudinary/ssimulacra2 C++ metric. The crate is used unmodified; this
//! module only converts frames and quantises the score.
//!
//! Units: the donor scores 0..=100 where 100 is identical. This module
//! publishes **hundredths of a donor point**, so
//! [`SSIM2_ONE`] (10000) is identical and 0 is worst. The door field is
//! `ssimulacra2_x10000` because 10000 is the identical value, the same
//! convention as `ssim_x10000` (where 10000 is 1.0). Higher is better, like
//! SSIM; unlike FLIP and deltaE, where 0 is identical.
//!
//! Determinism: the donor is float and NOT bit-reproducible across
//! machines (its own test tolerates +/- 0.25 points across CI runs), so this
//! module publishes a coarse fixed point, keeps the raw float out of every
//! reply and out of any simulation hash, and never returns a float to a
//! caller that writes a digest. Two runs on one machine give the same
//! integer; two machines may differ by a unit.
//!
//! Errors: [`Ssim2Error`] names the mismatched size or the frame that is
//! too small; the donor's own errors are mapped onto it. Nothing here
//! panics on caller data.
//!
//! Budget: one donor pass per pair (six scales, three planes each), so a
//! 1280x720 pair is a few tens of milliseconds in release.

use std::fmt;

use ssimulacra2::{ColorPrimaries, Rgb, TransferCharacteristic};

use super::metric::Frame;

/// Identical frames score this: 10000 hundredths of a donor point.
pub const SSIM2_ONE: u32 = 10_000;

/// Smallest frame edge the donor accepts on each side.
pub const MIN_EDGE: u32 = 8;

/// Why the second metric refused its inputs.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Ssim2Error {
    /// The two frames differ in size.
    SizeMismatch {
        /// First frame `(width, height)`.
        left: (u32, u32),
        /// Second frame `(width, height)`.
        right: (u32, u32),
    },
    /// A frame is smaller than [`MIN_EDGE`] on a side.
    TooSmall {
        /// The frame that is too small `(width, height)`.
        size: (u32, u32),
    },
    /// The donor refused the pixels.
    Donor(String),
}

impl fmt::Display for Ssim2Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SizeMismatch { left, right } => write!(
                f,
                "frames differ in size ({}x{} vs {}x{}): the second metric scores one size",
                left.0, left.1, right.0, right.1
            ),
            Self::TooSmall { size } => write!(
                f,
                "{}x{} is under the {MIN_EDGE}x{MIN_EDGE} floor: the second metric refuses it",
                size.0, size.1
            ),
            Self::Donor(why) => write!(f, "the second metric refused the pixels: {why}"),
        }
    }
}

impl std::error::Error for Ssim2Error {}

/// The donor's raw score for two frames: 0..=100, 100 is identical.
///
/// Float on purpose: this is the calibration value, never a reply field and
/// never a digest input. Use [`score_x10000`] for anything a machine reads.
pub fn score_points(source: &Frame, distorted: &Frame) -> Result<f64, Ssim2Error> {
    check_pair(source, distorted)?;
    let source_rgb = to_rgb(source)?;
    let distorted_rgb = to_rgb(distorted)?;
    ssimulacra2::compute_frame_ssimulacra2(source_rgb, distorted_rgb)
        .map_err(|error| Ssim2Error::Donor(error.to_string()))
}

/// The second metric for two frames, in hundredths of a donor point.
///
/// [`SSIM2_ONE`] (10000) is identical, 0 is worst, and the value is a coarse
/// fixed point so a machine reads a number and never a float.
pub fn score_x10000(source: &Frame, distorted: &Frame) -> Result<u32, Ssim2Error> {
    let points = score_points(source, distorted)?;
    Ok(to_x10000(points))
}

/// The donor score as hundredths of a point, clamped to 0..=10000.
fn to_x10000(points: f64) -> u32 {
    if !points.is_finite() {
        return 0;
    }
    let scaled = (points * 100.0).round();
    if scaled <= 0.0 {
        return 0;
    }
    if scaled >= f64::from(SSIM2_ONE) {
        return SSIM2_ONE;
    }
    // The float to integer step goes through the engine-core owner, so the
    // narrowing is checked rather than cast.
    let whole = engine_core::checked::saturating_f64_to_u64(scaled);
    u32::try_from(whole).unwrap_or(SSIM2_ONE)
}

/// Refuse a pair the donor cannot score: mismatched sizes, or a side
/// under the 8x8 floor.
fn check_pair(source: &Frame, distorted: &Frame) -> Result<(), Ssim2Error> {
    let left = (source.width(), source.height());
    let right = (distorted.width(), distorted.height());
    if left != right {
        return Err(Ssim2Error::SizeMismatch { left, right });
    }
    if left.0 < MIN_EDGE || left.1 < MIN_EDGE {
        return Err(Ssim2Error::TooSmall { size: left });
    }
    Ok(())
}

/// The frame as the donor wants it: gamma-encoded sRGB floats, BT709
/// primaries, one `[r, g, b]` per pixel in row-major order.
fn to_rgb(frame: &Frame) -> Result<Rgb, Ssim2Error> {
    let rgba = frame.rgba();
    let mut data = Vec::with_capacity(frame.pixels());
    for chunk in rgba.chunks_exact(4) {
        data.push([
            f32::from(chunk[0]) / 255.0,
            f32::from(chunk[1]) / 255.0,
            f32::from(chunk[2]) / 255.0,
        ]);
    }
    let width = usize::try_from(frame.width()).map_err(|_| Ssim2Error::TooSmall {
        size: (frame.width(), frame.height()),
    })?;
    let height = usize::try_from(frame.height()).map_err(|_| Ssim2Error::TooSmall {
        size: (frame.width(), frame.height()),
    })?;
    Rgb::new(
        data,
        width,
        height,
        TransferCharacteristic::SRGB,
        ColorPrimaries::BT709,
    )
    .map_err(|error| Ssim2Error::Donor(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::metric::Frame;
    use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};

    const W: u32 = 64;
    const H: u32 = 48;

    /// The judge's own sand palette, so the twin and the donor score the
    /// same scene.
    fn sand_at(x: u32, y: u32) -> [u8; 3] {
        let t = i32::try_from((x * 7 + y * 13 + (x * y) % 5) % 23).expect("small") - 11;
        [194i32, 160, 104].map(|c| u8::try_from((c + t).clamp(0, 255)).expect("clamped"))
    }

    fn png_of(rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        PngEncoder::new(&mut out)
            .write_image(rgba, W, H, ExtendedColorType::Rgba8)
            .expect("png encodes");
        out
    }

    fn sand_frame() -> Vec<u8> {
        let mut rgba = Vec::new();
        for y in 0..H {
            for x in 0..W {
                let [r, g, b] = sand_at(x, y);
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
        }
        png_of(&rgba)
    }

    /// The sand region (every row from y = 8 down) with a fifth brighter:
    /// the "worse capture" pair the gate scores, the same pair the Node
    /// twin in `tools/iqa-gate.mjs` scores.
    fn bright_frame() -> Vec<u8> {
        let mut rgba = Vec::new();
        for y in 0..H {
            for x in 0..W {
                let [r, g, b] = sand_at(x, y);
                let lift = |v: u8| {
                    let scaled = (u32::from(v) * 6 + 2) / 5;
                    u8::try_from(scaled.min(255)).expect("clamped")
                };
                if y >= 8 {
                    rgba.extend_from_slice(&[lift(r), lift(g), lift(b), 255]);
                } else {
                    rgba.extend_from_slice(&[r, g, b, 255]);
                }
            }
        }
        png_of(&rgba)
    }

    #[test]
    fn identical_frames_score_the_identical_value() {
        let frame = Frame::decode_png(&sand_frame()).expect("the frame decodes");
        let score = score_x10000(&frame, &frame).expect("the donor scores the pair");
        assert_eq!(score, SSIM2_ONE);
    }

    #[test]
    fn a_worse_frame_scores_below_the_identical_value() {
        let reference = Frame::decode_png(&sand_frame()).expect("the reference decodes");
        let worse = Frame::decode_png(&bright_frame()).expect("the capture decodes");
        let score = score_x10000(&reference, &worse).expect("the donor scores the pair");
        assert!(score < SSIM2_ONE, "{score}");
        assert!(score > 0, "{score}");
        // The donor is not bit-reproducible across machines (its own test
        // tolerates a quarter of a point), so the pinned band is a band,
        // never an equality. The Node twin in `tools/iqa-gate.mjs` reads the
        // same band for the same pair.
        assert!(
            (6_200..=6_600).contains(&score),
            "the sand pair scores {score}, outside the pinned band"
        );
    }

    #[test]
    fn the_same_pair_scores_the_same_integer_twice() {
        let reference = Frame::decode_png(&sand_frame()).expect("the reference decodes");
        let worse = Frame::decode_png(&bright_frame()).expect("the capture decodes");
        let first = score_x10000(&reference, &worse).expect("the donor scores the pair");
        let second = score_x10000(&reference, &worse).expect("the donor scores the pair");
        assert_eq!(first, second);
    }

    #[test]
    fn the_donor_points_are_the_hundredths_of_the_fixed_point() {
        let reference = Frame::decode_png(&sand_frame()).expect("the reference decodes");
        let worse = Frame::decode_png(&bright_frame()).expect("the capture decodes");
        let points = score_points(&reference, &worse).expect("the donor scores the pair");
        let fixed = score_x10000(&reference, &worse).expect("the donor scores the pair");
        let want = (points * 100.0).round();
        assert!(
            (f64::from(fixed) - want).abs() < 1.0,
            "{fixed} against {want}"
        );
    }

    #[test]
    fn the_metric_refuses_a_pair_it_cannot_score() {
        let reference = Frame::decode_png(&sand_frame()).expect("the reference decodes");
        let mut rgba = Vec::new();
        for _ in 0..(32 * 24) {
            rgba.extend_from_slice(&[128, 128, 128, 255]);
        }
        let mut out = Vec::new();
        PngEncoder::new(&mut out)
            .write_image(&rgba, 32, 24, ExtendedColorType::Rgba8)
            .expect("png encodes");
        let smaller = Frame::decode_png(&out).expect("the frame decodes");
        assert_eq!(
            score_x10000(&reference, &smaller).expect_err("two sizes refuse"),
            Ssim2Error::SizeMismatch {
                left: (W, H),
                right: (32, 24),
            }
        );
    }

    #[test]
    fn the_metric_refuses_a_frame_under_the_floor() {
        let mut rgba = Vec::new();
        for _ in 0..(4 * 4) {
            rgba.extend_from_slice(&[128, 128, 128, 255]);
        }
        let mut out = Vec::new();
        PngEncoder::new(&mut out)
            .write_image(&rgba, 4, 4, ExtendedColorType::Rgba8)
            .expect("png encodes");
        let tiny = Frame::decode_png(&out).expect("the frame decodes");
        assert_eq!(
            score_x10000(&tiny, &tiny).expect_err("a tiny frame refuses"),
            Ssim2Error::TooSmall { size: (4, 4) }
        );
    }
}
