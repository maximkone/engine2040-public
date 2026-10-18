//! What the arena page shows and measures, as pure functions so they test without a GPU: the
//! canvas size for a CSS box, the frame line, the JSON the headless check reads, and the
//! verdict on a captured frame (WEB5).

use crate::recovery::LossNote;
use crate::report::Object;

/// Most pixels a frame renders. A 4K tab at a high device pixel ratio would ask for three or
/// four times the work of 1080p; the page scales the render down to this and the browser
/// stretches the canvas to its CSS box.
pub const MAX_PIXELS: f64 = 1920.0 * 1080.0;

/// Frames drawn before the page asks the GPU for its self-check copy of the canvas. The first
/// frames are where pipelines compile and targets are made; the copy then shows a settled frame.
pub const CAPTURE_AFTER_FRAMES: u64 = 30;

/// The render size for a CSS box of `css_w` x `css_h` at `dpr`: the device pixels of the box,
/// scaled down together (the aspect stays) until both sides fit `max_dim` and the area fits
/// [`MAX_PIXELS`], never below one pixel on a side (a zero surface cannot be configured).
#[must_use]
pub fn frame_size(css_w: f64, css_h: f64, dpr: f64, max_dim: u32) -> (u32, u32) {
    let dpr = if dpr.is_finite() && dpr > 0.0 {
        dpr
    } else {
        1.0
    };
    let side = |css: f64| {
        if css.is_finite() && css >= 1.0 {
            css * dpr
        } else {
            1.0
        }
    };
    let (w, h) = (side(css_w), side(css_h));
    let limit = f64::from(max_dim.max(1));
    let scale = (limit / w)
        .min(limit / h)
        .min((MAX_PIXELS / (w * h)).sqrt())
        .min(1.0);
    let fit = |px: f64| -> u32 { (px * scale).round().clamp(1.0, limit) as u32 };
    (fit(w), fit(h))
}

/// [`frame_size`] for a render scale of `scale_pct` percent (QT6): the box in device pixels
/// (CSS size times `dpr`) times the scale. The scale only lowers the size: a scale over 100 reads
/// as 100 and one under 1 as 1, so the backing store is never bigger than the CSS box times the
/// device pixel ratio (the pixel budget and `max_dim` still cap it, as in [`frame_size`]).
#[must_use]
pub fn scaled_frame_size(
    css_w: f64,
    css_h: f64,
    dpr: f64,
    scale_pct: u32,
    max_dim: u32,
) -> (u32, u32) {
    let dpr = if dpr.is_finite() && dpr > 0.0 {
        dpr
    } else {
        1.0
    };
    let scale = f64::from(scale_pct.clamp(1, 100)) / 100.0;
    frame_size(css_w, css_h, dpr * scale, max_dim)
}

/// True when the page address asks for `name`, as `?name` or `?name=anything` among the other
/// keys (`?tier=web&freeze`). Only whole keys count: `?freezer` is not `freeze`.
#[must_use]
pub fn has_flag(search: &str, name: &str) -> bool {
    search
        .trim_start_matches('?')
        .split('&')
        .any(|pair| pair.split('=').next() == Some(name))
}

/// What a captured frame showed, in plain numbers (the render crate computes them).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptureFacts {
    pub width: u32,
    pub height: u32,
    /// FNV-1a 64 over the RGBA bytes.
    pub hash: u64,
    pub distinct: u64,
    pub dominant_share: f64,
    pub mean_luma: f64,
    pub blank: bool,
}

impl CaptureFacts {
    /// The JSON line the page keeps for the headless check; `hash` is 16 lowercase hex digits.
    #[must_use]
    pub fn line(&self, frame: u64, surface_format: &str) -> String {
        let mut o = Object::new();
        o.bool("ok", !self.blank)
            .uint("frame", frame)
            .uint("width", u64::from(self.width))
            .uint("height", u64::from(self.height))
            .str("hash", &format!("{:016x}", self.hash))
            .uint("distinct", self.distinct)
            .raw("dominantShare", &format!("{:.4}", self.dominant_share))
            .raw("meanLuma", &format!("{:.2}", self.mean_luma))
            .bool("blank", self.blank)
            .str("surfaceFormat", surface_format);
        o.finish()
    }
}

/// The counters the receipt JSON carries; every one is 0 on a healthy run except `frames`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Counters {
    pub frames: u64,
    /// Frames the presenter refused or failed with a typed error.
    pub frame_errors: u64,
    /// Validation errors the frames' own receipts reported.
    pub validation_errors: u64,
    /// Validation errors the browser reported after their frame (error-scope answers).
    pub deferred_validation_errors: u64,
    /// Errors the device raised outside any scope (`uncapturederror`).
    pub uncaptured_errors: u64,
    /// Pipelines built after the loader finished (WEB13); 0 on a healthy page.
    pub late_pipelines: u64,
    /// Fixed simulation steps run (QW11).
    pub steps: u64,
    /// The most steps any one frame ran: 1 on a healthy page.
    pub max_steps_per_frame: u32,
    /// Frames dropped because they spanned a hidden stretch (QT7).
    pub hidden_skips: u32,
    /// Device losses so far, over every device generation (a requested destroy is not one).
    pub losses: u32,
}

impl Counters {
    /// Everything the check counts as a validation error.
    #[must_use]
    pub fn validation_total(&self) -> u64 {
        self.validation_errors + self.deferred_validation_errors + self.uncaptured_errors
    }
}

/// The receipt JSON: the tier, counters, the frame time, the draw count, the last device loss
/// (the browser's reason and message, and who ended the device) and the first error text.
#[must_use]
pub fn receipt_line(
    tier: &str,
    counters: &Counters,
    frame_ms: f64,
    cpu_ms: f64,
    draws: u32,
    first_error: Option<&str>,
    last_loss: Option<&LossNote>,
) -> String {
    let mut o = Object::new();
    o.bool(
        "ok",
        counters.validation_total() == 0
            && counters.frame_errors == 0
            && counters.late_pipelines == 0,
    )
    .str("tier", tier)
    .uint("frames", counters.frames)
    .uint("frameErrors", counters.frame_errors)
    .uint("validationErrors", counters.validation_total())
    .uint("frameValidationErrors", counters.validation_errors)
    .uint(
        "deferredValidationErrors",
        counters.deferred_validation_errors,
    )
    .uint("uncapturedErrors", counters.uncaptured_errors)
    .uint("latePipelines", counters.late_pipelines)
    .raw("frameMs", &format!("{frame_ms:.2}"))
    .raw("cpuMs", &format!("{cpu_ms:.2}"))
    .uint("draws", u64::from(draws))
    .uint("steps", counters.steps)
    .uint("maxStepsPerFrame", u64::from(counters.max_steps_per_frame))
    .uint("hiddenSkips", u64::from(counters.hidden_skips))
    .uint("losses", u64::from(counters.losses));
    if let Some(loss) = last_loss {
        let mut note = Object::new();
        note.str("reason", &loss.reason)
            .str("cause", loss.cause.word())
            .str("message", &loss.message);
        o.raw("lastLoss", &note.finish());
    }
    if let Some(text) = first_error {
        o.str("firstError", text);
    }
    o.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_small_box_renders_at_its_device_pixels() {
        assert_eq!(frame_size(1280.0, 720.0, 1.0, 8192), (1280, 720));
        assert_eq!(frame_size(640.0, 360.0, 2.0, 8192), (1280, 720));
        assert_eq!(frame_size(1280.0, 720.0, 1.5, 8192), (1920, 1080));
    }

    #[test]
    fn a_big_box_scales_down_together_and_keeps_its_aspect() {
        for (w, h, dpr, max) in [
            (3840.0, 2160.0, 1.0, 16384),
            (1920.0, 1080.0, 2.0, 16384),
            (2560.0, 1080.0, 1.25, 2048),
            (500.0, 4000.0, 2.0, 2048),
            (1280.0, 720.0, 1.0, 640),
        ] {
            let (fw, fh) = frame_size(w, h, dpr, max);
            assert!(
                fw <= max && fh <= max,
                "{w}x{h}@{dpr} -> {fw}x{fh} past {max}"
            );
            assert!(
                f64::from(fw) * f64::from(fh) <= MAX_PIXELS * 1.001,
                "{fw}x{fh} past the pixel budget"
            );
            let want = w / h;
            let got = f64::from(fw) / f64::from(fh);
            assert!(
                (got - want).abs() / want < 0.02,
                "{w}x{h}@{dpr} -> {fw}x{fh}: aspect {got} vs {want}"
            );
        }
        assert_eq!(frame_size(3840.0, 2160.0, 1.0, 16384), (1920, 1080));
    }

    #[test]
    fn a_degenerate_box_still_gives_a_surface() {
        for (w, h, dpr) in [
            (0.0, 0.0, 1.0),
            (-5.0, 100.0, 1.0),
            (f64::NAN, 100.0, 1.0),
            (100.0, f64::INFINITY, 1.0),
            (640.0, 360.0, f64::NAN),
            (640.0, 360.0, 0.0),
        ] {
            let (fw, fh) = frame_size(w, h, dpr, 4096);
            assert!(
                fw >= 1 && fh >= 1 && fw <= 4096 && fh <= 4096,
                "{w}x{h}@{dpr} -> {fw}x{fh}"
            );
        }
    }

    #[test]
    fn the_capture_line_names_the_hash_in_sixteen_hex_digits_and_the_verdict() {
        let facts = CaptureFacts {
            width: 1280,
            height: 720,
            hash: 0xaf63_dc4c_8601_ec8c,
            distinct: 4096,
            dominant_share: 0.04286,
            mean_luma: 185.17,
            blank: false,
        };
        let line = facts.line(31, "bgra8unorm");
        assert!(line.contains("\"hash\":\"af63dc4c8601ec8c\""), "{line}");
        assert!(
            line.contains("\"ok\":true") && line.contains("\"blank\":false"),
            "{line}"
        );
        assert!(line.contains("\"dominantShare\":0.0429"), "{line}");
        assert!(!line.contains('\n'));
        let small = CaptureFacts {
            hash: 0xff,
            blank: true,
            ..facts
        }
        .line(3, "rgba8unorm");
        assert!(small.contains("\"hash\":\"00000000000000ff\""), "{small}");
        assert!(
            small.contains("\"ok\":false") && small.contains("\"blank\":true"),
            "{small}"
        );
    }

    #[test]
    fn the_receipt_is_ok_only_with_no_error_of_any_kind() {
        let healthy = Counters {
            frames: 120,
            ..Counters::default()
        };
        let line = receipt_line("web", &healthy, 16.7, 3.2, 13, None, None);
        assert!(
            line.contains("\"ok\":true") && line.contains("\"validationErrors\":0"),
            "{line}"
        );
        assert!(
            line.contains("\"tier\":\"web\"") && line.contains("\"latePipelines\":0"),
            "{line}"
        );
        assert!(!line.contains("firstError"), "{line}");
        for bad in [
            Counters {
                validation_errors: 1,
                ..healthy
            },
            Counters {
                deferred_validation_errors: 1,
                ..healthy
            },
            Counters {
                uncaptured_errors: 1,
                ..healthy
            },
            Counters {
                frame_errors: 1,
                ..healthy
            },
            Counters {
                late_pipelines: 1,
                ..healthy
            },
        ] {
            let line = receipt_line(
                "web",
                &bad,
                16.7,
                3.2,
                13,
                Some("bind group \"x\" is invalid"),
                None,
            );
            assert!(line.contains("\"ok\":false"), "{line}");
            assert!(line.contains("firstError"), "{line}");
        }
        let all = Counters {
            frames: 1,
            frame_errors: 0,
            validation_errors: 2,
            deferred_validation_errors: 3,
            uncaptured_errors: 4,
            late_pipelines: 0,
            ..Counters::default()
        };
        assert_eq!(all.validation_total(), 9, "the three kinds add up");
        assert!(
            receipt_line("web", &all, 1.0, 1.0, 1, None, None).contains("\"validationErrors\":9")
        );
    }

    #[test]
    fn the_receipt_carries_the_steps_the_skips_and_the_last_loss_with_its_reason() {
        let counters = Counters {
            frames: 300,
            steps: 290,
            max_steps_per_frame: 2,
            hidden_skips: 1,
            losses: 1,
            ..Counters::default()
        };
        let none = receipt_line("web", &counters, 16.7, 3.2, 13, None, None);
        for needle in [
            "\"steps\":290",
            "\"maxStepsPerFrame\":2",
            "\"hiddenSkips\":1",
            "\"losses\":1",
        ] {
            assert!(none.contains(needle), "{needle} missing from {none}");
        }
        assert!(!none.contains("lastLoss"), "{none}");
        let loss = LossNote {
            reason: "unknown".into(),
            message: "the GPU process \"crashed\"".into(),
            cause: crate::recovery::Cause::Forced,
        };
        let line = receipt_line("web", &counters, 16.7, 3.2, 13, None, Some(&loss));
        assert!(
            line.contains(
                "\"lastLoss\":{\"reason\":\"unknown\",\"cause\":\"forced\",\"message\":\"the GPU process \\\"crashed\\\"\"}"
            ),
            "{line}"
        );
        assert!(!line.contains('\n'));
    }

    #[test]
    fn a_flag_in_the_address_is_a_whole_key() {
        assert!(has_flag("?freeze", "freeze"));
        assert!(has_flag("?tier=web&freeze", "freeze"));
        assert!(has_flag("freeze=1&tier=web", "freeze"));
        assert!(!has_flag("?freezer", "freeze"));
        assert!(!has_flag("?tier=freeze", "freeze"));
        assert!(!has_flag("", "freeze"));
    }

    #[test]
    fn the_backing_size_is_the_scale_times_the_css_box_and_never_above_it() {
        // A 640x360 box at device pixel ratio 1 and 3.
        for (dpr, scale, want) in [
            (1.0, 100, (640, 360)),
            (1.0, 67, (429, 241)),
            (1.0, 50, (320, 180)),
            (3.0, 100, (1920, 1080)),
            (3.0, 67, (1286, 724)),
            (3.0, 50, (960, 540)),
        ] {
            assert_eq!(
                scaled_frame_size(640.0, 360.0, dpr, scale, 8192),
                want,
                "dpr {dpr} scale {scale}"
            );
        }
        // Over every box, ratio and scale: never above the box in device pixels, and a scale over
        // 100 is the same as 100.
        for (w, h) in [(640.0, 360.0), (1280.0, 720.0), (333.0, 777.0), (1.0, 1.0)] {
            for dpr in [1.0, 1.25, 2.0, 3.0] {
                let full = scaled_frame_size(w, h, dpr, 100, 8192);
                for scale in [0, 1, 10, 50, 67, 99, 100, 101, 150, 100_000] {
                    let (fw, fh) = scaled_frame_size(w, h, dpr, scale, 8192);
                    assert!(fw >= 1 && fh >= 1);
                    assert!(
                        f64::from(fw) <= (w * dpr).round().max(1.0)
                            && f64::from(fh) <= (h * dpr).round().max(1.0),
                        "{w}x{h}@{dpr} scale {scale}: {fw}x{fh}"
                    );
                    if scale >= 100 {
                        assert_eq!((fw, fh), full, "{w}x{h}@{dpr} scale {scale}");
                    }
                }
            }
        }
        // A bad ratio reads as 1.
        assert_eq!(
            scaled_frame_size(640.0, 360.0, f64::NAN, 100, 8192),
            (640, 360)
        );
    }
}
