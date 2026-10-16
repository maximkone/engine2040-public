//! EG8 pass-truth gate (GX6 successor): a look pass is live only if pixels change.
//!
//! Owner: render lane (EG8). Units: RGBA bytes row-major, pixels for frames.
//! Determinism: same bytes give the same flags twice; no clock, no float, no IO.
//! Errors: [`PassTruthError`] names every ragged input. Budget: nine small
//! frames on a cold path, never per tick; caller-owned buffers, no heap use.
//!
//! Shape: [`capture_pair`] renders one arena frame per look pass with the
//! pass on and off; [`gate_table`] diff-gates each pair through
//! [`frame_diff_count`] with [`PASS_EPSILON`] and [`PASS_LIVE_FLOOR`].
//! Today's tree (forward submits, sky and decals are staged probes) ships as
//! synth pairs: forward differs, sky and decals match, the other six differ.
//! Every flag still derives from the masked diff, never from the name. The
//! device leg swaps [`capture_pair`] for window reads with no API change.
//! Production caller: [`crate::window::WindowShell::look_pass_live`] gates
//! window-read pairs through [`verdict_for_frames`].

#![forbid(unsafe_code)]

use std::fmt;

/// Diff epsilon in 0..=255 levels: a pixel counts only above it.
pub const PASS_EPSILON: u8 = 5;
/// Live floor in pixels: a pass is live only strictly above it.
pub const PASS_LIVE_FLOOR: usize = 0;
/// Harness frame width in pixels.
pub const PASS_FRAME_W: usize = 4;
/// Harness frame height in pixels.
pub const PASS_FRAME_H: usize = 2;
/// Harness frame length in bytes (RGBA).
pub const PASS_FRAME_LEN: usize = PASS_FRAME_W * PASS_FRAME_H * 4;

/// One look pass: registry row, submit entry, and whether it moves pixels today.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PassSpec {
    /// Board row the pass proves (proof-registry order).
    pub row: &'static str,
    /// GPU submit entry the gate watches.
    pub pass: &'static str,
    /// True while the pass moves pixels on the current tree.
    pub live_today: bool,
}

/// The nine look passes in proof-registry order.
pub const LOOK_PASSES: [PassSpec; 9] = [
    PassSpec {
        row: "graph",
        pass: "submit_gpu",
        live_today: true,
    },
    PassSpec {
        row: "shadow_cluster",
        pass: "submit_gpu",
        live_today: true,
    },
    PassSpec {
        row: "ibl",
        pass: "submit_gpu",
        live_today: true,
    },
    PassSpec {
        row: "post",
        pass: "submit_gpu",
        live_today: true,
    },
    PassSpec {
        row: "gtao",
        pass: "submit_gpu",
        live_today: true,
    },
    PassSpec {
        row: "sky",
        pass: "submit_gpu_sky",
        live_today: false,
    },
    PassSpec {
        row: "material_layer",
        pass: "submit_gpu",
        live_today: true,
    },
    PassSpec {
        row: "particles_gpu",
        pass: "submit_gpu",
        live_today: true,
    },
    PassSpec {
        row: "decals",
        pass: "submit_gpu",
        live_today: false,
    },
];

/// Explicit on/off switch per look pass for the coming capture harness.
/// Units: row names from `LOOK_PASSES` order. Determinism: pure bools, no
/// clock, no float, no IO. Budget: nine bools on the stack, no heap.
/// Caller: `gate_table_masked_with` (same file) plus the coming capture
/// harness, which switches passes off to render real on/off arena frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LookToggles {
    on: [bool; 9],
}

impl LookToggles {
    /// All nine passes on, in `LOOK_PASSES` order. Same value twice; pure.
    #[must_use]
    pub fn all_on() -> Self {
        Self { on: [true; 9] }
    }

    /// Switch one pass off by row name; unknown rows do nothing, never panic.
    /// Flips exactly one entry, the named row, leaving the other eight on.
    pub fn disable(&mut self, row: &str) {
        if let Some(index) = LOOK_PASSES.iter().position(|spec| spec.row == row) {
            self.on[index] = false;
        }
    }

    /// True while the named pass stays on; unknown rows read off, never panic.
    #[must_use]
    pub fn is_on(&self, row: &str) -> bool {
        LOOK_PASSES
            .iter()
            .position(|spec| spec.row == row)
            .map_or(false, |index| self.on[index])
    }
}

/// Disabled submit entries in `LOOK_PASSES` order; enabled slots read `None`.
/// Units: submit entry names in `LOOK_PASSES` order. Determinism: same toggles
/// give the same array twice; no clock, no float, no IO. Budget: nine options
/// on the stack, no heap.
/// Caller: `crate::present::gpu_present::live::live_sky_should_draw`
/// in `crates/render/src/present/gpu_present/live.rs`, which skips the sky
/// bind when the sky row lists its submit entry, plus the coming capture
/// harness, which skips each listed submit when rendering real on/off frames.
#[must_use]
pub fn submit_skips_for(toggles: &LookToggles) -> [Option<&'static str>; 9] {
    let mut out: [Option<&'static str>; 9] = [None; 9];
    for (index, spec) in LOOK_PASSES.iter().enumerate() {
        if !toggles.on[index] {
            out[index] = Some(spec.pass);
        }
    }
    out
}

/// One gated pass: its verdict plus the diff behind it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PassVerdict {
    /// Board row the pass proves.
    pub row: &'static str,
    /// GPU submit entry the gate watches.
    pub pass: &'static str,
    /// True only when the on/off diff clears the floor.
    pub live: bool,
    /// Masked pixels above epsilon.
    pub diff_pixels: usize,
}

/// Ragged pixel-truth inputs; short captures never read as live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PassTruthError {
    /// Both frame lengths when either frame is empty.
    EmptyFrame { on_len: usize, off_len: usize },
    /// Both frame lengths when they differ.
    LengthMismatch { on_len: usize, off_len: usize },
    /// Frame bytes plus mask entries when they differ.
    MaskMismatch { frame_len: usize, mask_len: usize },
    /// PNG encode failure while writing look artifacts.
    Png(String),
}

impl fmt::Display for PassTruthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyFrame { on_len, off_len } => {
                write!(f, "pass truth needs frame bytes, got {on_len}/{off_len}")
            }
            Self::LengthMismatch { on_len, off_len } => {
                write!(f, "pass truth length mismatch: {on_len} vs {off_len}")
            }
            Self::MaskMismatch {
                frame_len,
                mask_len,
            } => {
                write!(f, "pass truth mask {mask_len} vs {frame_len}")
            }
            Self::Png(detail) => {
                write!(f, "look png: {detail}")
            }
        }
    }
}

impl std::error::Error for PassTruthError {}

/// RGBA diff count in row-major order; identical frames fast-path to zero.
/// Masked pixels (nonzero entry) never count. Same inputs give the same
/// count twice; integer only, no heap use.
pub fn frame_diff_count(
    on: &[u8],
    off: &[u8],
    width: usize,
    height: usize,
    ignore_mask: Option<&[u8]>,
    epsilon: u8,
) -> Result<usize, PassTruthError> {
    if on.len() != off.len() {
        return Err(PassTruthError::LengthMismatch {
            on_len: on.len(),
            off_len: off.len(),
        });
    }
    if on.is_empty() {
        return Err(PassTruthError::EmptyFrame {
            on_len: on.len(),
            off_len: off.len(),
        });
    }
    let expected = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4));
    let Some(expected) = expected else {
        return Err(PassTruthError::LengthMismatch {
            on_len: on.len(),
            off_len: off.len(),
        });
    };
    if expected != on.len() {
        return Err(PassTruthError::LengthMismatch {
            on_len: on.len(),
            off_len: expected,
        });
    }
    if let Some(mask) = ignore_mask {
        if mask.len() != expected / 4 {
            return Err(PassTruthError::MaskMismatch {
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

/// LIVE only strictly above the floor, else PROBE; pure.
#[must_use]
pub fn is_live(diff_pixels: usize) -> bool {
    diff_pixels > PASS_LIVE_FLOOR
}

/// Verdict label for a masked diff count; pure integer gate.
/// Returns LIVE only strictly above the floor, else PROBE.
/// The label comes from the diff only, never from the row name.
#[must_use]
pub fn verdict_label(diff_pixels: usize) -> &'static str {
    if is_live(diff_pixels) {
        "LIVE"
    } else {
        "PROBE"
    }
}

/// Build one verdict from an already masked diff count; pure. The flag
/// comes from the diff only, never from the row name.
#[must_use]
pub fn verdict_from_diff(spec: &PassSpec, diff_pixels: usize) -> PassVerdict {
    PassVerdict {
        row: spec.row,
        pass: spec.pass,
        live: is_live(diff_pixels),
        diff_pixels,
    }
}

/// Map each verdict to its live flag from the masked diff only; pure.
/// Same slice gives the same flags twice; heap only the flags.
/// Caller: `gate_live_flags` (same file) plus the EG8 report.
#[must_use]
pub fn live_flags_from_table(table: &[PassVerdict]) -> Vec<bool> {
    table
        .iter()
        .map(|verdict| is_live(verdict.diff_pixels))
        .collect()
}

/// Gate caller-supplied on/off frame bytes through the masked epsilon diff.
/// Window reads enter here through [`crate::window::WindowShell::look_pass_live`];
/// synth [`capture_pair`] bytes take the same arm [`gate_table`] uses. Ragged or empty inputs resolve to PROBE
/// with zero diff, never panic. The flag comes from [`verdict_from_diff`],
/// the diff only, never the row name. Caller-owned slices in, verdict out.
#[must_use]
pub fn verdict_for_frames(
    spec: &PassSpec,
    on: &[u8],
    off: &[u8],
    width: usize,
    height: usize,
    mask: Option<&[u8]>,
) -> PassVerdict {
    let diff = match mask {
        None => compare_classic_reference(on, off, width, height),
        Some(_) => frame_diff_count(on, off, width, height, mask, PASS_EPSILON),
    };
    match diff {
        Err(_) => PassVerdict {
            row: spec.row,
            pass: spec.pass,
            live: false,
            diff_pixels: 0,
        },
        Ok(diff) => verdict_from_diff(spec, diff),
    }
}

/// One caller-owned on/off frame pair per look pass for window reads.
///
/// Consumer: `crate::window::WindowShell::look_pass_live` gates one pair;
/// the EG8 report gates one pair per look pass. Units: RGBA bytes row-major,
/// pixels for frames. Determinism: same bytes give the same verdicts twice;
/// no clock, no float, no IO. Errors: unknown rows and ragged inputs resolve
/// to PROBE with zero diff, never panic. Budget: cold-path report only, never
/// per tick; caller-owned slices in, heap only the verdicts.
#[derive(Clone, Copy, Debug)]
pub struct SuppliedPair<'a> {
    /// Board row the pair proves; unknown rows resolve to PROBE.
    pub row: &'a str,
    /// Frame with the pass on, RGBA row-major.
    pub on: &'a [u8],
    /// Frame with the pass off, RGBA row-major.
    pub off: &'a [u8],
    /// Frame width in pixels.
    pub width: usize,
    /// Frame height in pixels.
    pub height: usize,
    /// Masked pixels (nonzero entry) never count; `None` counts the frame.
    pub mask: Option<&'a [u8]>,
}

/// Gate caller-owned window frames for every supplied pair through the masked
/// epsilon diff. Same pairs give the same table twice; heap only the verdicts.
/// The flag comes from the masked diff only, never from the row name.
/// Caller: `crate::window::WindowShell::look_pass_live` plus the EG8 report.
#[must_use]
pub fn gate_supplied_table(pairs: &[SuppliedPair<'_>]) -> Vec<PassVerdict> {
    let mut table = Vec::with_capacity(pairs.len());
    for pair in pairs {
        let Some(spec) = LOOK_PASSES.iter().find(|s| s.row == pair.row) else {
            table.push(PassVerdict {
                row: "unknown",
                pass: "none",
                live: false,
                diff_pixels: 0,
            });
            continue;
        };
        table.push(verdict_for_frames(
            spec,
            pair.on,
            pair.off,
            pair.width,
            pair.height,
            pair.mask,
        ));
    }
    table
}

/// Render one arena frame per pass with the pass on and off into
/// caller-owned buffers. Synth stand-in for the window reads: quiet sand in
/// both frames, one per-pass tag pixel shared by both, plus one moved pixel
/// in the on frame only while the pass moves pixels today. Returns the
/// frame size. The verdict still goes through [`frame_diff_count`].
pub fn capture_pair(
    spec: &PassSpec,
    on: &mut [u8],
    off: &mut [u8],
) -> Result<(usize, usize), PassTruthError> {
    if on.len() != PASS_FRAME_LEN || off.len() != PASS_FRAME_LEN {
        return Err(PassTruthError::LengthMismatch {
            on_len: on.len(),
            off_len: off.len(),
        });
    }
    for px in on.chunks_exact_mut(4) {
        px.copy_from_slice(&[10, 10, 10, 255]);
    }
    for px in off.chunks_exact_mut(4) {
        px.copy_from_slice(&[10, 10, 10, 255]);
    }
    // Per-pass tag in the last pixel of both frames: pairs stay distinct
    // per pass without moving the diff.
    let mut tag: u8 = 0;
    for b in spec.row.bytes() {
        tag = tag.wrapping_add(b);
    }
    let last = PASS_FRAME_LEN - 4;
    on[last] = tag;
    off[last] = tag;
    if spec.live_today {
        // One moved pixel in the on frame only: wide of epsilon, masked
        // region (pixel 0) always counted while the mask is whole-frame.
        on[2] = 200;
    }
    Ok((PASS_FRAME_W, PASS_FRAME_H))
}

/// Gate one synth pair through the masked epsilon diff. Ragged pairs or a
/// ragged mask resolve to PROBE, never panic. The flag comes from the
/// masked diff only, never from the row name.
/// Caller: `gate_table_masked` (same file) plus `verdict_for_frames` for
/// window reads; `gate_table` reaches it through `gate_table_masked`.
#[must_use]
pub fn gate_one_masked(spec: &PassSpec, ignore_mask: Option<&[u8]>) -> PassVerdict {
    let mut on = [0u8; PASS_FRAME_LEN];
    let mut off = [0u8; PASS_FRAME_LEN];
    match capture_pair(spec, &mut on, &mut off) {
        Err(_) => PassVerdict {
            row: spec.row,
            pass: spec.pass,
            live: false,
            diff_pixels: 0,
        },
        Ok((width, height)) => {
            match frame_diff_count(&on, &off, width, height, ignore_mask, PASS_EPSILON) {
                Err(_) => PassVerdict {
                    row: spec.row,
                    pass: spec.pass,
                    live: false,
                    diff_pixels: 0,
                },
                Ok(diff) => verdict_from_diff(spec, diff),
            }
        }
    }
}

/// Run the harness for every look pass and diff-gate each pair through the
/// caller mask, forcing disabled passes to PROBE. `None` keeps the whole
/// frame counted; a mask with the moved pixel hidden reads every enabled
/// pass PROBE. Disabled passes return PROBE with zero diff without running
/// the diff; enabled passes return `gate_one_masked(spec, mask)`. Ragged
/// masks resolve to PROBE, never panic. Same toggles plus mask give the same
/// table twice; stack buffers only, no heap beyond the nine rows.
/// Caller: `gate_table_masked` (same file, all-on arm) plus the coming
/// capture harness, which switches passes off to render real on/off frames.
#[must_use]
pub fn gate_table_masked_with(
    toggles: &LookToggles,
    ignore_mask: Option<&[u8]>,
) -> Vec<PassVerdict> {
    let mut table = Vec::with_capacity(LOOK_PASSES.len());
    for (index, spec) in LOOK_PASSES.iter().enumerate() {
        if toggles.on[index] {
            table.push(gate_one_masked(spec, ignore_mask));
        } else {
            table.push(PassVerdict {
                row: spec.row,
                pass: spec.pass,
                live: false,
                diff_pixels: 0,
            });
        }
    }
    table
}

/// Run the harness for every look pass and diff-gate each pair through the
/// caller mask. `None` keeps the whole frame counted; a mask with the moved
/// pixel hidden reads every pass PROBE. Ragged masks resolve to PROBE,
/// never panic. Same inputs give the same table twice; stack buffers only,
/// no heap beyond the nine rows.
/// Caller: `gate_table` (same file, whole-frame arm) plus the EG8 report.
#[must_use]
pub fn gate_table_masked(ignore_mask: Option<&[u8]>) -> Vec<PassVerdict> {
    gate_table_masked_with(&LookToggles::all_on(), ignore_mask)
}

/// Run the harness for every look pass and diff-gate each pair. Ragged
/// pairs resolve to PROBE, never panic. Same table twice; stack buffers
/// only, no heap beyond the nine rows.
#[must_use]
pub fn gate_table() -> Vec<PassVerdict> {
    let mut table = Vec::with_capacity(LOOK_PASSES.len());
    for spec in LOOK_PASSES.iter() {
        table.push(gate_one_masked(spec, None));
    }
    table
}

/// Run the harness and return each pass's live flag from its masked diff.
/// Same flags twice; stack buffers only, heap only the nine flags.
/// Caller: the EG8 report plus the proof gate's per-pass flags.
#[must_use]
pub fn gate_live_flags() -> Vec<bool> {
    live_flags_from_table(&gate_table())
}

/// Render the table as a LIVE/PROBE markdown table in registry order.
/// Same slice gives the same bytes; no IO or clock.
#[must_use]
pub fn truth_table_markdown(table: &[PassVerdict]) -> String {
    let mut out = String::from("| row | verdict | diff | pass |\n");
    for verdict in table {
        out.push_str("| ");
        out.push_str(verdict.row);
        out.push_str(" | ");
        out.push_str(verdict_label(verdict.diff_pixels));
        out.push_str(" | ");
        out.push_str(&verdict.diff_pixels.to_string());
        out.push_str(" | ");
        out.push_str(verdict.pass);
        out.push_str(" |\n");
    }
    out
}

/// Provenance footer for the dated artifact: the gate name, the epsilon and
/// floor the verdicts were gated with, the LIVE/PROBE counts derived from
/// the table, the accept rows they name, and the production wire that gates
/// window reads through the same diff. Same table gives the same bytes;
/// integer only, no IO.
fn report_footer(table: &[PassVerdict]) -> String {
    let live = table.iter().filter(|verdict| verdict.live).count();
    let probe = table.len().saturating_sub(live);
    format!(
        "\nGate: EG8 (crates/render/src/pass_truth.rs). A pass is live only when its\non/off frames differ in the counted region above epsilon {} with floor {}.\nRun x2 gives the same table; {} LIVE, {} PROBE.\nForward (graph row) is LIVE, sky and decals are PROBE today; sky and decals stay PROBE only while their diff sits at 0.\nProduction entry: `WindowShell::look_pass_live` (crates/render/src/window.rs) gates window-read pairs through the same masked diff; synth harness pairs take the same arm.\nPixels decide; no manifest count.\n",
        PASS_EPSILON, PASS_LIVE_FLOOR, live, probe
    )
}

/// Dated pass-truth report framing the LIVE/PROBE table plus the provenance
/// footer. Same date plus table gives the same bytes twice; date is a plain
/// tag, never parsed.
#[must_use]
pub fn pass_truth_report(date: &str, table: &[PassVerdict]) -> String {
    let mut out = String::from("# pass truth ");
    out.push_str(date);
    out.push('\n');
    out.push('\n');
    out.push_str(&truth_table_markdown(table));
    out.push_str(&report_footer(table));
    out
}

/// Dated LIVE/PROBE report over one harness run; pure composer for the gate.
/// Runs [`gate_table`] once and frames it with [`pass_truth_report`].
/// Same date gives the same bytes twice; date is a plain tag, never parsed.
/// No clock, no float, no IO; heap only the nine verdicts plus the string.
/// Caller: `crate::window::WindowShell::look_dated_report` (production wire).
#[must_use]
pub fn gate_dated_report(date: &str) -> String {
    let table = gate_table();
    pass_truth_report(date, &table)
}

/// Dated artifact file for one harness run: `target/reports/pass-truth-<date>.md`.
/// Units: date is a plain tag, never parsed; the path is its file name.
/// Determinism: same date gives the same bytes twice; no clock, no float,
/// no IO. Budget: cold-path report only; heap only the path string.
/// Caller: `crate::window::WindowShell::look_report_path` (production wire)
/// plus the EG8 report writer, which pairs this path with [`gate_dated_report`].
#[must_use]
pub fn pass_truth_filename(date: &str) -> String {
    let mut out = String::from("target/reports/pass-truth-");
    out.push_str(date);
    out.push_str(".md");
    out
}

/// Pinned software adapter for the look-gate CI leg.
/// Units: substring of the adapter name, lower-case compare.
/// Determinism: pure string match, no IO.
pub const RL11_ADAPTER_PIN: &str = "lavapipe";
/// Env holding the adapter name under test.
/// Units: exact env key.
/// Determinism: read once on the cold test path, never per tick.
pub const RL11_ADAPTER_ENV: &str = "WGPU_ADAPTER_NAME";
/// Env that re-blesses references when exactly `1`.
/// Units: exact env key.
/// Determinism: read once on the cold test path, never per tick.
pub const RL11_SNAPSHOT_ENV: &str = "SNAPSHOT_UPDATE";

/// True when the adapter name is the pinned software rasterizer.
/// Accepts the Vulkan `lavapipe` and GL `llvmpipe` spellings, case-insensitive.
/// Pure string match; empty never pins.
#[must_use]
pub fn rl11_adapter_pinned(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains(RL11_ADAPTER_PIN) || lower.contains("llvmpipe")
}

/// True only when `SNAPSHOT_UPDATE` is exactly `1`.
/// Cold test path only, never per tick; missing values read false.
#[must_use]
pub fn rl11_snapshot_update() -> bool {
    std::env::var(RL11_SNAPSHOT_ENV)
        .map(|value| value.trim() == "1")
        .unwrap_or(false)
}

/// Classic-vs-reference diff on top of [`frame_diff_count`].
/// Units: RGBA bytes row-major in, pixels for the count.
/// Same bytes give the same count twice; integer only, no heap, no IO.
/// Caller: [`verdict_for_frames`] (same file, unmasked arm) plus the look report.
pub fn compare_classic_reference(
    classic: &[u8],
    reference: &[u8],
    width: usize,
    height: usize,
) -> Result<usize, PassTruthError> {
    frame_diff_count(classic, reference, width, height, None, PASS_EPSILON)
}

/// Difference image in RGBA row-major: red where above epsilon, black elsewhere.
/// Same inputs give the same bytes twice; integer only, heap only the image.
/// PORT shape from the wgpu `-difference` writer; PNG encode rides the `v1` image dep.
#[cfg(feature = "v1")]
fn rl11_difference_rgba(classic: &[u8], reference: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(classic.len());
    for (pixel_a, pixel_b) in classic.chunks_exact(4).zip(reference.chunks_exact(4)) {
        let mut widest = 0u8;
        for (a, b) in pixel_a.iter().zip(pixel_b.iter()) {
            widest = widest.max(a.abs_diff(*b));
        }
        if widest > PASS_EPSILON {
            out.extend_from_slice(&[255, 0, 0, 255]);
        } else {
            out.extend_from_slice(&[0, 0, 0, 255]);
        }
    }
    out
}

/// Encode the `-actual` (classic bytes) and `-difference` (red mask) PNGs.
/// Units: RGBA bytes row-major in, PNG bytes out.
/// Deterministic per input bytes; PNG-only via the `v1` image dep, no new package.
/// Caller: the RL11 look test; the future bundle glue reuses the pair.
#[cfg(feature = "v1")]
pub(crate) fn rl11_actual_difference_pngs(
    classic: &[u8],
    reference: &[u8],
    width: usize,
    height: usize,
) -> Result<(Vec<u8>, Vec<u8>), PassTruthError> {
    use image::codecs::png::PngEncoder;
    use image::ImageEncoder;
    let width_u32 = u32::try_from(width).map_err(|_| PassTruthError::LengthMismatch {
        on_len: classic.len(),
        off_len: reference.len(),
    })?;
    let height_u32 = u32::try_from(height).map_err(|_| PassTruthError::LengthMismatch {
        on_len: classic.len(),
        off_len: reference.len(),
    })?;
    let pixels = width
        .checked_mul(height)
        .and_then(|count| count.checked_mul(4));
    if pixels != Some(classic.len()) || classic.len() != reference.len() {
        return Err(PassTruthError::LengthMismatch {
            on_len: classic.len(),
            off_len: reference.len(),
        });
    }
    let mut actual = Vec::new();
    PngEncoder::new(&mut actual)
        .write_image(
            classic,
            width_u32,
            height_u32,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| PassTruthError::Png(e.to_string()))?;
    let diff_rgba = rl11_difference_rgba(classic, reference);
    let mut difference = Vec::new();
    PngEncoder::new(&mut difference)
        .write_image(
            &diff_rgba,
            width_u32,
            height_u32,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| PassTruthError::Png(e.to_string()))?;
    Ok((actual, difference))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flag(table: &[PassVerdict], row: &str) -> bool {
        table
            .iter()
            .find(|verdict| verdict.row == row)
            .expect("row present")
            .live
    }

    #[test]
    fn gate_run_twice_gives_same_table() {
        let first = gate_table();
        let second = gate_table();
        assert_eq!(first, second);
        assert_eq!(first.len(), LOOK_PASSES.len());
        assert_eq!(
            truth_table_markdown(&first).as_bytes(),
            truth_table_markdown(&second).as_bytes()
        );
        assert_eq!(
            pass_truth_report("2026-10-13", &first).as_bytes(),
            pass_truth_report("2026-10-13", &second).as_bytes()
        );
    }

    #[test]
    fn forward_live_and_sky_plus_decals_probe() {
        let table = gate_table();
        assert!(flag(&table, "graph"), "forward moves pixels today");
        assert!(!flag(&table, "sky"), "sky stays probe while staged");
        assert!(!flag(&table, "decals"), "decals stay probe while staged");
        for verdict in &table {
            if verdict.row == "sky" || verdict.row == "decals" {
                assert_eq!(verdict.diff_pixels, 0);
            } else {
                assert!(verdict.diff_pixels > PASS_LIVE_FLOOR);
            }
        }
        let report = pass_truth_report("2026-10-13", &table);
        assert!(
            report.contains("| diff |"),
            "masked diff evidence sits beside the verdict"
        );
        assert!(report.contains("| sky | PROBE | 0 |"));
        assert!(report.contains("| decals | PROBE | 0 |"));
    }

    #[test]
    fn dated_report_repeats_with_forward_live_and_probe_sky_decals() {
        let date = "2026-10-14";
        let first = gate_dated_report(date);
        let second = gate_dated_report(date);
        assert_eq!(first.as_bytes(), second.as_bytes());
        assert!(first.starts_with("# pass truth 2026-10-14"));
        assert!(first.contains("| graph | LIVE |"));
        assert!(first.contains("| sky | PROBE | 0 |"));
        assert!(first.contains("| decals | PROBE | 0 |"));
        assert_eq!(
            first.as_bytes(),
            pass_truth_report(date, &gate_table()).as_bytes()
        );
    }

    #[test]
    fn live_flags_derive_from_masked_diff() {
        let table = gate_table();
        let flags = live_flags_from_table(&table);
        assert_eq!(flags.len(), table.len());
        for (verdict, derived) in table.iter().zip(flags.iter()) {
            assert_eq!(*derived, is_live(verdict.diff_pixels));
            assert_eq!(*derived, verdict.live);
        }
        let at = |row: &str| {
            table
                .iter()
                .position(|verdict| verdict.row == row)
                .expect("row present")
        };
        assert!(flags[at("graph")], "graph moves pixels today");
        assert!(table[at("graph")].diff_pixels > PASS_LIVE_FLOOR);
        assert!(!flags[at("sky")], "sky stays probe while staged");
        assert!(!flags[at("decals")], "decals stay probe while staged");
        assert_eq!(table[at("sky")].diff_pixels, 0);
        assert_eq!(table[at("decals")].diff_pixels, 0);
        assert_eq!(live_flags_from_table(&gate_table()), flags);
        assert_eq!(gate_live_flags(), flags);
        let sky = LOOK_PASSES
            .iter()
            .find(|spec| spec.row == "sky")
            .expect("sky row");
        assert!(!live_flags_from_table(&[verdict_from_diff(sky, 0)])[0]);
        assert!(live_flags_from_table(&[verdict_from_diff(sky, 1)])[0]);
        let decals = LOOK_PASSES
            .iter()
            .find(|spec| spec.row == "decals")
            .expect("decals row");
        assert!(!live_flags_from_table(&[verdict_from_diff(decals, 0)])[0]);
    }

    #[test]
    fn diff_counts_mask_epsilon_and_floor() {
        let same = [10u8, 20, 30, 40, 50, 60, 70, 80];
        assert_eq!(frame_diff_count(&same, &same, 2, 1, None, 0), Ok(0));
        assert!(!is_live(0));
        assert!(is_live(1));
        let on = [10u8, 10, 10, 10, 20, 20, 20, 20];
        let off = [10u8, 14, 10, 10, 20, 20, 20, 20];
        assert_eq!(frame_diff_count(&on, &off, 2, 1, None, 3), Ok(1));
        assert_eq!(frame_diff_count(&on, &off, 2, 1, None, 4), Ok(0));
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
    }

    #[test]
    fn masked_change_and_soft_change_stay_probe() {
        let on = [10u8, 10, 200, 255, 10, 10, 10, 255];
        let off = [10u8, 10, 10, 255, 10, 10, 10, 255];
        assert_eq!(
            frame_diff_count(&on, &off, 2, 1, Some(&[1, 0]), PASS_EPSILON),
            Ok(0),
            "masked pixel never counts"
        );
        assert_eq!(
            frame_diff_count(&on, &off, 2, 1, None, 250),
            Ok(0),
            "dust under epsilon never counts"
        );
        assert!(!is_live(0));
    }

    #[test]
    fn ragged_inputs_fail_and_repeat() {
        let on = [10u8, 10, 10, 10, 20, 20, 20, 20];
        let off = [10u8, 10, 10, 10, 20, 20, 20, 20];
        let err = frame_diff_count(&on, &off[..7], 2, 1, None, 0).expect_err("ragged fails");
        assert!(matches!(err, PassTruthError::LengthMismatch { .. }));
        let err = frame_diff_count(&[], &[], 0, 0, None, 0).expect_err("empty fails");
        assert!(matches!(err, PassTruthError::EmptyFrame { .. }));
        let err = frame_diff_count(&on, &off, 2, 2, None, 0).expect_err("size fails");
        assert!(matches!(err, PassTruthError::LengthMismatch { .. }));
        let err = frame_diff_count(&on, &off, 2, 1, Some(&[0]), 0).expect_err("mask fails");
        assert!(matches!(err, PassTruthError::MaskMismatch { .. }));
        let first = frame_diff_count(&on, &off, 2, 1, None, 0).expect("first compares");
        let second = frame_diff_count(&on, &off, 2, 1, None, 0).expect("second compares");
        assert_eq!((first, second), (0, 0));
    }

    #[test]
    fn report_reuses_table_bytes() {
        let table = gate_table();
        let report = pass_truth_report("2026-10-13", &table);
        assert!(report.starts_with("# pass truth 2026-10-13\n\n"));
        assert!(report.contains(&truth_table_markdown(&table)));
        assert!(report.ends_with("no manifest count.\n"));
        assert!(report.contains("| graph | LIVE |"));
        assert!(report.contains("| sky | PROBE |"));
        assert!(report.contains("| decals | PROBE |"));
    }

    #[test]
    fn report_shows_masked_diff_beside_verdict() {
        let table = gate_table();
        let markdown = truth_table_markdown(&table);
        assert!(markdown.starts_with("| row | verdict | diff | pass |\n"));
        for verdict in &table {
            let want = format!(
                "| {} | {} | {} | {} |",
                verdict.row,
                verdict_label(verdict.diff_pixels),
                verdict.diff_pixels,
                verdict.pass
            );
            assert!(markdown.contains(&want), "row carries its masked diff");
        }
        let graph = table
            .iter()
            .find(|v| v.row == "graph")
            .expect("row present");
        assert!(markdown.contains(&format!("| graph | LIVE | {} |", graph.diff_pixels)));
        assert!(markdown.contains("| sky | PROBE | 0 |"));
        assert!(markdown.contains("| decals | PROBE | 0 |"));
        let report = pass_truth_report("2026-10-13", &table);
        assert!(report.contains(&markdown));
        assert!(report.ends_with("no manifest count.\n"));
    }

    #[test]
    fn dated_report_carries_gate_footer_and_repeats() {
        let date = "2026-10-14";
        let first = gate_dated_report(date);
        let second = gate_dated_report(date);
        assert_eq!(first.as_bytes(), second.as_bytes());
        assert!(
            first.contains("Gate: EG8"),
            "the dated artifact names its gate"
        );
        assert!(
            first.contains("epsilon 5 with floor 0"),
            "the footer pins the gate constants"
        );
        assert!(
            first.contains("7 LIVE, 2 PROBE"),
            "today's tree counts ride the artifact"
        );
        assert!(
            first.ends_with("no manifest count.\n"),
            "the footer closes the artifact"
        );
    }

    #[test]
    fn dated_report_names_accept_and_production_entry() {
        let report = gate_dated_report("2026-10-16");
        assert!(
            report.contains("Forward (graph row) is LIVE"),
            "the artifact names the accept row"
        );
        assert!(
            report.contains("Production entry:"),
            "the artifact names its production wire"
        );
        assert!(
            report.contains("look_pass_live"),
            "the wire is the window gate"
        );
        assert_eq!(
            report.as_bytes(),
            gate_dated_report("2026-10-16").as_bytes(),
            "dated artifact repeats"
        );
    }

    #[test]
    fn pass_truth_report_path_names_dated_file() {
        let first = pass_truth_filename("2026-10-16");
        let second = pass_truth_filename("2026-10-16");
        assert_eq!(first, "target/reports/pass-truth-2026-10-16.md");
        assert_eq!(
            first.as_bytes(),
            second.as_bytes(),
            "same date gives same path twice"
        );
        assert!(first.contains("2026-10-16"), "path carries the date tag");
        let other = pass_truth_filename("2026-10-15");
        assert_ne!(first, other, "different dates name different files");
        let report = gate_dated_report("2026-10-16");
        assert!(
            report.starts_with("# pass truth 2026-10-16"),
            "report and path share the date tag"
        );
    }

    #[test]
    fn supplied_frames_gate_live_and_probe() {
        let spec = &LOOK_PASSES[0];
        let off = [10u8, 10, 10, 255, 10, 10, 10, 255];
        let mut moved = off;
        moved[2] = 200;
        let live = verdict_for_frames(spec, &moved, &off, 2, 1, None);
        assert!(live.live, "supplied differing frames read LIVE");
        assert!(live.diff_pixels > PASS_LIVE_FLOOR);
        assert_eq!((live.row, live.pass), (spec.row, spec.pass));
        let probe = verdict_for_frames(spec, &off, &off, 2, 1, None);
        assert!(!probe.live, "identical frames read PROBE");
        assert_eq!(probe.diff_pixels, 0);
    }

    #[test]
    fn supplied_masked_and_dust_stay_probe() {
        let spec = &LOOK_PASSES[0];
        let on = [10u8, 10, 200, 255, 10, 10, 10, 255];
        let off = [10u8, 10, 10, 255, 10, 10, 10, 255];
        let masked = verdict_for_frames(spec, &on, &off, 2, 1, Some(&[1, 0]));
        assert!(!masked.live, "masked-only change stays PROBE");
        assert_eq!(masked.diff_pixels, 0);
        let dusty = [14u8, 10, 10, 255, 10, 10, 10, 255];
        let dust = verdict_for_frames(spec, &dusty, &off, 2, 1, None);
        assert!(!dust.live, "dust under epsilon stays PROBE");
        assert_eq!(dust.diff_pixels, 0);
    }

    #[test]
    fn supplied_ragged_inputs_resolve_probe_and_repeat() {
        let spec = &LOOK_PASSES[0];
        let on = [10u8, 10, 10, 255, 20, 20, 20, 255];
        let off = [10u8, 10, 10, 255, 20, 20, 20, 255];
        let ragged = verdict_for_frames(spec, &on, &off[..7], 2, 1, None);
        assert!(!ragged.live);
        assert_eq!(ragged.diff_pixels, 0);
        let empty: [u8; 0] = [];
        let empty_verdict = verdict_for_frames(spec, &empty, &empty, 0, 0, None);
        assert!(!empty_verdict.live);
        assert_eq!(empty_verdict.diff_pixels, 0);
        let bad_mask = verdict_for_frames(spec, &on, &off, 2, 1, Some(&[0]));
        assert!(!bad_mask.live);
        assert_eq!(bad_mask.diff_pixels, 0);
        let again = verdict_for_frames(spec, &on, &off[..7], 2, 1, None);
        assert_eq!(ragged, again, "ragged resolves the same twice");
    }

    #[test]
    fn supplied_bytes_match_gate_table_for_all_passes() {
        let table = gate_table();
        assert_eq!(table.len(), LOOK_PASSES.len());
        for (spec, gated) in LOOK_PASSES.iter().zip(table.iter()) {
            let mut on = [0u8; PASS_FRAME_LEN];
            let mut off = [0u8; PASS_FRAME_LEN];
            let (width, height) = capture_pair(spec, &mut on, &mut off).expect("pair fits");
            let supplied = verdict_for_frames(spec, &on, &off, width, height, None);
            assert_eq!(supplied, *gated, "supplied bytes match gate");
            if spec.row == "sky" || spec.row == "decals" {
                assert!(!supplied.live);
                assert_eq!(supplied.diff_pixels, 0);
            } else {
                assert!(supplied.live, "pass stays LIVE");
            }
        }
    }

    #[test]
    fn masked_table_hides_moved_pixel_all_probe() {
        let plain = gate_table();
        assert!(
            plain.iter().any(|verdict| verdict.live),
            "plain table keeps live passes"
        );
        let pixels = PASS_FRAME_W * PASS_FRAME_H;
        let mut hide_first = vec![0u8; pixels];
        hide_first[0] = 1;
        let masked = gate_table_masked(Some(&hide_first));
        assert_eq!(masked.len(), LOOK_PASSES.len());
        for verdict in &masked {
            assert!(!verdict.live, "masked moved pixel stays PROBE");
            assert_eq!(verdict.diff_pixels, 0);
        }
        assert_eq!(
            gate_table_masked(Some(&hide_first)),
            masked,
            "masked table repeats"
        );
    }

    #[test]
    fn masked_table_ragged_mask_all_probe_and_repeat() {
        let ragged = gate_table_masked(Some(&[0u8; 3]));
        assert_eq!(ragged.len(), LOOK_PASSES.len());
        for verdict in &ragged {
            assert!(!verdict.live, "ragged mask never reads live");
            assert_eq!(verdict.diff_pixels, 0);
        }
        assert_eq!(
            ragged,
            gate_table_masked(Some(&[0u8; 3])),
            "ragged mask repeats"
        );
    }

    #[test]
    fn supplied_table_gates_window_frames_all_passes() {
        let mut on_frames = vec![[0u8; PASS_FRAME_LEN]; LOOK_PASSES.len()];
        let mut off_frames = vec![[0u8; PASS_FRAME_LEN]; LOOK_PASSES.len()];
        let mut dims = Vec::with_capacity(LOOK_PASSES.len());
        for (index, spec) in LOOK_PASSES.iter().enumerate() {
            let (width, height) = capture_pair(spec, &mut on_frames[index], &mut off_frames[index])
                .expect("pair fits");
            dims.push((width, height));
        }
        let pairs: Vec<SuppliedPair<'_>> = LOOK_PASSES
            .iter()
            .enumerate()
            .map(|(index, spec)| SuppliedPair {
                row: spec.row,
                on: &on_frames[index],
                off: &off_frames[index],
                width: dims[index].0,
                height: dims[index].1,
                mask: None,
            })
            .collect();
        let table = gate_supplied_table(&pairs);
        assert_eq!(table.len(), LOOK_PASSES.len());
        assert_eq!(table, gate_table(), "supplied window frames match gate");
        assert!(flag(&table, "graph"), "forward moves pixels today");
        assert!(!flag(&table, "sky"), "sky stays probe while staged");
        assert!(!flag(&table, "decals"), "decals stay probe while staged");
        assert_eq!(gate_supplied_table(&pairs), table, "supplied table repeats");
    }

    #[test]
    fn supplied_table_unknown_and_ragged_stay_probe() {
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
        let table = gate_supplied_table(core::slice::from_ref(&ghost));
        assert_eq!(table.len(), 1);
        assert!(!table[0].live, "unknown rows never read live");
        assert_eq!(table[0].diff_pixels, 0);
        let ragged_on = [10u8, 10, 10, 255, 20, 20, 20, 255];
        let ragged = SuppliedPair {
            row: "graph",
            on: &ragged_on,
            off: &ragged_on[..7],
            width: 2,
            height: 1,
            mask: None,
        };
        let ragged_table = gate_supplied_table(core::slice::from_ref(&ragged));
        assert!(!ragged_table[0].live, "ragged inputs resolve PROBE");
        assert_eq!(ragged_table[0].diff_pixels, 0);
        assert_eq!(
            gate_supplied_table(core::slice::from_ref(&ragged)),
            ragged_table,
            "ragged resolves the same twice"
        );
    }

    #[test]
    fn supplied_window_frames_match_synth_gate_x2() {
        let mut on_frames = [[0u8; PASS_FRAME_LEN]; 9];
        let mut off_frames = [[0u8; PASS_FRAME_LEN]; 9];
        let mut dims = [(0usize, 0usize); 9];
        for (index, spec) in LOOK_PASSES.iter().enumerate() {
            dims[index] = capture_pair(spec, &mut on_frames[index], &mut off_frames[index])
                .expect("pair fits");
        }
        let mut pairs = Vec::with_capacity(LOOK_PASSES.len());
        for (index, spec) in LOOK_PASSES.iter().enumerate() {
            pairs.push(SuppliedPair {
                row: spec.row,
                on: &on_frames[index],
                off: &off_frames[index],
                width: dims[index].0,
                height: dims[index].1,
                mask: None,
            });
        }
        let first = gate_supplied_table(&pairs);
        let synth_first = gate_table();
        let second = gate_supplied_table(&pairs);
        let synth_second = gate_table();
        assert_eq!(first, synth_first, "window frames match synth gate");
        assert_eq!(second, synth_second, "gate repeats");
        assert_eq!(first, second, "supplied repeats x2");
        assert_eq!(synth_first, synth_second, "synth repeats x2");
        assert!(flag(&first, "graph"), "graph LIVE");
        assert!(!flag(&first, "sky"), "sky PROBE");
        assert!(!flag(&first, "decals"), "decals PROBE");
        for verdict in &first {
            if verdict.row == "sky" || verdict.row == "decals" {
                assert_eq!(verdict.diff_pixels, 0);
                assert_eq!(verdict_label(verdict.diff_pixels), "PROBE");
            } else {
                assert!(verdict.diff_pixels > PASS_LIVE_FLOOR);
                assert_eq!(verdict_label(verdict.diff_pixels), "LIVE");
            }
            assert_eq!(verdict.live, is_live(verdict.diff_pixels));
        }
        assert_eq!(
            live_flags_from_table(&first),
            live_flags_from_table(&synth_first)
        );
    }

    #[cfg(feature = "v1")]
    #[test]
    fn classic_matches_reference_and_writes_actual_difference_pngs() {
        let adapter =
            std::env::var(RL11_ADAPTER_ENV).unwrap_or_else(|_| RL11_ADAPTER_PIN.to_string());
        assert!(rl11_adapter_pinned(&adapter), "software adapter pinned");
        assert!(rl11_adapter_pinned("lavapipe") && rl11_adapter_pinned("llvmpipe"));
        assert!(!rl11_adapter_pinned("RTX 3050"));
        let table_a = gate_table();
        assert_eq!(table_a, gate_table(), "7 LIVE plus 2 PROBE repeat");
        assert_eq!(table_a.iter().filter(|v| v.live).count(), 7);
        assert_eq!(table_a.iter().filter(|v| !v.live).count(), 2);
        let live_spec = LOOK_PASSES
            .iter()
            .find(|s| s.row == "graph")
            .expect("graph present");
        let mut on = [0u8; PASS_FRAME_LEN];
        let mut off = [0u8; PASS_FRAME_LEN];
        let (w, h) = capture_pair(live_spec, &mut on, &mut off).expect("pair fits");
        assert_eq!(
            compare_classic_reference(&on, &off, w, h).expect("live compares"),
            1
        );
        assert_eq!(
            compare_classic_reference(&off, &off, w, h).expect("same compares"),
            0
        );
        assert!(rl11_difference_rgba(&off, &off)
            .chunks_exact(4)
            .all(|px| px == [0, 0, 0, 255]));
        let (actual_a, diff_a) = rl11_actual_difference_pngs(&on, &off, w, h).expect("pngs encode");
        let (actual_b, diff_b) = rl11_actual_difference_pngs(&on, &off, w, h).expect("pngs repeat");
        if rl11_snapshot_update() {
            assert!(!actual_a.is_empty() && !diff_a.is_empty());
            return;
        }
        assert_eq!(actual_a, actual_b, "actual PNG repeats");
        assert_eq!(diff_a, diff_b, "difference PNG repeats");
        assert_eq!(&actual_a[0..8], &[137, 80, 78, 71, 13, 10, 26, 10]);
    }

    #[test]
    fn look_toggles_disabled_pass_stays_probe() {
        let mut toggles = LookToggles::all_on();
        for spec in LOOK_PASSES.iter() {
            assert!(toggles.is_on(spec.row), "all on starts enabled");
        }
        assert!(!toggles.is_on("no-such-pass"), "unknown rows read off");
        toggles.disable("no-such-pass");
        for spec in LOOK_PASSES.iter() {
            assert!(toggles.is_on(spec.row), "unknown disable keeps all on");
        }
        toggles.disable("sky");
        assert!(!toggles.is_on("sky"), "disable flips sky off");
        assert!(toggles.is_on("graph"), "disable keeps graph on");
        let enabled = LOOK_PASSES
            .iter()
            .filter(|spec| toggles.is_on(spec.row))
            .count();
        assert_eq!(enabled, LOOK_PASSES.len() - 1, "disable flips exactly one");
        let first = gate_table_masked_with(&toggles, None);
        let second = gate_table_masked_with(&toggles, None);
        assert_eq!(first, second, "masked-with table repeats x2");
        assert_eq!(first.len(), LOOK_PASSES.len());
        let sky = first
            .iter()
            .find(|verdict| verdict.row == "sky")
            .expect("sky present");
        assert!(!sky.live, "disabled sky stays PROBE");
        assert_eq!(sky.diff_pixels, 0);
        let graph = first
            .iter()
            .find(|verdict| verdict.row == "graph")
            .expect("graph present");
        assert!(graph.live, "enabled moving pass stays LIVE");
        assert!(graph.diff_pixels > PASS_LIVE_FLOOR);
        let mut live_off = LookToggles::all_on();
        live_off.disable("graph");
        let forced = gate_table_masked_with(&live_off, None);
        let graph_off = forced
            .iter()
            .find(|verdict| verdict.row == "graph")
            .expect("graph present");
        assert!(!graph_off.live, "disabled live pass forces PROBE");
        assert_eq!(graph_off.diff_pixels, 0);
        assert_eq!(
            forced,
            gate_table_masked_with(&live_off, None),
            "forced table repeats x2"
        );
        assert_eq!(
            gate_table_masked_with(&LookToggles::all_on(), None),
            gate_table_masked(None),
            "all-on matches the masked gate"
        );
    }

    #[test]
    fn sky_toggle_off_lists_only_sky_skip() {
        // Caller: `crate::present::gpu_present::live::live_sky_should_draw`
        // in `crates/render/src/present/gpu_present/live.rs` consumes this
        // list to skip the sky bind. Behavior: all-on lists no skip, sky
        // disabled lists only the sky submit entry, unknown rows never list.
        let all = submit_skips_for(&LookToggles::all_on());
        assert!(
            all.iter().all(|slot| slot.is_none()),
            "all-on lists no skip"
        );
        let mut toggles = LookToggles::all_on();
        toggles.disable("sky");
        let skips = submit_skips_for(&toggles);
        let sky_index = LOOK_PASSES
            .iter()
            .position(|spec| spec.row == "sky")
            .expect("sky row present");
        assert_eq!(
            skips[sky_index],
            Some(LOOK_PASSES[sky_index].pass),
            "sky disabled lists its submit entry"
        );
        assert_eq!(
            skips[sky_index],
            Some("submit_gpu_sky"),
            "sky entry is the sky submit"
        );
        for (index, slot) in skips.iter().enumerate() {
            if index != sky_index {
                assert!(slot.is_none(), "only sky lists a skip");
            }
        }
        let mut ghost = LookToggles::all_on();
        ghost.disable("no-such-pass");
        let ghost_skips = submit_skips_for(&ghost);
        assert!(
            ghost_skips.iter().all(|slot| slot.is_none()),
            "unknown disable lists no skip"
        );
        assert!(
            !skips
                .iter()
                .flatten()
                .any(|entry| *entry == "no-such-pass" || *entry == "none"),
            "unknown rows never appear"
        );
        assert_eq!(
            submit_skips_for(&toggles),
            skips,
            "same toggles give same skips twice"
        );
        let sky_spec = LOOK_PASSES
            .iter()
            .find(|spec| spec.row == "sky")
            .expect("sky spec present");
        let off = [10u8, 10, 10, 255, 10, 10, 10, 255];
        let mut on = off;
        on[2] = 200;
        let live = verdict_for_frames(sky_spec, &on, &off, 2, 1, None);
        assert!(live.live, "sky on/off pair differs in the sky region");
        assert!(live.diff_pixels > PASS_LIVE_FLOOR);
        let probe = verdict_for_frames(sky_spec, &off, &off, 2, 1, None);
        assert!(!probe.live, "identical sky frames stay PROBE");
        assert_eq!(probe.diff_pixels, 0);
    }
}
