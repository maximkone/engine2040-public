//! Machine score of one capture against its reference, in integers.
//!
//! Consumer: [`crate::machine_eyes`] scores each captured PNG through here;
//! the window and endpoint hook sites are the next callers. Units: SSIM in
//! fixed point (10000 is identical), mean deltaE in hundredths of a CIE76
//! unit, histogram distances in fixed point (10000 is disjoint). Determinism:
//! integer only through [`crate::look::metric`]; the same PNG pair scores the
//! same on every machine. Errors: [`ScoreError`] names the refusing side.
//! Budget: two bounded PNG decodes plus one full-frame compare pass.

use std::fmt;

use crate::look::metric::{self, Comparison, Frame, MetricError, Region};

/// Machine score of one frame against one reference PNG.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LookScore {
    /// SSIM in fixed point; 10000 is identical.
    pub ssim_x10000: i32,
    /// Mean CIE76 deltaE in hundredths of a unit.
    pub mean_de_x100: u32,
    /// Luminance histogram total variation; 0 is identical.
    pub hist_tv_x10000: u32,
    /// Luminance histogram earth mover's distance; 0 is identical.
    pub hist_emd_x10000: u32,
}

impl From<Comparison> for LookScore {
    fn from(compared: Comparison) -> Self {
        Self {
            ssim_x10000: compared.ssim_x10000,
            mean_de_x100: compared.mean_de_x100,
            hist_tv_x10000: compared.hist_tv_x10000,
            hist_emd_x10000: compared.hist_emd_x10000,
        }
    }
}

/// Which input refused: the live frame or the filed reference.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum ScoreError {
    /// The live capture bytes did not decode or match.
    Frame(MetricError),
    /// The reference bytes did not decode.
    Reference(MetricError),
}

impl fmt::Display for ScoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Frame(error) => write!(f, "frame refused: {error}"),
            Self::Reference(error) => write!(f, "reference refused: {error}"),
        }
    }
}

impl std::error::Error for ScoreError {}

/// Score two decoded frames over the whole frame with every metric.
///
/// # Errors
///
/// Any [`MetricError`] the metrics raise (size mismatch, empty region).
pub fn score_frames(frame: &Frame, reference: &Frame) -> Result<LookScore, MetricError> {
    metric::compare(frame, reference, Region::Full).map(LookScore::from)
}

/// Score a capture PNG against a reference PNG.
///
/// Decodes both through the bounded PNG path, then scores the whole frame.
/// The error names the refusing side, so a bad capture never reads as a bad
/// reference.
///
/// # Errors
///
/// [`ScoreError::Frame`] when the capture refuses, [`ScoreError::Reference`]
/// when the reference refuses.
pub fn score_pngs(frame_png: &[u8], reference_png: &[u8]) -> Result<LookScore, ScoreError> {
    let frame = Frame::decode_png(frame_png).map_err(ScoreError::Frame)?;
    let reference = Frame::decode_png(reference_png).map_err(ScoreError::Reference)?;
    score_frames(&frame, &reference).map_err(ScoreError::Frame)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::look::metric::SSIM_ONE;
    use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};

    const W: u32 = 64;
    const H: u32 = 48;

    fn sand_at(x: u32, y: u32) -> [u8; 3] {
        let t = i32::try_from((x * 7 + y * 13 + (x * y) % 5) % 23).expect("small") - 11;
        let base = [194i32, 160, 104];
        base.map(|c| u8::try_from((c + t).clamp(0, 255)).expect("clamped"))
    }

    fn brighten20(rgb: [u8; 3]) -> [u8; 3] {
        rgb.map(|v| u8::try_from(((u32::from(v) * 6 + 2) / 5).min(255)).expect("clamped"))
    }

    fn rgba_of(paint: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        let mut rgba = Vec::new();
        for y in 0..H {
            for x in 0..W {
                let [r, g, b] = paint(x, y);
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
        }
        rgba
    }

    fn png_of(rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        PngEncoder::new(&mut out)
            .write_image(rgba, W, H, ExtendedColorType::Rgba8)
            .expect("png encodes");
        out
    }

    #[test]
    fn look_metric_equal_pngs_score_ssim_one_and_delta_e_zero() {
        let png = png_of(&rgba_of(sand_at));
        let score = score_pngs(&png, &png).expect("equal pngs score");
        assert_eq!(score.ssim_x10000, SSIM_ONE);
        assert_eq!(score.mean_de_x100, 0);
        assert_eq!((score.hist_tv_x10000, score.hist_emd_x10000), (0, 0));
    }

    #[test]
    fn look_metric_brightened_sand_drops_ssim_below_9800() {
        let reference = png_of(&rgba_of(sand_at));
        let live = png_of(&rgba_of(|x, y| {
            if y >= 8 {
                brighten20(sand_at(x, y))
            } else {
                sand_at(x, y)
            }
        }));
        let score = score_pngs(&live, &reference).expect("brightened scores");
        assert!(
            score.ssim_x10000 < 9800,
            "brightened sand must drop SSIM, got {}",
            score.ssim_x10000
        );
        assert!(score.mean_de_x100 > 0);
        assert!(score.hist_tv_x10000 > 0);
        let again = score_pngs(&live, &reference).expect("scores again");
        assert_eq!(score, again);
    }

    #[test]
    fn look_metric_bad_bytes_name_the_refusing_side() {
        let good = png_of(&rgba_of(sand_at));
        assert!(matches!(
            score_pngs(b"not a png", &good),
            Err(ScoreError::Frame(_))
        ));
        assert!(matches!(
            score_pngs(&good, b"not a png"),
            Err(ScoreError::Reference(_))
        ));
        let frame_err = score_pngs(b"not a png", &good).unwrap_err();
        assert!(frame_err.to_string().starts_with("frame refused"));
        let ref_err = score_pngs(&good, b"not a png").unwrap_err();
        assert!(ref_err.to_string().starts_with("reference refused"));
    }
}
