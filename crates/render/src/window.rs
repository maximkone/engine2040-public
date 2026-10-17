//! window: std-only WindowShell. Frame count + fixed-dt accumulator only; window events never feed state_hash.

use super::NullRenderer;
use crate::pass_truth::{PassVerdict, SuppliedPair};

/// Fixed simulation dt: 60Hz. Accumulator clamps spiral-of-death at 5 steps.
/// Single owner is `engine_core::FIXED_DT`; this re-export keeps the render path.
pub use engine_core::FIXED_DT;
pub const MAX_STEPS_PER_PUMP: u32 = 5;

/// Window surface state. Informational only — never hashed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceState {
    Headless,
    Open,
    Closed,
}

pub struct WindowShell {
    renderer: NullRenderer,
    surface: SurfaceState,
    width: u32,
    height: u32,
    acc: f32,
    ticks: u64,
}

impl Default for WindowShell {
    fn default() -> Self {
        Self::new()
    }
}

impl WindowShell {
    pub fn new() -> Self {
        Self {
            renderer: NullRenderer::new(),
            surface: SurfaceState::Headless,
            width: 1280,
            height: 720,
            acc: 0.0,
            ticks: 0,
        }
    }

    /// OS event: open window. Does not touch sim.
    pub fn open(&mut self, width: u32, height: u32) {
        self.surface = SurfaceState::Open;
        self.width = width;
        self.height = height;
    }

    /// OS event: close window. Does not touch sim.
    pub fn close(&mut self) {
        self.surface = SurfaceState::Closed;
    }

    /// OS event: resize surface. Drops/recreates surface only. Does not touch sim.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.width = width;
        self.height = height;
    }

    /// Pump real elapsed seconds through a fixed-dt accumulator.
    /// Calls `step_tick(tick)` once per fixed step, then `begin_frame` once per call.
    /// Returns number of fixed steps taken (0..=MAX_STEPS_PER_PUMP).
    pub fn pump(&mut self, elapsed_s: f32, tick: u64, mut step_tick: impl FnMut(u64)) -> u32 {
        let clamped = elapsed_s.min(FIXED_DT * MAX_STEPS_PER_PUMP as f32);
        self.acc += clamped;
        let mut steps = 0;
        while self.acc >= FIXED_DT && steps < MAX_STEPS_PER_PUMP {
            self.acc -= FIXED_DT;
            step_tick(tick + steps as u64);
            steps += 1;
            self.ticks += 1;
        }
        self.renderer.begin_frame(tick);
        steps
    }

    pub fn surface(&self) -> SurfaceState {
        self.surface
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    pub fn draw_count(&self) -> u64 {
        self.renderer.draw_count()
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    /// EG8: gate one caller-owned on/off window frame pair through the masked
    /// epsilon diff. Units: RGBA bytes row-major, pixels for frames. The flag
    /// comes from the diff only, never the row name; unknown rows and ragged
    /// or empty inputs resolve to PROBE with zero diff, never panic.
    /// Determinism: same bytes give the same verdict twice; no clock, no
    /// float, no IO. Budget: cold-path gate only, never per tick;
    /// caller-owned slices in, verdict out. Consumer: the window host hands
    /// each captured on/off pair here once per frame; the EG8 report gates
    /// one pair per look pass.
    pub fn look_pass_live(
        &self,
        row: &str,
        on: &[u8],
        off: &[u8],
        width: usize,
        height: usize,
        mask: Option<&[u8]>,
    ) -> PassVerdict {
        let Some(spec) = crate::pass_truth::LOOK_PASSES
            .iter()
            .find(|candidate| candidate.row == row)
        else {
            return PassVerdict {
                row: "unknown",
                pass: "none",
                live: false,
                diff_pixels: 0,
            };
        };
        crate::pass_truth::verdict_for_frames(spec, on, off, width, height, mask)
    }

    /// EG8: batch-gate caller-owned window on/off pairs for the look passes
    /// through the masked epsilon diff. Units: RGBA bytes row-major in, LIVE
    /// or PROBE verdicts out. The flag comes from the diff only, never the
    /// row name; unknown rows and ragged inputs resolve to PROBE with zero
    /// diff, never panic. Determinism: same pairs give the same table twice;
    /// no clock, no float, no IO. Budget: cold-path gate only, never per
    /// tick; caller-owned slices in, heap only the verdicts. Consumer: the
    /// window host hands one pair per look pass here; the EG8 report gates
    /// the same pairs. Production wire for `gate_supplied_table`.
    pub fn look_supplied_table(&self, pairs: &[SuppliedPair<'_>]) -> Vec<PassVerdict> {
        crate::pass_truth::gate_supplied_table(pairs)
    }

    /// EG8: dated LIVE/PROBE report over one harness run. Units: date is a
    /// plain tag, never parsed. Determinism: same date gives the same bytes
    /// twice; no clock, no float, no IO. Budget: cold-path report only;
    /// heap only the nine verdicts plus the string. Consumer: the EG8
    /// `target/reports/pass-truth-<date>.md` artifact.
    pub fn look_dated_report(&self, date: &str) -> String {
        crate::pass_truth::gate_dated_report(date)
    }

    /// EG8: dated artifact file for one harness run. Units: date is a plain
    /// tag, never parsed. Determinism: same date gives the same bytes twice;
    /// no clock, no float, no IO. Budget: cold-path report only; heap only
    /// the path string. Consumer: the EG8 report writer, which pairs this
    /// path with [`WindowShell::look_dated_report`]. Production wire for
    /// `crate::pass_truth::pass_truth_filename`.
    pub fn look_report_path(&self, date: &str) -> String {
        crate::pass_truth::pass_truth_filename(date)
    }

    /// EG8: write the dated LIVE/PROBE artifact through the gate writer.
    /// Units: date is a plain tag, never parsed; returns the file path written.
    /// Determinism: same date gives the same bytes twice; no clock, no float.
    /// Errors: `std::io::Error` when the report file cannot be written.
    /// Budget: cold-path report only; heap only the report plus the path.
    /// Consumer: the EG8 `target/reports/pass-truth-<date>.md` artifact.
    pub fn write_look_report(&self, date: &str) -> Result<String, std::io::Error> {
        crate::pass_truth::write_gate_report(date)
    }

    /// EG8: dated LIVE/PROBE artifact over the caller's real window reads,
    /// each row naming where its frames came from. Units: date is a plain tag,
    /// never parsed. Determinism: same date plus pairs give the same bytes
    /// twice; no clock, no float, no IO. Consumer: the EG8
    /// `target/reports/pass-truth-<date>.md` artifact. Caller wire for
    /// `crate::pass_truth::reads::window_read_report`.
    pub fn look_read_report(&self, date: &str, pairs: &[SuppliedPair<'_>]) -> String {
        crate::pass_truth::reads::window_read_report(date, pairs)
    }

    /// EG8: write that artifact to its dated path, returning the path written.
    /// Errors: `std::io::Error` when the report file cannot be written. Caller
    /// wire for `crate::pass_truth::reads::write_window_read_report`.
    pub fn write_look_read_report(
        &self,
        date: &str,
        pairs: &[SuppliedPair<'_>],
    ) -> Result<String, std::io::Error> {
        crate::pass_truth::reads::write_window_read_report(date, pairs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_close_resize_never_blocks_frames() {
        let mut w = WindowShell::new();
        w.open(1920, 1080);
        assert_eq!(w.surface(), SurfaceState::Open);
        let n = w.pump(1.0 / 60.0, 0, |_| {});
        assert_eq!(n, 1);
        w.resize(800, 600);
        assert_eq!(w.size(), (800, 600));
        let n2 = w.pump(1.0 / 60.0, 1, |_| {});
        assert_eq!(n2, 1);
        w.close();
        assert_eq!(w.surface(), SurfaceState::Closed);
        let n3 = w.pump(1.0 / 60.0, 2, |_| {});
        assert_eq!(n3, 1);
        assert_eq!(w.draw_count(), 3);
    }

    #[test]
    fn accumulator_clamps_spiral() {
        let mut w = WindowShell::new();
        // 10s elapsed clamps to 5 fixed steps, not 600.
        let n = w.pump(10.0, 0, |_| {});
        assert_eq!(n, MAX_STEPS_PER_PUMP);
        assert_eq!(w.ticks(), MAX_STEPS_PER_PUMP as u64);
    }

    #[test]
    fn sub_frame_elapsed_accumulates() {
        let mut w = WindowShell::new();
        let n1 = w.pump(1.0 / 120.0, 0, |_| {});
        assert_eq!(n1, 0);
        let n2 = w.pump(1.0 / 120.0, 0, |_| {});
        assert_eq!(n2, 1);
    }

    #[test]
    fn look_pass_live_gates_window_frames() {
        let shell = WindowShell::new();
        let off = [10u8, 10, 10, 255, 10, 10, 10, 255];
        let mut moved = off;
        moved[2] = 200;
        let live = shell.look_pass_live("graph", &moved, &off, 2, 1, None);
        assert!(live.live, "differing window frames read LIVE");
        assert!(live.diff_pixels > 0);
        assert_eq!(live.row, "graph");
        let probe = shell.look_pass_live("graph", &off, &off, 2, 1, None);
        assert!(!probe.live, "identical window frames read PROBE");
        assert_eq!(probe.diff_pixels, 0);
    }

    #[test]
    fn look_pass_live_masked_and_dust_stay_probe() {
        let shell = WindowShell::new();
        let off = [10u8, 10, 10, 255, 10, 10, 10, 255];
        let moved = [10u8, 10, 200, 255, 10, 10, 10, 255];
        let masked = shell.look_pass_live("graph", &moved, &off, 2, 1, Some(&[1, 0]));
        assert!(!masked.live, "masked-only move stays PROBE");
        assert_eq!(masked.diff_pixels, 0);
        let dusty = [14u8, 10, 10, 255, 10, 10, 10, 255];
        let dust = shell.look_pass_live("graph", &dusty, &off, 2, 1, None);
        assert!(!dust.live, "dust under epsilon stays PROBE");
        assert_eq!(dust.diff_pixels, 0);
    }

    #[test]
    fn look_pass_live_unknown_and_ragged_stay_probe() {
        let shell = WindowShell::new();
        let off = [10u8, 10, 10, 255, 10, 10, 10, 255];
        let mut moved = off;
        moved[2] = 200;
        let ghost = shell.look_pass_live("no-such-pass", &moved, &off, 2, 1, None);
        assert!(!ghost.live, "unknown rows never read live");
        assert_eq!(ghost.diff_pixels, 0);
        let ragged = shell.look_pass_live("graph", &moved, &off[..7], 2, 1, None);
        assert!(!ragged.live, "ragged window reads resolve PROBE");
        assert_eq!(ragged.diff_pixels, 0);
        let empty: [u8; 0] = [];
        let empty_verdict = shell.look_pass_live("graph", &empty, &empty, 0, 0, None);
        assert!(!empty_verdict.live);
        assert_eq!(empty_verdict.diff_pixels, 0);
    }

    #[test]
    fn look_dated_report_repeats_with_gate_footer() {
        let shell = WindowShell::new();
        let first = shell.look_dated_report("2026-10-15");
        let second = shell.look_dated_report("2026-10-15");
        assert_eq!(first.as_bytes(), second.as_bytes());
        assert!(
            first.starts_with("# pass truth 2026-10-15"),
            "window report carries the date"
        );
        assert!(first.contains("Gate: EG8"), "report names its gate");
        assert!(first.contains("| graph | LIVE |"));
        assert!(first.contains("| sky | PROBE | 0 |"));
        assert!(first.contains("| decals | PROBE | 0 |"));
    }

    #[test]
    fn look_report_path_matches_gate_file() {
        let shell = WindowShell::new();
        let first = shell.look_report_path("2026-10-16");
        assert_eq!(
            first,
            crate::pass_truth::pass_truth_filename("2026-10-16"),
            "window path uses the gate file"
        );
        assert_eq!(
            first.as_bytes(),
            shell.look_report_path("2026-10-16").as_bytes(),
            "same date gives same path twice"
        );
        assert_ne!(
            first,
            shell.look_report_path("2026-10-15"),
            "different dates name different files"
        );
    }

    #[test]
    fn look_supplied_table_batches_window_frames() {
        let shell = WindowShell::new();
        let off = [10u8, 10, 10, 255, 10, 10, 10, 255];
        let mut moved = off;
        moved[2] = 200;
        let pairs = [
            SuppliedPair {
                row: "graph",
                on: &moved,
                off: &off,
                width: 2,
                height: 1,
                mask: None,
            },
            SuppliedPair {
                row: "sky",
                on: &off,
                off: &off,
                width: 2,
                height: 1,
                mask: None,
            },
        ];
        let table = shell.look_supplied_table(&pairs);
        assert_eq!(table.len(), 2);
        assert!(table[0].live, "differing batch pair reads LIVE");
        assert!(!table[1].live, "identical batch pair reads PROBE");
        assert_eq!(table, shell.look_supplied_table(&pairs), "batch repeats");
    }

    #[test]
    fn look_supplied_table_unknown_and_ragged_stay_probe() {
        let shell = WindowShell::new();
        let off = [10u8, 10, 10, 255, 10, 10, 10, 255];
        let mut moved = off;
        moved[2] = 200;
        let ghost = SuppliedPair {
            row: "no-such-pass",
            on: &moved,
            off: &off,
            width: 2,
            height: 1,
            mask: None,
        };
        let ragged = SuppliedPair {
            row: "graph",
            on: &moved,
            off: &off[..7],
            width: 2,
            height: 1,
            mask: None,
        };
        let table = shell.look_supplied_table(&[ghost, ragged]);
        assert_eq!(table.len(), 2);
        assert!(!table[0].live, "unknown batch row stays PROBE");
        assert!(!table[1].live, "ragged batch pair stays PROBE");
    }

    #[test]
    fn look_read_report_arm_writes_dated_file() {
        let shell = WindowShell::new();
        let off = [10u8, 10, 10, 255, 10, 10, 10, 255];
        let mut moved = off;
        moved[2] = 200;
        let pairs = [SuppliedPair {
            row: "graph",
            on: &moved,
            off: &off,
            width: 2,
            height: 1,
            mask: None,
        }];
        let wrote = shell
            .write_look_read_report("eg8-window-read-arm", &pairs)
            .expect("window read report writes");
        assert_eq!(wrote, shell.look_report_path("eg8-window-read-arm"));
        assert_eq!(
            std::fs::read(wrote).expect("written report readable"),
            shell
                .look_read_report("eg8-window-read-arm", &pairs)
                .as_bytes(),
            "the window wire writes the window-read report"
        );
        assert!(
            shell
                .look_read_report("eg8-window-read-arm", &pairs)
                .contains("| graph | LIVE | 1 | window |"),
            "the wired report keeps the window-read row"
        );
    }
}
