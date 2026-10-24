//! EG8 live-frame arm: gate the frames the window itself draws.
//!
//! Units: RGBA8 bytes row-major, one `u32` draw id per pixel, pixels for
//! counts. A look pass is LIVE only when its on and off frames differ above
//! `PASS_EPSILON` inside the pixels the pass paints ([`PassRegion`]); the
//! region comes from the frame's own draw-id target, so the sky is exactly the
//! pixels no draw covers and a lit surface is exactly a covered pixel.
//! Determinism: same pixels give the same rows and the same report bytes
//! twice; no clock, no float, no IO outside the writer. A pass with no pair,
//! with ragged pixels, or whose region holds no pixel reads PROBE, never LIVE.
//! Caller: `super::harness` (device leg) renders the pairs this file gates.

#![forbid(unsafe_code)]

use super::{
    frame_diff_count, pass_truth_filename, verdict_for_frames, verdict_label, PassRegion,
    PassTruthError, PassVerdict, LOOK_PASSES, PASS_EPSILON, PASS_LIVE_FLOOR,
};
use std::path::{Path, PathBuf};

/// Draw id of a pixel no draw covers: the id target's clear value, which the
/// sky draw also writes. The presenter's id target clears to this word.
pub const NO_DRAW_ID: u32 = u32::MAX;

/// Gate mask for a region over the frame's draw ids: nonzero entries are
/// ignored by [`super::frame_diff_count`]. `None` counts the whole frame.
/// Pure; one byte per pixel.
#[must_use]
pub fn region_mask(region: PassRegion, ids: &[u32]) -> Option<Vec<u8>> {
    let ignored = |covered_by_draw: bool| match region {
        PassRegion::Frame => false,
        PassRegion::Sky => covered_by_draw,
        PassRegion::Geometry => !covered_by_draw,
    };
    match region {
        PassRegion::Frame => None,
        PassRegion::Sky | PassRegion::Geometry => Some(
            ids.iter()
                .map(|id| u8::from(ignored(*id != NO_DRAW_ID)))
                .collect(),
        ),
    }
}

/// Pixels covered by the listed draws that differ above epsilon between two
/// frames: the region test for a named surface (an armor draw, the sand
/// draw). `ids` are the draw ids of the on frame, one per pixel; a draw id is
/// the draw's index in the submitted list. Ragged inputs fail typed.
/// Caller: the LF2 region proofs beside the gate.
pub fn diff_in_draws(
    on: &[u8],
    off: &[u8],
    ids: &[u32],
    draws: &[u32],
    width: usize,
    height: usize,
) -> Result<usize, PassTruthError> {
    let mask: Vec<u8> = ids.iter().map(|id| u8::from(!draws.contains(id))).collect();
    frame_diff_count(on, off, width, height, Some(&mask), PASS_EPSILON)
}

/// One caller-owned on/off pair of the window's live frame for one pass.
/// `ids` are the draw ids of the on frame, one per pixel.
#[derive(Clone, Copy, Debug)]
pub struct LivePair<'a> {
    /// Board row the pair proves; unknown rows resolve to PROBE.
    pub row: &'a str,
    /// Frame with every look pass on, RGBA row-major.
    pub on: &'a [u8],
    /// Frame with this pass off, RGBA row-major.
    pub off: &'a [u8],
    /// Draw id per pixel of the on frame.
    pub ids: &'a [u32],
    /// Frame width in pixels.
    pub width: usize,
    /// Frame height in pixels.
    pub height: usize,
}

/// One gated pass: the verdict, the region it was judged on, how many pixels
/// that region holds, and whether any pair existed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LiveRow {
    /// Verdict from the masked diff only.
    pub verdict: PassVerdict,
    /// The pixels the pass paints.
    pub region: PassRegion,
    /// Pixels inside the region (the denominator of the diff).
    pub counted_px: usize,
    /// True when a pair was supplied for this pass.
    pub paired: bool,
}

/// Pixels a region keeps: the mask's zero entries, or the whole frame.
fn counted_pixels(mask: Option<&[u8]>, pixels: usize) -> usize {
    match mask {
        None => pixels,
        Some(mask) if mask.len() == pixels => mask.iter().filter(|m| **m == 0).count(),
        Some(_) => 0,
    }
}

/// Gate one pair through the region-masked epsilon diff. Unknown rows and
/// ragged inputs resolve to PROBE with zero diff, never panic; an empty
/// region (no pixel to judge) is PROBE whatever the bytes say.
#[must_use]
pub fn gate_live_pair(pair: &LivePair<'_>) -> LiveRow {
    let Some(spec) = LOOK_PASSES.iter().find(|spec| spec.row == pair.row) else {
        return LiveRow {
            verdict: PassVerdict {
                row: "unknown",
                pass: "none",
                live: false,
                diff_pixels: 0,
            },
            region: PassRegion::Frame,
            counted_px: 0,
            paired: true,
        };
    };
    let pixels = pair.width.saturating_mul(pair.height);
    let mask = region_mask(spec.region, pair.ids);
    let counted_px = counted_pixels(mask.as_deref(), pixels);
    let mut verdict = verdict_for_frames(
        spec,
        pair.on,
        pair.off,
        pair.width,
        pair.height,
        mask.as_deref(),
    );
    if counted_px == 0 {
        verdict.live = false;
        verdict.diff_pixels = 0;
    }
    LiveRow {
        verdict,
        region: spec.region,
        counted_px,
        paired: true,
    }
}

/// Gate every look pass in registry order. A pass with no supplied pair reads
/// PROBE with `paired` false: the report never publishes LIVE without pixels.
#[must_use]
pub fn gate_live_rows(pairs: &[LivePair<'_>]) -> Vec<LiveRow> {
    LOOK_PASSES
        .iter()
        .map(
            |spec| match pairs.iter().find(|pair| pair.row == spec.row) {
                Some(pair) => gate_live_pair(pair),
                None => LiveRow {
                    verdict: PassVerdict {
                        row: spec.row,
                        pass: spec.pass,
                        live: false,
                        diff_pixels: 0,
                    },
                    region: spec.region,
                    counted_px: 0,
                    paired: false,
                },
            },
        )
        .collect()
}

/// Dated `# pass truth <date>` report over gated live-frame rows: one table
/// row per look pass, then the gate footer. `noise_px` is the all-on frame
/// diffed against itself (a repeatable frame reads 0, and the harness refuses
/// to gate a frame that does not). Same inputs give the same bytes twice.
#[must_use]
pub fn live_gate_report(
    date: &str,
    scene: &str,
    size: (usize, usize),
    noise_px: usize,
    rows: &[LiveRow],
) -> String {
    let live = rows.iter().filter(|row| row.verdict.live).count();
    let mut out = format!(
        "# pass truth {date}\n\nscene: {scene}, {}x{} px, the window's live frame path\nnoise: {noise_px} px (all-on frame diffed against itself)\n\n| row | verdict | diff | source | region | counted | pass |\n",
        size.0, size.1
    );
    for row in rows {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            row.verdict.row,
            verdict_label(row.verdict.diff_pixels),
            row.verdict.diff_pixels,
            if row.paired { "live" } else { "none" },
            row.region.name(),
            row.counted_px,
            row.verdict.pass,
        ));
    }
    out.push_str(&format!(
        "\nGate: EG8 live-frame arm (crates/render/src/pass_truth/live_gate.rs). Each look pass is switched off in the frame the window draws and diffed against the all-on frame; a pass is LIVE only when pixels inside its own region (frame, sky or geometry, cut from the frame's draw-id target) differ above epsilon {} with floor {}. A pass with no switch in the live frame renders two equal frames and reads PROBE. {} rows, {} LIVE, {} PROBE. Pixels decide; no manifest count.\n",
        PASS_EPSILON,
        PASS_LIVE_FLOOR,
        rows.len(),
        live,
        rows.len().saturating_sub(live),
    ));
    out
}

/// Write the report to `<root>/target/reports/pass-truth-<date>.md` and return
/// the path. Errors: `std::io::Error` when the file cannot be written.
pub fn write_live_gate_report(
    root: &Path,
    date: &str,
    report: &str,
) -> Result<PathBuf, std::io::Error> {
    let path = root.join(pass_truth_filename(date));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, report.as_bytes())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: usize = 4;
    const H: usize = 2;

    /// Ids: left half covered by draw 7, right half sky.
    fn ids() -> Vec<u32> {
        (0..W * H)
            .map(|i| if i % W < 2 { 7 } else { NO_DRAW_ID })
            .collect()
    }

    fn flat(value: u8) -> Vec<u8> {
        [value, value, value, 255].repeat(W * H)
    }

    /// `frame` with pixel `index` raised by `delta` on the red channel.
    fn bumped(frame: &[u8], index: usize, delta: u8) -> Vec<u8> {
        let mut out = frame.to_vec();
        out[index * 4] = out[index * 4].saturating_add(delta);
        out
    }

    fn pair<'a>(row: &'a str, on: &'a [u8], off: &'a [u8], ids: &'a [u32]) -> LivePair<'a> {
        LivePair {
            row,
            on,
            off,
            ids,
            width: W,
            height: H,
        }
    }

    #[test]
    fn region_masks_cut_sky_and_geometry_from_the_draw_ids() {
        let ids = ids();
        assert_eq!(region_mask(PassRegion::Frame, &ids), None);
        let sky = region_mask(PassRegion::Sky, &ids).expect("sky has a mask");
        let geometry = region_mask(PassRegion::Geometry, &ids).expect("geometry has a mask");
        for (i, id) in ids.iter().enumerate() {
            let covered = *id != NO_DRAW_ID;
            assert_eq!(sky[i] == 0, !covered, "sky counts only uncovered pixels");
            assert_eq!(geometry[i] == 0, covered, "geometry counts only covered");
        }
        assert_eq!(sky.iter().filter(|m| **m == 0).count(), 4);
        assert_eq!(geometry.iter().filter(|m| **m == 0).count(), 4);
    }

    #[test]
    fn a_pass_reads_live_only_from_its_own_region() {
        let ids = ids();
        let off = flat(10);
        // Pixel 0 is covered by a draw, pixel 3 is sky.
        let geometry_change = bumped(&off, 0, 120);
        let sky_change = bumped(&off, 3, 120);
        let sky_row = gate_live_pair(&pair("sky", &geometry_change, &off, &ids));
        assert!(!sky_row.verdict.live, "a surface change is not the sky");
        assert_eq!(sky_row.verdict.diff_pixels, 0);
        let sky_row = gate_live_pair(&pair("sky", &sky_change, &off, &ids));
        assert!(sky_row.verdict.live, "a sky pixel change is the sky");
        assert_eq!(sky_row.verdict.diff_pixels, 1);
        assert_eq!(sky_row.counted_px, 4);
        let ibl_row = gate_live_pair(&pair("ibl", &sky_change, &off, &ids));
        assert!(!ibl_row.verdict.live, "a sky change is not lighting");
        let ibl_row = gate_live_pair(&pair("ibl", &geometry_change, &off, &ids));
        assert!(ibl_row.verdict.live, "a surface change is lighting");
        let forward = gate_live_pair(&pair("graph", &sky_change, &off, &ids));
        assert!(forward.verdict.live, "forward owns the whole frame");
        assert_eq!(forward.counted_px, W * H);
    }

    #[test]
    fn dust_under_epsilon_and_equal_frames_stay_probe() {
        let ids = ids();
        let off = flat(10);
        let dust = bumped(&off, 0, PASS_EPSILON);
        let row = gate_live_pair(&pair("graph", &dust, &off, &ids));
        assert!(!row.verdict.live, "a diff at epsilon is not above it");
        assert_eq!(row.verdict.diff_pixels, 0);
        let above = bumped(&off, 0, PASS_EPSILON + 1);
        let row = gate_live_pair(&pair("graph", &above, &off, &ids));
        assert!(row.verdict.live, "one level over epsilon counts");
        let row = gate_live_pair(&pair("graph", &off, &off, &ids));
        assert!(!row.verdict.live, "equal frames never read LIVE");
    }

    #[test]
    fn empty_region_ragged_input_and_unknown_rows_stay_probe() {
        let off = flat(10);
        let on = bumped(&off, 0, 120);
        // Every pixel covered: the sky region holds nothing to judge.
        let all_covered = vec![7u32; W * H];
        let row = gate_live_pair(&pair("sky", &on, &off, &all_covered));
        assert!(!row.verdict.live);
        assert_eq!(row.counted_px, 0, "no sky pixel in this frame");
        // Ids that do not cover the frame make the mask ragged.
        let short_ids = vec![7u32; 3];
        let row = gate_live_pair(&pair("ibl", &on, &off, &short_ids));
        assert!(!row.verdict.live, "a ragged mask never reads LIVE");
        assert_eq!(row.counted_px, 0);
        let ids = ids();
        let row = gate_live_pair(&pair("no-such-pass", &on, &off, &ids));
        assert!(!row.verdict.live);
        assert_eq!(row.verdict.row, "unknown");
        let row = gate_live_pair(&LivePair {
            on: &on[..7],
            ..pair("graph", &on, &off, &ids)
        });
        assert!(!row.verdict.live, "a ragged frame never reads LIVE");
    }

    #[test]
    fn rows_run_in_registry_order_and_unpaired_passes_read_probe_none() {
        let ids = ids();
        let off = flat(10);
        let on = bumped(&off, 0, 120);
        let pairs = [pair("ibl", &on, &off, &ids), pair("graph", &on, &off, &ids)];
        let rows = gate_live_rows(&pairs);
        assert_eq!(rows.len(), LOOK_PASSES.len());
        for (row, spec) in rows.iter().zip(LOOK_PASSES.iter()) {
            assert_eq!(row.verdict.row, spec.row, "registry order");
            assert_eq!(row.region, spec.region);
            let supplied = spec.row == "ibl" || spec.row == "graph";
            assert_eq!(row.paired, supplied);
            assert_eq!(row.verdict.live, supplied, "only paired differing passes");
        }
        assert_eq!(gate_live_rows(&pairs), rows, "the gate repeats");
    }

    #[test]
    fn report_lists_every_pass_with_its_region_and_repeats() {
        let ids = ids();
        let off = flat(10);
        let on = bumped(&off, 0, 120);
        let rows = gate_live_rows(&[pair("graph", &on, &off, &ids)]);
        let first = live_gate_report("2026-10-17", "unit", (W, H), 0, &rows);
        let second = live_gate_report("2026-10-17", "unit", (W, H), 0, &rows);
        assert_eq!(first.as_bytes(), second.as_bytes(), "same rows, same bytes");
        assert!(first.starts_with("# pass truth 2026-10-17\n\nscene: unit, 4x2 px"));
        for spec in LOOK_PASSES.iter() {
            let needle = format!(
                "
| {} |",
                spec.row
            );
            assert_eq!(
                first.matches(&needle).count(),
                1,
                "{} listed once",
                spec.row
            );
        }
        assert!(first.contains("| graph | LIVE | 1 | live | frame | 8 | submit_gpu |"));
        assert!(first.contains("| sky | PROBE | 0 | none | sky | 0 | submit_gpu_sky |"));
        assert!(first.contains("10 rows, 1 LIVE, 9 PROBE"));
        assert!(first.contains("noise: 0 px"));
    }

    #[test]
    fn report_writes_under_the_root_and_repeats() {
        let ids = ids();
        let off = flat(10);
        let rows = gate_live_rows(&[pair("graph", &off, &off, &ids)]);
        let report = live_gate_report("tmp-live-gate-writer", "unit", (W, H), 0, &rows);
        let root = std::env::temp_dir().join("engine2040-live-gate-test");
        let path =
            write_live_gate_report(&root, "tmp-live-gate-writer", &report).expect("report writes");
        assert!(path.ends_with("target/reports/pass-truth-tmp-live-gate-writer.md"));
        assert_eq!(
            std::fs::read(&path).expect("report reads back"),
            report.as_bytes()
        );
        let again = write_live_gate_report(&root, "tmp-live-gate-writer", &report)
            .expect("report rewrites");
        assert_eq!(again, path);
        assert_eq!(
            std::fs::read(&again).expect("report rereads"),
            report.as_bytes(),
            "x2 writes are byte-identical"
        );
    }
}
