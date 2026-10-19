//! Overlay: the in-game perf overlay model and the hitch catcher.
//!
//! Consumer: the arena window (`crates/game` `windowed.rs`) toggles
//! [`Overlay`], calls [`Overlay::rows`] once per frame to draw, and feeds each
//! finished frame to [`HitchCatcher::observe`]. This module is the pure model:
//! it decides which rows exist and when a hitch is worth a file. Nothing here
//! opens a window, draws a pixel or reads a clock.
//!
//! Rows: the latest frame's ms, its allocation count, then the top
//! [`OVERLAY_TOP_PASSES`] GPU passes by ms. Pass timings arrive from
//! [`crate::profiler::GpuSample`] and stay empty until PF25 lands; the overlay
//! then prints `none` instead of inventing a timing source.
//! Draws: [`Overlay::draw_count`] is 0 while the toggle is off, so the off path
//! costs no draw; on, it equals the [`Overlay::rows`] length.
//!
//! Hitch: a frame over twice the median of the last [`HITCH_WINDOW_FRAMES`]
//! frames writes those frames to `hitch-<UTC>.json` (Dagor lagCatcher idea).
//! One hitch event writes one file: a run of slow frames stays latched until a
//! frame comes back under the threshold. A second event in the same second
//! takes a `-2` suffix instead of overwriting the first report.
//!
//! Units: milliseconds and allocation counts. Determinism: the decision path
//! takes the UTC stamp and the output directory as arguments, so the same
//! frames and stamp write the same bytes twice; the caller passes the wall
//! clock. Errors: [`OverlayError`] for a refused sample and [`HitchError`] for
//! a bad stamp or a failed write; nothing panics.

use std::collections::VecDeque;
use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};

use crate::profiler::GpuSample;

/// GPU pass rows drawn under the frame and allocation rows.
pub const OVERLAY_TOP_PASSES: usize = 3;
/// Frame reports the hitch catcher keeps and writes: the row's 120 frames.
pub const HITCH_WINDOW_FRAMES: usize = 120;
/// A frame is a hitch over this multiple of the window median ms.
pub const HITCH_FACTOR: f64 = 2.0;
/// Hitch reports one stamp may name before the writer refuses (`-2` … suffix).
const HITCH_REPORTS_PER_STAMP: u32 = 999;

// ---------------------------------------------------------------------------
// Samples: one frame plus the pass timings measured for it.
// ---------------------------------------------------------------------------

/// Why a sample was refused. Timings must be finite and non-negative; a wall
/// clock never produces anything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayError {
    /// The named row carried NaN, infinite or negative ms.
    InvalidMs {
        /// Which row (`frame` or a pass name).
        what: &'static str,
    },
}

impl Display for OverlayError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMs { what } => {
                write!(f, "overlay: {what} ms is not finite non-negative")
            }
        }
    }
}

impl std::error::Error for OverlayError {}

/// One measured GPU pass: its name and ms. The name stays open (`PF25` reports
/// more passes than [`crate::profiler::GpuPass`] lists today).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PassTiming {
    /// Pass name as the render receipt prints it.
    pub pass: &'static str,
    /// Pass time in ms, finite and non-negative.
    pub ms: f64,
}

impl PassTiming {
    /// Build a pass row, refusing a bad timing.
    pub fn new(pass: &'static str, ms: f64) -> Result<Self, OverlayError> {
        if !ms.is_finite() || ms < 0.0 {
            return Err(OverlayError::InvalidMs { what: pass });
        }
        Ok(Self { pass, ms })
    }

    /// Bridge from a profiler sample: GPU ms when the operator's run resolved
    /// it, otherwise the CPU reference ms. No timing source is invented here.
    pub fn from_sample(sample: GpuSample) -> Result<Self, OverlayError> {
        Self::new(sample.pass.name(), sample.gpu_ms.unwrap_or(sample.cpu_ms))
    }
}

/// One finished frame the overlay draws from and the catcher keeps.
#[derive(Clone, Debug, PartialEq)]
pub struct FrameSample {
    /// Frame number as the window counted it.
    pub frame: u64,
    /// Frame time in ms, finite and non-negative.
    pub frame_ms: f64,
    /// Allocations the counting allocator charged to this frame.
    pub allocations: u64,
    /// Pass timings for this frame; empty until `PF25` feeds them.
    pub passes: Vec<PassTiming>,
}

impl FrameSample {
    /// Build a frame row, refusing a bad frame time.
    pub fn new(
        frame: u64,
        frame_ms: f64,
        allocations: u64,
        passes: Vec<PassTiming>,
    ) -> Result<Self, OverlayError> {
        if !frame_ms.is_finite() || frame_ms < 0.0 {
            return Err(OverlayError::InvalidMs { what: "frame" });
        }
        Ok(Self {
            frame,
            frame_ms,
            allocations,
            passes,
        })
    }
}

/// The costliest `limit` passes, descending by ms; ties keep input order, so
/// the same frame reports the same passes twice.
#[must_use]
pub fn top_passes(passes: &[PassTiming], limit: usize) -> Vec<PassTiming> {
    let mut sorted = passes.to_vec();
    sorted.sort_by(|a, b| b.ms.total_cmp(&a.ms));
    sorted.truncate(limit);
    sorted
}

// ---------------------------------------------------------------------------
// Overlay: rows to draw, and a draw count the off path keeps at zero.
// ---------------------------------------------------------------------------

/// One overlay line: a stable label plus the text the window renders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OverlayRow {
    /// Row group: `frame`, `alloc` or `gpu`. Stable, so a window can style it.
    pub label: &'static str,
    /// Text to draw; numbers vary, the shape does not.
    pub text: String,
}

/// The perf overlay toggle and its row model. Off by default, matching the
/// profiler switch: the game shows nothing until someone asks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Overlay {
    enabled: bool,
}

impl Overlay {
    /// A fresh overlay: off.
    #[must_use]
    pub const fn new() -> Self {
        Self { enabled: false }
    }

    /// Whether the overlay would draw this frame.
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Set the toggle (the window's keybind calls this).
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
    }

    /// Flip the toggle and return the new state, so a keybind can report it.
    pub fn toggle(&mut self) -> bool {
        self.enabled = !self.enabled;
        self.enabled
    }

    /// Rows for the latest frame: frame ms, allocations, then the top passes
    /// (or one `none` row). Off, or with no frame yet, there are no rows.
    #[must_use]
    pub fn rows(&self, frames: &[FrameSample]) -> Vec<OverlayRow> {
        if !self.enabled {
            return Vec::new();
        }
        let Some(last) = frames.last() else {
            return Vec::new();
        };
        let mut rows = vec![
            OverlayRow {
                label: "frame",
                text: format!("{:.2} ms", last.frame_ms),
            },
            OverlayRow {
                label: "alloc",
                text: format!("{} / frame", last.allocations),
            },
        ];
        rows.extend(pass_rows(last));
        rows
    }

    /// Draw calls this frame costs: 0 off, otherwise one per row. The accept
    /// line's off-cost proof reads this without a window.
    #[must_use]
    pub fn draw_count(&self, frames: &[FrameSample]) -> usize {
        if !self.enabled {
            return 0;
        }
        self.rows(frames).len()
    }
}

/// Pass rows for one frame: the top passes, or a single `none` until `PF25`.
fn pass_rows(frame: &FrameSample) -> Vec<OverlayRow> {
    let top = top_passes(&frame.passes, OVERLAY_TOP_PASSES);
    if top.is_empty() {
        return vec![OverlayRow {
            label: "gpu",
            text: "none".to_owned(),
        }];
    }
    top.into_iter()
        .map(|pass| OverlayRow {
            label: "gpu",
            text: format!("{} {:.2} ms", pass.pass, pass.ms),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Hitch catcher: one file per hitch event, naming the slow pass.
// ---------------------------------------------------------------------------

/// Why no hitch report was written. A bad stamp is refused rather than
/// sanitized: the caller owns the filename.
#[derive(Debug)]
pub enum HitchError {
    /// The UTC stamp is empty or holds a character a filename must not carry.
    BadStamp {
        /// The rejected stamp.
        stamp: String,
    },
    /// The report directory or file could not be written; the cause is carried.
    Io(std::io::Error),
}

impl Display for HitchError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadStamp { stamp } => write!(
                f,
                "hitch report: stamp {stamp:?} is empty or not filename-safe \
                 (use yyyy-MM-ddTHHmmZ)"
            ),
            Self::Io(err) => write!(f, "hitch report: {err}"),
        }
    }
}

impl std::error::Error for HitchError {}

/// The hitch catcher: a ring of the last [`HITCH_WINDOW_FRAMES`] frames and a
/// latch, so one run of slow frames writes one report.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HitchCatcher {
    window: VecDeque<FrameSample>,
    latched: bool,
}

impl HitchCatcher {
    /// A catcher with an empty window.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            window: VecDeque::new(),
            latched: false,
        }
    }

    /// Frames held now; never more than [`HITCH_WINDOW_FRAMES`].
    #[must_use]
    pub fn window_len(&self) -> usize {
        self.window.len()
    }

    /// Feed one finished frame and write a report when it is a hitch.
    ///
    /// The frame joins the window first, then the window median decides. A
    /// frame over [`HITCH_FACTOR`] times the median writes the window to
    /// `<dir>/hitch-<utc>.json` and returns its path. A run of slow frames
    /// stays latched after the first write, so it produces one file; a frame
    /// back under the threshold unlatches. `utc` is the caller's wall clock
    /// (for example `2026-10-19T1200Z`), so tests are deterministic.
    pub fn observe(
        &mut self,
        frame: FrameSample,
        utc: &str,
        dir: &Path,
    ) -> Result<Option<PathBuf>, HitchError> {
        self.window.push_back(frame.clone());
        while self.window.len() > HITCH_WINDOW_FRAMES {
            self.window.pop_front();
        }
        let median = median_of(self.window.iter().map(|f| f.frame_ms).collect());
        if frame.frame_ms <= HITCH_FACTOR * median {
            self.latched = false;
            return Ok(None);
        }
        if self.latched {
            return Ok(None);
        }
        self.latched = true;
        let path = hitch_path(dir, utc)?;
        let report = report_json(&self.window, &frame, utc, median);
        std::fs::write(&path, report).map_err(HitchError::Io)?;
        Ok(Some(path))
    }
}

/// Median of the samples; `0.0` for an empty slice. Sorts a copy, so the
/// caller's order survives and the same samples give the same median twice.
fn median_of(mut ms: Vec<f64>) -> f64 {
    if ms.is_empty() {
        return 0.0;
    }
    ms.sort_by(f64::total_cmp);
    let mid = ms.len() / 2;
    if ms.len().is_multiple_of(2) {
        (ms[mid - 1] + ms[mid]) / 2.0
    } else {
        ms[mid]
    }
}

/// `<dir>/hitch-<utc>.json`, or `-2`, `-3` … when that name is taken, so two
/// hitch events stamped in the same second never overwrite each other. The
/// stamp must be filename-safe, so a caller cannot walk out of `dir`.
fn hitch_path(dir: &Path, utc: &str) -> Result<PathBuf, HitchError> {
    let safe = !utc.is_empty()
        && utc
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'));
    if !safe {
        return Err(HitchError::BadStamp {
            stamp: utc.to_owned(),
        });
    }
    std::fs::create_dir_all(dir).map_err(HitchError::Io)?;
    for seq in 1..=HITCH_REPORTS_PER_STAMP {
        let name = if seq == 1 {
            format!("hitch-{utc}.json")
        } else {
            format!("hitch-{utc}-{seq}.json")
        };
        let path = dir.join(name);
        if !path.exists() {
            return Ok(path);
        }
    }
    Err(HitchError::Io(std::io::Error::other(format!(
        "more than {HITCH_REPORTS_PER_STAMP} hitch reports at {utc}"
    ))))
}

/// The report body: header, the slow pass, then the whole window.
fn report_json(
    window: &VecDeque<FrameSample>,
    spike: &FrameSample,
    utc: &str,
    median_ms: f64,
) -> String {
    let mut out = String::from("{\n");
    out.push_str("  \"schema\": \"engine2040.hitch/1\",\n");
    out.push_str(&format!("  \"utc\": \"{utc}\",\n"));
    out.push_str(&format!("  \"frame\": {},\n", spike.frame));
    out.push_str(&format!("  \"frame_ms\": {:.3},\n", spike.frame_ms));
    out.push_str(&format!("  \"median_ms\": {:.3},\n", median_ms));
    out.push_str(&format!(
        "  \"threshold_ms\": {:.3},\n",
        HITCH_FACTOR * median_ms
    ));
    out.push_str(&format!("  \"allocations\": {},\n", spike.allocations));
    out.push_str(&format!("  \"window_frames\": {},\n", window.len()));
    out.push_str(&slow_pass_json(spike));
    out.push_str("  \"window\": [\n");
    out.push_str(&window_json(window));
    out.push_str("  ]\n}\n");
    out
}

/// The spike frame's costliest pass, or `null` while `PF25` has no timings.
fn slow_pass_json(spike: &FrameSample) -> String {
    match top_passes(&spike.passes, 1).first() {
        Some(slow) => format!(
            "  \"slow_pass\": \"{}\",\n  \"slow_pass_ms\": {:.3},\n",
            slow.pass, slow.ms
        ),
        None => "  \"slow_pass\": null,\n".to_owned(),
    }
}

/// The window's frame rows, comma separated. Names come from the caller and
/// static pass names, so no JSON escaping is needed beyond the stamp check.
fn window_json(window: &VecDeque<FrameSample>) -> String {
    let rows: Vec<String> = window.iter().map(frame_json).collect();
    let mut out = rows.join(",\n");
    out.push('\n');
    out
}

/// One window frame row.
fn frame_json(frame: &FrameSample) -> String {
    let passes: Vec<String> = frame
        .passes
        .iter()
        .map(|pass| format!("{{\"pass\": \"{}\", \"ms\": {:.3}}}", pass.pass, pass.ms))
        .collect();
    format!(
        "    {{\"frame\": {}, \"frame_ms\": {:.3}, \"allocations\": {}, \"passes\": [{}]}}",
        frame.frame,
        frame.frame_ms,
        frame.allocations,
        passes.join(", ")
    )
}

#[cfg(test)]
mod tests {
    use engine_core::{splitmix64, Rng};

    use super::*;

    /// Fixed stamp: the catcher never reads a clock, so tests pass one.
    const STAMP: &str = "2026-10-19T1200Z";
    /// Test root under the OS temp dir; removed before each test.
    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("engine2040-perf-hitch")
            .join(test);
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn sample(frame: u64, frame_ms: f64, passes: Vec<PassTiming>) -> FrameSample {
        FrameSample::new(frame, frame_ms, 128, passes).expect("finite frame ms")
    }

    fn pass(name: &'static str, ms: f64) -> PassTiming {
        PassTiming::new(name, ms).expect("finite pass ms")
    }

    /// Files in `dir`, sorted by name; empty when the dir was never created.
    fn files(dir: &Path) -> Vec<PathBuf> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut paths: Vec<PathBuf> = entries
            .map(|entry| entry.expect("readable entry").path())
            .collect();
        paths.sort();
        paths
    }

    #[test]
    fn hitch_spike_writes_one_report_naming_the_slow_pass() {
        // Seeded baseline: 119 healthy frames at 16..19 ms, then one frame at
        // 3x their median. The overlay's four pass timings put ra1.forward on
        // top, so the report must name it.
        let dir = temp_dir("spike");
        let mut rng = Rng::new(splitmix64(27));
        let mut catcher = HitchCatcher::new();
        let mut frames = Vec::new();
        for frame in 1..=119 {
            let jitter = f64::from(u32::try_from(rng.next_u64() % 4).unwrap_or(0));
            frames.push(sample(frame, 16.0 + jitter, Vec::new()));
        }
        let baseline: Vec<f64> = frames.iter().map(|f| f.frame_ms).collect();
        let spike_ms = 3.0 * median_of(baseline.clone());
        let mut all = baseline;
        all.push(spike_ms);
        let expected_median = median_of(all);
        for frame in frames {
            let written = catcher.observe(frame, STAMP, &dir).expect("quiet frame");
            assert!(written.is_none(), "a baseline frame must not write");
        }
        let spike = sample(
            120,
            spike_ms,
            vec![
                pass("rl1.sun_shadow", 1.0),
                pass("ra1.forward", 9.5),
                pass("rp1.post", 3.0),
                pass("rl2.ibl", 0.25),
            ],
        );
        let written = catcher.observe(spike, STAMP, &dir).expect("spike writes");
        let written = written.expect("a 3x spike writes a report");
        let found = files(&dir);
        assert_eq!(found, vec![written.clone()], "exactly one report file");
        assert!(written.ends_with(format!("hitch-{STAMP}.json")));
        let text = std::fs::read_to_string(&written).expect("report reads back");
        assert!(text.contains("\"frame\": 120"), "names the spike frame");
        assert!(text.contains(&format!("\"frame_ms\": {spike_ms:.3}")));
        assert!(
            text.contains("\"slow_pass\": \"ra1.forward\""),
            "names the slow pass"
        );
        assert!(text.contains("\"slow_pass_ms\": 9.500"));
        assert!(text.contains("\"window_frames\": 120"));
        assert!(text.contains("\"allocations\": 128"));
        assert!(text.contains(&format!("\"median_ms\": {expected_median:.3}")));
        assert!(text.contains(&format!(
            "\"threshold_ms\": {:.3}",
            HITCH_FACTOR * expected_median
        )));
    }

    #[test]
    fn hitch_quiet_frames_write_no_file() {
        // F2P: with no frame over 2x the median, no report and no directory.
        let dir = temp_dir("quiet");
        let mut catcher = HitchCatcher::new();
        for frame in 1..=120 {
            let written = catcher
                .observe(sample(frame, 16.0, Vec::new()), STAMP, &dir)
                .expect("quiet frame");
            assert!(written.is_none());
        }
        assert_eq!(catcher.window_len(), HITCH_WINDOW_FRAMES);
        assert!(!dir.exists(), "no hitch file without a hitch");
    }

    #[test]
    fn hitch_run_of_slow_frames_writes_one_file() {
        // A hitch lasts 30 frames; the latch keeps it to one report.
        let dir = temp_dir("latch");
        let mut catcher = HitchCatcher::new();
        for frame in 1..=90 {
            catcher
                .observe(sample(frame, 16.0, Vec::new()), STAMP, &dir)
                .expect("baseline");
        }
        let mut written = 0;
        for frame in 91..=120 {
            if catcher
                .observe(sample(frame, 48.0, Vec::new()), STAMP, &dir)
                .expect("slow frame")
                .is_some()
            {
                written += 1;
            }
        }
        assert_eq!(written, 1, "one hitch event, one file");
        assert_eq!(files(&dir).len(), 1);
        // A frame back under the threshold re-arms the catcher.
        catcher
            .observe(sample(121, 16.0, Vec::new()), STAMP, &dir)
            .expect("recovery");
        let again = catcher
            .observe(sample(122, 48.0, Vec::new()), STAMP, &dir)
            .expect("second hitch");
        assert!(again.is_some(), "a new hitch event writes again");
        assert_eq!(files(&dir).len(), 2);
        assert!(
            again
                .as_deref()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.contains("-2.json")),
            "the same-second report takes the -2 suffix"
        );
    }

    #[test]
    fn hitch_overlay_off_draws_nothing() {
        // The accept line: overlay off costs 0 draws, on costs one per row.
        let frames = vec![sample(
            7,
            16.0,
            vec![
                pass("ra1.forward", 9.5),
                pass("rp1.post", 3.0),
                pass("rl1.sun_shadow", 1.0),
                pass("rl2.ibl", 0.25),
            ],
        )];
        let overlay = Overlay::new();
        assert!(!overlay.is_enabled());
        assert_eq!(overlay.draw_count(&frames), 0);
        assert!(overlay.rows(&frames).is_empty());
        let mut overlay = overlay;
        assert!(overlay.toggle(), "toggle turns it on");
        let rows = overlay.rows(&frames);
        assert_eq!(rows.len(), 2 + OVERLAY_TOP_PASSES);
        assert_eq!(overlay.draw_count(&frames), rows.len());
        assert_eq!(rows[0].label, "frame");
        assert_eq!(rows[1].label, "alloc");
        let gpu: Vec<&str> = rows[2..].iter().map(|row| row.text.as_str()).collect();
        assert_eq!(
            gpu,
            vec![
                "ra1.forward 9.50 ms",
                "rp1.post 3.00 ms",
                "rl1.sun_shadow 1.00 ms"
            ],
            "top three passes, descending"
        );
        assert!(!overlay.toggle(), "toggle turns it off again");
        assert_eq!(overlay.draw_count(&frames), 0);
    }

    #[test]
    fn hitch_overlay_renders_none_without_gpu_passes() {
        // PF25 is open: no timings must render `none`, not an invented number.
        let mut overlay = Overlay::new();
        overlay.set_enabled(true);
        let frames = vec![sample(1, 16.0, Vec::new())];
        let rows = overlay.rows(&frames);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[2].label, "gpu");
        assert_eq!(rows[2].text, "none");
        assert_eq!(overlay.draw_count(&frames), 3);
        assert!(overlay.rows(&[]).is_empty(), "no frame yet, no rows");
        assert_eq!(overlay.draw_count(&[]), 0);
    }

    #[test]
    fn hitch_bad_stamp_is_refused_and_writes_nothing() {
        let dir = temp_dir("stamp");
        let mut catcher = HitchCatcher::new();
        for frame in 1..=10 {
            catcher
                .observe(sample(frame, 16.0, Vec::new()), STAMP, &dir)
                .expect("baseline");
        }
        let err = catcher
            .observe(sample(11, 48.0, Vec::new()), "../escape", &dir)
            .expect_err("a path-walking stamp is refused");
        assert!(matches!(err, HitchError::BadStamp { .. }));
        assert!(!dir.exists());
        assert!(HitchError::BadStamp {
            stamp: "../escape".to_owned()
        }
        .to_string()
        .contains("../escape"));

        let mut overlay = Overlay::new();
        overlay.set_enabled(true);
        let offline: Vec<FrameSample> = Vec::new();
        assert_eq!(overlay.draw_count(&offline), 0);
    }

    #[test]
    fn hitch_frame_sample_refuses_bad_ms() {
        assert!(matches!(
            FrameSample::new(1, f64::NAN, 0, Vec::new()),
            Err(OverlayError::InvalidMs { what: "frame" })
        ));
        assert!(matches!(
            PassTiming::new("ra1.forward", -1.0),
            Err(OverlayError::InvalidMs { .. })
        ));
        assert!(OverlayError::InvalidMs { what: "frame" }
            .to_string()
            .contains("finite"));
    }
}
