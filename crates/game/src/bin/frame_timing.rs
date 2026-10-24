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

/// Potato-tier frame budget (owner 2026-10-11: every game runs on a weak GPU
/// and 8 GB): the p99 between-frame interval of a plain run stays under this.
/// One 60 Hz vsync frame is 16.7 ms, so 20 ms leaves the slow tail room for a
/// couple of missed vblanks per hundred frames and fails a run that hitches.
pub(crate) const POTATO_FRAME_BUDGET_MS: f64 = 20.0;

/// Fewest sampled frames a budget verdict needs: a p99 over a handful of
/// frames is the startup, not the run.
pub(crate) const BUDGET_MIN_FRAMES: usize = 100;

/// The `arena frame-budget:` receipt line plus whether it failed.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BudgetVerdict {
    /// The receipt line, printed on every exit run.
    pub(crate) line: String,
    /// True when the p99 interval went over the budget; `--potato-budget`
    /// turns this into a nonzero exit.
    pub(crate) failed: bool,
}

/// Judge a run's sorted between-frame intervals (microseconds) against
/// `budget_ms`. `blocking_probes` marks a `--look-probes` run, whose probe
/// frames block on the GPU by design: it reads `SKIP`, never `FAIL`. Too few
/// frames read `SKIP` too. Pure, so the budget is pinned without a window.
pub(crate) fn frame_budget(
    interval_sorted_us: &[u64],
    budget_ms: f64,
    blocking_probes: bool,
) -> BudgetVerdict {
    let frames = interval_sorted_us.len();
    let p99_us = crate::percentile_us(interval_sorted_us, 99);
    let p99_ms = f64::from(u32::try_from(p99_us).unwrap_or(u32::MAX)) / 1000.0;
    let (verdict, failed) = if blocking_probes {
        ("SKIP(look-probes block by design)", false)
    } else if frames < BUDGET_MIN_FRAMES {
        ("SKIP(too few frames)", false)
    } else if p99_ms > budget_ms {
        ("FAIL", true)
    } else {
        ("PASS", false)
    };
    BudgetVerdict {
        line: format!(
            "arena frame-budget: tier=potato interval_p99={p99_ms:.3}ms budget={budget_ms:.3}ms frames={frames} verdict={verdict}"
        ),
        failed,
    }
}

/// Print the `arena frame-budget:` receipt for a run's (unsorted) intervals.
/// A FAIL verdict with `--potato-budget` on also names itself on stderr and
/// sets `failure` to 1; without the flag the line only prints.
pub(crate) fn report_frame_budget(
    interval_us: &[u64],
    look_probes: bool,
    enforce: bool,
    failure: &mut Option<i32>,
) {
    let mut sorted = interval_us.to_vec();
    sorted.sort_unstable();
    let verdict = frame_budget(&sorted, POTATO_FRAME_BUDGET_MS, look_probes);
    say!("{}", verdict.line);
    if verdict.failed && enforce {
        tell!("arena: --potato-budget: {}", verdict.line);
        *failure = Some(1);
    }
}

/// Whether the window frame about to present runs the blocking look probes:
/// always under `--look-probes`; under an armed capture alone only on the
/// frame the sink saves, so the frames between stay the non-blocking live
/// frame (a probed frame waits on the GPU for its readbacks, and one in four
/// did, 2026-10-24: 156 s for a 25 s run). Pure over its inputs.
pub(crate) fn probes_for_frame(flag: bool, capture_armed: bool, capture_due: bool) -> bool {
    flag || (capture_armed && capture_due)
}

/// The (probe, fog) frame spacing the window sets: the opt-in `--look-probes`
/// spacing under the flag, every frame otherwise (only a capture's saved frame
/// ever probes, and it wants its fog fresh).
pub(crate) fn probe_cadence(flag: bool, probe_every: u32, fog_every: u32) -> (u32, u32) {
    if flag {
        (probe_every, fog_every)
    } else {
        (1, 1)
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    fn run(p99_us: u64) -> Vec<u64> {
        // 98 steady vsync frames, then the 99th-percentile frame and a worse one.
        let mut v = vec![16_600; 98];
        v.push(p99_us);
        v.push(p99_us + 40_000);
        v.sort_unstable();
        v
    }

    #[test]
    fn a_steady_run_passes_the_potato_budget() {
        let verdict = frame_budget(&run(17_300), POTATO_FRAME_BUDGET_MS, false);
        assert!(!verdict.failed, "{}", verdict.line);
        assert!(
            verdict.line.contains("interval_p99=17.300ms"),
            "{}",
            verdict.line
        );
        assert!(verdict.line.ends_with("verdict=PASS"), "{}", verdict.line);
    }

    #[test]
    fn a_hitching_run_fails_the_potato_budget() {
        // The 2026-10-24 capture run: every frame blocked, p99 far over 20 ms.
        let verdict = frame_budget(&run(35_500), POTATO_FRAME_BUDGET_MS, false);
        assert!(verdict.failed, "{}", verdict.line);
        assert!(verdict.line.ends_with("verdict=FAIL"), "{}", verdict.line);
    }

    #[test]
    fn capture_probes_only_the_frames_it_saves() {
        // An armed capture keeps the live frame between its saved frames;
        // only `--look-probes` probes every frame.
        assert!(!probes_for_frame(false, false, false), "plain run is live");
        assert!(!probes_for_frame(false, true, false), "capture idles live");
        assert!(
            probes_for_frame(false, true, true),
            "the saved frame probes"
        );
        assert!(
            probes_for_frame(true, false, false),
            "the flag probes always"
        );
        assert!(
            !probes_for_frame(false, false, true),
            "no capture, no probe"
        );
    }

    #[test]
    fn probe_spacing_follows_the_flag() {
        assert_eq!(probe_cadence(true, 120, 4), (120, 4));
        assert_eq!(probe_cadence(false, 120, 4), (1, 1));
    }

    #[test]
    fn probes_and_short_runs_are_skipped_not_failed() {
        let slow = vec![90_000; 200];
        let probed = frame_budget(&slow, POTATO_FRAME_BUDGET_MS, true);
        assert!(
            !probed.failed && probed.line.contains("SKIP"),
            "{}",
            probed.line
        );
        let short = frame_budget(&[90_000; 10], POTATO_FRAME_BUDGET_MS, false);
        assert!(
            !short.failed && short.line.contains("too few"),
            "{}",
            short.line
        );
    }
}
