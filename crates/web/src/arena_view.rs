//! What the arena page shows and measures, as pure functions so they test without a GPU: the
//! canvas size for a CSS box, the frame line, the JSON the headless check reads, and the
//! verdict on a captured frame (WEB5).

use engine_audio::FightCue;

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

/// The value of `name` in a `?name=value&other` search (`None` when the key is absent or bare).
/// QW10 reads `?bundle=off|on|ab` this way.
#[must_use]
pub fn flag_value<'a>(search: &'a str, name: &str) -> Option<&'a str> {
    search
        .trim_start_matches('?')
        .split('&')
        .find_map(|pair| match pair.split_once('=') {
            Some((key, value)) if key == name => Some(value),
            _ => None,
        })
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

/// Everything the receipt line says, in one place: the tier, the counters, the frame time, the
/// draw count, the first error text, the last device loss (the browser's reason and message, and
/// who ended the device) and the scene the frame drew (PW37).
#[derive(Clone, Copy, Debug, Default)]
pub struct Receipt<'a> {
    pub tier: &'a str,
    pub counters: Counters,
    pub frame_ms: f64,
    pub cpu_ms: f64,
    pub draws: u32,
    pub first_error: Option<&'a str>,
    pub last_loss: Option<&'a LossNote>,
    /// The scene the last frame drew, as [`scene_word`] names it.
    pub scene: &'a str,
    /// The word for the last fight cue the page saw (`PARRY`, `BLOCK`, ...), if any.
    pub cue: Option<&'a str>,
    /// What the frame gate measured against the reference capture, once a frame was captured.
    pub gate: Option<Gate>,
    /// QW10: draw calls and CPU milliseconds per frame, as [`crate::perf_window::Run::json`]
    /// builds it. Already JSON, printed verbatim.
    pub perf: Option<&'a str>,
}

/// The receipt JSON: the tier, counters, the frame time, the draw count, the last device loss
/// (the browser's reason and message, and who ended the device), the first error text and the
/// name of the scene the frame drew.
#[must_use]
pub fn receipt_line(r: &Receipt<'_>) -> String {
    let counters = &r.counters;
    let mut o = Object::new();
    o.bool(
        "ok",
        counters.validation_total() == 0
            && counters.frame_errors == 0
            && counters.late_pipelines == 0,
    )
    .str("tier", r.tier)
    .str("scene", r.scene)
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
    .raw("frameMs", &format!("{:.2}", r.frame_ms))
    .raw("cpuMs", &format!("{:.2}", r.cpu_ms))
    .uint("draws", u64::from(r.draws))
    .uint("steps", counters.steps)
    .uint("maxStepsPerFrame", u64::from(counters.max_steps_per_frame))
    .uint("hiddenSkips", u64::from(counters.hidden_skips))
    .uint("losses", u64::from(counters.losses));
    if let Some(cue) = r.cue {
        o.str("cue", cue);
    }
    if let Some(gate) = r.gate {
        let mut g = Object::new();
        g.raw("ssim", &format!("{:.4}", gate.ssim))
            .raw("bar", &format!("{:.2}", gate.bar))
            .bool("pass", gate.pass);
        o.raw("gate", &g.finish());
    }
    if let Some(perf) = r.perf {
        o.raw("perf", perf);
    }
    if let Some(loss) = r.last_loss {
        let mut note = Object::new();
        note.str("reason", &loss.reason)
            .str("cause", loss.cause.word())
            .str("message", &loss.message);
        o.raw("lastLoss", &note.finish());
    }
    if let Some(text) = r.first_error {
        o.str("firstError", text);
    }
    o.finish()
}

/// The scene name for a drawn frame: the cooked kit's own name once any of it draws resident,
/// `procedural-fight` for a played match on the stand-in ring, `procedural-ring` for the
/// stand-in arena alone. A capture can be checked against the scene the receipt names.
#[must_use]
pub fn scene_word(kit_drawn: usize, played: bool) -> &'static str {
    if kit_drawn > 0 {
        "arena-kit"
    } else if played {
        "procedural-fight"
    } else {
        "procedural-ring"
    }
}

/// The page's word for one frame of fight cues, `None` on a quiet tick. Two cues can land on
/// one tick (a block and the chip hit behind it), so the words have a fixed order: a knockdown
/// speaks loudest, then a parry, a block, a plain hit, a dodge and an empty stamina bar.
#[must_use]
pub fn cue_word(cues: &[FightCue]) -> Option<&'static str> {
    let rank = |cue: &FightCue| match *cue {
        FightCue::Knockdown { .. } => Some(("DOWN", 6u8)),
        FightCue::Parried { .. } => Some(("PARRY", 5)),
        FightCue::Blocked { .. } => Some(("BLOCK", 4)),
        FightCue::Hit { .. } => Some(("HIT", 3)),
        FightCue::Dodged { .. } => Some(("DODGE", 2)),
        FightCue::StaminaOut { .. } => Some(("TIRED", 1)),
    };
    cues.iter()
        .filter_map(rank)
        .max_by_key(|(_, weight)| *weight)
        .map(|(word, _)| word)
}

/// The luma grid the frame gate compares: a fixed picture, small enough to keep in the module
/// and coarse enough that two runs of the same scene land on it.
pub const GRID_W: usize = 160;
/// The grid's height in cells: 90 rows over a 16:9 frame.
pub const GRID_H: usize = 90;
/// The SSIM a captured frame must reach against the reference capture to pass the gate.
pub const GATE_BAR: f64 = 0.95;
/// The reference capture of the cooked arena: the luma grid of a real page capture, one luma
/// byte per cell. It is the picture the gate measures against.
pub const REFERENCE: &[u8] = include_bytes!("../fixtures/arena-gate-reference.luma");

/// The luma grid of an RGBA frame: every pixel lands in one cell of a fixed 160x90 grid and a
/// cell holds the mean luma of its pixels (0.2126 r + 0.7152 g + 0.0722 b, colours in 0..1).
/// A frame too small for the grid gives an empty grid, and an empty grid fails the gate.
#[must_use]
pub fn luma_grid(rgba: &[u8], width: u32, height: u32) -> Vec<f64> {
    let mut grid = vec![0.0f64; GRID_W * GRID_H];
    let (w, h) = (
        usize::try_from(width).unwrap_or(0),
        usize::try_from(height).unwrap_or(0),
    );
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return Vec::new();
    }
    // Every cell owns the pixels between its own share of the width and the next one's, so
    // each pixel is counted once and a cell holds a true average of its own pixels.
    for gy in 0..GRID_H {
        let y0 = (gy * h) / GRID_H;
        let y1 = ((gy + 1) * h) / GRID_H;
        for gx in 0..GRID_W {
            let x0 = (gx * w) / GRID_W;
            let x1 = ((gx + 1) * w) / GRID_W;
            let mut sum = 0.0f64;
            let mut count = 0u32;
            for y in y0..y1 {
                for x in x0..x1 {
                    let at = (y * w + x) * 4;
                    sum += 0.2126 * f64::from(rgba[at])
                        + 0.7152 * f64::from(rgba[at + 1])
                        + 0.0722 * f64::from(rgba[at + 2]);
                    count += 1;
                }
            }
            if count > 0 {
                grid[gy * GRID_W + gx] = sum / f64::from(count) / 255.0;
            }
        }
    }
    grid
}

/// The mean SSIM of two luma grids over the 8x8 windows they share: how alike the two
/// pictures are, 1.0 for the same picture.
#[must_use]
pub fn ssim(a: &[f64], b: &[f64]) -> f64 {
    const WIN: usize = 8;
    const C1: f64 = 0.01 * 0.01;
    const C2: f64 = 0.03 * 0.03;
    let cells = f64::from(u32::try_from(WIN * WIN).unwrap_or(u32::MAX));
    let mut total = 0.0f64;
    let mut windows = 0usize;
    let mut y = 0usize;
    while y + WIN <= GRID_H {
        let mut x = 0usize;
        while x + WIN <= GRID_W {
            let mut sa = 0.0f64;
            let mut sb = 0.0f64;
            let mut saa = 0.0f64;
            let mut sbb = 0.0f64;
            let mut sab = 0.0f64;
            for j in 0..WIN {
                for i in 0..WIN {
                    let at = (y + j) * GRID_W + (x + i);
                    let (Some(&va), Some(&vb)) = (a.get(at), b.get(at)) else {
                        return 0.0;
                    };
                    sa += va;
                    sb += vb;
                    saa += va * va;
                    sbb += vb * vb;
                    sab += va * vb;
                }
            }
            let ma = sa / cells;
            let mb = sb / cells;
            let va = saa / cells - ma * ma;
            let vb = sbb / cells - mb * mb;
            let cov = sab / cells - ma * mb;
            total += ((2.0 * ma * mb + C1) * (2.0 * cov + C2))
                / ((ma * ma + mb * mb + C1) * (va + vb + C2));
            windows += 1;
            x += WIN;
        }
        y += WIN;
    }
    if windows == 0 {
        return 0.0;
    }
    total / f64::from(u32::try_from(windows).unwrap_or(u32::MAX))
}

/// What the frame gate measured: the SSIM the page's own capture reached against the reference
/// capture, the bar it had to reach, and whether it did.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gate {
    pub ssim: f64,
    pub bar: f64,
    pub pass: bool,
}

impl Gate {
    /// The gate's own line: `frame gate: ssim=0.9812 bar=0.95 verdict=pass scene=arena-kit`.
    #[must_use]
    pub fn line(self, scene: &str) -> String {
        format!(
            "frame gate: ssim={:.4} bar={:.2} verdict={} scene={scene}",
            self.ssim,
            self.bar,
            if self.pass { "pass" } else { "fail" }
        )
    }
}

/// The reference capture's luma grid: the stored bytes, one luma byte per cell.
#[must_use]
pub fn reference_grid() -> Vec<f64> {
    luma_bytes_grid(REFERENCE)
}

/// A stored luma grid (one byte per cell) as the numbers the gate compares.
#[must_use]
pub fn luma_bytes_grid(bytes: &[u8]) -> Vec<f64> {
    bytes.iter().map(|byte| f64::from(*byte) / 255.0).collect()
}

/// The gate for one captured frame: the SSIM of its luma grid against the reference capture's
/// grid, and whether it reached the bar.
#[must_use]
pub fn frame_gate(page: &[f64]) -> Gate {
    let ssim = ssim(page, &reference_grid());
    Gate {
        ssim,
        bar: GATE_BAR,
        pass: ssim >= GATE_BAR,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second real capture of the same page, from another browser run: the gate's own margin.
    const PAGE_CAPTURE: &[u8] = include_bytes!("../fixtures/arena-gate-page.luma");
    /// A real capture of the stand-in ring (`?nokit`): the gate must tell the cooked arena
    /// from the placeholder.
    const RING_CAPTURE: &[u8] = include_bytes!("../fixtures/arena-gate-ring.luma");

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

    /// A receipt for a healthy page: the tier `web`, the counters given, no error and no loss.
    fn receipt<'a>(counters: &Counters, scene: &'a str) -> Receipt<'a> {
        Receipt {
            tier: "web",
            counters: *counters,
            frame_ms: 16.7,
            cpu_ms: 3.2,
            draws: 13,
            first_error: None,
            last_loss: None,
            scene,
            cue: None,
            gate: None,
            perf: None,
        }
    }

    #[test]
    fn qw10_the_receipt_carries_the_perf_object_verbatim() {
        let counters = Counters::default();
        let mut r = receipt(&counters, "procedural-ring");
        r.perf = Some("{\"mode\":\"on\",\"arm\":{\"draws\":14}}");
        let line = receipt_line(&r);
        assert!(
            line.contains("\"perf\":{\"mode\":\"on\",\"arm\":{\"draws\":14}}"),
            "{line}"
        );
    }

    #[test]
    fn a_flag_value_reads_the_value_and_only_the_whole_key() {
        assert_eq!(flag_value("?bundle=ab&freeze", "bundle"), Some("ab"));
        assert_eq!(flag_value("bundle=off", "bundle"), Some("off"));
        assert_eq!(
            flag_value("?bundle", "bundle"),
            None,
            "a bare key has no value"
        );
        assert_eq!(flag_value("?bundles=ab", "bundle"), None);
        assert_eq!(flag_value("?tier=web", "bundle"), None);
        assert_eq!(flag_value("", "bundle"), None);
    }

    #[test]
    fn the_receipt_is_ok_only_with_no_error_of_any_kind() {
        let healthy = Counters {
            frames: 120,
            ..Counters::default()
        };
        let line = receipt_line(&receipt(&healthy, "arena-kit"));
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
            let mut r = receipt(&bad, "arena-kit");
            r.first_error = Some("bind group \"x\" is invalid");
            let line = receipt_line(&r);
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
        assert!(receipt_line(&receipt(&all, "arena-kit")).contains("\"validationErrors\":9"));
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
        let none = receipt_line(&receipt(&counters, "arena-kit"));
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
        let mut r = receipt(&counters, "arena-kit");
        r.last_loss = Some(&loss);
        let line = receipt_line(&r);
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

    #[test]
    fn the_receipt_names_the_scene_the_frame_drew() {
        // The cooked kit's own name once any of it draws resident.
        assert_eq!(scene_word(1, true), "arena-kit");
        assert_eq!(scene_word(37, true), "arena-kit");
        // A played match on the stand-in ring, and the ring alone.
        assert_eq!(scene_word(0, true), "procedural-fight");
        assert_eq!(scene_word(0, false), "procedural-ring");
        let line = receipt_line(&receipt(&Counters::default(), "arena-kit"));
        assert!(line.contains("\"scene\":\"arena-kit\""), "{line}");
        let ring = receipt_line(&receipt(&Counters::default(), "procedural-ring"));
        assert!(ring.contains("\"scene\":\"procedural-ring\""), "{ring}");
        // A quiet fight has no cue word; a fight that parried names it.
        assert!(!line.contains("\"cue\""), "{line}");
        let mut cued = receipt(&Counters::default(), "arena-kit");
        cued.cue = Some("PARRY");
        assert!(
            receipt_line(&cued).contains("\"cue\":\"PARRY\""),
            "{cued:?}"
        );
    }

    #[test]
    fn the_page_speaks_the_strongest_cue_of_a_tick() {
        let hit = FightCue::Hit {
            attacker: 0,
            victim: 1,
            damage: 120,
        };
        let block = FightCue::Blocked {
            attacker: 1,
            victim: 0,
            chip: 40,
        };
        let parry = FightCue::Parried {
            attacker: 1,
            victim: 0,
        };
        let dodge = FightCue::Dodged {
            attacker: 1,
            victim: 0,
        };
        let tired = FightCue::StaminaOut { fighter: 0 };
        let down = FightCue::Knockdown {
            attacker: 1,
            victim: 0,
        };
        assert_eq!(cue_word(&[]), None, "a quiet tick says nothing");
        assert_eq!(cue_word(&[hit]), Some("HIT"));
        assert_eq!(cue_word(&[block]), Some("BLOCK"));
        assert_eq!(cue_word(&[parry]), Some("PARRY"));
        assert_eq!(cue_word(&[dodge]), Some("DODGE"));
        assert_eq!(cue_word(&[tired]), Some("TIRED"));
        assert_eq!(cue_word(&[down]), Some("DOWN"));
        // Two cues on one tick: the loudest word wins, whatever the order.
        assert_eq!(cue_word(&[hit, block]), Some("BLOCK"));
        assert_eq!(cue_word(&[block, hit]), Some("BLOCK"));
        assert_eq!(cue_word(&[block, parry]), Some("PARRY"));
        assert_eq!(cue_word(&[hit, down]), Some("DOWN"));
        assert_eq!(cue_word(&[tired, dodge]), Some("DODGE"));
    }

    #[test]
    fn the_luma_grid_holds_the_mean_luma_of_every_cell() {
        // A frame with one pixel per grid cell: red in the first cell, green in the second.
        let mut frame = vec![0u8; GRID_W * GRID_H * 4];
        frame[0] = 255; // the first pixel is red
        frame[5] = 255; // the second pixel is green
        let (w, h) = (
            u32::try_from(GRID_W).unwrap_or(160),
            u32::try_from(GRID_H).unwrap_or(90),
        );
        let grid = luma_grid(&frame, w, h);
        assert_eq!(grid.len(), GRID_W * GRID_H);
        assert!((grid[0] - 0.2126).abs() < 1e-9, "{}", grid[0]);
        assert!((grid[1] - 0.7152).abs() < 1e-9, "{}", grid[1]);
        // A frame too small for the grid gives an empty grid: nothing to measure.
        assert!(luma_grid(&frame[..8], 2, 2).is_empty());
        assert!(luma_grid(&frame, 0, 2).is_empty());
    }

    #[test]
    fn the_same_picture_measures_one_and_a_different_one_less() {
        let grid = luma_grid(&[128u8; 64 * 64 * 4], 64, 64);
        assert!(
            (ssim(&grid, &grid) - 1.0).abs() < 1e-9,
            "the same picture is one"
        );
        // A picture of a different brightness is not the same picture.
        let darker: Vec<f64> = grid.iter().map(|v| v * 0.5).collect();
        let value = ssim(&grid, &darker);
        assert!(value < 1.0, "a darker picture is not one: {value}");
        // The gate measures against the reference capture: the reference passes it, a flat
        // grey frame does not.
        let gate = frame_gate(&reference_grid());
        assert!(gate.pass, "{gate:?}");
        assert_eq!(gate.bar, GATE_BAR);
        assert!(
            gate.line("arena-kit").contains("verdict=pass"),
            "{:?}",
            gate.line("arena-kit")
        );
        let grey = frame_gate(&grid);
        assert!(!grey.pass, "a grey frame is not the arena: {grey:?}");
    }

    #[test]
    fn the_gate_measures_the_stored_captures_of_the_real_page() {
        // The reference capture is a real page capture, and the page capture beside it is a
        // second real run of the same page: the two must measure above the bar.
        let reference = reference_grid();
        assert_eq!(
            reference.len(),
            GRID_W * GRID_H,
            "the reference is one grid"
        );
        let page = luma_bytes_grid(PAGE_CAPTURE);
        let gate = frame_gate(&page);
        assert!(
            gate.ssim >= GATE_BAR,
            "a second run of the same page measured {gate:?}, under the bar"
        );
        // The stand-in ring is a different picture: the gate must tell it apart.
        let ring = luma_bytes_grid(RING_CAPTURE);
        let ring_gate = frame_gate(&ring);
        assert!(
            ring_gate.ssim < GATE_BAR,
            "the stand-in ring measured {ring_gate:?}, at or over the bar"
        );
    }
}
