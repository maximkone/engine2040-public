//! arena_frame::trace: the per-frame wall split of the arena window, so a
//! perf run can name the section that ate a slow frame. Presentation
//! evidence only: never hashed, never read by the game.
//!
//! Owner: engine-game bin; consumer: `arena.rs` (`present_frame`,
//! `tick_and_present`) plus the perf run that reads the CSV back.
//! Units: microseconds, one row per presented frame.
//! Errors: none; the writer reports a plain IO error to its caller.
use super::super::*;

/// The present path's own split for one presented frame, in microseconds.
/// The four spans come from the render crate: the swapchain acquire, the
/// stage before the command encoder opens, the encode through
/// `queue.submit` and the `queue.present` call.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FrameSplit {
    /// Wall time of the swapchain acquire (`get_current_texture`).
    pub(crate) acquire_us: u64,
    /// Wall time of the stage before the command encoder opens.
    pub(crate) stage_us: u64,
    /// Wall time of the encode through `queue.submit`.
    pub(crate) submit_us: u64,
    /// Wall time of the `queue.present` call.
    pub(crate) present_call_us: u64,
}

impl FrameSplit {
    /// The split of one live surface frame: the acquire the presenter
    /// measured plus the three spans the live wall split carries.
    pub(crate) fn of_live_frame(
        acquire_us: u64,
        live: &engine_render::present::LiveWallSplit,
    ) -> Self {
        Self {
            acquire_us,
            stage_us: live.stage_us,
            submit_us: live.submit_us,
            present_call_us: live.present_call_us,
        }
    }

    /// The span that carries the most of this frame's render time, with
    /// its name: the section that ate the frame.
    pub(crate) fn heaviest(&self) -> (&'static str, u64) {
        let spans = [
            ("acquire", self.acquire_us),
            ("stage", self.stage_us),
            ("submit", self.submit_us),
            ("present_call", self.present_call_us),
        ];
        spans
            .into_iter()
            .max_by_key(|(_, us)| *us)
            .unwrap_or(("acquire", 0))
    }
}

/// One presented frame: where it sat in the run and what it cost.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FrameTrace {
    /// 0-based presented-frame index.
    pub(crate) index: u32,
    /// Microseconds from the run start to this frame's present.
    pub(crate) wall_us: u64,
    /// Whole-frame CPU wall: fixed ticks plus the present.
    pub(crate) work_us: u64,
    /// Wall between this frame's start and the last one's.
    pub(crate) interval_us: u64,
    /// The phase before the present (ticks, input, feel).
    pub(crate) sim_us: u64,
    /// The present wall.
    pub(crate) present_us: u64,
    /// Fighter skin and upload wall.
    pub(crate) fighters_us: u64,
    /// The present path's own split.
    pub(crate) split: FrameSplit,
    /// Fixed ticks done when the frame was presented.
    pub(crate) ticks: u32,
    /// Whether the window held the keyboard focus at this frame.
    pub(crate) focused: bool,
    /// Whether the window was occluded at this frame.
    pub(crate) occluded: bool,
}

/// CSV header of the per-frame trace.
pub(crate) const TRACE_HEADER: &str = "frame,wall_us,work_us,interval_us,sim_us,present_us,\
fighters_us,acquire_us,stage_us,submit_us,present_call_us,ticks,focused,occluded";

/// One CSV row of the per-frame trace.
pub(crate) fn trace_row(trace: &FrameTrace) -> String {
    format!(
        "{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
        trace.index,
        trace.wall_us,
        trace.work_us,
        trace.interval_us,
        trace.sim_us,
        trace.present_us,
        trace.fighters_us,
        trace.split.acquire_us,
        trace.split.stage_us,
        trace.split.submit_us,
        trace.split.present_call_us,
        trace.ticks,
        u8::from(trace.focused),
        u8::from(trace.occluded),
    )
}

/// Write the per-frame trace as CSV (header plus one row per frame).
pub(crate) fn write_trace_csv(path: &std::path::Path, trace: &[FrameTrace]) -> std::io::Result<()> {
    let mut out = String::from(TRACE_HEADER);
    out.push('\n');
    for row in trace {
        out.push_str(&trace_row(row));
        out.push('\n');
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    std::fs::write(path, out)
}

/// A frame is slow when its work wall is over this many times the trace's
/// work p50.
pub(crate) const SLOW_FRAME_FACTOR: u64 = 2;

/// The frames whose work wall is over twice the trace's work p50, slowest
/// first (ties by index).
pub(crate) fn slow_frames(trace: &[FrameTrace]) -> Vec<&FrameTrace> {
    let mut work: Vec<u64> = trace.iter().map(|row| row.work_us).collect();
    work.sort_unstable();
    let p50 = percentile_us(&work, 50);
    let limit = p50.saturating_mul(SLOW_FRAME_FACTOR);
    let mut slow: Vec<&FrameTrace> = trace
        .iter()
        .filter(|row| p50 > 0 && row.work_us > limit)
        .collect();
    slow.sort_by(|a, b| b.work_us.cmp(&a.work_us).then(a.index.cmp(&b.index)));
    slow
}

/// The slow-frame receipt line: how many frames ran over twice the work
/// p50, the first of them and their indices.
pub(crate) fn slow_frames_line(trace: &[FrameTrace]) -> String {
    let slow = slow_frames(trace);
    let first = slow
        .iter()
        .map(|row| row.index)
        .min()
        .map_or_else(|| "-".to_string(), |index| index.to_string());
    let indices = slow
        .iter()
        .map(|row| row.index.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let indices = if indices.is_empty() {
        "-".to_string()
    } else {
        indices
    };
    format!("count={} first={} idx={}", slow.len(), first, indices)
}

/// The receipt line of one slow frame: its cost, its split and the span
/// that carried it.
pub(crate) fn slow_frame_line(row: &FrameTrace) -> String {
    let (heaviest, heaviest_us) = row.split.heaviest();
    format!(
        "idx={} wall_us={} work={}ms present={}ms sim={}ms fighters={}ms \
acquire={}ms stage={}ms submit={}ms present_call={}ms heaviest={heaviest}:{}ms ticks={} \
focused={} occluded={}",
        row.index,
        row.wall_us,
        fmt_us_as_ms(row.work_us),
        fmt_us_as_ms(row.present_us),
        fmt_us_as_ms(row.sim_us),
        fmt_us_as_ms(row.fighters_us),
        fmt_us_as_ms(row.split.acquire_us),
        fmt_us_as_ms(row.split.stage_us),
        fmt_us_as_ms(row.split.submit_us),
        fmt_us_as_ms(row.split.present_call_us),
        fmt_us_as_ms(heaviest_us),
        row.ticks,
        u8::from(row.focused),
        u8::from(row.occluded),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(index: u32, work_us: u64) -> FrameTrace {
        FrameTrace {
            index,
            wall_us: u64::from(index) * 2_000,
            work_us,
            interval_us: 2_000,
            sim_us: 100,
            present_us: work_us.saturating_sub(100),
            fighters_us: 50,
            split: FrameSplit {
                acquire_us: 10,
                stage_us: 20,
                submit_us: 30,
                present_call_us: 40,
            },
            ticks: index,
            focused: true,
            occluded: false,
        }
    }

    #[test]
    fn trace_row_names_every_span() {
        let text = trace_row(&row(7, 1_000));
        assert_eq!(
            text, "7,14000,1000,2000,100,900,50,10,20,30,40,7,1,0",
            "{text}"
        );
    }

    #[test]
    fn trace_header_matches_the_row_width() {
        assert_eq!(
            TRACE_HEADER.split(',').count(),
            trace_row(&row(0, 0)).split(',').count(),
            "the header and the rows must line up"
        );
    }

    #[test]
    fn slow_frames_pick_the_ones_over_twice_the_p50() {
        let mut trace: Vec<FrameTrace> = (0..10).map(|i| row(i, 1_000)).collect();
        trace[4].work_us = 9_000;
        let slow = slow_frames(&trace);
        assert_eq!(slow.len(), 1, "one frame is over twice the p50: {slow:?}");
        assert_eq!(slow[0].index, 4);
        let line = slow_frames_line(&trace);
        assert!(line.starts_with("count=1 first=4 idx=4"), "{line}");
    }

    #[test]
    fn slow_frame_line_names_the_heaviest_span() {
        let mut slow = row(3, 5_000);
        slow.split.acquire_us = 4_000;
        let text = slow_frame_line(&slow);
        assert!(text.contains("idx=3"), "{text}");
        assert!(text.contains("acquire=4.000ms"), "{text}");
        assert!(text.contains("heaviest=acquire:4.000ms"), "{text}");
        assert!(text.contains("ticks=3"), "{text}");
    }

    #[test]
    fn an_empty_trace_names_no_slow_frame() {
        assert_eq!(slow_frames_line(&[]), "count=0 first=- idx=-");
    }
}
