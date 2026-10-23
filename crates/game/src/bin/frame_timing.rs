//! Frame timing for the arena bin: the exit deadline, the timed-smoke CSV and
//! the presentation pacer. Split out of the 5k-line bin root; same behavior,
//! same callers.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use engine_game::windowed::{
    check_exit_deadline_with_limit, frame_times_csv_row, WinError, FRAME_TIMES_CSV_HEADER,
    WIN_FIXED_DT_S,
};

use crate::arena_frame;

/// Exit-path deadline check with the `Timeout` receipt: the shared limit
/// check reports a stall as `Render`, but the `--exit` watchdog names a
/// `Timeout` carrying elapsed plus limit, so the binary refiles that message
/// under `Timeout` (every other error passes through untouched). Pure over
/// the same inputs, so the forced-stall drill pins it without waiting.
pub(crate) fn check_exit_wall_deadline(elapsed_ms: f64, limit_ms: u32) -> Result<(), WinError> {
    check_exit_deadline_with_limit(elapsed_ms, limit_ms).map_err(|err| match err {
        WinError::Render(msg) => WinError::Timeout(msg),
        other => other,
    })
}

/// Write the timed-smoke frame-times CSV: header plus one row per presented
/// frame, so the proof perf mode reads a real run. Wall time rides the
/// between-frame interval with the frame-body cost as the input-to-present
/// leg; missing samples fall back to the run average, so a short smoke still
/// files one row per frame. Creates parent dirs. Presentation only, never
/// hashed; IO failures fail loudly.
pub(crate) fn write_frame_times_csv(
    path: &Path,
    frames: u32,
    work_us: &[u64],
    interval_us: &[u64],
    elapsed_ms: f64,
) -> Result<(), WinError> {
    if !elapsed_ms.is_finite() || elapsed_ms < 0.0 {
        return Err(WinError::BadArgs(format!(
            "frame-times elapsed_ms {elapsed_ms} is not finite nonnegative"
        )));
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|err| {
                WinError::Render(format!("frame-times dir {}: {err}", parent.display()))
            })?;
        }
    }
    let avg_ms = elapsed_ms / f64::from(frames.max(1));
    let us_to_ms = |us: u64| {
        let whole = u32::try_from(us / 1000).unwrap_or(u32::MAX);
        let frac = u32::try_from(us % 1000).unwrap_or(999);
        f64::from(whole) + f64::from(frac) / 1000.0
    };
    let mut text = String::from(FRAME_TIMES_CSV_HEADER);
    text.push('\n');
    for index in 0..frames {
        let at = usize::try_from(index).unwrap_or(usize::MAX);
        let wall_ms = interval_us.get(at).map_or(avg_ms, |us| us_to_ms(*us));
        let present_ms = work_us.get(at).map_or(wall_ms, |us| us_to_ms(*us));
        let row = frame_times_csv_row(index, wall_ms, present_ms)?;
        text.push_str(&row);
        text.push('\n');
    }
    std::fs::write(path, text)
        .map_err(|err| WinError::Render(format!("frame-times {}: {err}", path.display())))?;
    Ok(())
}

/// Whole microseconds of a wall duration, saturating (diagnostics only).
pub(crate) fn duration_us(elapsed: Duration) -> u64 {
    u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX)
}

/// Where the per-frame present-path trace is written, from
/// `E2040_FRAME_TRACE` (a path). `None` when the variable is unset or
/// empty, so the default run pays nothing and writes nothing.
pub(crate) fn frame_trace_path() -> Option<PathBuf> {
    let raw = std::env::var("E2040_FRAME_TRACE").ok()?;
    let path = raw.trim();
    if path.is_empty() {
        return None;
    }
    Some(PathBuf::from(path))
}

/// The presentation pacer: one frame every `--fixed-dt <ms>` when the flag
/// names a period, else one frame per fixed tick (the game's own rate).
/// The window parks between slots instead of free-running: a free-running
/// window outruns the display, and the compositor answers with ~80 ms
/// swapchain acquire stalls (the 2026-10-20 perf lane).
pub(crate) fn frame_pacer(period_ms: Option<u32>) -> arena_frame::FixedDtPacer {
    let period = match period_ms {
        Some(ms) => Duration::from_millis(u64::from(ms)),
        None => Duration::from_secs_f64(WIN_FIXED_DT_S),
    };
    arena_frame::FixedDtPacer::new(period, Instant::now())
}
