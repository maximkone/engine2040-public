//! EG8 window-read arm: gate caller-owned on/off captures into one row per look
//! pass, in registry order, and publish the dated artifact from those rows.
//!
//! Units: RGBA bytes row-major in, verdicts out. Determinism: same pairs give
//! the same rows and the same report bytes twice; no clock, no float, no IO
//! outside the writer. A look pass with no supplied pair reads PROBE and names
//! source `none` in the table, so a harness stand-in never publishes LIVE here.
//! Caller: [`crate::window::WindowShell::look_read_report`].

#![forbid(unsafe_code)]

use crate::pass_truth::{
    pass_truth_filename, verdict_for_frames, verdict_label, PassVerdict, SuppliedPair, LOOK_PASSES,
    PASS_EPSILON, PASS_LIVE_FLOOR,
};

/// Gate caller-supplied window-read pairs into one verdict per look pass, in
/// registry order. A pass with no supplied pair reads PROBE with zero diff:
/// the artifact never publishes LIVE without a pair. Ragged pairs resolve to
/// PROBE, never panic. Same pairs give the same rows twice; heap only the rows.
/// Caller: [`window_read_report`].
#[must_use]
pub fn gate_window_rows(pairs: &[SuppliedPair<'_>]) -> Vec<PassVerdict> {
    let mut rows = Vec::with_capacity(LOOK_PASSES.len());
    for spec in LOOK_PASSES.iter() {
        rows.push(match pairs.iter().find(|pair| pair.row == spec.row) {
            Some(pair) => {
                verdict_for_frames(spec, pair.on, pair.off, pair.width, pair.height, pair.mask)
            }
            None => PassVerdict {
                row: spec.row,
                pass: spec.pass,
                live: false,
                diff_pixels: 0,
            },
        });
    }
    rows
}

/// Dated `# pass truth <date>` artifact over the caller's window reads: one
/// markdown row per look pass with its verdict, diff, pair source and submit
/// entry, then the gate footer. Source is `window` only where the caller
/// supplied a pair; a pair that names no look pass is counted as stray, never
/// published as a pass. Units: date is a plain tag, never parsed. Determinism:
/// same date plus pairs give the same bytes twice. Caller:
/// [`crate::window::WindowShell::look_read_report`].
#[must_use]
pub fn window_read_report(date: &str, pairs: &[SuppliedPair<'_>]) -> String {
    let rows = gate_window_rows(pairs);
    let live = rows.iter().filter(|row| row.live).count();
    let supplied = |row: &PassVerdict| pairs.iter().any(|pair| pair.row == row.row);
    let stray = pairs
        .iter()
        .filter(|pair| LOOK_PASSES.iter().all(|spec| spec.row != pair.row))
        .count();
    let mut out = format!("# pass truth {date}\n\n| row | verdict | diff | source | pass |\n");
    for row in &rows {
        let source = if supplied(row) { "window" } else { "none" };
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            row.row,
            verdict_label(row.diff_pixels),
            row.diff_pixels,
            source,
            row.pass
        ));
    }
    out.push_str(&format!(
        "\nGate: EG8 window-read arm (crates/render/src/pass_truth/reads.rs). Every look pass with a supplied on/off pair is gated through the masked epsilon diff ({} levels, floor {}); a pass with no pair reads PROBE and names source none, so no harness stand-in publishes LIVE here. Runs x2 give the same rows; {} rows, {} LIVE, {} PROBE, {} stray pairs. Pixels decide; no manifest count.\n",
        PASS_EPSILON,
        PASS_LIVE_FLOOR,
        rows.len(),
        live,
        rows.len().saturating_sub(live),
        stray
    ));
    out
}

/// Write that artifact to `target/reports/pass-truth-<date>.md`, returning the
/// path written. Errors: `std::io::Error` when the file cannot be written.
/// Caller: [`crate::window::WindowShell::write_look_read_report`].
pub fn write_window_read_report(
    date: &str,
    pairs: &[SuppliedPair<'_>],
) -> Result<String, std::io::Error> {
    let report = window_read_report(date, pairs);
    let path = pass_truth_filename(date);
    if let Some(parent) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, report.as_bytes())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OFF: [u8; 8] = [10, 10, 10, 255, 10, 10, 10, 255];

    fn pair<'a>(row: &'a str, on: &'a [u8], off: &'a [u8]) -> SuppliedPair<'a> {
        SuppliedPair {
            row,
            on,
            off,
            width: 2,
            height: 1,
            mask: None,
        }
    }

    #[test]
    fn window_read_arm_publishes_only_supplied_pairs() {
        // Caller: `crate::window::WindowShell::look_read_report`. Behavior: a
        // supplied differing pair reads LIVE with source window, an unpaired
        // look pass reads PROBE with source none, and the artifact writes x2.
        let moved = [10u8, 10, 200, 255, 10, 10, 10, 255];
        let pairs = [pair("graph", &moved, &OFF), pair("sky", &OFF, &OFF)];
        let rows = gate_window_rows(&pairs);
        assert_eq!(rows.len(), LOOK_PASSES.len(), "one row per look pass");
        assert!(rows[0].live, "differing window pair reads LIVE");
        let sky = rows
            .iter()
            .find(|row| row.row == "sky")
            .expect("sky row present");
        assert!(!sky.live, "identical window pair stays PROBE");
        assert!(
            rows.iter().all(|row| row.live == (row.row == "graph")),
            "only the supplied differing pass reads LIVE"
        );
        let report = window_read_report("eg8-window-read-arm-test", &pairs);
        assert!(report.starts_with("# pass truth eg8-window-read-arm-test"));
        assert!(report.contains("| graph | LIVE | 1 | window |"));
        assert!(
            report.contains("| decals | PROBE | 0 | none |"),
            "an unpaired pass names source none"
        );
        let empty: [SuppliedPair<'_>; 0] = [];
        let none = window_read_report("2026-10-17", &empty);
        assert!(
            !none.contains("| LIVE |"),
            "no pair means no LIVE verdict row in the artifact"
        );
        let stray = window_read_report("2026-10-17", &[pair("no-such-pass", &OFF, &OFF)]);
        assert!(
            stray.contains("1 stray pairs"),
            "a pair naming no look pass is counted, never published"
        );
        assert!(
            stray.contains("| graph | PROBE | 0 | none |"),
            "a stray pair never makes a look pass live"
        );
        let path = write_window_read_report("eg8-window-read-arm-test", &pairs)
            .expect("window read report writes");
        assert_eq!(path, pass_truth_filename("eg8-window-read-arm-test"));
        assert_eq!(
            std::fs::read(&path).expect("written report readable"),
            report.as_bytes(),
            "the file holds the report bytes"
        );
        write_window_read_report("eg8-window-read-arm-test", &pairs).expect("rewrites");
        assert_eq!(
            std::fs::read(&path).expect("rewritten report readable"),
            report.as_bytes(),
            "x2 writes are byte-identical"
        );
    }
}
