//! Trace file: one Chrome Trace Event JSON file a trace viewer reads with no
//! trace-viewer code in the engine.
//!
//! Consumer: an operator asks the perf harness for a trace file and opens it
//! in a trace viewer (the same JSON the viewers read). The engine writes
//! nothing itself; this module is the writer and the capture.
//!
//! Units: microseconds for every timestamp and duration, as the format wants.
//! Determinism: the rows are built from the caller's numbers, so the same rows
//! give the same bytes twice; the file carries wall-clock timings, which are
//! evidence only and never enter a simulation hash.
//!
//! Cost: the writer is off the hot path. A trace row is pushed into a `Vec`
//! (one small allocation that grows), and the file is written once at the
//! end, so the cost of a trace is one write of the whole document, not one
//! write per row. The measured cost of writing a 600-frame document is in
//! the module tests below.
//!
//! Donor: thoren-d/tracing-chrome (MIT): the Chrome Trace Event JSON shape,
//! one object per span with `ts`, `dur` and `cat`, and the counter phase for
//! a value series. No donor bytes are vendored and no dependency was added:
//! the writer is pure std, so the trace path stays out of the dependency
//! graph and out of the fixed tick. The JSON escaping is
//! `engine_core::json::escape_into`, the one owner.
//!
//! Error type: [`TraceFileError`]; a bad row, a bad frame count or a failed
//! write is named, not panicked.

use std::fmt::{self, Display, Formatter};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::profiler::{self, FrameReport};

/// Default process id of the trace: one process, so one id.
pub const TRACE_PID: u32 = 1;
/// Default thread id of the trace: the rows come from one capture thread.
pub const TRACE_TID: u32 = 1;

/// The category of a tick span.
pub const CAT_TICK: &str = "tick";
/// The category of a GPU pass span.
pub const CAT_GPU: &str = "gpu";
/// The category of a frame-mark span.
pub const CAT_FRAME: &str = "frame";
/// The category of a counter row.
pub const CAT_COUNTER: &str = "counter";

/// Name of the frame-mark span: one per captured frame.
pub const SPAN_FRAME: &str = "frame";
/// Name of the frame-time counter: one sample per captured frame.
pub const COUNTER_FRAME_MS: &str = "frame_ms";

/// Frames one capture records when the caller names no count.
pub const TRACE_FRAMES_DEFAULT: u32 = 10;
/// Most frames one capture records; bounds the capture time.
pub const TRACE_FRAMES_MAX: u32 = 120;

/// What one row of the trace file says.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// A span with a start and a duration (the `X` phase).
    Span {
        /// The span name, shown as the label.
        name: String,
        /// The category the span belongs to.
        cat: &'static str,
        /// Start timestamp in microseconds.
        ts_us: u64,
        /// Duration in microseconds.
        dur_us: u64,
    },
    /// A counter sample (the `C` phase): a value at one timestamp.
    Counter {
        /// The counter name.
        name: String,
        /// The category the counter belongs to.
        cat: &'static str,
        /// Timestamp of the sample in microseconds.
        ts_us: u64,
        /// The value of the sample.
        value: f64,
    },
}

impl Row {
    /// The timestamp of the row in microseconds.
    #[must_use]
    pub fn ts_us(&self) -> u64 {
        match self {
            Self::Span { ts_us, .. } | Self::Counter { ts_us, .. } => *ts_us,
        }
    }

    /// The duration of the row in microseconds; a counter has none.
    #[must_use]
    pub fn dur_us(&self) -> u64 {
        match self {
            Self::Span { dur_us, .. } => *dur_us,
            Self::Counter { .. } => 0,
        }
    }

    /// The category of the row.
    #[must_use]
    pub fn cat(&self) -> &'static str {
        match self {
            Self::Span { cat, .. } | Self::Counter { cat, .. } => cat,
        }
    }

    /// The name of the row.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Span { name, .. } | Self::Counter { name, .. } => name,
        }
    }
}

/// Why a trace file was refused.
#[derive(Clone, Debug, PartialEq)]
pub enum TraceFileError {
    /// A counter carried a value that is not finite, so it cannot be plotted.
    NonFiniteValue {
        /// The counter that carried the bad value.
        name: String,
    },
    /// The frame count was outside `1..=TRACE_FRAMES_MAX`.
    InvalidFrames {
        /// The frame count that was asked for.
        frames: u32,
    },
    /// The capture itself failed; the reason is carried.
    Capture {
        /// Why the capture refused.
        reason: String,
    },
    /// The file could not be written; the path and the cause are carried.
    Write {
        /// The path that could not be written.
        path: PathBuf,
        /// The operating system's reason.
        reason: String,
    },
}

impl Display for TraceFileError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteValue { name } => {
                write!(f, "trace file: counter {name} value is not finite")
            }
            Self::InvalidFrames { frames } => {
                write!(
                    f,
                    "trace file: frames must be 1..={TRACE_FRAMES_MAX}, got {frames}"
                )
            }
            Self::Capture { reason } => write!(f, "trace file: capture failed: {reason}"),
            Self::Write { path, reason } => write!(f, "trace file: {}: {reason}", path.display()),
        }
    }
}

impl std::error::Error for TraceFileError {}

/// One trace file: the rows, then the JSON bytes they become.
///
/// The rows keep their order, so the file reads in the order the caller
/// pushed them. A counter with a value that is not finite is refused when it
/// is pushed, so a file that exists is always plottable.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraceFile {
    rows: Vec<Row>,
}

impl TraceFile {
    /// An empty trace file.
    #[must_use]
    pub fn new() -> Self {
        Self { rows: Vec::new() }
    }

    /// Push one span: a named piece of work with a start and a duration.
    pub fn push_span(&mut self, name: &str, cat: &'static str, ts_us: u64, dur_us: u64) {
        self.rows.push(Row::Span {
            name: name.to_owned(),
            cat,
            ts_us,
            dur_us,
        });
    }

    /// Push one counter sample: a value at a timestamp.
    ///
    /// # Errors
    ///
    /// Returns [`TraceFileError::NonFiniteValue`] when the value is NaN or
    /// infinite; a counter with such a value cannot be plotted.
    pub fn push_counter(
        &mut self,
        name: &str,
        cat: &'static str,
        ts_us: u64,
        value: f64,
    ) -> Result<(), TraceFileError> {
        if !value.is_finite() {
            return Err(TraceFileError::NonFiniteValue {
                name: name.to_owned(),
            });
        }
        self.rows.push(Row::Counter {
            name: name.to_owned(),
            cat,
            ts_us,
            value,
        });
        Ok(())
    }

    /// The rows of the file, in the order they were pushed.
    #[must_use]
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }

    /// How many rows the file holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// True while the file holds no row.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The file as Chrome Trace Event JSON bytes.
    ///
    /// The shape is the one the trace viewers read: a `traceEvents` array
    /// where every event carries a `ph` phase, and every span carries `ts`,
    /// `dur` and `cat`.
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut out = String::with_capacity(64 + self.rows.len() * 96);
        out.push_str("{\"traceEvents\":[");
        let pid = TRACE_PID.to_string();
        let tid = TRACE_TID.to_string();
        for (index, row) in self.rows.iter().enumerate() {
            if index > 0 {
                out.push(',');
            }
            match row {
                Row::Span {
                    name,
                    cat,
                    ts_us,
                    dur_us,
                } => {
                    out.push_str("{\"name\":\"");
                    engine_core::json::escape_into(&mut out, name);
                    out.push_str("\",\"cat\":\"");
                    engine_core::json::escape_into(&mut out, cat);
                    out.push_str("\",\"ph\":\"X\",\"pid\":");
                    out.push_str(&pid);
                    out.push_str(",\"tid\":");
                    out.push_str(&tid);
                    out.push_str(",\"ts\":");
                    out.push_str(&ts_us.to_string());
                    out.push_str(",\"dur\":");
                    out.push_str(&dur_us.to_string());
                    out.push('}');
                }
                Row::Counter {
                    name,
                    cat,
                    ts_us,
                    value,
                } => {
                    out.push_str("{\"name\":\"");
                    engine_core::json::escape_into(&mut out, name);
                    out.push_str("\",\"cat\":\"");
                    engine_core::json::escape_into(&mut out, cat);
                    out.push_str("\",\"ph\":\"C\",\"pid\":");
                    out.push_str(&pid);
                    out.push_str(",\"tid\":");
                    out.push_str(&tid);
                    out.push_str(",\"ts\":");
                    out.push_str(&ts_us.to_string());
                    out.push_str(",\"args\":{\"");
                    engine_core::json::escape_into(&mut out, name);
                    out.push_str("\":");
                    out.push_str(&counter_value(*value));
                    out.push_str("}}");
                }
            }
        }
        out.push_str("],\"displayTimeUnit\":\"ms\"}");
        out
    }

    /// Write the file to `path`, creating the parent folder when it is
    /// missing.
    ///
    /// The write is one call for the whole document, so the cost of a trace
    /// is one write, not one write per row.
    ///
    /// # Errors
    ///
    /// Returns [`TraceFileError::Write`] when the folder or the file could
    /// not be written.
    pub fn write_to(&self, path: &Path) -> Result<(), TraceFileError> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|error| TraceFileError::Write {
                    path: path.to_path_buf(),
                    reason: format!("{}: {error}", parent.display()),
                })?;
            }
        }
        std::fs::write(path, self.to_json()).map_err(|error| TraceFileError::Write {
            path: path.to_path_buf(),
            reason: error.to_string(),
        })
    }
}

/// Capture `frames` frames of the engine's own measurements as trace rows.
///
/// The capture runs the perf crate's fixed probes once per frame: the four
/// simulation workloads (the tick spans) and the six GPU pass probes (the GPU
/// spans). Every frame becomes twelve rows: one frame span, four tick spans,
/// six GPU spans and one frame-time counter.
///
/// The frame span covers the whole capture call; the parts sit inside it in
/// the order the capture runs them, so a viewer reads one track per name.
/// Only wall-clock milliseconds enter the file, so the capture never touches
/// a simulation hash.
///
/// The profiler's master switch is on for the capture and restored to the
/// caller's value afterwards.
///
/// # Errors
///
/// Returns [`TraceFileError::InvalidFrames`] outside `1..=TRACE_FRAMES_MAX`
/// and [`TraceFileError::Capture`] when the profiler refuses the capture.
pub fn capture_trace(frames: u32) -> Result<TraceFile, TraceFileError> {
    if frames == 0 || frames > TRACE_FRAMES_MAX {
        return Err(TraceFileError::InvalidFrames { frames });
    }
    let was_enabled = profiler::is_enabled();
    profiler::set_enabled(true);
    let captured = capture_frames(frames);
    profiler::set_enabled(was_enabled);
    captured
}

/// One capture call per frame, then the rows of that frame.
fn capture_frames(frames: u32) -> Result<TraceFile, TraceFileError> {
    let mut file = TraceFile::new();
    let mut clock_us: u64 = 0;
    for _ in 0..frames {
        let started = Instant::now();
        let report = profiler::capture(1).map_err(|error| TraceFileError::Capture {
            reason: error.to_string(),
        })?;
        let frame_us = micros_since(started);
        push_frame_rows(&mut file, &report, clock_us, frame_us)?;
        clock_us = clock_us.saturating_add(frame_us);
    }
    Ok(file)
}

/// The rows of one captured frame: the frame span, the tick spans, the GPU
/// spans and the frame-time counter.
///
/// # Errors
///
/// Returns [`TraceFileError::NonFiniteValue`] when the frame time is not a
/// plottable number.
fn push_frame_rows(
    file: &mut TraceFile,
    report: &FrameReport,
    frame_start_us: u64,
    frame_us: u64,
) -> Result<(), TraceFileError> {
    file.push_span(SPAN_FRAME, CAT_FRAME, frame_start_us, frame_us);
    let mut offset_us = frame_start_us;
    for system in &report.systems {
        let dur_us = ms_to_us(system.quantiles.p50_ms);
        file.push_span(system.span, CAT_TICK, offset_us, dur_us);
        offset_us = offset_us.saturating_add(dur_us);
    }
    for pass in &report.passes {
        let dur_us = ms_to_us(pass.quantiles.p50_ms);
        file.push_span(pass.pass.name(), CAT_GPU, offset_us, dur_us);
        offset_us = offset_us.saturating_add(dur_us);
    }
    file.push_counter(
        COUNTER_FRAME_MS,
        CAT_COUNTER,
        frame_start_us,
        report.frame_time_smoothed_ms,
    )
}

/// Microseconds since `started`, saturated at the largest microsecond count.
fn micros_since(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX)
}

/// Milliseconds to whole microseconds; a bad number becomes zero and a
/// huge one saturates.
fn ms_to_us(ms: f64) -> u64 {
    if !ms.is_finite() || ms <= 0.0 {
        return 0;
    }
    // A day in ms: past it the number cannot be a real measurement, and the
    // conversion below would overflow the duration.
    if ms >= 86_400_000.0 {
        return u64::MAX;
    }
    let micros = Duration::from_secs_f64(ms / 1000.0).as_micros();
    u64::try_from(micros).unwrap_or(u64::MAX)
}

/// The counter value as JSON writes it: a finite number, or `null` for a
/// value that is not finite (the push already refused those).
fn counter_value(value: f64) -> String {
    if value.is_finite() {
        format!("{value}")
    } else {
        "null".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder under the OS temp dir, emptied before the test writes into it.
    fn temp_dir(test: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("engine2040-perf-trace")
            .join(test);
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn trace_file_writes_a_json_document_a_viewer_reads() {
        // The file is parsed, not string-matched: a viewer reads the same
        // bytes, so the proof parses them.
        let dir = temp_dir("json");
        let path = dir.join("trace.json");
        let mut file = TraceFile::new();
        file.push_span("tick", CAT_TICK, 0, 16_667);
        file.push_span("ra1.forward", CAT_GPU, 16_667, 2_400);
        file.push_counter(COUNTER_FRAME_MS, CAT_COUNTER, 0, 16.5)
            .expect("finite value");
        file.write_to(&path).expect("the file is written");
        let text = std::fs::read_to_string(&path).expect("the file is readable");
        let value: serde_json::Value =
            serde_json::from_str(&text).expect("the file is one JSON document");
        let events = value["traceEvents"]
            .as_array()
            .expect("the events are an array");
        assert_eq!(events.len(), 3, "{text}");
        // A span row: the phase, the start, the duration and the category.
        assert_eq!(events[0]["ph"], "X", "{text}");
        assert_eq!(events[0]["ts"], 0, "{text}");
        assert_eq!(events[0]["dur"], 16_667, "{text}");
        assert_eq!(events[0]["cat"], CAT_TICK, "{text}");
        assert_eq!(events[0]["pid"], TRACE_PID, "{text}");
        assert_eq!(events[0]["tid"], TRACE_TID, "{text}");
        assert_eq!(events[1]["name"], "ra1.forward", "{text}");
        // A counter row: the phase and the value in the args.
        assert_eq!(events[2]["ph"], "C", "{text}");
        assert_eq!(events[2]["args"][COUNTER_FRAME_MS], 16.5, "{text}");
    }

    #[test]
    fn trace_file_rows_carry_ts_dur_and_cat() {
        // The file the trace viewers read: every span carries a start, a
        // duration and a category, and every row sits in `traceEvents`.
        let mut file = TraceFile::new();
        file.push_span("tick", CAT_TICK, 0, 16_667);
        file.push_span("ra1.forward", CAT_GPU, 16_667, 2_400);
        file.push_span("frame", CAT_FRAME, 19_067, 1_000);
        let text = file.to_json();
        assert!(text.starts_with("{\"traceEvents\":["), "{text}");
        assert!(text.ends_with("],\"displayTimeUnit\":\"ms\"}"), "{text}");
        // Every span row names its phase, its start, its duration and its
        // category, and the numbers are the ones that were pushed.
        for (name, ts, dur) in [
            ("tick", "0", "16667"),
            ("ra1.forward", "16667", "2400"),
            ("frame", "19067", "1000"),
        ] {
            let row = format!(
                "\"name\":\"{name}\",\"cat\":\"{}\",\"ph\":\"X\",\"pid\":1,\"tid\":1,\"ts\":{ts},\"dur\":{dur}",
                if name == "tick" { CAT_TICK } else if name == "frame" { CAT_FRAME } else { CAT_GPU }
            );
            assert!(text.contains(&row), "missing row {name} in {text}");
        }
        assert_eq!(file.len(), 3);
        assert!(!file.is_empty());
    }

    #[test]
    fn trace_file_counters_plot_a_value() {
        // A counter is the `C` phase: a name, a timestamp and the value in
        // `args`, so a viewer plots it as a series.
        let mut file = TraceFile::new();
        file.push_counter("frame_ms", CAT_COUNTER, 1_000, 16.5)
            .expect("finite value");
        file.push_counter("frame_ms", CAT_COUNTER, 2_000, 17.0)
            .expect("finite value");
        let text = file.to_json();
        assert!(text.contains("\"ph\":\"C\""), "{text}");
        assert!(text.contains("\"args\":{\"frame_ms\":16.5}"), "{text}");
        assert!(text.contains("\"args\":{\"frame_ms\":17}"), "{text}");
    }

    #[test]
    fn trace_file_refuses_a_counter_that_is_not_finite() {
        // A counter with a NaN or infinite value cannot be plotted, so it is
        // refused at the push and never reaches the file.
        let mut file = TraceFile::new();
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let error = file
                .push_counter("frame_ms", CAT_COUNTER, 0, bad)
                .expect_err("a bad counter is refused");
            assert_eq!(
                error,
                TraceFileError::NonFiniteValue {
                    name: "frame_ms".to_owned()
                }
            );
        }
        assert!(file.is_empty());
        assert!(error_text_names_itself());
    }

    /// The error names itself, so a reader knows what went wrong.
    fn error_text_names_itself() -> bool {
        let text = TraceFileError::NonFiniteValue {
            name: "frame_ms".to_owned(),
        }
        .to_string();
        text.contains("frame_ms")
    }

    #[test]
    fn trace_file_escapes_a_name_that_would_break_the_json() {
        // A name with a quote, a backslash or a control character must not
        // break the document: it is escaped, so the file stays one JSON
        // object.
        let mut file = TraceFile::new();
        file.push_span("a\"b\\c\nd", CAT_TICK, 0, 1);
        let text = file.to_json();
        assert!(text.contains("a\\\"b\\\\c\\nd"), "{text}");
        // The escaped name is still inside one string, so the quotes around
        // the name are the only unescaped ones.
        assert_eq!(text.matches("\"name\":\"").count(), 1, "{text}");
    }

    #[test]
    fn trace_file_writes_one_file_and_reads_it_back() {
        // The file lands on disk, and its bytes are the bytes the writer
        // produced.
        let dir = temp_dir("write");
        let path = dir.join("trace.json");
        let mut file = TraceFile::new();
        file.push_span("tick", CAT_TICK, 0, 16_667);
        file.write_to(&path)
            .expect("the folder and the file are written");
        let text = std::fs::read_to_string(&path).expect("the file is readable");
        assert_eq!(text, file.to_json());
    }

    #[test]
    fn trace_file_write_names_the_path_it_failed_on() {
        // A write that cannot happen names the path and the reason instead of
        // panicking.
        let dir = temp_dir("failure");
        std::fs::create_dir_all(&dir).expect("the folder is created");
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, "not a folder").expect("the blocker is written");
        let path = blocker.join("trace.json");
        let error = TraceFile::new()
            .write_to(&path)
            .expect_err("writing under a file fails");
        match error {
            TraceFileError::Write {
                path: named,
                reason,
            } => {
                assert_eq!(named, path);
                assert!(!reason.is_empty(), "{reason}");
                // The reason names the folder that could not be made.
                assert!(reason.contains("blocker"), "{reason}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn trace_file_capture_refuses_a_bad_frame_count() {
        // The frame count is refused before the profiler's switch is touched,
        // so this test never moves the process-wide flag.
        let error = capture_trace(0).expect_err("zero frames is refused");
        assert_eq!(error, TraceFileError::InvalidFrames { frames: 0 });
        let error = capture_trace(TRACE_FRAMES_MAX + 1).expect_err("too many frames is refused");
        assert_eq!(
            error,
            TraceFileError::InvalidFrames {
                frames: TRACE_FRAMES_MAX + 1
            }
        );
    }

    #[test]
    fn trace_file_write_cost_is_named() {
        // The cost of a trace: one write of the whole document, measured on
        // a 600-frame document (the row shape: 12 rows per frame). The number
        // is printed, never asserted, because a timing assert belongs in a
        // release run.
        let dir = temp_dir("cost");
        let path = dir.join("trace.json");
        let mut file = TraceFile::new();
        for frame in 0..600u64 {
            let start = frame * 16_667;
            file.push_span(SPAN_FRAME, CAT_FRAME, start, 16_667);
            for span in profiler::SYSTEM_SPANS {
                file.push_span(span, CAT_TICK, start, 1_000);
            }
            for pass in profiler::GpuPass::ALL {
                file.push_span(pass.name(), CAT_GPU, start, 2_400);
            }
            file.push_counter(COUNTER_FRAME_MS, CAT_COUNTER, start, 16.5)
                .expect("finite value");
        }
        // Warmup, then the measured samples.
        let _ = file.write_to(&path);
        let mut samples = Vec::new();
        for _ in 0..9 {
            let started = Instant::now();
            file.write_to(&path).expect("the file is written");
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        samples.sort_by(f64::total_cmp);
        let bytes = std::fs::metadata(&path).expect("the file is there").len();
        eprintln!(
            "trace file cost: rows={} bytes={} p50={:.3}ms p95={:.3}ms",
            file.len(),
            bytes,
            samples[samples.len() / 2],
            samples[(samples.len() * 95) / 100],
        );
        assert!(bytes > 0, "the file holds bytes");
    }
}
