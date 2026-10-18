//! Machine eyes: one counted machine score per captured frame.
//!
//! Consumer: the window host hands each due capture PNG to
//! [`MachineEyes::note_frame`] once; the endpoint capture hunk is the next
//! caller. Data owner: the capture path owns the PNG bytes; the eyes own one
//! decoded reference copy plus the call counts. Units: calls and faults in
//! frames, scores from [`crate::look_score`]. Determinism: integer only; the
//! same bytes score the same twice. Errors: a bad PNG records
//! [`MachineEyes::last_fault`] and still counts. Budget: one bounded decode
//! plus one compare pass per frame; the reference decodes once.

use crate::look::metric::Frame;
use crate::look_score::{score_frames, LookScore};

/// One scored frame from the eyes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct EyeScore {
    /// Calls so far, including this frame.
    pub calls: u64,
    /// Match against the reference; none without one or on a fault.
    pub score: Option<LookScore>,
    /// True when the PNG did not decode or the match failed.
    pub fault: bool,
}

/// Per-frame machine eyes: one counted score per capture.
pub struct MachineEyes {
    reference: Option<Frame>,
    calls: u64,
    faults: u64,
    last: Option<LookScore>,
    last_fault: Option<String>,
}

impl MachineEyes {
    /// Arm the eyes with an optional reference PNG. A bad reference means no
    /// reference; the eyes still count frames.
    pub fn register(reference_png: Option<&[u8]>) -> Self {
        Self {
            reference: reference_png.and_then(|bytes| Frame::decode_png(bytes).ok()),
            calls: 0,
            faults: 0,
            last: None,
            last_fault: None,
        }
    }

    /// Score one capture PNG exactly once. A bad PNG records
    /// [`Self::last_fault`] and still counts.
    pub fn note_frame(&mut self, png: &[u8]) -> EyeScore {
        self.calls = self.calls.saturating_add(1);
        let frame = match Frame::decode_png(png) {
            Ok(frame) => frame,
            Err(error) => {
                self.faults = self.faults.saturating_add(1);
                self.last = None;
                self.last_fault = Some(error.to_string());
                return EyeScore {
                    calls: self.calls,
                    score: None,
                    fault: true,
                };
            }
        };
        let Some(reference) = self.reference.as_ref() else {
            self.last = None;
            self.last_fault = None;
            return EyeScore {
                calls: self.calls,
                score: None,
                fault: false,
            };
        };
        match score_frames(&frame, reference) {
            Ok(score) => {
                self.last = Some(score);
                self.last_fault = None;
                EyeScore {
                    calls: self.calls,
                    score: Some(score),
                    fault: false,
                }
            }
            Err(error) => {
                self.faults = self.faults.saturating_add(1);
                self.last = None;
                self.last_fault = Some(error.to_string());
                EyeScore {
                    calls: self.calls,
                    score: None,
                    fault: true,
                }
            }
        }
    }

    /// Frames handed so far.
    pub fn calls(&self) -> u64 {
        self.calls
    }

    /// Bad frames so far.
    pub fn faults(&self) -> u64 {
        self.faults
    }

    /// Last match; none without a reference or on a fault.
    pub fn last(&self) -> Option<LookScore> {
        self.last
    }

    /// Last fault words; none when the last frame scored.
    pub fn last_fault(&self) -> Option<&str> {
        self.last_fault.as_deref()
    }
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
    fn look_metric_eyes_score_once_per_frame() {
        let rgba = rgba_of(sand_at);
        let first = png_of(&rgba);
        let rebuilt = png_of(&rgba);
        let mut eyes = MachineEyes::register(Some(&first));
        let one = eyes.note_frame(&first);
        let two = eyes.note_frame(&rebuilt);
        assert_eq!(eyes.calls(), 2);
        assert_eq!((one.calls, two.calls), (1, 2));
        let (a, b) = (one.score.expect("scores"), two.score.expect("scores"));
        assert_eq!((a.ssim_x10000, a.mean_de_x100), (SSIM_ONE, 0));
        assert_eq!(a, b);
        assert_eq!(eyes.last(), Some(a));
        assert_eq!(eyes.faults(), 0);
        assert_eq!(eyes.last_fault(), None);
    }

    #[test]
    fn look_metric_eyes_bad_png_counts_with_fault() {
        let good = png_of(&rgba_of(sand_at));
        let mut eyes = MachineEyes::register(Some(&good));
        let bad = eyes.note_frame(b"not a png");
        assert_eq!((eyes.calls(), eyes.faults()), (1, 1));
        assert!((bad.fault, bad.score.is_none()) == (true, true));
        assert!(!eyes.last_fault().expect("fault words").is_empty());
        let next = eyes.note_frame(&good);
        assert_eq!((eyes.calls(), eyes.faults()), (2, 1));
        assert!(!next.fault);
        assert_eq!(eyes.last(), next.score);
    }

    #[test]
    fn look_metric_eyes_without_reference_count_without_scores() {
        let good = png_of(&rgba_of(sand_at));
        let mut eyes = MachineEyes::register(None);
        let noted = eyes.note_frame(&good);
        assert_eq!((eyes.calls(), eyes.faults()), (1, 0));
        assert!((!noted.fault, noted.score.is_none()) == (true, true));
    }
}
