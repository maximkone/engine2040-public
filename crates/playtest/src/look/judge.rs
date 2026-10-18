//! look.judge core: score a capture set by machine, from pixels.
//!
//! Consumer: the `look.judge` door verb and any Look gate that must accept on
//! a number instead of an opened PNG. Data owner: the capture path owns the
//! frames and the manifest; this module borrows bytes and returns one typed
//! [`LookJudgement`]. Rubric: nine items scored in permille (`0..=1000`),
//! each with a pass bar, so a Look row accepts on a number.
//!
//! | id | name | pixel evidence |
//! |---|---|---|
//! | L1 | `fighters_present` | fighter-sized foreground silhouettes (two required) |
//! | L2 | `scene_structure` | foreground that is not a fighter (kit, stands, walls) |
//! | L3 | `ground_texture` | luma variance over the ground colour |
//! | L4 | `real_capture` | not a known placeholder, not the 64x48 class, not noise |
//! | L5 | `frame_clean` | `frame_check` finds no black, white or empty frame |
//! | L6 | `tonal_range` | occupied bins of the 64-bin luma histogram |
//! | L7 | `fighter_palette` | fighter pixels vs the fighter preview renders |
//! | L8 | `q1_reference` | full-frame SSIM vs the paired Q1 reference frame |
//! | L9 | `hero_shots_pass` | share of frames passing every applicable item |
//!
//! Fighters-present never reads a manifest count. The manifest only grounds
//! regions on entity ids and checks the filed PNG digest; a manifest that
//! claims fighters over a blank frame scores L1 zero. A silhouette is a
//! 4-connected blob of pixels that differs from the frame's dominant ground
//! colour by CIE76 deltaE, with an area band, a bounding-box band and no
//! error-mesh magenta. Limit: pixels alone cannot tell a small neutral cube
//! from a fighter silhouette, so L7 (palette against the previews) is the
//! second witness and L1 is calibrated for arena-wide shots, not close-ups
//! (raise `max_blob_permille` for those).
//!
//! Units: permille for scores, hundredths of a deltaE unit, SSIM x10000,
//! pixels, millimetres in manifests. Determinism: integer only (see
//! [`super::metric`]); the report digest covers every byte of the JSON body,
//! so the same capture set gives byte-identical JSON twice. Errors:
//! [`JudgeError`] for a set that cannot be judged at all; a single bad frame
//! is a scored fault inside the report, never a panic.
//! Budget: at most [`MAX_FRAMES`] frames of at most [`MAX_PNG_BYTES`] each;
//! one frame is held decoded beside its predecessor at a time.

use std::fmt;
use std::path::Path;

use super::manifest::{parse_manifest, Manifest, ManifestError, ManifestFigure, ManifestFrame};
use super::metric::{self, Comparison, Frame, Histogram, Region, SSIM_ONE};
use crate::frame_check::{
    check_digest_window, check_frame, digest_hex, CapturedFrame, FrameFault, Thresholds,
};

/// Report schema version, first field of the JSON.
pub const SCHEMA: u32 = 1;
/// Rubric items, L1 through L9.
pub const RUBRIC_ITEMS: usize = 9;
/// Per-frame items, L1 through L8 (L9 is set level).
const FRAME_ITEMS: usize = 8;
/// Most frames judged in one set.
pub const MAX_FRAMES: usize = 64;
/// Most reference images read per pool.
pub const MAX_REFERENCES: usize = 32;
/// Largest PNG accepted, bytes (the capture ceiling).
pub const MAX_PNG_BYTES: usize = 4 * 1024 * 1024;
/// Largest manifest accepted, bytes.
pub const MAX_MANIFEST_BYTES: usize = 1 << 20;
/// Manifest file name inside a capture directory.
pub const MANIFEST_NAME: &str = "manifest.json";
/// Staged-fight frames map the arena top-down: half extent in millimetres.
const STAGED_HALF_EXTENT_MM: i64 = 20_000;
/// Staged-fight frames keep this many pixels clear on every edge.
const STAGED_MARGIN_PX: i64 = 4;
/// Pixels a manifest fighter's projected point may sit outside a blob.
const GROUND_SLACK_PX: usize = 4;

/// `(id, name, unit of the raw evidence)` for L1 through L9.
const RUBRIC: [(&str, &str, &str); RUBRIC_ITEMS] = [
    ("L1", "fighters_present", "fighter silhouettes"),
    ("L2", "scene_structure", "structure permille of frame"),
    ("L3", "ground_texture", "ground luma variance"),
    ("L4", "real_capture", "placeholder code, 0 is real"),
    ("L5", "frame_clean", "frame faults"),
    ("L6", "tonal_range", "occupied luma bins"),
    ("L7", "fighter_palette", "best histogram EMD x10000"),
    ("L8", "q1_reference", "SSIM x10000"),
    ("L9", "hero_shots_pass", "frames passing all"),
];

/// Why a capture set could not be judged at all.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum JudgeError {
    /// The set holds no frame.
    NoFrames,
    /// More than [`MAX_FRAMES`] frames, or more than [`MAX_REFERENCES`]
    /// references in one pool.
    TooMany {
        /// Which pool was over.
        what: &'static str,
        /// Count offered.
        count: usize,
    },
    /// A PNG or manifest is over its byte bound.
    TooLarge {
        /// File name.
        name: String,
        /// Bytes offered.
        bytes: usize,
    },
    /// The manifest is present and unreadable (garbled or the wrong shape).
    BadManifest(ManifestError),
    /// A directory or file could not be read.
    Io {
        /// Path (lossy; for diagnostics).
        path: String,
        /// IO failure kind.
        kind: std::io::ErrorKind,
    },
}

impl fmt::Display for JudgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoFrames => write!(f, "capture set holds no frame: point at a directory with frame PNGs"),
            Self::TooMany { what, count } => {
                write!(f, "{count} {what} offered; at most {MAX_FRAMES} frames and {MAX_REFERENCES} references")
            }
            Self::TooLarge { name, bytes } => write!(
                f,
                "{name} is {bytes} bytes; at most {MAX_PNG_BYTES} per PNG and {MAX_MANIFEST_BYTES} per manifest"
            ),
            Self::BadManifest(error) => write!(f, "{error}; fix or remove manifest.json and judge again"),
            Self::Io { path, kind } => write!(f, "cannot read {path} ({kind:?})"),
        }
    }
}

impl std::error::Error for JudgeError {}

/// One PNG with the name it travels under.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct NamedPng {
    /// File name (no directory).
    pub name: String,
    /// PNG bytes.
    pub png: Vec<u8>,
}

/// A capture set: frames in order, plus the manifest text when present.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct CaptureSet {
    /// Frames in capture order.
    pub frames: Vec<NamedPng>,
    /// `manifest.json` text, when the capture wrote one.
    pub manifest: Option<String>,
}

/// Reference images: fighter preview renders and Q1 arena frames.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct References {
    /// Fighter preview renders (an isolated fighter on a flat ground).
    pub fighters: Vec<NamedPng>,
    /// Q1 reference frames, paired with capture frames by position.
    pub q1: Vec<NamedPng>,
}

/// Judge thresholds. The defaults suit arena-wide shots of two fighters.
#[derive(Clone, PartialEq, Debug)]
pub struct JudgeConfig {
    /// Silhouettes that earn full marks on L1.
    pub required_fighters: u32,
    /// Smallest CIE76 deltaE (hundredths) from the ground that is foreground.
    pub foreground_de_x100: u32,
    /// Same, for the ground of a fighter preview render.
    pub reference_de_x100: u32,
    /// Smallest silhouette in pixels, whatever the frame size.
    pub min_blob_px: u32,
    /// Smallest silhouette as permille of the frame area.
    pub min_blob_permille: u32,
    /// Largest silhouette as permille of the frame area.
    pub max_blob_permille: u32,
    /// Widest silhouette bounding box as permille of the frame width.
    pub max_blob_width_permille: u32,
    /// Tallest silhouette bounding box as permille of the frame height.
    pub max_blob_height_permille: u32,
    /// Ground luma variance that earns full marks on L3.
    pub texture_target_variance: u32,
    /// Structure permille of the frame that earns full marks on L2.
    pub structure_target_permille: u32,
    /// Occupied luma bins that earn full marks on L6.
    pub tonal_target_bins: u32,
    /// Share of pixels (permille) a luma bin needs to count as occupied.
    pub tonal_bin_floor_permille: u32,
    /// Frames at or under this pixel count are the 64x48 placeholder class.
    pub placeholder_max_pixels: u32,
    /// Luma step between neighbours that counts as a noise edge.
    pub noise_step: u32,
    /// Noise edges (permille of neighbour pairs) that mark a noise frame.
    pub noise_max_permille: u32,
    /// Per-frame item score (permille) that passes the item.
    pub pass_bar_permille: u32,
    /// Share of frames (permille) that must pass an item (and L9).
    pub hero_pass_permille: u32,
    /// PNG digests ([`digest_hex`]) of known cube or noise captures.
    pub refused_png_fnv: Vec<String>,
    /// Thresholds handed to `frame_check`.
    pub frame_thresholds: Thresholds,
}

impl Default for JudgeConfig {
    fn default() -> Self {
        Self {
            required_fighters: 2,
            foreground_de_x100: 2000,
            reference_de_x100: 800,
            min_blob_px: 16,
            min_blob_permille: 1,
            max_blob_permille: 200,
            max_blob_width_permille: 600,
            max_blob_height_permille: 900,
            texture_target_variance: 25,
            structure_target_permille: 100,
            tonal_target_bins: 12,
            tonal_bin_floor_permille: 5,
            placeholder_max_pixels: 64 * 48,
            noise_step: 32,
            noise_max_permille: 400,
            pass_bar_permille: 700,
            hero_pass_permille: 800,
            refused_png_fnv: Vec::new(),
            frame_thresholds: Thresholds::default(),
        }
    }
}

/// One scored rubric item over the whole set.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RubricItem {
    /// `L1` through `L9`.
    pub id: &'static str,
    /// Item name.
    pub name: &'static str,
    /// False when no frame could be scored on this item (no references).
    pub applicable: bool,
    /// Mean permille over the frames scored, `0..=1000`.
    pub score_permille: u32,
    /// True when enough frames pass this item (see `hero_pass_permille`).
    pub pass: bool,
    /// Frames that pass the item.
    pub frames_passing: u32,
    /// Raw per-frame evidence in text, unit named.
    pub detail: String,
}

/// One fighter silhouette found in the pixels, grounded on an entity.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FigureRegion {
    /// Manifest slot the silhouette was grounded on, or none.
    pub entity: Option<u32>,
    /// Left edge of the bounding box.
    pub x: u32,
    /// Top edge of the bounding box.
    pub y: u32,
    /// Bounding box width.
    pub w: u32,
    /// Bounding box height.
    pub h: u32,
    /// Foreground pixels in the silhouette.
    pub pixels: u32,
}

/// Fighter evidence for one frame.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct FigureEvidence {
    /// Silhouettes found in the pixels.
    pub count: u32,
    /// Foreground pixels across them.
    pub pixels: u32,
    /// The silhouettes, in raster order of their first pixel.
    pub regions: Vec<FigureRegion>,
    /// Slots the manifest lists (producer-reported; never scored).
    pub manifest_slots: Vec<u32>,
    /// Listed slots no silhouette was found for.
    pub manifest_unseen: Vec<u32>,
}

/// Best fighter-preview match for one frame.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PreviewMatch {
    /// Preview render the fighter pixels matched best.
    pub name: String,
    /// Luma histogram earth mover's distance, `0` identical.
    pub emd_x10000: u32,
    /// CIE76 deltaE between the mean fighter colours, hundredths.
    pub mean_de_x100: u32,
}

/// Comparison with the paired Q1 reference frame.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Q1Match {
    /// Reference name.
    pub name: String,
    /// Metrics, or none when the sizes differ.
    pub comparison: Option<Comparison>,
}

/// Everything the judge found about one frame.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FrameReport {
    /// File name.
    pub name: String,
    /// FNV-1a hex over the PNG bytes (the manifest's `png_fnv`).
    pub png_fnv: String,
    /// FNV-1a hex over the decoded pixels; empty when the PNG did not decode.
    pub pixel_fnv: String,
    /// Width in pixels (`0` when the PNG did not decode).
    pub width: u32,
    /// Height in pixels (`0` when the PNG did not decode).
    pub height: u32,
    /// Refusal or fault in words, none for a clean frame.
    pub fault: Option<String>,
    /// Manifest tick for this frame, when a row binds.
    pub manifest_tick: Option<u64>,
    /// Whether the manifest's `png_fnv` equals the PNG bytes' digest.
    pub manifest_png_ok: Option<bool>,
    /// Per-frame L1..L8 permille; none when the item did not apply.
    pub scores: [Option<u32>; FRAME_ITEMS],
    /// Raw measurement behind each score.
    pub raw: [Option<i64>; FRAME_ITEMS],
    /// Fighter silhouettes and their grounding.
    pub fighters: FigureEvidence,
    /// Best fighter-preview match, when previews were supplied.
    pub preview: Option<PreviewMatch>,
    /// Q1 comparison, when a reference pairs with this frame.
    pub q1: Option<Q1Match>,
}

/// Sequence-level findings over the whole set.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Sequence {
    /// Frames in the set.
    pub frames: u32,
    /// Distinct pixel digests among them.
    pub distinct: u32,
    /// True when three or more consecutive frames share one pixel digest.
    pub frozen: bool,
    /// SSIM between each frame and its predecessor (none across a size
    /// change or an undecodable frame); one entry per frame after the first.
    pub motion_ssim_x10000: Vec<Option<i32>>,
}

/// The typed result of `look.judge`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LookJudgement {
    /// FNV-1a hex over the JSON body (everything after the digest field).
    pub digest: String,
    /// True when every applicable rubric item passes, the manifest binds
    /// every frame and the sequence is not frozen.
    pub pass: bool,
    /// Mean score over the applicable rubric items, permille.
    pub score_permille: u32,
    /// False when a manifest row disagrees with its PNG or the counts differ.
    pub integrity_ok: bool,
    /// L1 through L9.
    pub rubric: Vec<RubricItem>,
    /// Sequence findings.
    pub sequence: Sequence,
    /// One report per frame, in set order.
    pub frames: Vec<FrameReport>,
    /// Plain-language notes (skipped references, manifest disagreements).
    pub notes: Vec<String>,
}

impl LookJudgement {
    /// The rubric item with this id (`L1`..`L9`).
    pub fn item(&self, id: &str) -> Option<&RubricItem> {
        self.rubric.iter().find(|item| item.id == id)
    }
}

// ---------------------------------------------------------------------------
// Pixel analysis: ground, foreground, silhouettes
// ---------------------------------------------------------------------------

fn u32_of(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

fn permille(part: u64, whole: u64) -> u32 {
    if whole == 0 {
        return 0;
    }
    u32::try_from((part * 1000 + whole / 2) / whole).unwrap_or(u32::MAX)
}

/// Dominant colour: mean RGB of the most populated 5-bit-per-channel cell
/// (lowest cell wins a tie).
fn dominant_colour(frame: &Frame) -> [u8; 3] {
    let cell = |px: &[u8]| {
        (usize::from(px[0] >> 3) << 10) | (usize::from(px[1] >> 3) << 5) | usize::from(px[2] >> 3)
    };
    let mut counts = vec![0u32; 1 << 15];
    for px in frame.rgba().chunks_exact(4) {
        counts[cell(px)] += 1;
    }
    let mut best = 0usize;
    for (index, &count) in counts.iter().enumerate() {
        if count > counts[best] {
            best = index;
        }
    }
    let (mut sum, mut n) = ([0u64; 3], 0u64);
    for px in frame.rgba().chunks_exact(4).filter(|px| cell(px) == best) {
        for (acc, &c) in sum.iter_mut().zip(px) {
            *acc += u64::from(c);
        }
        n += 1;
    }
    sum.map(|s| u8::try_from(s / n.max(1)).unwrap_or(u8::MAX))
}

/// Per-pixel flag: differs from `ground` by at least `min_de_x100`.
fn foreground_mask(frame: &Frame, ground: [u8; 3], min_de_x100: u32) -> Vec<bool> {
    let ground_lab = metric::lab_x100(ground);
    let mut memo: Option<([u8; 3], bool)> = None;
    frame
        .rgba()
        .chunks_exact(4)
        .map(|px| {
            let rgb = [px[0], px[1], px[2]];
            if let Some((seen, flag)) = memo {
                if seen == rgb {
                    return flag;
                }
            }
            let flag = metric::lab_distance_x100(ground_lab, metric::lab_x100(rgb)) >= min_de_x100;
            memo = Some((rgb, flag));
            flag
        })
        .collect()
}

/// One 4-connected foreground component.
struct Blob {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    sum_rgb: [u64; 3],
    pixels: Vec<usize>,
}

impl Blob {
    fn width(&self) -> usize {
        self.x1 - self.x0 + 1
    }

    fn height(&self) -> usize {
        self.y1 - self.y0 + 1
    }

    fn mean_rgb(&self) -> [u8; 3] {
        let n = u64::try_from(self.pixels.len()).unwrap_or(1).max(1);
        self.sum_rgb.map(|s| u8::try_from(s / n).unwrap_or(u8::MAX))
    }
}

/// Label foreground components in raster order of their first pixel.
fn find_blobs(frame: &Frame, fg: &[bool]) -> Vec<Blob> {
    let (w, h) = (usize_of(frame.width()), usize_of(frame.height()));
    let mut seen = vec![false; fg.len()];
    let mut blobs = Vec::new();
    let mut stack = Vec::new();
    for start in 0..fg.len() {
        if !fg[start] || seen[start] {
            continue;
        }
        seen[start] = true;
        stack.push(start);
        let mut blob = Blob {
            x0: start % w,
            y0: start / w,
            x1: start % w,
            y1: start / w,
            sum_rgb: [0; 3],
            pixels: Vec::new(),
        };
        while let Some(index) = stack.pop() {
            let (x, y) = (index % w, index / w);
            blob.x0 = blob.x0.min(x);
            blob.x1 = blob.x1.max(x);
            blob.y0 = blob.y0.min(y);
            blob.y1 = blob.y1.max(y);
            for (acc, c) in blob.sum_rgb.iter_mut().zip(frame.rgb_at(index)) {
                *acc += u64::from(c);
            }
            blob.pixels.push(index);
            let around = [
                (x > 0).then(|| index - 1),
                (x + 1 < w).then(|| index + 1),
                (y > 0).then(|| index - w),
                (y + 1 < h).then(|| index + w),
            ];
            for next in around.into_iter().flatten() {
                if fg[next] && !seen[next] {
                    seen[next] = true;
                    stack.push(next);
                }
            }
        }
        blobs.push(blob);
    }
    blobs
}

fn usize_of(value: u32) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}

/// Error-mesh magenta: the colour the arena draws a missing asset in.
fn is_error_magenta(rgb: [u8; 3]) -> bool {
    rgb[0] >= 150 && rgb[2] >= 150 && rgb[1] <= 100
}

/// True for a silhouette inside the fighter bands.
fn is_fighter_sized(blob: &Blob, frame: &Frame, cfg: &JudgeConfig) -> bool {
    let area = u64::try_from(blob.pixels.len()).unwrap_or(u64::MAX);
    let frame_px = u64::try_from(frame.pixels()).unwrap_or(u64::MAX);
    let floor = u64::from(cfg.min_blob_px).max(frame_px * u64::from(cfg.min_blob_permille) / 1000);
    let ceiling = frame_px * u64::from(cfg.max_blob_permille) / 1000;
    let wide = u64::try_from(blob.width()).unwrap_or(u64::MAX) * 1000
        <= u64::from(frame.width()) * u64::from(cfg.max_blob_width_permille);
    let tall = u64::try_from(blob.height()).unwrap_or(u64::MAX) * 1000
        <= u64::from(frame.height()) * u64::from(cfg.max_blob_height_permille);
    area >= floor && area <= ceiling && wide && tall && !is_error_magenta(blob.mean_rgb())
}

/// Luma variance over pixels not flagged foreground (population variance,
/// integer); zero when under 5 percent of the frame is ground.
fn ground_variance(frame: &Frame, fg: &[bool]) -> u64 {
    let (mut n, mut sum, mut sum_sq) = (0u128, 0u128, 0u128);
    for (index, _) in fg.iter().enumerate().filter(|(_, &flag)| !flag) {
        let l = u128::from(metric::luma_of(frame.rgb_at(index)));
        n += 1;
        sum += l;
        sum_sq += l * l;
    }
    let frame_px = u128::try_from(frame.pixels()).unwrap_or(u128::MAX);
    if n == 0 || n * 20 < frame_px {
        return 0;
    }
    u64::try_from((n * sum_sq - sum * sum) / (n * n)).unwrap_or(u64::MAX)
}

/// Fraction of horizontal neighbour pairs whose luma differs by `step` or
/// more, in permille: a noise frame sits far above any rendered scene.
fn noise_permille(luma: &[u8], width: usize, step: u32) -> u32 {
    let (mut edges, mut pairs) = (0u64, 0u64);
    for row in luma.chunks_exact(width) {
        for pair in row.windows(2) {
            pairs += 1;
            if u32::from(pair[0].abs_diff(pair[1])) >= step {
                edges += 1;
            }
        }
    }
    permille(edges, pairs)
}

// ---------------------------------------------------------------------------
// Entity grounding
// ---------------------------------------------------------------------------

/// Project one arena axis (millimetres) to a pixel the way the staged-fight
/// frames do: top-down, linear, clamped inside a four-pixel margin.
fn project_axis(value_mm: i64, pixels: usize) -> i64 {
    let inner = i64::try_from(pixels).unwrap_or(0) - 2 * STAGED_MARGIN_PX;
    let shifted = (value_mm + STAGED_HALF_EXTENT_MM).clamp(0, 2 * STAGED_HALF_EXTENT_MM);
    STAGED_MARGIN_PX + shifted * inner.max(0) / (2 * STAGED_HALF_EXTENT_MM)
}

/// Ground each silhouette on the nearest unclaimed manifest fighter whose
/// projected point lies inside the box (plus a small slack). Returns the
/// regions plus the manifest slots no silhouette claimed.
fn ground_regions(
    frame: &Frame,
    blobs: &[&Blob],
    listed: &[ManifestFigure],
) -> (Vec<FigureRegion>, Vec<u32>) {
    let (w, h) = (usize_of(frame.width()), usize_of(frame.height()));
    let points: Vec<(u32, i64, i64)> = listed
        .iter()
        .map(|f| (f.slot, project_axis(f.x_mm, w), project_axis(f.z_mm, h)))
        .collect();
    let mut claimed = vec![false; points.len()];
    let mut regions = Vec::new();
    for blob in blobs {
        let (cx, cy) = (
            i64::try_from(blob.x0 + blob.x1).unwrap_or(0) / 2,
            i64::try_from(blob.y0 + blob.y1).unwrap_or(0) / 2,
        );
        let slack = i64::try_from(GROUND_SLACK_PX).unwrap_or(0);
        let inside = |px: i64, py: i64| {
            px >= i64::try_from(blob.x0).unwrap_or(0) - slack
                && px <= i64::try_from(blob.x1).unwrap_or(0) + slack
                && py >= i64::try_from(blob.y0).unwrap_or(0) - slack
                && py <= i64::try_from(blob.y1).unwrap_or(0) + slack
        };
        let pick = points
            .iter()
            .enumerate()
            .filter(|(i, (_, px, py))| !claimed[*i] && inside(*px, *py))
            .min_by_key(|(_, (slot, px, py))| ((px - cx).pow(2) + (py - cy).pow(2), *slot))
            .map(|(i, (slot, _, _))| (i, *slot));
        if let Some((i, _)) = pick {
            claimed[i] = true;
        }
        regions.push(FigureRegion {
            entity: pick.map(|(_, slot)| slot),
            x: u32_of(blob.x0),
            y: u32_of(blob.y0),
            w: u32_of(blob.width()),
            h: u32_of(blob.height()),
            pixels: u32_of(blob.pixels.len()),
        });
    }
    let mut unseen: Vec<u32> = points
        .iter()
        .zip(&claimed)
        .filter(|(_, &c)| !c)
        .map(|((slot, _, _), _)| *slot)
        .collect();
    unseen.sort_unstable();
    (regions, unseen)
}

// ---------------------------------------------------------------------------
// References
// ---------------------------------------------------------------------------

/// Foreground luma histogram and mean colour of one fighter preview render.
struct PreviewRef {
    name: String,
    hist: Histogram,
    mean_rgb: [u8; 3],
}

fn mean_colour(frame: &Frame, indices: impl Iterator<Item = usize>) -> [u8; 3] {
    let (mut sum, mut n) = ([0u64; 3], 0u64);
    for index in indices {
        for (acc, c) in sum.iter_mut().zip(frame.rgb_at(index)) {
            *acc += u64::from(c);
        }
        n += 1;
    }
    sum.map(|s| u8::try_from(s / n.max(1)).unwrap_or(u8::MAX))
}

fn load_previews(list: &[NamedPng], cfg: &JudgeConfig, notes: &mut Vec<String>) -> Vec<PreviewRef> {
    let mut out = Vec::new();
    for item in list {
        let frame = match Frame::decode_png(&item.png) {
            Ok(frame) => frame,
            Err(error) => {
                notes.push(format!("fighter reference {} skipped: {error}", item.name));
                continue;
            }
        };
        let fg = foreground_mask(&frame, dominant_colour(&frame), cfg.reference_de_x100);
        let Ok(hist) = metric::luma_histogram(&frame, Region::Mask(&fg)) else {
            notes.push(format!(
                "fighter reference {} skipped: no foreground",
                item.name
            ));
            continue;
        };
        let mean_rgb = mean_colour(&frame, (0..fg.len()).filter(|&i| fg[i]));
        out.push(PreviewRef {
            name: item.name.clone(),
            hist,
            mean_rgb,
        });
    }
    out
}

fn load_q1(list: &[NamedPng], notes: &mut Vec<String>) -> Vec<Option<(String, Frame)>> {
    list.iter()
        .map(|item| match Frame::decode_png(&item.png) {
            Ok(frame) => Some((item.name.clone(), frame)),
            Err(error) => {
                notes.push(format!("q1 reference {} skipped: {error}", item.name));
                None
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Per-frame scoring
// ---------------------------------------------------------------------------

struct Context<'a> {
    cfg: &'a JudgeConfig,
    previews: Vec<PreviewRef>,
    q1: Vec<Option<(String, Frame)>>,
}

fn scaled(value: u64, target: u64) -> u32 {
    if target == 0 {
        return 1000;
    }
    u32::try_from((value * 1000 / target).min(1000)).unwrap_or(1000)
}

fn empty_report(name: &str, png: &[u8]) -> FrameReport {
    FrameReport {
        name: name.to_owned(),
        png_fnv: digest_hex(png),
        pixel_fnv: String::new(),
        width: 0,
        height: 0,
        fault: None,
        manifest_tick: None,
        manifest_png_ok: None,
        scores: [None; FRAME_ITEMS],
        raw: [None; FRAME_ITEMS],
        fighters: FigureEvidence::default(),
        preview: None,
        q1: None,
    }
}

fn set_item(report: &mut FrameReport, item: usize, score: u32, raw: i64) {
    report.scores[item] = Some(score);
    report.raw[item] = Some(raw);
}

/// Why a frame is not a real capture, as a code (0 is real) plus words.
fn placeholder_verdict(frame: &Frame, png_fnv: &str, cfg: &JudgeConfig) -> (i64, Option<String>) {
    if cfg.refused_png_fnv.iter().any(|known| known == png_fnv) {
        return (1, Some("known placeholder capture digest".to_owned()));
    }
    if u32_of(frame.pixels()) <= cfg.placeholder_max_pixels {
        return (
            2,
            Some(format!(
                "{}x{} is the placeholder size class",
                frame.width(),
                frame.height()
            )),
        );
    }
    let noise = noise_permille(&frame.luma_plane(), usize_of(frame.width()), cfg.noise_step);
    if noise > cfg.noise_max_permille {
        return (
            3,
            Some(format!(
                "noise frame: {noise} permille of neighbours are edges"
            )),
        );
    }
    (0, None)
}

fn frame_fault_words(fault: &FrameFault) -> String {
    match fault {
        FrameFault::BlackScreen(_) => "black_screen",
        FrameFault::WhiteScreen(_) => "white_screen",
        FrameFault::MissingMesh(_) => "missing_mesh",
        FrameFault::Clipping(_) => "clipping",
        FrameFault::Frozen(_) => "frozen",
        FrameFault::BadCapture(_) => "bad_capture",
        // FrameFault grew this variant after this file was deleted; keep the word per variant.
        FrameFault::Stale(_) => "stale",
    }
    .to_owned()
}

/// Score one decoded frame on L1..L8. The manifest row only grounds regions.
fn score_decoded(
    ctx: &Context<'_>,
    index: usize,
    frame: &Frame,
    row: Option<&ManifestFrame>,
    report: &mut FrameReport,
) {
    let cfg = ctx.cfg;
    let frame_px = u64::try_from(frame.pixels()).unwrap_or(u64::MAX);
    let ground = dominant_colour(frame);
    let fg = foreground_mask(frame, ground, cfg.foreground_de_x100);
    let blobs = find_blobs(frame, &fg);
    let fighters: Vec<&Blob> = blobs
        .iter()
        .filter(|b| is_fighter_sized(b, frame, cfg))
        .collect();
    let fighter_px: u64 = fighters
        .iter()
        .map(|b| u64::try_from(b.pixels.len()).unwrap_or(0))
        .sum();
    let fg_px = u64::try_from(fg.iter().filter(|&&f| f).count()).unwrap_or(0);

    // L1: silhouettes, never the manifest.
    let count = u32_of(fighters.len());
    set_item(
        report,
        0,
        scaled(u64::from(count), u64::from(cfg.required_fighters.max(1))),
        i64::from(count),
    );
    let listed = row.map_or(&[][..], |r| r.fighters.as_slice());
    let (regions, unseen) = ground_regions(frame, &fighters, listed);
    report.fighters = FigureEvidence {
        count,
        pixels: u32_of(usize::try_from(fighter_px).unwrap_or(usize::MAX)),
        regions,
        manifest_slots: listed.iter().map(|f| f.slot).collect(),
        manifest_unseen: unseen,
    };

    // L2 structure, L3 ground texture.
    let structure = permille(fg_px.saturating_sub(fighter_px), frame_px);
    set_item(
        report,
        1,
        scaled(
            u64::from(structure),
            u64::from(cfg.structure_target_permille),
        ),
        i64::from(structure),
    );
    let variance = ground_variance(frame, &fg);
    set_item(
        report,
        2,
        scaled(variance, u64::from(cfg.texture_target_variance)),
        i64::try_from(variance).unwrap_or(i64::MAX),
    );

    // L4 real capture, L5 frame clean.
    let (code, why) = placeholder_verdict(frame, &report.png_fnv, cfg);
    set_item(report, 3, if code == 0 { 1000 } else { 0 }, code);
    let captured = CapturedFrame {
        width: frame.width(),
        height: frame.height(),
        rgba: frame.rgba().to_vec(),
        draws: 1,
        expected_draws: 1,
        near_slices: 0,
    };
    let verdict = check_frame(&captured, &cfg.frame_thresholds);
    let clean = verdict.is_ok();
    set_item(report, 4, if clean { 1000 } else { 0 }, i64::from(!clean));
    report.fault = why.or_else(|| verdict.err().map(|fault| frame_fault_words(&fault)));

    // L6 tonal range.
    let bins = metric::luma_histogram(frame, Region::Full)
        .map_or(0, |h| h.occupied_bins(cfg.tonal_bin_floor_permille));
    set_item(
        report,
        5,
        scaled(u64::from(bins), u64::from(cfg.tonal_target_bins)),
        i64::from(bins),
    );

    // L7 fighter palette vs previews, L8 Q1 reference.
    score_palette(ctx, frame, &fighters, report);
    score_q1(ctx, index, frame, report);
}

fn score_palette(ctx: &Context<'_>, frame: &Frame, fighters: &[&Blob], report: &mut FrameReport) {
    if ctx.previews.is_empty() {
        return;
    }
    let mut mask = vec![false; frame.pixels()];
    for blob in fighters {
        for &index in &blob.pixels {
            mask[index] = true;
        }
    }
    let Ok(hist) = metric::luma_histogram(frame, Region::Mask(&mask)) else {
        set_item(report, 6, 0, 10_000);
        return;
    };
    let mean = mean_colour(frame, (0..mask.len()).filter(|&i| mask[i]));
    let best = ctx
        .previews
        .iter()
        .filter_map(|p| {
            let d = metric::histogram_distance(&hist, &p.hist).ok()?;
            Some((d.emd_x10000, p))
        })
        .min_by(|a, b| (a.0, &a.1.name).cmp(&(b.0, &b.1.name)));
    if let Some((emd, preview)) = best {
        set_item(report, 6, 1000 - emd.min(10_000) / 10, i64::from(emd));
        report.preview = Some(PreviewMatch {
            name: preview.name.clone(),
            emd_x10000: emd,
            mean_de_x100: metric::delta_e_x100(mean, preview.mean_rgb),
        });
    }
}

fn score_q1(ctx: &Context<'_>, index: usize, frame: &Frame, report: &mut FrameReport) {
    let Some(Some((name, reference))) = ctx.q1.get(index) else {
        return;
    };
    let comparison = metric::compare(frame, reference, Region::Full);
    let (score, raw) = match &comparison {
        Ok(c) => (
            u32::try_from(c.ssim_x10000.max(0) / 10).unwrap_or(0),
            i64::from(c.ssim_x10000),
        ),
        Err(_) => (0, i64::from(-SSIM_ONE)),
    };
    set_item(report, 7, score, raw);
    report.q1 = Some(Q1Match {
        name: name.clone(),
        comparison: comparison.ok(),
    });
}

/// One frame: decode, bind its manifest row, score it. Returns the decoded
/// frame so the caller can compare neighbours.
fn judge_frame(
    ctx: &Context<'_>,
    index: usize,
    item: &NamedPng,
    row: Option<&ManifestFrame>,
) -> (FrameReport, Option<Frame>) {
    let mut report = empty_report(&item.name, &item.png);
    if let Some(row) = row {
        report.manifest_tick = row.tick;
        report.manifest_png_ok = row.png_fnv.as_ref().map(|filed| *filed == report.png_fnv);
    }
    match Frame::decode_png(&item.png) {
        Ok(frame) => {
            report.width = frame.width();
            report.height = frame.height();
            report.pixel_fnv = digest_hex(frame.rgba());
            score_decoded(ctx, index, &frame, row, &mut report);
            (report, Some(frame))
        }
        Err(error) => {
            report.fault = Some(format!("bad_png: {error}"));
            for slot in 0..6 {
                set_item(&mut report, slot, 0, 0);
            }
            (report, None)
        }
    }
}

// ---------------------------------------------------------------------------
// Set level
// ---------------------------------------------------------------------------

/// Bind frames to manifest rows by file name when the rows carry names,
/// else by position. Returns the row per frame plus an integrity verdict.
fn bind_rows<'m>(
    frames: &[NamedPng],
    manifest: Option<&'m Manifest>,
    notes: &mut Vec<String>,
) -> (Vec<Option<&'m ManifestFrame>>, bool) {
    let Some(manifest) = manifest else {
        return (vec![None; frames.len()], true);
    };
    let named = manifest.frames.iter().any(|row| row.file.is_some());
    let rows: Vec<Option<&ManifestFrame>> = frames
        .iter()
        .enumerate()
        .map(|(i, frame)| {
            if named {
                manifest
                    .frames
                    .iter()
                    .find(|row| row.file.as_deref() == Some(frame.name.as_str()))
            } else {
                manifest.frames.get(i)
            }
        })
        .collect();
    let mut ok = true;
    if manifest.frames.len() != frames.len() {
        notes.push(format!(
            "manifest lists {} rows for {} frames",
            manifest.frames.len(),
            frames.len()
        ));
        ok = false;
    }
    for (frame, row) in frames.iter().zip(&rows) {
        if row.is_none() {
            notes.push(format!("no manifest row binds {}", frame.name));
            ok = false;
        }
    }
    (rows, ok)
}

fn mean(values: &[u32]) -> u32 {
    let n = u64::try_from(values.len()).unwrap_or(1).max(1);
    let sum: u64 = values.iter().map(|&v| u64::from(v)).sum();
    u32::try_from((sum + n / 2) / n).unwrap_or(u32::MAX)
}

/// Fold per-frame scores into the nine rubric items.
fn build_rubric(frames: &[FrameReport], cfg: &JudgeConfig) -> Vec<RubricItem> {
    let total = u32_of(frames.len());
    let mut items = Vec::with_capacity(RUBRIC_ITEMS);
    for (slot, (id, name, unit)) in RUBRIC.iter().take(FRAME_ITEMS).enumerate() {
        let scored: Vec<u32> = frames.iter().filter_map(|f| f.scores[slot]).collect();
        let raws: Vec<i64> = frames.iter().filter_map(|f| f.raw[slot]).collect();
        let passing = u32_of(
            scored
                .iter()
                .filter(|&&s| s >= cfg.pass_bar_permille)
                .count(),
        );
        let applicable = !scored.is_empty();
        items.push(RubricItem {
            id,
            name,
            applicable,
            score_permille: mean(&scored),
            pass: applicable
                && permille(u64::from(passing), u64::from(u32_of(scored.len())))
                    >= cfg.hero_pass_permille,
            frames_passing: passing,
            detail: format!("{unit} per frame: {raws:?}"),
        });
    }
    let hero = u32_of(frames.iter().filter(|f| passes_all(f, cfg)).count());
    let share = permille(u64::from(hero), u64::from(total));
    let (id, name, unit) = RUBRIC[FRAME_ITEMS];
    items.push(RubricItem {
        id,
        name,
        applicable: total > 0,
        score_permille: share,
        pass: share >= cfg.hero_pass_permille,
        frames_passing: hero,
        detail: format!("{unit}: {hero} of {total}"),
    });
    items
}

/// True when every applicable per-frame item clears the pass bar.
fn passes_all(frame: &FrameReport, cfg: &JudgeConfig) -> bool {
    frame
        .scores
        .iter()
        .flatten()
        .all(|&s| s >= cfg.pass_bar_permille)
        && frame.scores[0].is_some()
}

/// Judge a capture set against optional references.
///
/// # Errors
///
/// [`JudgeError::NoFrames`], [`JudgeError::TooMany`], [`JudgeError::TooLarge`]
/// or [`JudgeError::BadManifest`] when the set cannot be judged at all.
pub fn judge(
    set: &CaptureSet,
    refs: &References,
    cfg: &JudgeConfig,
) -> Result<LookJudgement, JudgeError> {
    check_bounds(set, refs)?;
    let manifest = set
        .manifest
        .as_deref()
        .map(parse_manifest)
        .transpose()
        .map_err(JudgeError::BadManifest)?;
    let mut notes = Vec::new();
    let ctx = Context {
        cfg,
        previews: load_previews(&refs.fighters, cfg, &mut notes),
        q1: load_q1(&refs.q1, &mut notes),
    };
    let (rows, integrity) = bind_rows(&set.frames, manifest.as_ref(), &mut notes);
    let mut frames = Vec::with_capacity(set.frames.len());
    let mut motion = Vec::new();
    let mut digests = Vec::new();
    let mut previous: Option<Frame> = None;
    for (index, (item, row)) in set.frames.iter().zip(&rows).enumerate() {
        let (report, decoded) = judge_frame(&ctx, index, item, *row);
        if index > 0 {
            motion.push(match (&previous, &decoded) {
                (Some(a), Some(b)) => metric::ssim(b, a, Region::Full)
                    .ok()
                    .map(|s| s.score_x10000),
                _ => None,
            });
        }
        digests.push(if report.pixel_fnv.is_empty() {
            report.png_fnv.clone()
        } else {
            report.pixel_fnv.clone()
        });
        frames.push(report);
        previous = decoded;
    }
    let mut distinct = digests.clone();
    distinct.sort();
    distinct.dedup();
    let sequence = Sequence {
        frames: u32_of(frames.len()),
        distinct: u32_of(distinct.len()),
        frozen: check_digest_window(&digests).is_err(),
        motion_ssim_x10000: motion,
    };
    let integrity_ok = integrity && frames.iter().all(|f| f.manifest_png_ok != Some(false));
    let rubric = build_rubric(&frames, cfg);
    let applicable: Vec<u32> = rubric
        .iter()
        .filter(|i| i.applicable)
        .map(|i| i.score_permille)
        .collect();
    let mut judgement = LookJudgement {
        digest: String::new(),
        pass: integrity_ok && !sequence.frozen && rubric.iter().all(|i| !i.applicable || i.pass),
        score_permille: mean(&applicable),
        integrity_ok,
        rubric,
        sequence,
        frames,
        notes,
    };
    judgement.digest = digest_hex(judgement.json_body().as_bytes());
    Ok(judgement)
}

fn check_bounds(set: &CaptureSet, refs: &References) -> Result<(), JudgeError> {
    if set.frames.is_empty() {
        return Err(JudgeError::NoFrames);
    }
    for (what, count, cap) in [
        ("frames", set.frames.len(), MAX_FRAMES),
        ("fighter references", refs.fighters.len(), MAX_REFERENCES),
        ("q1 references", refs.q1.len(), MAX_REFERENCES),
    ] {
        if count > cap {
            return Err(JudgeError::TooMany { what, count });
        }
    }
    let pngs = set.frames.iter().chain(&refs.fighters).chain(&refs.q1);
    if let Some(big) = pngs.into_iter().find(|p| p.png.len() > MAX_PNG_BYTES) {
        return Err(JudgeError::TooLarge {
            name: big.name.clone(),
            bytes: big.png.len(),
        });
    }
    match &set.manifest {
        Some(text) if text.len() > MAX_MANIFEST_BYTES => Err(JudgeError::TooLarge {
            name: MANIFEST_NAME.to_owned(),
            bytes: text.len(),
        }),
        _ => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// JSON (fixed field order, integers only)
// ---------------------------------------------------------------------------

fn json_str(out: &mut String, text: &str) {
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn json_opt<T: fmt::Display>(out: &mut String, value: Option<T>) {
    match value {
        Some(v) => out.push_str(&v.to_string()),
        None => out.push_str("null"),
    }
}

fn json_list<T>(out: &mut String, items: &[T], mut write: impl FnMut(&mut String, &T)) {
    out.push('[');
    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write(out, item);
    }
    out.push(']');
}

fn write_item(out: &mut String, item: &RubricItem) {
    out.push_str(&format!("{{\"id\":\"{}\",\"name\":\"{}\",\"applicable\":{},\"score\":{},\"pass\":{},\"frames_passing\":{},\"detail\":", item.id, item.name, item.applicable, item.score_permille, item.pass, item.frames_passing));
    json_str(out, &item.detail);
    out.push('}');
}

fn write_comparison(out: &mut String, c: &Comparison) {
    out.push_str(&format!(
        "{{\"ssim\":{},\"mean_de\":{},\"hist_tv\":{},\"hist_emd\":{}}}",
        c.ssim_x10000, c.mean_de_x100, c.hist_tv_x10000, c.hist_emd_x10000
    ));
}

fn write_frame(out: &mut String, f: &FrameReport) {
    out.push_str("{\"name\":");
    json_str(out, &f.name);
    out.push_str(&format!(
        ",\"png_fnv\":\"{}\",\"pixel_fnv\":\"{}\",\"width\":{},\"height\":{},\"fault\":",
        f.png_fnv, f.pixel_fnv, f.width, f.height
    ));
    match &f.fault {
        Some(words) => json_str(out, words),
        None => out.push_str("null"),
    }
    out.push_str(",\"manifest_tick\":");
    json_opt(out, f.manifest_tick);
    out.push_str(",\"manifest_png_ok\":");
    json_opt(out, f.manifest_png_ok);
    out.push_str(",\"scores\":");
    json_list(out, &f.scores, |o, s| json_opt(o, *s));
    out.push_str(",\"raw\":");
    json_list(out, &f.raw, |o, r| json_opt(o, *r));
    out.push_str(&format!(
        ",\"fighters\":{{\"count\":{},\"pixels\":{},\"regions\":",
        f.fighters.count, f.fighters.pixels
    ));
    json_list(out, &f.fighters.regions, |o, r| {
        o.push_str("{\"entity\":");
        json_opt(o, r.entity);
        o.push_str(&format!(
            ",\"x\":{},\"y\":{},\"w\":{},\"h\":{},\"pixels\":{}}}",
            r.x, r.y, r.w, r.h, r.pixels
        ));
    });
    out.push_str(",\"manifest_slots\":");
    json_list(out, &f.fighters.manifest_slots, |o, s| {
        o.push_str(&s.to_string())
    });
    out.push_str(",\"manifest_unseen\":");
    json_list(out, &f.fighters.manifest_unseen, |o, s| {
        o.push_str(&s.to_string())
    });
    out.push_str("},\"preview\":");
    match &f.preview {
        Some(p) => {
            out.push_str("{\"name\":");
            json_str(out, &p.name);
            out.push_str(&format!(
                ",\"emd\":{},\"mean_de\":{}}}",
                p.emd_x10000, p.mean_de_x100
            ));
        }
        None => out.push_str("null"),
    }
    out.push_str(",\"q1\":");
    match &f.q1 {
        Some(q) => {
            out.push_str("{\"name\":");
            json_str(out, &q.name);
            out.push_str(",\"comparison\":");
            match &q.comparison {
                Some(c) => write_comparison(out, c),
                None => out.push_str("null"),
            }
            out.push('}');
        }
        None => out.push_str("null"),
    }
    out.push('}');
}

impl LookJudgement {
    /// Every JSON member after the digest, without the braces: the bytes the
    /// digest covers.
    fn json_body(&self) -> String {
        let mut out = format!(
            "\"pass\":{},\"score\":{},\"integrity_ok\":{},\"rubric\":",
            self.pass, self.score_permille, self.integrity_ok
        );
        json_list(&mut out, &self.rubric, |o, i| write_item(o, i));
        let s = &self.sequence;
        out.push_str(&format!(
            ",\"sequence\":{{\"frames\":{},\"distinct\":{},\"frozen\":{},\"motion_ssim\":",
            s.frames, s.distinct, s.frozen
        ));
        json_list(&mut out, &s.motion_ssim_x10000, |o, m| json_opt(o, *m));
        out.push_str("},\"frames\":");
        json_list(&mut out, &self.frames, |o, f| write_frame(o, f));
        out.push_str(",\"notes\":");
        json_list(&mut out, &self.notes, |o, n| json_str(o, n));
        out
    }

    /// The typed result as JSON: fixed field order, integers only, so the
    /// same capture set gives byte-identical text twice.
    pub fn to_json(&self) -> String {
        format!(
            "{{\"schema\":{SCHEMA},\"digest\":\"{}\",{}}}",
            self.digest,
            self.json_body()
        )
    }
}

// ---------------------------------------------------------------------------
// Disk
// ---------------------------------------------------------------------------

fn io_error(path: &Path, error: &std::io::Error) -> JudgeError {
    JudgeError::Io {
        path: path.display().to_string(),
        kind: error.kind(),
    }
}

fn read_bounded(path: &Path, name: &str, cap: usize) -> Result<Vec<u8>, JudgeError> {
    let meta = std::fs::metadata(path).map_err(|e| io_error(path, &e))?;
    let bytes = usize::try_from(meta.len()).unwrap_or(usize::MAX);
    if bytes > cap {
        return Err(JudgeError::TooLarge {
            name: name.to_owned(),
            bytes,
        });
    }
    std::fs::read(path).map_err(|e| io_error(path, &e))
}

/// Read every `*.png` in a directory, sorted by file name.
///
/// # Errors
///
/// [`JudgeError::Io`] for an unreadable directory or file,
/// [`JudgeError::TooMany`] past [`MAX_FRAMES`], [`JudgeError::TooLarge`] for
/// a PNG over [`MAX_PNG_BYTES`].
pub fn load_png_dir(dir: &Path) -> Result<Vec<NamedPng>, JudgeError> {
    let entries = std::fs::read_dir(dir).map_err(|e| io_error(dir, &e))?;
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| io_error(dir, &e))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.to_ascii_lowercase().ends_with(".png") {
            names.push(name);
        }
    }
    names.sort();
    if names.len() > MAX_FRAMES {
        return Err(JudgeError::TooMany {
            what: "png files",
            count: names.len(),
        });
    }
    names
        .into_iter()
        .map(|name| {
            let png = read_bounded(&dir.join(&name), &name, MAX_PNG_BYTES)?;
            Ok(NamedPng { name, png })
        })
        .collect()
}

/// Read a capture directory: its PNGs sorted by name plus `manifest.json`
/// when present.
///
/// # Errors
///
/// As [`load_png_dir`], plus [`JudgeError::Io`] for an unreadable manifest.
pub fn load_capture_dir(dir: &Path) -> Result<CaptureSet, JudgeError> {
    let frames = load_png_dir(dir)?;
    let path = dir.join(MANIFEST_NAME);
    let manifest = if path.is_file() {
        let bytes = read_bounded(&path, MANIFEST_NAME, MAX_MANIFEST_BYTES)?;
        Some(String::from_utf8_lossy(&bytes).into_owned())
    } else {
        None
    };
    Ok(CaptureSet { frames, manifest })
}

// ---------------------------------------------------------------------------
// Per-frame hook: one counted score per capture
// ---------------------------------------------------------------------------

/// One scored frame from the per-frame hook.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HookScore {
    /// Calls so far, including this frame.
    pub calls: u64,
    /// Match against the reference, none without one or on a fault.
    pub comparison: Option<Comparison>,
    /// True when the PNG did not decode or the match failed.
    pub fault: bool,
}

/// Per-frame look hook: one counted score per capture.
///
/// Consumer: the window host hands each due capture PNG to
/// [`LookHook::note_frame`] once; the endpoint `capture_with` hunk and the
/// `capture_offscreen` site are the intended callers once the dep edge lands.
/// Data owner: the capture path owns the PNG bytes; the hook owns one decoded
/// reference copy. Units: calls and faults in frames, scores from
/// [`metric::compare`]. Determinism: integer only; the same bytes score the
/// same twice. Errors: a bad PNG records [`LookHook::last_fault`] and still
/// counts.
pub struct LookHook {
    reference: Option<Frame>,
    calls: u64,
    faults: u64,
    last: Option<Comparison>,
    last_fault: Option<String>,
}

impl LookHook {
    /// Arm the hook with an optional reference PNG for [`metric::compare`].
    /// A bad reference means no reference; the hook still counts frames.
    pub fn register(reference_png: Option<&[u8]>) -> Self {
        let reference = reference_png.and_then(|bytes| Frame::decode_png(bytes).ok());
        Self {
            reference,
            calls: 0,
            faults: 0,
            last: None,
            last_fault: None,
        }
    }

    /// Score one capture PNG exactly once. Decodes through
    /// [`Frame::decode_png`] and runs [`metric::compare`] against the
    /// reference when one is armed.
    pub fn note_frame(&mut self, png: &[u8]) -> HookScore {
        self.calls = self.calls.saturating_add(1);
        let frame = match Frame::decode_png(png) {
            Ok(frame) => frame,
            Err(error) => {
                self.faults = self.faults.saturating_add(1);
                self.last = None;
                self.last_fault = Some(error.to_string());
                return HookScore {
                    calls: self.calls,
                    comparison: None,
                    fault: true,
                };
            }
        };
        let Some(reference) = self.reference.as_ref() else {
            self.last = None;
            self.last_fault = None;
            return HookScore {
                calls: self.calls,
                comparison: None,
                fault: false,
            };
        };
        match metric::compare(&frame, reference, Region::Full) {
            Ok(comparison) => {
                self.last = Some(comparison);
                self.last_fault = None;
                HookScore {
                    calls: self.calls,
                    comparison: Some(comparison),
                    fault: false,
                }
            }
            Err(error) => {
                self.faults = self.faults.saturating_add(1);
                self.last = None;
                self.last_fault = Some(error.to_string());
                HookScore {
                    calls: self.calls,
                    comparison: None,
                    fault: true,
                }
            }
        }
    }

    /// Frames handed so far.
    pub fn calls(&self) -> u64 {
        self.calls
    }

    /// Bad frames so far.
    pub fn faults(&self) -> u64 {
        self.faults
    }

    /// Last match, none without a reference or on a fault.
    pub fn last(&self) -> Option<Comparison> {
        self.last
    }

    /// Last fault words, none when the last frame scored.
    pub fn last_fault(&self) -> Option<&str> {
        self.last_fault.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame_check::{scripted, Script};
    use image::{codecs::png::PngEncoder, ExtendedColorType, ImageEncoder};

    const W: u32 = 128;
    const H: u32 = 96;
    const WHITE: [u8; 3] = [255, 255, 255];
    const GOLD: [u8; 3] = [217, 179, 38];
    const RED: [u8; 3] = [191, 38, 31];
    const EDGE: [u8; 3] = [20, 16, 12];

    fn sand_at(x: u32, y: u32) -> [u8; 3] {
        let t = i32::try_from((x * 7 + y * 13 + (x * y) % 5) % 23).expect("small") - 11;
        [194i32, 160, 104].map(|c| u8::try_from((c + t).clamp(0, 255)).expect("clamped"))
    }

    fn rgba_of(width: u32, height: u32, paint: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        let mut rgba = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let [r, g, b] = paint(x, y);
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
        }
        rgba
    }

    fn png_of(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        PngEncoder::new(&mut out)
            .write_image(rgba, width, height, ExtendedColorType::Rgba8)
            .expect("png encodes");
        out
    }

    fn named(name: &str, width: u32, height: u32, rgba: &[u8]) -> NamedPng {
        NamedPng {
            name: name.to_owned(),
            png: png_of(width, height, rgba),
        }
    }

    /// One drawn body: arena position in millimetres plus fill colour.
    struct Dot {
        x_mm: i64,
        z_mm: i64,
        fill: [u8; 3],
    }

    /// Frame centre pixel of an arena axis under the staged top-down map,
    /// written out again here so the test does not lean on the judge's own.
    fn centre(value_mm: i64, pixels: u32) -> u32 {
        let inner = i64::from(pixels) - 8;
        u32::try_from(4 + (value_mm + 20_000) * inner / 40_000).expect("inside frame")
    }

    fn draw_body(rgba: &mut [u8], dot: &Dot) {
        let (cx, cy) = (centre(dot.x_mm, W), centre(dot.z_mm, H));
        for dy in 0..9u32 {
            for dx in 0..7u32 {
                let (x, y) = (cx - 3 + dx, cy - 4 + dy);
                let rim = dx == 0 || dy == 0 || dx == 6 || dy == 8;
                let [r, g, b] = if rim { EDGE } else { dot.fill };
                let at = usize::try_from((y * W + x) * 4).expect("fits");
                rgba[at..at + 4].copy_from_slice(&[r, g, b, 255]);
            }
        }
    }

    /// Stands as a grey gradient across the top quarter, textured sand
    /// below, one body per dot. Passes every item that needs no reference.
    fn arena_rgba(dots: &[Dot]) -> Vec<u8> {
        let mut rgba = rgba_of(W, H, |x, y| {
            if y < 24 {
                [u8::try_from(40 + y * 4).expect("shade"); 3]
            } else {
                sand_at(x, y)
            }
        });
        dots.iter().for_each(|dot| draw_body(&mut rgba, dot));
        rgba
    }

    fn two_fighters() -> [Dot; 2] {
        [
            Dot {
                x_mm: -10_000,
                z_mm: 0,
                fill: WHITE,
            },
            Dot {
                x_mm: 10_000,
                z_mm: 0,
                fill: GOLD,
            },
        ]
    }

    fn arena(name: &str, dots: &[Dot]) -> NamedPng {
        named(name, W, H, &arena_rgba(dots))
    }

    fn set_of(frames: Vec<NamedPng>) -> CaptureSet {
        CaptureSet {
            frames,
            manifest: None,
        }
    }

    fn judged(frames: Vec<NamedPng>) -> LookJudgement {
        judge(
            &set_of(frames),
            &References::default(),
            &JudgeConfig::default(),
        )
        .expect("judges")
    }

    fn score(judgement: &LookJudgement, id: &str) -> u32 {
        judgement.item(id).expect("item exists").score_permille
    }

    #[test]
    fn look_judge_blank_frame_scores_zero_on_l1() {
        let blank = named("blank.png", W, H, &rgba_of(W, H, sand_at));
        let judgement = judged(vec![blank]);
        assert_eq!(score(&judgement, "L1"), 0);
        assert_eq!(judgement.frames[0].fighters.count, 0);
        assert!(!judgement.pass);
    }

    #[test]
    fn look_judge_cube_only_frames_score_zero_on_l1() {
        // The frame-check cube scene: a copper box over dark slate.
        let cube = scripted(Script::Clean, 96, 64, 7);
        let copper = named("cube.png", cube.width, cube.height, &cube.rgba);
        // The error mesh: a magenta cube on sand.
        let mut magenta = rgba_of(W, H, sand_at);
        for y in 40..54u32 {
            for x in 50..64u32 {
                let at = usize::try_from((y * W + x) * 4).expect("fits");
                magenta[at..at + 4].copy_from_slice(&[230, 20, 200, 255]);
            }
        }
        let judgement = judged(vec![copper, named("magenta.png", W, H, &magenta)]);
        assert_eq!(judgement.frames[0].scores[0], Some(0));
        assert_eq!(judgement.frames[1].scores[0], Some(0));
        assert_eq!(judgement.frames[1].fighters.count, 0);
        assert_eq!(score(&judgement, "L1"), 0);
    }

    #[test]
    fn look_judge_frame_with_drawn_fighters_scores_above_zero_on_l1() {
        let judgement = judged(vec![arena("two.png", &two_fighters())]);
        assert_eq!(score(&judgement, "L1"), 1000);
        let frame = &judgement.frames[0];
        assert_eq!((frame.fighters.count, frame.fighters.pixels), (2, 126));
        let one = judged(vec![arena("one.png", &two_fighters()[..1])]);
        assert_eq!(score(&one, "L1"), 500);
        assert!(score(&one, "L1") > 0);
    }

    #[test]
    fn look_judge_manifest_counts_never_feed_l1() {
        // A manifest that swears four fighters over a blank frame.
        let blank = named("blank.png", W, H, &rgba_of(W, H, sand_at));
        let fnv = digest_hex(&blank.png);
        let manifest = format!(
            "[{{\"tick\":1,\"png_fnv\":\"{fnv}\",\"fighters\":[{}],\"size\":[{W},{H}]}}]",
            (0..4)
                .map(|s| format!(
                    "{{\"slot\":{s},\"x\":{}.000,\"z\":0.000,\"hp\":1000,\"hit\":false}}",
                    s * 5 - 8
                ))
                .collect::<Vec<_>>()
                .join(",")
        );
        let set = CaptureSet {
            frames: vec![blank],
            manifest: Some(manifest),
        };
        let lie = judge(&set, &References::default(), &JudgeConfig::default()).expect("judges");
        assert_eq!(score(&lie, "L1"), 0);
        assert_eq!(lie.frames[0].fighters.manifest_slots, vec![0, 1, 2, 3]);
        assert_eq!(lie.frames[0].fighters.manifest_unseen, vec![0, 1, 2, 3]);
        // And the reverse: fighters drawn, manifest silent about them.
        let drawn = arena("drawn.png", &two_fighters());
        let silent = CaptureSet {
            frames: vec![drawn],
            manifest: Some("[{\"tick\":1,\"fighters\":[]}]".to_owned()),
        };
        let truth =
            judge(&silent, &References::default(), &JudgeConfig::default()).expect("judges");
        assert_eq!(score(&truth, "L1"), 1000);
        assert!(truth.frames[0]
            .fighters
            .regions
            .iter()
            .all(|r| r.entity.is_none()));
    }

    #[test]
    fn look_judge_regions_ground_on_manifest_entities() {
        let drawn = arena("grounded.png", &two_fighters());
        let fnv = digest_hex(&drawn.png);
        let row = |slot: u32, x: &str| {
            format!("{{\"slot\":{slot},\"x\":{x},\"z\":0.000,\"hp\":1000,\"hit\":false}}")
        };
        let manifest = format!(
            "[{{\"tick\":4,\"png_fnv\":\"{fnv}\",\"fighters\":[{},{},{},{}],\"size\":[{W},{H}]}}]",
            row(3, "10.000"),
            row(0, "-10.000"),
            row(2, "-18.000"),
            row(1, "18.000"),
        );
        let set = CaptureSet {
            frames: vec![drawn],
            manifest: Some(manifest),
        };
        let judgement =
            judge(&set, &References::default(), &JudgeConfig::default()).expect("judges");
        let frame = &judgement.frames[0];
        // Slot 0 stands at x = -10 m and slot 3 at x = +10 m.
        assert_eq!(
            frame.fighters.regions,
            vec![
                FigureRegion {
                    entity: Some(0),
                    x: 31,
                    y: 44,
                    w: 7,
                    h: 9,
                    pixels: 63
                },
                FigureRegion {
                    entity: Some(3),
                    x: 91,
                    y: 44,
                    w: 7,
                    h: 9,
                    pixels: 63
                },
            ]
        );
        assert_eq!(frame.fighters.manifest_unseen, vec![1, 2]);
        assert_eq!(frame.manifest_tick, Some(4));
        assert_eq!(frame.manifest_png_ok, Some(true));
        assert!(judgement.integrity_ok);
    }

    #[test]
    fn look_judge_same_capture_gives_byte_identical_json_twice() {
        let frames = || {
            vec![
                arena("a.png", &two_fighters()),
                arena(
                    "b.png",
                    &[
                        Dot {
                            x_mm: -4_000,
                            z_mm: 0,
                            fill: RED,
                        },
                        Dot {
                            x_mm: 6_000,
                            z_mm: 0,
                            fill: WHITE,
                        },
                    ],
                ),
            ]
        };
        let (first, second) = (judged(frames()), judged(frames()));
        assert_eq!(first, second);
        let (a, b) = (first.to_json(), second.to_json());
        assert_eq!(a, b);
        assert!(a.starts_with("{\"schema\":1,\"digest\":\""));
        // The digest covers every byte after it.
        let body = a.split_once("\",").expect("digest field").1;
        assert_eq!(
            first.digest,
            digest_hex(body.trim_end_matches('}').as_bytes())
        );
        // Rubric ids appear once each, in order.
        let at: Vec<usize> = (1..=9)
            .map(|n| a.find(&format!("\"id\":\"L{n}\"")).expect("id present"))
            .collect();
        assert!(at.windows(2).all(|pair| pair[0] < pair[1]));
        // No float anywhere outside strings: integers only.
        let mut outside = String::new();
        let mut in_string = false;
        let mut escaped = false;
        for ch in a.chars() {
            match (in_string, ch) {
                (true, _) if escaped => escaped = false,
                (true, '\\') => escaped = true,
                (_, '"') => in_string = !in_string,
                (false, c) => outside.push(c),
                _ => {}
            }
        }
        assert!(!outside.contains('.'), "float in JSON: {outside}");
    }

    #[test]
    fn look_judge_passes_when_enough_frames_pass_every_item() {
        let moving = |shift: i64| {
            arena(
                "f.png",
                &[
                    Dot {
                        x_mm: -10_000 + shift,
                        z_mm: 0,
                        fill: WHITE,
                    },
                    Dot {
                        x_mm: 10_000 - shift,
                        z_mm: 0,
                        fill: GOLD,
                    },
                ],
            )
        };
        let blank = || named("blank.png", W, H, &rgba_of(W, H, sand_at));
        let good = judged(vec![
            moving(0),
            moving(500),
            moving(1000),
            moving(1500),
            blank(),
        ]);
        for id in ["L2", "L3", "L4", "L5", "L6"] {
            assert!(good.item(id).expect("item").pass, "{id} should pass");
        }
        assert_eq!(good.item("L9").expect("L9").frames_passing, 4);
        assert_eq!(score(&good, "L9"), 800);
        assert!(good.item("L9").expect("L9").pass);
        assert!(good.pass, "{:?}", good.rubric);
        assert!(!good.item("L7").expect("L7").applicable);
        assert!(!good.item("L8").expect("L8").applicable);
        let weak = judged(vec![moving(0), moving(500), moving(1000), blank(), blank()]);
        assert_eq!(score(&weak, "L9"), 600);
        assert!(!weak.pass);
    }

    #[test]
    fn look_judge_texture_and_structure_follow_the_pixels() {
        let flat = named(
            "flat.png",
            W,
            H,
            &rgba_of(W, H, |_, y| if y < 24 { [60; 3] } else { [194, 160, 104] }),
        );
        let textured = arena("textured.png", &[]);
        let judgement = judged(vec![flat, textured]);
        assert_eq!(
            judgement.frames[0].scores[2],
            Some(0),
            "flat ground has no texture"
        );
        assert_eq!(judgement.frames[1].scores[2], Some(1000));
        assert_eq!(
            judgement.frames[0].scores[1],
            Some(1000),
            "the stand band is structure"
        );
        let bare = judged(vec![named("bare.png", W, H, &rgba_of(W, H, sand_at))]);
        assert_eq!(bare.frames[0].scores[1], Some(0));
    }

    #[test]
    fn look_judge_placeholder_frames_fail_l4() {
        // The 64x48 noise class: random bytes at the placeholder size.
        let mut state = 0x2545_f491_u32;
        let mut next = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            u8::try_from(state >> 24).expect("byte")
        };
        let noise: Vec<u8> = (0..64 * 48)
            .flat_map(|_| [next(), next(), next(), 255])
            .collect();
        let small = named("noise64.png", 64, 48, &noise);
        // Noise at a legal size is refused by the edge measure.
        let big_noise: Vec<u8> = (0..W * H)
            .flat_map(|_| [next(), next(), next(), 255])
            .collect();
        let big = named("noise128.png", W, H, &big_noise);
        // A known placeholder digest in the refuse list.
        let known = arena("known.png", &two_fighters());
        let mut cfg = JudgeConfig::default();
        cfg.refused_png_fnv.push(digest_hex(&known.png));
        let set = set_of(vec![
            small,
            big,
            known,
            arena("real.png", &two_fighters()[..1]),
        ]);
        let judgement = judge(&set, &References::default(), &cfg).expect("judges");
        let verdicts: Vec<Option<u32>> = judgement.frames.iter().map(|f| f.scores[3]).collect();
        assert_eq!(verdicts, vec![Some(0), Some(0), Some(0), Some(1000)]);
        assert!(judgement.frames[0]
            .fault
            .as_deref()
            .expect("fault")
            .contains("placeholder size"));
        assert!(judgement.frames[1]
            .fault
            .as_deref()
            .expect("fault")
            .contains("noise"));
        assert!(judgement.frames[2]
            .fault
            .as_deref()
            .expect("fault")
            .contains("known placeholder"));
        assert_eq!(judgement.frames[3].fault, None);
    }

    #[test]
    fn look_judge_runs_frame_check_and_scores_dark_and_white_frames() {
        let black = named("black.png", W, H, &rgba_of(W, H, |_, _| [0, 0, 0]));
        let white = named("white.png", W, H, &rgba_of(W, H, |_, _| [255, 255, 255]));
        let judgement = judged(vec![black, white, arena("ok.png", &two_fighters())]);
        assert_eq!(judgement.frames[0].fault.as_deref(), Some("black_screen"));
        assert_eq!(judgement.frames[1].fault.as_deref(), Some("white_screen"));
        assert_eq!(judgement.frames[2].fault, None);
        let l5: Vec<Option<u32>> = judgement.frames.iter().map(|f| f.scores[4]).collect();
        assert_eq!(l5, vec![Some(0), Some(0), Some(1000)]);
    }

    #[test]
    fn look_judge_a_bad_png_is_a_scored_fault_not_a_panic() {
        let junk = NamedPng {
            name: "junk.png".to_owned(),
            png: b"definitely not a png".to_vec(),
        };
        let judgement = judged(vec![junk, arena("ok.png", &two_fighters())]);
        let frame = &judgement.frames[0];
        assert!(frame
            .fault
            .as_deref()
            .expect("fault")
            .starts_with("bad_png"));
        assert_eq!((frame.width, frame.pixel_fnv.as_str()), (0, ""));
        assert_eq!(frame.scores[0], Some(0));
        assert!(!judgement.pass);
    }

    #[test]
    fn look_judge_frozen_sequence_fails_the_set() {
        let still = || arena("same.png", &two_fighters());
        let judgement = judged(vec![still(), still(), still()]);
        assert!(judgement.sequence.frozen);
        assert_eq!(judgement.sequence.distinct, 1);
        assert_eq!(
            judgement.sequence.motion_ssim_x10000,
            vec![Some(SSIM_ONE), Some(SSIM_ONE)]
        );
        assert!(!judgement.pass);
        let moving = judged(vec![still(), arena("m.png", &two_fighters()[..1]), still()]);
        assert!(!moving.sequence.frozen);
        assert_eq!(moving.sequence.distinct, 2);
        assert!(moving
            .sequence
            .motion_ssim_x10000
            .iter()
            .flatten()
            .all(|&s| s < SSIM_ONE));
    }

    #[test]
    fn look_judge_tampered_manifest_digest_fails_integrity() {
        let drawn = arena("a.png", &two_fighters());
        let honest = format!(
            "[{{\"tick\":1,\"png_fnv\":\"{}\"}}]",
            digest_hex(&drawn.png)
        );
        let forged = "[{\"tick\":1,\"png_fnv\":\"0000000000000000\"}]".to_owned();
        let run = |manifest: String| {
            let set = CaptureSet {
                frames: vec![drawn.clone()],
                manifest: Some(manifest),
            };
            judge(&set, &References::default(), &JudgeConfig::default()).expect("judges")
        };
        let (ok, bad) = (run(honest), run(forged));
        assert!(ok.integrity_ok);
        assert_eq!(ok.frames[0].manifest_png_ok, Some(true));
        assert!(!bad.integrity_ok);
        assert_eq!(bad.frames[0].manifest_png_ok, Some(false));
        assert!(!bad.pass);
        // A manifest with fewer rows than frames is an integrity note too.
        let short = CaptureSet {
            frames: vec![drawn.clone(), arena("b.png", &two_fighters()[..1])],
            manifest: Some("[{\"tick\":1}]".to_owned()),
        };
        let judgement =
            judge(&short, &References::default(), &JudgeConfig::default()).expect("judges");
        assert!(!judgement.integrity_ok);
        assert!(judgement
            .notes
            .iter()
            .any(|n| n.contains("1 rows for 2 frames")));
    }

    /// A grey ground with one outlined body of the given fill, like a
    /// fighter preview render (flat ground, one subject).
    fn preview(fill: [u8; 3]) -> NamedPng {
        named(
            "preview.png",
            64,
            64,
            &rgba_of(64, 64, |x, y| {
                let body = (24..40).contains(&x) && (8..56).contains(&y);
                let rim = x == 24 || x == 39 || y == 8 || y == 55;
                match (body, rim) {
                    (true, true) => EDGE,
                    (true, false) => fill,
                    _ => [64, 64, 64],
                }
            }),
        )
    }

    #[test]
    fn look_judge_scores_fighter_palette_against_previews() {
        let frames = vec![arena("a.png", &two_fighters())];
        let bright = NamedPng {
            name: "bright.png".to_owned(),
            ..preview([235, 235, 235])
        };
        let dark = NamedPng {
            name: "dark.png".to_owned(),
            ..preview([20, 20, 20])
        };
        let run = |previews: Vec<NamedPng>| {
            let refs = References {
                fighters: previews,
                q1: Vec::new(),
            };
            judge(&set_of(frames.clone()), &refs, &JudgeConfig::default()).expect("judges")
        };
        let near = run(vec![bright.clone()]);
        let far = run(vec![dark.clone()]);
        assert!(near.item("L7").expect("L7").applicable);
        assert!(
            score(&near, "L7") > score(&far, "L7"),
            "{} vs {}",
            score(&near, "L7"),
            score(&far, "L7")
        );
        // The best of two previews wins, named in the report.
        let both = run(vec![dark, bright]);
        assert_eq!(
            both.frames[0].preview.as_ref().expect("match").name,
            "bright.png"
        );
        assert_eq!(score(&both, "L7"), score(&near, "L7"));
        // No fighter pixels leave nothing to compare.
        let empty = judge(
            &set_of(vec![named("blank.png", W, H, &rgba_of(W, H, sand_at))]),
            &References {
                fighters: vec![preview([235; 3])],
                q1: Vec::new(),
            },
            &JudgeConfig::default(),
        )
        .expect("judges");
        assert_eq!(empty.frames[0].scores[6], Some(0));
        // A preview that is not a PNG is skipped with a note, never fatal.
        let junk = NamedPng {
            name: "junk.png".to_owned(),
            png: vec![1, 2, 3],
        };
        let skipped = run(vec![junk]);
        assert!(!skipped.item("L7").expect("L7").applicable);
        assert!(skipped.notes.iter().any(|n| n.contains("junk.png skipped")));
    }

    #[test]
    fn look_judge_scores_the_q1_reference_by_ssim() {
        let frames = vec![arena("a.png", &two_fighters())];
        let same = arena("q1.png", &two_fighters());
        let moved = arena(
            "q1m.png",
            &[Dot {
                x_mm: -4_000,
                z_mm: 3_000,
                fill: RED,
            }],
        );
        let wrong_size = named("q1s.png", 64, 48, &rgba_of(64, 48, sand_at));
        let run = |q1: Vec<NamedPng>| {
            let refs = References {
                fighters: Vec::new(),
                q1,
            };
            judge(&set_of(frames.clone()), &refs, &JudgeConfig::default()).expect("judges")
        };
        let hit = run(vec![same]);
        assert_eq!(score(&hit, "L8"), 1000);
        let comparison = hit.frames[0]
            .q1
            .as_ref()
            .expect("q1")
            .comparison
            .expect("same size");
        assert_eq!(
            comparison,
            Comparison {
                ssim_x10000: SSIM_ONE,
                mean_de_x100: 0,
                hist_tv_x10000: 0,
                hist_emd_x10000: 0
            }
        );
        let miss = run(vec![moved]);
        assert!(
            score(&miss, "L8") < 1000 && score(&miss, "L8") > 900,
            "{}",
            score(&miss, "L8")
        );
        let skew = run(vec![wrong_size]);
        assert_eq!(score(&skew, "L8"), 0);
        assert_eq!(skew.frames[0].q1.as_ref().expect("q1").comparison, None);
        assert!(!run(Vec::new()).item("L8").expect("L8").applicable);
    }

    #[test]
    fn look_judge_refuses_sets_it_cannot_judge() {
        let cfg = JudgeConfig::default();
        let refs = References::default();
        assert_eq!(
            judge(&set_of(Vec::new()), &refs, &cfg).unwrap_err(),
            JudgeError::NoFrames
        );
        let many = set_of(
            (0..=MAX_FRAMES)
                .map(|i| arena(&format!("{i}.png"), &[]))
                .collect(),
        );
        assert_eq!(
            judge(&many, &refs, &cfg).unwrap_err(),
            JudgeError::TooMany {
                what: "frames",
                count: MAX_FRAMES + 1
            }
        );
        let huge = NamedPng {
            name: "huge.png".to_owned(),
            png: vec![0; MAX_PNG_BYTES + 1],
        };
        assert!(matches!(
            judge(&set_of(vec![huge]), &refs, &cfg),
            Err(JudgeError::TooLarge { .. })
        ));
        let garbled = CaptureSet {
            frames: vec![arena("a.png", &[])],
            manifest: Some("{{".to_owned()),
        };
        let error = judge(&garbled, &refs, &cfg).unwrap_err();
        assert!(matches!(error, JudgeError::BadManifest(_)));
        assert!(error.to_string().contains("remove manifest.json"));
        let wrong_shape = CaptureSet {
            frames: vec![arena("a.png", &[])],
            manifest: Some("{\"x\":1}".to_owned()),
        };
        assert!(matches!(
            judge(&wrong_shape, &refs, &cfg),
            Err(JudgeError::BadManifest(_))
        ));
    }

    fn scratch_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("look-judge-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn look_judge_reads_a_capture_directory_in_name_order() {
        let dir = scratch_dir("dir");
        let late = arena("frame_000001.png", &two_fighters()[..1]);
        let early = arena("frame_000000.png", &two_fighters());
        let manifest = format!(
            "[{{\"tick\":0,\"file\":\"frame_000000.png\",\"png_fnv\":\"{}\"}},{{\"tick\":60,\"file\":\"frame_000001.png\",\"png_fnv\":\"{}\"}}]",
            digest_hex(&early.png),
            digest_hex(&late.png)
        );
        std::fs::write(dir.join(&late.name), &late.png).expect("write late");
        std::fs::write(dir.join(&early.name), &early.png).expect("write early");
        std::fs::write(dir.join(MANIFEST_NAME), manifest).expect("write manifest");
        std::fs::write(dir.join("notes.txt"), "ignored").expect("write notes");
        let set = load_capture_dir(&dir).expect("loads");
        let names: Vec<&str> = set.frames.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, vec!["frame_000000.png", "frame_000001.png"]);
        let judgement =
            judge(&set, &References::default(), &JudgeConfig::default()).expect("judges");
        assert!(judgement.integrity_ok);
        assert_eq!(judgement.frames[1].manifest_tick, Some(60));
        assert_eq!(load_png_dir(&dir).expect("pngs").len(), 2);
        assert!(matches!(
            load_capture_dir(&dir.join("missing")),
            Err(JudgeError::Io { .. })
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn look_judge_reads_the_real_fighter_previews() {
        let previews = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../content/fighters/previews");
        let all = load_png_dir(&previews).expect("previews on disk");
        let picked: Vec<NamedPng> = all
            .into_iter()
            .filter(|p| p.name == "hero_rest.png" || p.name == "opp_male_light_rest.png")
            .collect();
        assert_eq!(picked.len(), 2, "the two rest renders are committed");
        let refs = References {
            fighters: picked,
            q1: Vec::new(),
        };
        let judgement = judge(
            &set_of(vec![arena("a.png", &two_fighters())]),
            &refs,
            &JudgeConfig::default(),
        )
        .expect("judges");
        assert!(judgement.item("L7").expect("L7").applicable);
        let found = judgement.frames[0]
            .preview
            .as_ref()
            .expect("a preview matched");
        assert!(found.name.ends_with("_rest.png"));
        assert!(score(&judgement, "L7") > 0);
        assert!(judgement.notes.is_empty(), "{:?}", judgement.notes);
    }

    #[test]
    fn look_metric_hook_scores_once_per_frame() {
        let rgba = rgba_of(64, 48, sand_at);
        let first = png_of(64, 48, &rgba);
        let rebuilt = png_of(64, 48, &rgba);
        let mut hook = LookHook::register(Some(&first));
        let one = hook.note_frame(&first);
        let two = hook.note_frame(&rebuilt);
        assert_eq!(hook.calls(), 2);
        assert_eq!((one.calls, two.calls), (1, 2));
        let (a, b) = (
            one.comparison.expect("scores"),
            two.comparison.expect("scores"),
        );
        assert_eq!((a.ssim_x10000, a.mean_de_x100), (SSIM_ONE, 0));
        assert_eq!(a, b);
        assert_eq!(hook.last(), Some(a));
        assert_eq!(hook.faults(), 0);
        let bad = hook.note_frame(b"not a png");
        assert_eq!((hook.calls(), hook.faults()), (3, 1));
        assert!((bad.fault, bad.comparison.is_none()) == (true, true));
        assert!(
            hook.last_fault().expect("fault words").contains("BadPng")
                || !hook.last_fault().expect("fault words").is_empty()
        );
    }
}
