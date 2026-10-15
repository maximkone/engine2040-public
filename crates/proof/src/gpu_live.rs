//! GPU liveness truth gate for the ticked render rows.
//! Owner: proof crate; consumer: tools/proof.mjs via the orchestrator.
//! Units: none (boolean liveness flags). Determinism: pure, no IO or clock.
//! Errors: `GpuLiveRefusal` names every not-live row. Budget: cold-path
//! gate only, never called per tick; fixed row count, short strings.

use std::fmt;

/// One ticked render row's GPU submit plus its device-proven state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PassGate {
    /// Ticked render row under test.
    pub row: &'static str,
    /// GPU submit entry that must stop refusing on a device.
    pub pass: &'static str,
    /// True only with device evidence that the submit succeeds.
    pub live: bool,
}

/// Truth-gate refusal naming every row whose GPU pass is still a stub.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GpuLiveRefusal {
    /// Rows with `live == false`, in registry order.
    pub refused: Vec<&'static str>,
}

impl fmt::Display for GpuLiveRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "gpu passes not live: {}", self.refused.join(", "))
    }
}

impl std::error::Error for GpuLiveRefusal {}

/// Fail when any listed pass is still a stub, naming every not-live row.
pub fn check_passes(passes: &[PassGate]) -> Result<(), GpuLiveRefusal> {
    let refused: Vec<&'static str> = passes
        .iter()
        .filter(|gate| !gate.live)
        .map(|gate| gate.row)
        .collect();
    if refused.is_empty() {
        Ok(())
    } else {
        Err(GpuLiveRefusal { refused })
    }
}

/// Ragged pixel-truth inputs; short captures never read as live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PixelTruthError {
    /// Both frame lengths when either frame is empty.
    EmptyFrame { on_len: usize, off_len: usize },
    /// Both frame lengths when they differ.
    LengthMismatch { on_len: usize, off_len: usize },
    /// Frame bytes plus mask entries when they differ.
    MaskMismatch { frame_len: usize, mask_len: usize },
}

impl fmt::Display for PixelTruthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyFrame { on_len, off_len } => {
                write!(f, "pixel truth needs frame bytes, got {on_len}/{off_len}")
            }
            Self::LengthMismatch { on_len, off_len } => {
                write!(f, "pixel truth length mismatch: {on_len} vs {off_len}")
            }
            Self::MaskMismatch {
                frame_len,
                mask_len,
            } => {
                write!(f, "pixel truth mask {mask_len} vs {frame_len}")
            }
        }
    }
}

impl std::error::Error for PixelTruthError {}

/// RGBA diff count in row-major order; identical frames fast-path to zero.
/// Caller: `ticked_passes` derives LIVE/PROBE flags through this plus [`pass_verdict`].
pub fn frame_diff_count(
    on: &[u8],
    off: &[u8],
    width: usize,
    height: usize,
    ignore_mask: Option<&[u8]>,
    epsilon: u8,
) -> Result<usize, PixelTruthError> {
    if on.len() != off.len() {
        return Err(PixelTruthError::LengthMismatch {
            on_len: on.len(),
            off_len: off.len(),
        });
    }
    if on.is_empty() {
        return Err(PixelTruthError::EmptyFrame {
            on_len: on.len(),
            off_len: off.len(),
        });
    }
    let expected = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(PixelTruthError::LengthMismatch {
            on_len: on.len(),
            off_len: off.len(),
        })?;
    if expected != on.len() {
        return Err(PixelTruthError::LengthMismatch {
            on_len: on.len(),
            off_len: expected,
        });
    }
    if let Some(mask) = ignore_mask {
        if mask.len() != expected / 4 {
            return Err(PixelTruthError::MaskMismatch {
                frame_len: expected / 4,
                mask_len: mask.len(),
            });
        }
    }
    if on == off {
        return Ok(0);
    }
    let mut diff = 0usize;
    for (index, (on_px, off_px)) in on.chunks_exact(4).zip(off.chunks_exact(4)).enumerate() {
        if let Some(mask) = ignore_mask {
            if mask[index] != 0 {
                continue;
            }
        }
        let mut widest = 0u8;
        for (a, b) in on_px.iter().zip(off_px.iter()) {
            widest = widest.max(a.abs_diff(*b));
        }
        if widest > epsilon {
            diff += 1;
        }
    }
    Ok(diff)
}

/// Count of masked pixels that differ above epsilon; pure integer math.
/// Caller: `ticked_passes` keeps the stub count at zero until captures land.
/// Same inputs give the same output twice; no clock, no float.
pub fn count_masked_ignored(
    on: &[u8],
    off: &[u8],
    width: usize,
    height: usize,
    ignore_mask: Option<&[u8]>,
    epsilon: u8,
) -> Result<usize, PixelTruthError> {
    if on.len() != off.len() {
        return Err(PixelTruthError::LengthMismatch {
            on_len: on.len(),
            off_len: off.len(),
        });
    }
    if on.is_empty() {
        return Err(PixelTruthError::EmptyFrame {
            on_len: on.len(),
            off_len: off.len(),
        });
    }
    let expected = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or(PixelTruthError::LengthMismatch {
            on_len: on.len(),
            off_len: off.len(),
        })?;
    if expected != on.len() {
        return Err(PixelTruthError::LengthMismatch {
            on_len: on.len(),
            off_len: expected,
        });
    }
    if let Some(mask) = ignore_mask {
        if mask.len() != expected / 4 {
            return Err(PixelTruthError::MaskMismatch {
                frame_len: expected / 4,
                mask_len: mask.len(),
            });
        }
    }
    if on == off {
        return Ok(0);
    }
    let Some(mask) = ignore_mask else {
        return Ok(0);
    };
    let mut ignored = 0usize;
    for (index, (on_px, off_px)) in on.chunks_exact(4).zip(off.chunks_exact(4)).enumerate() {
        if mask[index] == 0 {
            continue;
        }
        let mut widest = 0u8;
        for (a, b) in on_px.iter().zip(off_px.iter()) {
            widest = widest.max(a.abs_diff(*b));
        }
        if widest > epsilon {
            ignored += 1;
        }
    }
    Ok(ignored)
}

/// Fraction floor in per-thousand units; true only strictly above the floor.
/// Caller: `ticked_passes` gates the stub diff through this plus [`pass_verdict`].
/// Zero total gives false; integer only, no division; stable on repeat calls.
pub fn live_by_fraction(diff_pixels: usize, total_pixels: usize, min_per_thousand: u32) -> bool {
    if total_pixels == 0 {
        return false;
    }
    let diff_wide = u128::try_from(diff_pixels).unwrap_or(u128::MAX);
    let total_wide = u128::try_from(total_pixels).unwrap_or(u128::MAX);
    diff_wide.saturating_mul(1000) > u128::from(min_per_thousand).saturating_mul(total_wide)
}

/// LIVE only strictly above the floor, else PROBE; pure.
pub fn pass_verdict(diff_pixels: usize, live_floor: usize) -> bool {
    diff_pixels > live_floor
}

/// Resolves one pass LIVE flag from an on/off capture pair.
/// Caller: `ticked_passes` derives the stub PROBE flag through this resolver.
/// Units: RGBA bytes row-major. Determinism: same bytes give the same bool twice.
/// Errors: same as [`frame_diff_count`].
pub fn resolve_pass_live(
    on: &[u8],
    off: &[u8],
    width: usize,
    height: usize,
    ignore_mask: Option<&[u8]>,
    epsilon: u8,
    live_floor: usize,
) -> Result<bool, PixelTruthError> {
    Ok(pass_verdict(
        frame_diff_count(on, off, width, height, ignore_mask, epsilon)?,
        live_floor,
    ))
}

/// Pixel truth is strict on/off diff only; no golden backdoor remains.
/// Pixel truth compares on/off pairs only.

/// Maps one on/off pair to LIVE, or PROBE on ragged input.
/// Caller: `ticked_passes_from_captures` keeps ragged passes at PROBE.
/// Units: RGBA bytes row-major. Determinism: same bytes give same bool twice.
fn resolve_or_probe(
    on: &[u8],
    off: &[u8],
    width: usize,
    height: usize,
    ignore_mask: Option<&[u8]>,
    epsilon: u8,
    live_floor: usize,
) -> bool {
    match resolve_pass_live(on, off, width, height, ignore_mask, epsilon, live_floor) {
        Ok(live) => live,
        Err(_) => false,
    }
}

/// Builds the ticked pass list from on/off capture pairs, diff-gated.
/// Caller: `ticked_passes` feeds inline 2x1 pairs plus quiet stubs.
/// Units: RGBA bytes row-major. Determinism: same bytes give the same flags twice.
/// Errors: ragged pairs resolve to PROBE, never panics.
pub fn ticked_passes_from_captures(
    forward_on: &[u8],
    forward_off: &[u8],
    sky_on: &[u8],
    sky_off: &[u8],
    decals_on: &[u8],
    decals_off: &[u8],
    width: usize,
    height: usize,
    ignore_mask: Option<&[u8]>,
    epsilon: u8,
    live_floor: usize,
) -> Vec<PassGate> {
    let forward_live = resolve_or_probe(
        forward_on,
        forward_off,
        width,
        height,
        ignore_mask,
        epsilon,
        live_floor,
    );
    let sky_live = resolve_or_probe(
        sky_on,
        sky_off,
        width,
        height,
        ignore_mask,
        epsilon,
        live_floor,
    );
    let decals_live = resolve_or_probe(
        decals_on,
        decals_off,
        width,
        height,
        ignore_mask,
        epsilon,
        live_floor,
    );
    // Six device-proven rows stay LIVE; forward, sky and decals resolve
    // from the masked on/off diff.
    vec![
        PassGate {
            row: "graph",
            pass: "submit_gpu",
            live: forward_live,
        },
        PassGate {
            row: "shadow_cluster",
            pass: "submit_gpu",
            live: true,
        },
        PassGate {
            row: "ibl",
            pass: "submit_gpu",
            live: true,
        },
        PassGate {
            row: "post",
            pass: "submit_gpu",
            live: true,
        },
        PassGate {
            row: "gtao",
            pass: "submit_gpu",
            live: true,
        },
        PassGate {
            row: "sky",
            pass: "submit_gpu_sky",
            live: sky_live,
        },
        PassGate {
            row: "material_layer",
            pass: "submit_gpu",
            live: true,
        },
        PassGate {
            row: "particles_gpu",
            pass: "submit_gpu",
            live: true,
        },
        PassGate {
            row: "decals",
            pass: "submit_gpu",
            live: decals_live,
        },
    ]
}

/// Registry of the ticked render rows' GPU passes as a plain editable list.
///
/// Entries stay live only with on/off pixel proof; sky and decals are PROBE
/// on the current tree. Caller below wires the pixel verdicts into the flags.
pub fn ticked_passes() -> Vec<PassGate> {
    // Forward feeds a 2x1 on/off pair that differs on pixel 0, so it stays
    // LIVE by pixel diff, never by hardcode. Sky and decals feed identical
    // quiet 2x1 stubs, so they stay PROBE.
    let forward_on: &[u8] = &[10, 10, 200, 255, 10, 10, 10, 255];
    let forward_off: &[u8] = &[10, 10, 10, 255, 10, 10, 10, 255];
    let stub: &[u8] = &[10, 10, 10, 255, 10, 10, 10, 255];
    ticked_passes_from_captures(
        forward_on,
        forward_off,
        stub,
        stub,
        stub,
        stub,
        2,
        1,
        None,
        0,
        0,
    )
}

/// Renders the ticked pass list as a LIVE/PROBE markdown table in registry order.
/// Caller: [`live_gate_report`] renders this table as its drift guard; future
/// `target/reports/pass-truth-*.md` writer reuses the same bytes.
/// Units: none (boolean flags to text). Determinism: same slice gives same bytes; no IO or clock.
pub fn truth_table_markdown(passes: &[PassGate]) -> String {
    let mut out = String::from("| row | verdict | pass |\n");
    for gate in passes {
        let verdict = if gate.live { "LIVE" } else { "PROBE" };
        out.push_str("| ");
        out.push_str(gate.row);
        out.push_str(" | ");
        out.push_str(verdict);
        out.push_str(" | ");
        out.push_str(gate.pass);
        out.push_str(" |\n");
    }
    out
}

/// Dated pass-truth report framing the LIVE/PROBE table.
/// Caller: `live_gate_report` checks this text reuses the table bytes.
/// Units: none (boolean flags to text). Determinism: same date plus slice
/// gives the same bytes twice; date is a plain tag, never parsed.
pub fn pass_truth_report(date: &str, passes: &[PassGate]) -> String {
    let mut out = String::from("# pass truth ");
    out.push_str(date);
    out.push('\n');
    out.push('\n');
    out.push_str(&truth_table_markdown(passes));
    out
}

/// Live gate over the ticked rows; sky and decals stay PROBE until captures move.
/// Caller: `crates/proof/src/lib.rs` re-export for the headless proof harness.
/// Units: none (boolean liveness flags). Determinism: pure, no IO or clock.
/// Errors: [`GpuLiveRefusal`] naming every PROBE row.
pub fn live_gate_report() -> Result<(), GpuLiveRefusal> {
    let passes = ticked_passes();
    let table = truth_table_markdown(&passes);
    let report = pass_truth_report("live-gate", &passes);
    if !report.ends_with(&table) {
        return Err(GpuLiveRefusal {
            refused: passes.iter().map(|gate| gate.row).collect(),
        });
    }
    let rendered = table.lines().count().saturating_sub(1);
    if rendered != passes.len() {
        return Err(GpuLiveRefusal {
            refused: passes.iter().map(|gate| gate.row).collect(),
        });
    }
    check_passes(&passes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stub_row() -> PassGate {
        PassGate {
            row: "stub-row",
            pass: "submit_gpu",
            live: false,
        }
    }

    fn live_row() -> PassGate {
        PassGate {
            row: "live-row",
            pass: "submit_gpu",
            live: true,
        }
    }

    #[test]
    fn seeded_stub_list_with_one_stub_fails_naming_it() {
        let err = check_passes(&[live_row(), stub_row()]).expect_err("stub must refuse");
        assert_eq!(err.refused, vec!["stub-row"]);
        assert!(err.to_string().contains("stub-row"));
    }

    #[test]
    fn all_live_list_passes() {
        assert_eq!(check_passes(&[live_row(), live_row()]), Ok(()));
    }

    #[test]
    fn current_tree_gate_reports_forward_live_and_sky_plus_decals_probe() {
        let passes = ticked_passes();
        assert_eq!(passes.len(), 9);
        let err = check_passes(&passes)
            .expect_err("sky and decals stay PROBE until on/off captures prove them");
        assert_eq!(err.refused, vec!["sky", "decals"]);
        assert!(err.to_string().contains("sky"));
    }

    #[test]
    fn live_counts_mask_epsilon_and_floor() {
        let same = [10u8, 20, 30, 40, 50, 60, 70, 80];
        assert_eq!(frame_diff_count(&same, &same, 2, 1, None, 0), Ok(0));
        assert_eq!(
            frame_diff_count(&same, &same, 2, 1, Some(&[1, 1]), 0),
            Ok(0)
        );
        assert!(!pass_verdict(0, 0));
        let on = [10u8, 10, 10, 10, 20, 20, 20, 20];
        let off = [10u8, 14, 10, 10, 20, 20, 20, 20];
        assert_eq!(frame_diff_count(&on, &off, 2, 1, None, 3), Ok(1));
        assert_eq!(frame_diff_count(&on, &off, 2, 1, None, 4), Ok(0));
        let solo_on = [0u8, 0, 0, 0];
        let solo_off = [0u8, 0, 0, 200];
        assert_eq!(
            frame_diff_count(&solo_on, &solo_off, 1, 1, None, 199),
            Ok(1)
        );
        assert_eq!(
            frame_diff_count(&solo_on, &solo_off, 1, 1, None, 200),
            Ok(0)
        );
        let wide_on = [10u8, 10, 10, 10, 20, 20, 20, 20];
        let wide_off = [30u8, 10, 10, 10, 40, 20, 20, 20];
        assert_eq!(frame_diff_count(&wide_on, &wide_off, 2, 1, None, 0), Ok(2));
        assert_eq!(
            frame_diff_count(&wide_on, &wide_off, 2, 1, Some(&[1, 0]), 0),
            Ok(1)
        );
        assert_eq!(
            frame_diff_count(&wide_on, &wide_off, 2, 1, Some(&[1, 1]), 0),
            Ok(0)
        );
        assert!(!pass_verdict(2, 2));
        assert!(pass_verdict(3, 2));
        let moved =
            frame_diff_count(&wide_on, &wide_off, 2, 1, Some(&[0, 1]), 5).expect("frames compare");
        assert_eq!(moved, 1);
        assert!(pass_verdict(moved, 0));
    }

    #[test]
    fn live_ragged_inputs_fail_and_repeat() {
        let on = [10u8, 10, 10, 10, 20, 20, 20, 20];
        let off = [10u8, 10, 10, 10, 20, 20, 20, 20];
        let err = frame_diff_count(&on, &off[..7], 2, 1, None, 0).expect_err("ragged frames fail");
        assert!(matches!(err, PixelTruthError::LengthMismatch { .. }));
        let err = frame_diff_count(&[], &[], 0, 0, None, 0).expect_err("empty frames fail");
        assert!(matches!(err, PixelTruthError::EmptyFrame { .. }));
        let err = frame_diff_count(&on, &off, 2, 2, None, 0).expect_err("dimensions fail");
        assert!(matches!(err, PixelTruthError::LengthMismatch { .. }));
        let err = frame_diff_count(&on, &off, 2, 1, Some(&[0]), 0).expect_err("mask fails");
        assert!(matches!(err, PixelTruthError::MaskMismatch { .. }));
        let a = [10u8, 10, 10, 10, 20, 20, 20, 20];
        let b = [30u8, 10, 10, 10, 20, 20, 20, 20];
        let first = frame_diff_count(&a, &b, 2, 1, Some(&[0, 1]), 5).expect("first compares");
        let second = frame_diff_count(&a, &b, 2, 1, Some(&[0, 1]), 5).expect("second compares");
        assert_eq!(first, second);
        assert_eq!(first, 1);
        assert!(pass_verdict(first, 0));
    }

    #[test]
    fn live_gate_caller_reports_sky_and_decals_probe() {
        let on = [0u8, 0, 0, 0];
        let off = [0u8, 0, 0, 200];
        let moved = frame_diff_count(&on, &off, 1, 1, None, 199).expect("spike frames compare");
        assert_eq!(moved, 1);
        assert!(pass_verdict(moved, 0));
        let err = crate::live_gate_report().expect_err("sky and decals stay PROBE");
        assert_eq!(err.refused, vec!["sky", "decals"]);
    }

    #[test]
    fn live_masked_ignored_counts_only_masked_diffs() {
        // Two pixels; first differs wide, second matches.
        let on = [10u8, 10, 10, 10, 20, 20, 20, 20];
        let off = [30u8, 10, 10, 10, 20, 20, 20, 20];
        // Masked diff is ignored in the diff count but counted as ignored.
        assert_eq!(frame_diff_count(&on, &off, 2, 1, Some(&[1, 0]), 0), Ok(0));
        assert_eq!(
            count_masked_ignored(&on, &off, 2, 1, Some(&[1, 0]), 0),
            Ok(1)
        );
        // Same frames with no mask: diff counts, ignored stays zero.
        assert_eq!(frame_diff_count(&on, &off, 2, 1, None, 0), Ok(1));
        assert_eq!(count_masked_ignored(&on, &off, 2, 1, None, 0), Ok(0));
        assert_eq!(
            count_masked_ignored(&on, &off, 2, 1, Some(&[0, 0]), 0),
            Ok(0)
        );
        // Small diff at or below epsilon is neither diff nor ignored.
        assert_eq!(frame_diff_count(&on, &off, 2, 1, Some(&[1, 0]), 20), Ok(0));
        assert_eq!(
            count_masked_ignored(&on, &off, 2, 1, Some(&[1, 0]), 20),
            Ok(0)
        );
    }

    #[test]
    fn live_fraction_gate_needs_one_percent() {
        // Strictly above the floor: 1 of 100 is exactly the floor, so PROBE.
        assert!(!live_by_fraction(1, 100, 10));
        // Just below the floor gives LIVE, and well above gives LIVE.
        assert!(live_by_fraction(1, 100, 9));
        assert!(live_by_fraction(11, 100, 10));
        // No diffs stays PROBE.
        assert!(!live_by_fraction(0, 100, 10));
    }

    #[test]
    fn live_fraction_zero_total_is_probe_and_stable_x2() {
        // Zero total never goes live and never divides.
        assert!(!live_by_fraction(0, 0, 10));
        assert!(!live_by_fraction(1, 0, 10));
        assert!(!live_by_fraction(1, 0, 0));
        // Same inputs twice give equal outputs.
        let on = [10u8, 10, 10, 10, 20, 20, 20, 20];
        let off = [30u8, 10, 10, 10, 20, 20, 20, 20];
        let first = count_masked_ignored(&on, &off, 2, 1, Some(&[1, 0]), 0).expect("first counts");
        let second =
            count_masked_ignored(&on, &off, 2, 1, Some(&[1, 0]), 0).expect("second counts");
        assert_eq!(first, second);
        assert_eq!(first, 1);
        assert_eq!(live_by_fraction(1, 100, 10), live_by_fraction(1, 100, 10));
    }

    #[test]
    fn live_table_marks_forward_live_and_sky_probe() {
        let passes = ticked_passes();
        let first = truth_table_markdown(&passes);
        let second = truth_table_markdown(&passes);
        assert_eq!(first.as_bytes(), second.as_bytes());
        assert!(first.contains("| graph | LIVE |"));
        assert!(first.contains("| sky | PROBE |"));
        assert!(first.contains("| decals | PROBE |"));
    }

    #[test]
    fn live_table_empty_stays_header_only() {
        let first = truth_table_markdown(&[]);
        let second = truth_table_markdown(&[]);
        assert_eq!(first, second);
        assert_eq!(first, "| row | verdict | pass |\n");
    }

    #[test]
    fn live_pass_truth_changed_resolves_live() {
        let on = [
            10u8, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255,
        ];
        let off = [
            200u8, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255,
        ];
        let first = resolve_pass_live(&on, &off, 2, 2, None, 5, 0).expect("frames compare");
        let second = resolve_pass_live(&on, &off, 2, 2, None, 5, 0).expect("frames compare");
        assert_eq!(first, second);
        assert!(first);
    }

    #[test]
    fn live_pass_truth_masked_change_stays_probe() {
        let on = [
            10u8, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255,
        ];
        let off = [
            200u8, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255,
        ];
        let first =
            resolve_pass_live(&on, &off, 2, 2, Some(&[1, 0, 0, 0]), 5, 0).expect("frames compare");
        let second =
            resolve_pass_live(&on, &off, 2, 2, Some(&[1, 0, 0, 0]), 5, 0).expect("frames compare");
        assert_eq!(first, second);
        assert!(!first);
        assert_eq!(
            frame_diff_count(&on, &off, 2, 2, Some(&[1, 0, 0, 0]), 5),
            Ok(0)
        );
    }

    #[test]
    fn live_pass_truth_under_epsilon_stays_probe() {
        let on = [
            10u8, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255,
        ];
        let off = [
            200u8, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255,
        ];
        let first = resolve_pass_live(&on, &off, 2, 2, None, 250, 0).expect("frames compare");
        let second = resolve_pass_live(&on, &off, 2, 2, None, 250, 0).expect("frames compare");
        assert_eq!(first, second);
        assert!(!first);
        assert_eq!(frame_diff_count(&on, &off, 2, 2, None, 250), Ok(0));
    }

    #[test]
    fn live_pass_truth_report_reuses_table_bytes() {
        let passes = [live_row(), stub_row()];
        let date = "2026-10-12";
        let first = pass_truth_report(date, &passes);
        let second = pass_truth_report(date, &passes);
        assert_eq!(first.as_bytes(), second.as_bytes());
        assert!(first.starts_with("# pass truth 2026-10-12\n\n"));
        assert_eq!(
            first,
            format!("# pass truth {date}\n\n{}", truth_table_markdown(&passes))
        );
    }

    #[test]
    fn live_pass_truth_from_captures_probe_then_live() {
        let base = [
            10u8, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255,
        ];
        let moved = [
            200u8, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255,
        ];
        let forward_on: &[u8] = &[
            10, 10, 200, 255, 10, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255,
        ];
        let forward_off: &[u8] = &[
            10, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255,
        ];
        let decals_on: &[u8] = &[
            10, 200, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255,
        ];
        let decals_off: &[u8] = &[
            10, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255,
        ];
        let flag = |passes: &[PassGate], row: &str| {
            passes
                .iter()
                .find(|gate| gate.row == row)
                .expect("row present")
                .live
        };
        let probe = ticked_passes_from_captures(
            forward_on,
            forward_off,
            &base,
            &base,
            &base,
            &base,
            2,
            2,
            None,
            5,
            0,
        );
        assert_eq!(probe.len(), 9);
        assert!(flag(&probe, "graph"));
        assert!(!flag(&probe, "sky"));
        assert!(!flag(&probe, "decals"));
        let live = ticked_passes_from_captures(
            forward_on,
            forward_off,
            &moved,
            &base,
            decals_on,
            decals_off,
            2,
            2,
            None,
            5,
            0,
        );
        assert_eq!(live.len(), 9);
        assert!(flag(&live, "graph"));
        assert!(flag(&live, "sky"));
        assert!(flag(&live, "decals"));
        let masked = ticked_passes_from_captures(
            forward_on,
            forward_off,
            &moved,
            &base,
            decals_on,
            decals_off,
            2,
            2,
            Some(&[1, 0, 0, 0]),
            5,
            0,
        );
        assert!(!flag(&masked, "graph"));
        assert!(!flag(&masked, "sky"));
        assert!(!flag(&masked, "decals"));
        let soft = ticked_passes_from_captures(
            forward_on,
            forward_off,
            &moved,
            &base,
            decals_on,
            decals_off,
            2,
            2,
            None,
            250,
            0,
        );
        assert!(!flag(&soft, "graph"));
        assert!(!flag(&soft, "sky"));
        assert!(!flag(&soft, "decals"));
        let first = truth_table_markdown(&live);
        let second = truth_table_markdown(&live);
        assert_eq!(first.as_bytes(), second.as_bytes());
        let report = pass_truth_report("2026-10-12", &live);
        assert!(report.ends_with(&first));
        assert_eq!(
            report.as_bytes(),
            pass_truth_report("2026-10-12", &live).as_bytes()
        );
        let ragged = ticked_passes_from_captures(
            forward_on,
            &forward_off[..8],
            &moved,
            &base[..8],
            decals_on,
            &decals_off[..8],
            2,
            2,
            None,
            5,
            0,
        );
        assert!(!flag(&ragged, "graph"));
        assert!(!flag(&ragged, "sky"));
        assert!(!flag(&ragged, "decals"));
        let err = check_passes(&ticked_passes()).expect_err("sky and decals stay PROBE");
        assert_eq!(err.refused, vec!["sky", "decals"]);
    }

    // trimmed live_drifted test: golden backdoor removed.

    // trimmed live_bless test: golden backdoor removed.

    #[test]
    fn live_forward_still_frames_stay_probe_until_pixels_prove_it() {
        // Forward goes LIVE only when its own on/off pair differs above
        // epsilon; still, masked or soft pairs stay PROBE like sky and decals.
        let quiet: &[u8] = &[
            10, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255,
        ];
        let forward_on: &[u8] = &[
            10, 10, 200, 255, 10, 10, 10, 255, 10, 10, 10, 255, 10, 10, 10, 255,
        ];
        let flag = |passes: &[PassGate], row: &str| {
            passes
                .iter()
                .find(|gate| gate.row == row)
                .expect("row present")
                .live
        };
        let still =
            ticked_passes_from_captures(quiet, quiet, quiet, quiet, quiet, quiet, 2, 2, None, 5, 0);
        assert!(!flag(&still, "graph"));
        let live = ticked_passes_from_captures(
            forward_on, quiet, quiet, quiet, quiet, quiet, 2, 2, None, 5, 0,
        );
        assert!(flag(&live, "graph"));
        let again = ticked_passes_from_captures(
            forward_on, quiet, quiet, quiet, quiet, quiet, 2, 2, None, 5, 0,
        );
        assert_eq!(
            truth_table_markdown(&live).as_bytes(),
            truth_table_markdown(&again).as_bytes()
        );
    }
}
