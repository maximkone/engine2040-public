//! The view pack's art lane: a contact sheet of captures, a measured look
//! proposal and the ignored-region compare score.
//!
//! Consumer: `packs::view` registers the two verbs and calls
//! [`score_x10000`] from `view.compare`. Data owner: captures are read, never
//! written; [`run_sheet`] writes one sheet PNG under a relative dir and
//! [`run_propose`] writes nothing.
//! Units: pixels, path chars, hundredths of a point for scores.
//! Determinism: fixed ladders, no RNG, no clock; the same tiles give
//! byte-equal sheet bytes, and the same pair gives the same candidates twice.
//! Field docs on the transport DTOs are omitted on purpose, as in
//! `packs::view`: every doc line inflates the advertised schemas, and the
//! discovery budget is a gate.

use std::path::Path;

use engine_core::hash::hash_hex;
use engine_core::text::clip_bytes;
use engine_playtest::look::metric::{self, Frame, Region};
use engine_playtest::look::ssim2;
use rmcp::handler::server::router::tool::{SyncTool, ToolBase};
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};
use rmcp::serde_json::{self, Value};

use super::CompareRegion;
use super::{bad_args, check_dir, check_png, read_frame};
use crate::mcp::{McpServer, McpToolError};
use crate::play::play_session::encode_png_rgb;

/// Verb: tile N captures into one contact sheet plus a row per tile.
pub const TOOL_VIEW_SHEET: &str = "view.sheet";

/// Verb: three fixed art deltas, each with a predicted and a measured score.
pub const TOOL_LOOK_PROPOSE: &str = "look.propose";

/// Gutter between sheet cells, in pixels.
pub const SHEET_GUTTER: u32 = 4;

/// Caption band under every tile, in pixels.
pub const SHEET_CAPTION: u32 = 7;

/// Largest tile edge a sheet accepts, in pixels.
pub const MAX_TILE_DIM: u32 = 1024;

/// Most tiles one sheet may hold.
pub const MAX_SHEET_TILES: usize = 64;

/// Narrowest sheet, in pixels.
pub const MIN_SHEET_WIDTH: u32 = 64;

/// Widest sheet, in pixels.
pub const MAX_SHEET_WIDTH: u32 = 4096;

/// Knob names `look.propose` accepts, in ladder order.
pub const PROPOSE_KNOBS: [&str; 3] = ["exposure_ev", "key_fill", "palette_shift"];

/// The fixed ladder value of each knob: EV for `exposure_ev`, a share of the
/// way to the reference for the other two.
pub const PROPOSE_LADDER: [f64; 3] = [0.5, 0.25, 0.25];

// ---------------------------------------------------------------------------
// view.sheet
// ---------------------------------------------------------------------------

/// One tile of a contact sheet: a name, a relative PNG path and a view label.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SheetTile {
    pub name: String,
    pub image: String,
    pub view: Option<String>,
}

/// Args for `view.sheet`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ViewSheetArgs {
    pub tiles: Vec<SheetTile>,
    pub target_width: u32,
    pub dir: String,
    pub out: String,
    pub reference: Option<String>,
}

/// One contact-sheet row.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct SheetRow {
    pub name: String,
    pub image: String,
    pub view: Option<String>,
    pub width: u32,
    pub height: u32,
    pub ssimulacra2_x10000: Option<u32>,
    pub worst_tile: [u32; 4],
    pub worst_luma: u32,
}

/// `view.sheet` result.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ViewSheetOutput {
    pub png: String,
    pub digest: String,
    pub width: u32,
    pub height: u32,
    pub gutter: u32,
    pub cols: u32,
    pub rows: u32,
    pub tiles: Vec<SheetRow>,
    pub worst: Vec<String>,
}

/// Refuse a sheet request before any file is read.
fn check_sheet(args: &ViewSheetArgs) -> Result<(), McpToolError> {
    if args.tiles.is_empty() || args.tiles.len() > MAX_SHEET_TILES {
        return Err(bad_args(format!(
            "tiles holds {} entries; legal sheets hold 1..={MAX_SHEET_TILES}: re-run with a tile \
             list such as [{{name: front, image: a.png}}]",
            args.tiles.len()
        )));
    }
    if !(MIN_SHEET_WIDTH..=MAX_SHEET_WIDTH).contains(&args.target_width) {
        return Err(bad_args(format!(
            "target_width {} is outside {MIN_SHEET_WIDTH}..={MAX_SHEET_WIDTH} pixels: re-run with \
             a sheet width such as 1024",
            args.target_width
        )));
    }
    let mut seen = std::collections::BTreeSet::new();
    for tile in &args.tiles {
        let name = tile.name.trim();
        if name.is_empty() || name.chars().count() > 64 {
            return Err(bad_args(format!(
                "tile name `{}` is empty or over 64 chars: re-run with 1..=64 chars",
                clip_bytes(name, 64)
            )));
        }
        if !seen.insert(name.to_owned()) {
            return Err(bad_args(format!(
                "tile name `{name}` appears twice: re-run with one name per tile"
            )));
        }
        check_png("image", &tile.image)?;
    }
    check_dir("dir", &args.dir)?;
    check_png("out", &args.out)?;
    if args.out.contains('/') {
        return Err(bad_args(format!(
            "out `{}` must be a file name, not a path: put directories in `dir`",
            clip_bytes(&args.out, 80)
        )));
    }
    if let Some(reference) = &args.reference {
        check_png("reference", reference)?;
    }
    Ok(())
}

/// Mean luma of one block, clipped to the frame's edges.
fn block_luma(frame: &Frame, x0: u32, y0: u32, x1: u32, y1: u32) -> u32 {
    let width = usize::try_from(frame.width()).unwrap_or(0);
    let (mut sum, mut count) = (0u64, 0u64);
    for y in y0..y1 {
        for x in x0..x1 {
            let at = usize::try_from(y).unwrap_or(0) * width + usize::try_from(x).unwrap_or(0);
            sum += u64::from(metric::luma_of(frame.rgb_at(at)));
            count += 1;
        }
    }
    u32::try_from((sum + count / 2) / count.max(1)).unwrap_or(0)
}

/// The darkest 32x32 block of one frame, as `[x, y, w, h]` and its mean luma.
fn worst_tile(frame: &Frame) -> ([u32; 4], u32) {
    let (w, h) = (frame.width(), frame.height());
    let mut worst = ([0, 0, w.min(32), h.min(32)], u32::MAX);
    let mut y0 = 0;
    while y0 < h {
        let mut x0 = 0;
        while x0 < w {
            let (x1, y1) = ((x0 + 32).min(w), (y0 + 32).min(h));
            let luma = block_luma(frame, x0, y0, x1, y1);
            if luma < worst.1 {
                worst = ([x0, y0, x1 - x0, y1 - y0], luma);
            }
            x0 += 32;
        }
        y0 += 32;
    }
    worst
}

/// 3x5 digit glyphs for the caption band, one 3-bit row per line.
const DIGITS: [[u8; 5]; 10] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b010, 0b010, 0b010],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
];

/// The sheet canvas: integer pixel writes only, so two runs agree byte for byte.
struct Canvas {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

impl Canvas {
    /// A canvas of `width` x `height` filled with `fill`.
    fn new(width: u32, height: u32, fill: u8) -> Self {
        let pixels = usize::try_from(width)
            .unwrap_or(0)
            .saturating_mul(usize::try_from(height).unwrap_or(0));
        Self {
            width,
            height,
            rgb: vec![fill; pixels * 3],
        }
    }

    /// Write one pixel; an out-of-range write is dropped.
    fn put(&mut self, x: u32, y: u32, rgb: [u8; 3]) {
        if x >= self.width || y >= self.height {
            return;
        }
        let at = (usize::try_from(y).unwrap_or(0) * usize::try_from(self.width).unwrap_or(0)
            + usize::try_from(x).unwrap_or(0))
            * 3;
        self.rgb[at..at + 3].copy_from_slice(&rgb);
    }

    /// Fill one rectangle.
    fn fill(&mut self, x0: u32, y0: u32, w: u32, h: u32, rgb: [u8; 3]) {
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                self.put(x, y, rgb);
            }
        }
    }

    /// Copy a frame's RGB pixels in at `(x0, y0)`.
    fn blit(&mut self, frame: &Frame, x0: u32, y0: u32) {
        let width = usize::try_from(frame.width()).unwrap_or(0);
        for y in 0..frame.height() {
            for x in 0..frame.width() {
                let at = usize::try_from(y).unwrap_or(0) * width + usize::try_from(x).unwrap_or(0);
                self.put(x0 + x, y0 + y, frame.rgb_at(at));
            }
        }
    }

    /// Draw a decimal number in 3x5 digits, left-aligned at `(x0, y0)`.
    fn number(&mut self, mut value: u32, x0: u32, y0: u32, ink: [u8; 3]) {
        let mut digits = Vec::new();
        loop {
            digits.push(value % 10);
            value /= 10;
            if value == 0 {
                break;
            }
        }
        digits.reverse();
        for (at, digit) in digits.into_iter().enumerate() {
            let Some(glyph) = DIGITS.get(usize::try_from(digit).unwrap_or(0)) else {
                continue;
            };
            let dx = x0 + u32::try_from(at).unwrap_or(0) * 4;
            for (row, mask) in glyph.iter().enumerate() {
                for col in 0..3u32 {
                    if mask & (0b100 >> col) != 0 {
                        self.put(dx + col, y0 + u32::try_from(row).unwrap_or(0), ink);
                    }
                }
            }
        }
    }
}

/// Grid width in tiles for a target sheet width: as many cells as fit.
fn cols_for(tile_w: u32, target: u32) -> u32 {
    let cell = tile_w.saturating_add(SHEET_GUTTER).max(1);
    let cols = target.saturating_add(SHEET_GUTTER) / cell;
    cols.clamp(1, u32::try_from(MAX_SHEET_TILES).unwrap_or(64))
}

/// Rows of a grid of `count` tiles at `cols` per row.
fn grid_rows(count: usize, cols: u32) -> u32 {
    let per_row = usize::try_from(cols).unwrap_or(1).max(1);
    u32::try_from(count.div_ceil(per_row)).unwrap_or(1)
}

/// Compose the sheet: each tile in its cell, then a caption band carrying the
/// tile's request index and a bar as long as its own score.
fn compose(frames: &[Frame], scores: &[u32], cols: u32) -> (u32, u32, Canvas) {
    let (tw, th) = frames.first().map_or((1, 1), |f| (f.width(), f.height()));
    let rows = grid_rows(frames.len(), cols);
    let width = cols * tw + (cols + 1) * SHEET_GUTTER;
    let height = rows * (th + SHEET_CAPTION) + (rows + 1) * SHEET_GUTTER;
    let mut canvas = Canvas::new(width, height, 18);
    for (index, tile) in frames.iter().enumerate() {
        let at = u32::try_from(index).unwrap_or(0);
        let (col, row) = (at % cols, at / cols);
        let x0 = SHEET_GUTTER + col * (tw + SHEET_GUTTER);
        let y0 = SHEET_GUTTER + row * (th + SHEET_CAPTION + SHEET_GUTTER);
        canvas.blit(tile, x0, y0);
        canvas.fill(x0, y0 + th, tw, SHEET_CAPTION, [30, 30, 30]);
        canvas.number(at, x0 + 2, y0 + th + 1, [235, 235, 235]);
        let score = scores.get(index).copied().unwrap_or(0);
        let bar = tw.saturating_mul(score) / 10_000;
        canvas.fill(x0, y0 + th + SHEET_CAPTION - 2, bar, 2, [80, 200, 120]);
    }
    (width, height, canvas)
}

/// Read one tile, naming the row and one recovery on failure.
fn sheet_tile(root: &Path, tile: &SheetTile) -> Result<Frame, McpToolError> {
    read_frame("image", root, &tile.image).map_err(|error| {
        McpToolError::new(
            error.code,
            format!(
                "tile `{}`: {}; re-run with a capture PNG under the project root",
                clip_bytes(tile.name.trim(), 64),
                error.message
            ),
        )
    })
}

/// Score one tile against the optional reference.
fn sheet_score(
    reference: Option<&Frame>,
    frame: &Frame,
    name: &str,
) -> Result<Option<u32>, McpToolError> {
    let Some(reference) = reference else {
        return Ok(None);
    };
    ssim2::score_x10000(reference, frame)
        .map(Some)
        .map_err(|error| {
            McpToolError::new(
                "bad_image",
                format!(
                "tile `{}` does not score against the reference: {error}; re-run with a tile and \
                 a reference of one size, each side at least 8 px",
                clip_bytes(name, 64)
            ),
            )
        })
}

/// Build the sheet and write it under `dir`.
fn run_sheet(root: &Path, args: &ViewSheetArgs) -> Result<ViewSheetOutput, McpToolError> {
    check_sheet(args)?;
    let reference = match &args.reference {
        Some(rel) => Some(read_frame("reference", root, rel)?),
        None => None,
    };
    let mut frames: Vec<Frame> = Vec::with_capacity(args.tiles.len());
    let mut rows = Vec::with_capacity(args.tiles.len());
    for tile in &args.tiles {
        let frame = sheet_tile(root, tile)?;
        if frame.width() > MAX_TILE_DIM || frame.height() > MAX_TILE_DIM {
            return Err(bad_args(format!(
                "tile `{}` is {}x{}; legal tiles are at most {MAX_TILE_DIM} per side: re-capture \
                 it smaller",
                clip_bytes(tile.name.trim(), 64),
                frame.width(),
                frame.height()
            )));
        }
        if let Some(first) = frames.first() {
            let want = (first.width(), first.height());
            let got = (frame.width(), frame.height());
            if want != got {
                return Err(bad_args(format!(
                    "tile `{}` is {}x{} but the first tile is {}x{}; a sheet tiles one size: \
                     re-capture every tile at one size",
                    clip_bytes(tile.name.trim(), 64),
                    got.0,
                    got.1,
                    want.0,
                    want.1
                )));
            }
        }
        let score = sheet_score(reference.as_ref(), &frame, &tile.name)?;
        let (worst, luma) = worst_tile(&frame);
        rows.push(SheetRow {
            name: tile.name.trim().to_owned(),
            image: tile.image.clone(),
            view: tile.view.clone(),
            width: frame.width(),
            height: frame.height(),
            ssimulacra2_x10000: score,
            worst_tile: worst,
            worst_luma: luma,
        });
        frames.push(frame);
    }
    // One real score per tile: the reference score when there is a reference,
    // else the dark-tile read, so a dark tile sorts first.
    let scores: Vec<u32> = rows
        .iter()
        .map(|row| row.ssimulacra2_x10000.unwrap_or(row.worst_luma))
        .collect();
    let mut order: Vec<usize> = (0..rows.len()).collect();
    order.sort_by_key(|at| (scores[*at], *at));
    let cols = cols_for(frames.first().map_or(1, Frame::width), args.target_width);
    let (width, height, canvas) = compose(&frames, &scores, cols);
    let bytes = encode_png_rgb(width, height, &canvas.rgb);
    let path = root.join(&args.dir).join(&args.out);
    let failed = |why: std::io::Error| {
        McpToolError::new(
            "write_failed",
            format!(
                "could not write sheet `{}/{}` under the project root ({why}); re-run with a \
                 writable dir such as target/reports/sheets",
                args.dir, args.out
            ),
        )
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(failed)?;
    }
    std::fs::write(&path, &bytes).map_err(failed)?;
    Ok(ViewSheetOutput {
        png: format!("{}/{}", args.dir, args.out),
        digest: hash_hex(&bytes),
        width,
        height,
        gutter: SHEET_GUTTER,
        cols,
        rows: grid_rows(frames.len(), cols),
        worst: order.into_iter().map(|at| rows[at].name.clone()).collect(),
        tiles: rows,
    })
}

/// `view.sheet` tool: N captures into one contact-sheet PNG plus rows.
pub struct ViewSheetTool;

impl ToolBase for ViewSheetTool {
    type Parameter = ViewSheetArgs;
    type Output = ViewSheetOutput;
    type Error = McpToolError;

    fn name() -> std::borrow::Cow<'static, str> {
        TOOL_VIEW_SHEET.into()
    }

    fn description() -> Option<std::borrow::Cow<'static, str>> {
        Some(
            "Tile 1..=64 capture PNGs of one size into one contact sheet under a relative `dir`: \
             the grid width follows `target_width`, each tile gets a caption band, and every row \
             carries its name, image, view, size, ssimulacra2_x10000 against an optional \
             reference and its darkest 32x32 block; `worst` lists names lowest score first. Same \
             tiles twice give byte-equal PNG and digest. Read-only on the tiles."
                .into(),
        )
    }

    fn annotations() -> Option<rmcp::model::ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for ViewSheetTool {
    fn invoke(_service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        run_sheet(&crate::packs::codex::project_root(None), &param)
    }
}

// ---------------------------------------------------------------------------
// ignored regions: the image-diff `ignore` mechanism for view.compare
// ---------------------------------------------------------------------------

/// The scored-pixel mask of one compare: every pixel except the ignored
/// rectangles, then narrowed to the region's rect when one is named.
fn ignore_mask(
    width: u32,
    height: u32,
    region: Option<metric::Rect>,
    ignore: &[CompareRegion],
) -> Result<Vec<bool>, McpToolError> {
    let w = usize::try_from(width).unwrap_or(0);
    let h = usize::try_from(height).unwrap_or(0);
    let mut mask = vec![true; w.saturating_mul(h)];
    for entry in ignore {
        let Some(rect) = entry.rect() else {
            return Err(bad_args(
                "ignore names a full-frame region: legal entries are rectangles \
                 {\"kind\":\"rect\",\"x\":..,\"y\":..,\"w\":..,\"h\":..} inside the frame; re-run \
                 without that entry or with a rect"
                    .to_owned(),
            ));
        };
        let inside = u64::from(rect.x) + u64::from(rect.w) <= u64::from(width)
            && u64::from(rect.y) + u64::from(rect.h) <= u64::from(height);
        if rect.w == 0 || rect.h == 0 || !inside {
            return Err(bad_args(format!(
                "ignored rect {}x{} at ({},{}) does not fit the {}x{} frame; re-run with a rect \
                 inside the picture",
                rect.w, rect.h, rect.x, rect.y, width, height
            )));
        }
        for y in rect.y..rect.y + rect.h {
            for x in rect.x..rect.x + rect.w {
                let at = usize::try_from(y).unwrap_or(0) * w + usize::try_from(x).unwrap_or(0);
                mask[at] = false;
            }
        }
    }
    if let Some(rect) = region {
        for y in 0..height {
            for x in 0..width {
                let inside = x >= rect.x
                    && x < rect.x.saturating_add(rect.w)
                    && y >= rect.y
                    && y < rect.y.saturating_add(rect.h);
                let at = usize::try_from(y).unwrap_or(0) * w + usize::try_from(x).unwrap_or(0);
                if !inside {
                    mask[at] = false;
                }
            }
        }
    }
    if mask.iter().all(|scored| !scored) {
        return Err(bad_args(
            "the resolved region and the ignored rects leave no pixel to score: re-run with a \
             wider region or a smaller ignore list"
                .to_owned(),
        ));
    }
    Ok(mask)
}

/// `1 - diff/total` over the scored pixels, in hundredths of a point: a pair
/// that differs only outside the scored set reads 10000.
fn score_x10000(left: &Frame, right: &Frame, mask: &[bool]) -> u32 {
    let (mut diff, mut total) = (0u64, 0u64);
    for at in 0..left.pixels() {
        if !mask.get(at).copied().unwrap_or(false) {
            continue;
        }
        let (a, b) = (left.rgb_at(at), right.rgb_at(at));
        for channel in 0..3 {
            diff += u64::from(a[channel].abs_diff(b[channel]));
        }
        total += 3 * 255;
    }
    if total == 0 {
        return 0;
    }
    u32::try_from((total - diff.min(total)) * 10_000 / total).unwrap_or(0)
}

/// Score one pair of same-size frames over the pixels the region keeps and
/// the ignore list leaves: the masked metric block and `1 - diff/total`.
pub(super) fn compare_with_ignore(
    left: &Frame,
    right: &Frame,
    region: Option<CompareRegion>,
    ignore: &[CompareRegion],
) -> Result<(metric::Comparison, u32), McpToolError> {
    let mask = ignore_mask(
        left.width(),
        left.height(),
        region.and_then(CompareRegion::rect),
        ignore,
    )?;
    let compared = metric::compare(left, right, Region::Mask(&mask)).map_err(|error| {
        McpToolError::new(
            "bad_image",
            format!("ignored-region compare refused the pair: {error}"),
        )
    })?;
    Ok((compared, score_x10000(left, right, &mask)))
}

// ---------------------------------------------------------------------------
// look.propose
// ---------------------------------------------------------------------------

/// Args for `look.propose`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LookProposeArgs {
    pub capture: String,
    pub reference: String,
    pub knobs: Option<Vec<String>>,
    pub target: Option<String>,
}

/// One candidate delta with both numbers.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ProposeCandidate {
    pub knob: String,
    pub delta: f64,
    pub predicted_x10000: u32,
    pub measured_x10000: u32,
    pub ops: Vec<Value>,
}

/// `look.propose` result.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct LookProposeOutput {
    pub capture: String,
    pub reference: String,
    pub baseline_x10000: u32,
    pub candidates: Vec<ProposeCandidate>,
}

/// Requested knobs in ladder order; a name outside the ladder refuses.
fn check_knobs(knobs: Option<&Vec<String>>) -> Result<Vec<&'static str>, McpToolError> {
    let legal = PROPOSE_KNOBS.join("`, `");
    let wanted = knobs.cloned().unwrap_or_else(|| {
        PROPOSE_KNOBS
            .iter()
            .map(|knob| (*knob).to_owned())
            .collect()
    });
    if wanted.is_empty() {
        return Err(bad_args(format!(
            "knobs is empty; legal values are `{legal}`: re-run with some or drop the field"
        )));
    }
    let mut picked = Vec::with_capacity(wanted.len());
    for name in &wanted {
        let Some(knob) = PROPOSE_KNOBS.iter().find(|known| **known == name.as_str()) else {
            return Err(bad_args(format!(
                "knob `{}` is not one of `{legal}`: re-run with a legal knob name",
                clip_bytes(name, 32)
            )));
        };
        if !picked.contains(knob) {
            picked.push(*knob);
        }
    }
    Ok(picked)
}

/// sRGB byte to linear light in `0..=1`.
fn to_linear(c: u8) -> f32 {
    let v = f32::from(c) / 255.0;
    if v <= 0.040_45 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// Linear light back to an sRGB byte, clamped.
fn to_srgb(lin: f32) -> u8 {
    let v = if lin <= 0.003_130_8 {
        f64::from(lin) * 12.92
    } else {
        1.055 * f64::from(lin).powf(1.0 / 2.4) - 0.055
    };
    let byte = (v.clamp(0.0, 1.0) * 255.0).round();
    u8::try_from(engine_core::checked::saturating_f64_to_u64(byte)).unwrap_or(255)
}

/// Mean RGB of one frame, `0..=255` per channel.
fn mean_rgb(frame: &Frame) -> [u8; 3] {
    let pixels = u64::try_from(frame.pixels()).unwrap_or(1).max(1);
    let mut sums = [0u64; 3];
    for at in 0..frame.pixels() {
        let rgb = frame.rgb_at(at);
        for channel in 0..3 {
            sums[channel] += u64::from(rgb[channel]);
        }
    }
    sums.map(|sum| u8::try_from(sum / pixels).unwrap_or(0))
}

/// Exposure knob gain: `2^0.5` at the exact f32 the view compare has always
/// used. The decimal literal rounds one ULP above `f32::SQRT_2`; the compare
/// pins pixels, so the bits stay.
const EXPOSURE_GAIN: f32 = f32::from_bits(0x3FB5_04F4);

/// Re-grade one frame with one knob, on the CPU and with integer channels.
///
/// `exposure_ev` scales linear light by 2^0.5, `key_fill` lifts only the
/// shadows by a quarter, `palette_shift` mixes each channel a quarter of the
/// way to the reference's mean channel. No RNG, no clock.
fn regrade(capture: &Frame, reference: &Frame, knob: &str) -> Frame {
    let mean = mean_rgb(reference);
    let mut rgba = Vec::with_capacity(capture.pixels() * 4);
    for at in 0..capture.pixels() {
        let rgb = capture.rgb_at(at);
        let out = match knob {
            "exposure_ev" => rgb.map(|c| to_srgb(to_linear(c) * EXPOSURE_GAIN)),
            "key_fill" if metric::luma_of(rgb) < 128 => rgb.map(|c| to_srgb(to_linear(c) * 1.25)),
            "palette_shift" => {
                let mut mixed = rgb;
                for channel in 0..3 {
                    let from = i32::from(rgb[channel]);
                    let to = i32::from(mean[channel]);
                    mixed[channel] =
                        u8::try_from((from + (to - from) / 4).clamp(0, 255)).unwrap_or(0);
                }
                mixed
            }
            _ => rgb,
        };
        rgba.extend_from_slice(&out);
        rgba.push(255);
    }
    Frame::from_rgba(capture.width(), capture.height(), rgba).unwrap_or_else(|_| capture.clone())
}

/// `10000` minus the luma histogram earth mover's distance to the reference.
fn predicted(candidate: &Frame, reference: &Frame) -> u32 {
    metric::luma_histogram(candidate, Region::Full)
        .ok()
        .zip(metric::luma_histogram(reference, Region::Full).ok())
        .and_then(|(a, b)| metric::histogram_distance(&a, &b).ok())
        .map_or(0, |distance| 10_000u32.saturating_sub(distance.emd_x10000))
}

/// The `tx.preview` ops one knob becomes, bound to `target`.
fn knob_ops(knob: &str, target: Option<&String>) -> Vec<Value> {
    let Some(target) = target else {
        return Vec::new();
    };
    let op = match knob {
        "palette_shift" => serde_json::json!({
            "op": "set_material",
            "id": target,
            "material": "palette_shift",
        }),
        _ => {
            let intensity = if knob == "exposure_ev" { 1.414 } else { 1.25 };
            serde_json::json!({
                "op": "set_component",
                "id": target,
                "component": "light",
                "value": {
                    "kind": "directional",
                    "color": [1.0, 1.0, 1.0],
                    "intensity": intensity,
                    "range": 0,
                },
            })
        }
    };
    vec![op]
}

/// Measure the capture, then propose and measure one delta per knob.
fn run_propose(root: &Path, args: &LookProposeArgs) -> Result<LookProposeOutput, McpToolError> {
    let knobs = check_knobs(args.knobs.as_ref())?;
    check_png("capture", &args.capture)?;
    check_png("reference", &args.reference)?;
    let capture = read_frame("capture", root, &args.capture)?;
    let reference = read_frame("reference", root, &args.reference)?;
    let baseline = ssim2::score_x10000(&reference, &capture).map_err(|error| {
        McpToolError::new(
            "bad_image",
            format!(
                "capture and reference do not score together: {error}; re-run with two PNGs of \
                 one size, each side at least 8 px"
            ),
        )
    })?;
    let mut candidates = Vec::with_capacity(knobs.len());
    for knob in knobs {
        let graded = regrade(&capture, &reference, knob);
        let measured = ssim2::score_x10000(&reference, &graded).map_err(|error| {
            McpToolError::new(
                "bad_image",
                format!("the `{knob}` re-grade does not score: {error}; re-run with a capture of the reference's size"),
            )
        })?;
        let at = PROPOSE_KNOBS
            .iter()
            .position(|known| *known == knob)
            .unwrap_or(0);
        candidates.push(ProposeCandidate {
            knob: knob.to_owned(),
            delta: PROPOSE_LADDER[at],
            predicted_x10000: predicted(&graded, &reference),
            measured_x10000: measured,
            ops: knob_ops(knob, args.target.as_ref()),
        });
    }
    Ok(LookProposeOutput {
        capture: args.capture.clone(),
        reference: args.reference.clone(),
        baseline_x10000: baseline,
        candidates,
    })
}

/// `look.propose` tool: three fixed deltas, each predicted and measured.
pub struct LookProposeTool;

impl ToolBase for LookProposeTool {
    type Parameter = LookProposeArgs;
    type Output = LookProposeOutput;
    type Error = McpToolError;

    fn name() -> std::borrow::Cow<'static, str> {
        TOOL_LOOK_PROPOSE.into()
    }

    fn description() -> Option<std::borrow::Cow<'static, str>> {
        Some(
            "Score a capture against a reference, then return one fixed delta per knob \
             (exposure_ev +0.5 EV, key_fill +0.25 on the shadows, palette_shift +0.25 toward the \
             reference mean), each re-graded on the CPU with predicted_x10000 and measured_x10000 \
             side by side and tx.preview ops when `target` is named. No RNG, no clock, no model; \
             the same pair gives the same rows twice. Read-only."
                .into(),
        )
    }

    fn annotations() -> Option<rmcp::model::ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for LookProposeTool {
    fn invoke(_service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        run_propose(&crate::packs::codex::project_root(None), &param)
    }
}

// ---------------------------------------------------------------------------
// view.frame: a picture as a framing score
// ---------------------------------------------------------------------------

/// Verb: one PNG scored as a framing composition.
pub const TOOL_VIEW_FRAME: &str = "view.frame";

/// Luma gap from the border ring's tone that marks a pixel as subject, of 255.
pub const SUBJECT_LUMA_GAP: u32 = 24;

/// Horizontal plus vertical neighbour step that marks a pixel as subject, of 255.
pub const SUBJECT_EDGE: u32 = 48;

/// Ideal subject share of the frame, in permille, inclusive.
pub const COVERAGE_BAND: (u32, u32) = (120, 600);

/// Ideal top margin of the subject mass, in permille, inclusive.
pub const HEADROOM_BAND: (u32, u32) = (50, 150);

/// The four framing parts and their weights; the weights sum to 100.
pub const FRAMING_WEIGHTS: [(&str, u32); 4] = [
    ("coverage", 30),
    ("headroom", 20),
    ("thirds", 30),
    ("horizon", 20),
];

/// Args for `view.frame`: one PNG already on disk.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ViewFrameArgs {
    pub image: String,
}

/// `view.frame` result: one integer-permille reading per part plus the total.
///
/// The subject mask: a pixel is subject when its luma differs from the border
/// ring's median luma by more than [`SUBJECT_LUMA_GAP`], or when its horizontal
/// plus vertical neighbour step exceeds [`SUBJECT_EDGE`]. Weights are
/// [`FRAMING_WEIGHTS`]; each part scores 0..=1000 and `total_permille` is their
/// weighted mean. `coverage_permille` wants [`COVERAGE_BAND`] (penalty 2 per
/// permille outside), `headroom_permille` is the top row of the subject mass and
/// wants [`HEADROOM_BAND`] (penalty 3), `thirds_permille` is the subject
/// centroid's distance from the nearest third intersection in permille of each
/// axis, combined by the norm (lower is better, penalty 2), and
/// `horizon_permille` is the strongest near-horizontal row as a share of the
/// height with `horizon_tilt_mrad` from the left and right subject masses (it
/// wants a third line, within 333 or 667, penalty 3, plus half the tilt).
/// `worst_part` names the part with the lowest score.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct ViewFrameOutput {
    pub width: u32,
    pub height: u32,
    pub subject_pixels: u32,
    pub coverage_permille: u32,
    pub headroom_permille: u32,
    pub thirds_permille: u32,
    pub horizon_permille: u32,
    pub horizon_tilt_mrad: i32,
    pub total_permille: u32,
    pub worst_part: String,
}

/// Integer square root of a `u64`.
fn isqrt(value: u64) -> u64 {
    let mut root = 0u64;
    let mut bit = 1u64 << 62;
    let mut rest = value;
    while bit > rest {
        bit >>= 2;
    }
    while bit != 0 {
        if rest >= root + bit {
            rest -= root + bit;
            root = (root >> 1) + bit;
        } else {
            root >>= 1;
        }
        bit >>= 2;
    }
    root
}

/// One luma sample, clamped to the frame.
fn luma_at(luma: &[u8], width: usize, height: usize, x: usize, y: usize) -> u32 {
    let px = x.min(width.saturating_sub(1));
    let py = y.min(height.saturating_sub(1));
    luma.get(py * width + px).copied().map_or(0, u32::from)
}

/// Horizontal plus vertical neighbour step at one pixel.
fn edge_at(luma: &[u8], width: usize, height: usize, x: usize, y: usize) -> u32 {
    let dx = luma_at(luma, width, height, x.saturating_add(1), y).abs_diff(luma_at(
        luma,
        width,
        height,
        x.saturating_sub(1),
        y,
    ));
    let dy = luma_at(luma, width, height, x, y.saturating_add(1)).abs_diff(luma_at(
        luma,
        width,
        height,
        x,
        y.saturating_sub(1),
    ));
    dx.saturating_add(dy)
}

/// Median luma of the frame's border ring: the background tone.
fn border_median(luma: &[u8], width: usize, height: usize) -> u8 {
    let mut hist = [0u32; 256];
    let mut total = 0u32;
    let mut push = |x: usize, y: usize, hist: &mut [u32; 256], total: &mut u32| {
        let value = luma_at(luma, width, height, x, y);
        if let Some(slot) = hist.get_mut(usize::try_from(value).unwrap_or(0)) {
            *slot += 1;
            *total += 1;
        }
    };
    for x in 0..width {
        for y in [0, height.saturating_sub(1)] {
            push(x, y, &mut hist, &mut total);
        }
    }
    for y in 1..height.saturating_sub(1) {
        for x in [0, width.saturating_sub(1)] {
            push(x, y, &mut hist, &mut total);
        }
    }
    let half = total / 2;
    let mut seen = 0u32;
    for (value, count) in hist.iter().enumerate() {
        seen = seen.saturating_add(*count);
        if seen > half {
            return u8::try_from(value).unwrap_or(0);
        }
    }
    0
}

/// The documented subject mask: tone gap from the border ring or a strong step.
fn subject_mask(frame: &Frame) -> Vec<bool> {
    let luma = frame.luma_plane();
    let width = usize::try_from(frame.width()).unwrap_or(0);
    let height = usize::try_from(frame.height()).unwrap_or(0);
    let tone = u32::from(border_median(&luma, width, height));
    let mut mask = vec![false; width.saturating_mul(height)];
    for y in 0..height {
        for x in 0..width {
            let value = luma_at(&luma, width, height, x, y);
            let gap = value.abs_diff(tone);
            let subject =
                gap > SUBJECT_LUMA_GAP || edge_at(&luma, width, height, x, y) > SUBJECT_EDGE;
            if let Some(slot) = mask.get_mut(y * width + x) {
                *slot = subject;
            }
        }
    }
    mask
}

/// Per-row edge energy from luma: the horizontal step over the full row.
fn luma_row_energy(luma: &[u8], width: usize, height: usize) -> Vec<u32> {
    let mut energy = vec![0u32; height];
    for (y, slot) in energy.iter_mut().enumerate() {
        if y == 0 || y + 1 >= height {
            continue;
        }
        let mut sum = 0u32;
        for x in 0..width {
            sum = sum.saturating_add(luma_at(luma, width, height, x, y + 1).abs_diff(luma_at(
                luma,
                width,
                height,
                x,
                y.saturating_sub(1),
            )));
        }
        *slot = sum;
    }
    energy
}

/// Subject pixels per row.
fn row_mass(mask: &[bool], width: usize, height: usize) -> Vec<u32> {
    let mut mass = vec![0u32; height];
    for (y, slot) in mass.iter_mut().enumerate() {
        let mut count = 0u32;
        for x in 0..width {
            if mask.get(y * width + x).copied().unwrap_or(false) {
                count = count.saturating_add(1);
            }
        }
        *slot = count;
    }
    mass
}

/// Per-row edge energy from the mask mass: the step of the subject count.
fn mass_row_energy(mass: &[u32]) -> Vec<u32> {
    let mut energy = vec![0u32; mass.len()];
    for (y, slot) in energy.iter_mut().enumerate() {
        let above = y.checked_sub(1).and_then(|at| mass.get(at)).copied();
        let below = mass.get(y + 1).copied();
        if let (Some(above), Some(below)) = (above, below) {
            *slot = above.abs_diff(below);
        }
    }
    energy
}

/// The strongest near-horizontal row (permille of the height) and the tilt in
/// milliradians between the left and right subject masses.
fn horizon_of(mask: &[bool], width: usize, height: usize, row_energy: &[u32]) -> (u32, i32) {
    let mut best = (0usize, 0u32);
    for y in 1..height.saturating_sub(1) {
        let energy = row_energy.get(y).copied().unwrap_or(0);
        if energy > best.1 {
            best = (y, energy);
        }
    }
    let horizon = if best.1 == 0 {
        500
    } else {
        u32::try_from(best.0).unwrap_or(0).saturating_mul(1000)
            / u32::try_from(height).unwrap_or(1).max(1)
    };
    let rows = |from: usize, to: usize| -> Option<u32> {
        let (mut sum, mut count) = (0u64, 0u64);
        for y in 0..height {
            for x in from..to.min(width) {
                if mask.get(y * width + x).copied().unwrap_or(false) {
                    sum += u64::try_from(y).unwrap_or(0);
                    count += 1;
                }
            }
        }
        (count > 0).then(|| u32::try_from(sum.saturating_mul(1000) / count).unwrap_or(0))
    };
    let half = width / 2;
    let tilt = match (rows(0, half), rows(half, width)) {
        (Some(left), Some(right)) => {
            i32::try_from(right).unwrap_or(0) - i32::try_from(left).unwrap_or(0)
        }
        _ => 0,
    };
    (horizon, tilt.saturating_mul(2))
}

/// A band score: 1000 inside the band, minus `penalty` per permille outside.
fn band_score(value: u32, band: (u32, u32), penalty: u32) -> u32 {
    let outside = band.0.saturating_sub(value) + value.saturating_sub(band.1);
    1000u32.saturating_sub(outside.saturating_mul(penalty))
}

/// Score one subject mask: the four parts, the weighted total, the worst part.
pub(crate) fn framing_of(
    mask: &[bool],
    width: u32,
    height: u32,
    row_energy: &[u32],
) -> ViewFrameOutput {
    let (w, h) = (
        usize::try_from(width).unwrap_or(0),
        usize::try_from(height).unwrap_or(0),
    );
    let subject = mask.iter().filter(|pixel| **pixel).count();
    if subject == 0 || w == 0 || h == 0 {
        return ViewFrameOutput {
            width,
            height,
            subject_pixels: 0,
            coverage_permille: 0,
            headroom_permille: 0,
            thirds_permille: 0,
            horizon_permille: 0,
            horizon_tilt_mrad: 0,
            total_permille: 0,
            worst_part: FRAMING_WEIGHTS[0].0.to_owned(),
        };
    }
    let total_pixels = w.saturating_mul(h).max(1);
    let coverage = u32::try_from(
        u64::try_from(subject).unwrap_or(0).saturating_mul(1000)
            / u64::try_from(total_pixels).unwrap_or(1),
    )
    .unwrap_or(0);
    let first_row = mask.iter().position(|pixel| *pixel).unwrap_or(0) / w.max(1);
    let headroom = u32::try_from(first_row).unwrap_or(0).saturating_mul(1000) / height.max(1);
    let mut sum_x = 0u64;
    let mut sum_y = 0u64;
    for y in 0..h {
        for x in 0..w {
            if !mask.get(y * w + x).copied().unwrap_or(false) {
                continue;
            }
            sum_x += u64::try_from(x).unwrap_or(0);
            sum_y += u64::try_from(y).unwrap_or(0);
        }
    }
    let count = u64::try_from(subject).unwrap_or(1).max(1);
    let span_x = u64::try_from(w).unwrap_or(1).max(1);
    let span_y = u64::try_from(h).unwrap_or(1).max(1);
    let center_x =
        u32::try_from(sum_x.saturating_mul(1000) / count.saturating_mul(span_x)).unwrap_or(0);
    let center_y =
        u32::try_from(sum_y.saturating_mul(1000) / count.saturating_mul(span_y)).unwrap_or(0);
    let dx = i64::from(center_x)
        .saturating_sub(333)
        .abs()
        .min(i64::from(center_x).saturating_sub(667).abs());
    let dy = i64::from(center_y)
        .saturating_sub(333)
        .abs()
        .min(i64::from(center_y).saturating_sub(667).abs());
    let thirds = u32::try_from(isqrt(
        u64::try_from(dx.saturating_mul(dx) + dy.saturating_mul(dy)).unwrap_or(0),
    ))
    .unwrap_or(0);
    let (horizon, tilt) = horizon_of(mask, w, h, row_energy);
    let coverage_score = band_score(coverage, COVERAGE_BAND, 2);
    let headroom_score = band_score(headroom, HEADROOM_BAND, 3);
    let thirds_score = 1000u32.saturating_sub(thirds.saturating_mul(2));
    let horizon_gap = u32::try_from(
        i64::from(horizon)
            .saturating_sub(333)
            .abs()
            .min(i64::from(horizon).saturating_sub(667).abs()),
    )
    .unwrap_or(0);
    let horizon_score = 1000u32
        .saturating_sub(horizon_gap.saturating_mul(3))
        .saturating_sub(u32::try_from(tilt.unsigned_abs()).unwrap_or(0) / 2);
    let scores = [coverage_score, headroom_score, thirds_score, horizon_score];
    let mut total = 0u32;
    let mut worst = (scores[0], FRAMING_WEIGHTS[0].0);
    for (at, (name, weight)) in FRAMING_WEIGHTS.iter().enumerate() {
        total = total.saturating_add(scores[at].saturating_mul(*weight) / 100);
        if scores[at] < worst.0 {
            worst = (scores[at], name);
        }
    }
    ViewFrameOutput {
        width,
        height,
        subject_pixels: u32::try_from(subject).unwrap_or(0),
        coverage_permille: coverage,
        headroom_permille: headroom,
        thirds_permille: thirds,
        horizon_permille: horizon,
        horizon_tilt_mrad: tilt,
        total_permille: total.min(1000),
        worst_part: worst.1.to_owned(),
    }
}

/// A subject mask holding one box, clamped to the frame: the camera frame
/// self-check's stand-in for a picture.
pub(crate) fn box_mask(width: u32, height: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> Vec<bool> {
    let w = usize::try_from(width).unwrap_or(0);
    let h = usize::try_from(height).unwrap_or(0);
    let mut mask = vec![false; w.saturating_mul(h)];
    let last_x = width.saturating_sub(1);
    let last_y = height.saturating_sub(1);
    for y in y0.min(last_y)..=y1.min(last_y) {
        for x in x0.min(last_x)..=x1.min(last_x) {
            let at = usize::try_from(y).unwrap_or(0) * w + usize::try_from(x).unwrap_or(0);
            if let Some(slot) = mask.get_mut(at) {
                *slot = true;
            }
        }
    }
    mask
}

/// Row energy from the mask's own mass, for a mask-shaped subject.
pub(crate) fn box_row_energy(mask: &[bool], width: u32, height: u32) -> Vec<u32> {
    let w = usize::try_from(width).unwrap_or(0);
    let h = usize::try_from(height).unwrap_or(0);
    mass_row_energy(&row_mass(mask, w, h))
}

/// Read one PNG and score its framing.
fn run_view_frame(root: &Path, args: &ViewFrameArgs) -> Result<ViewFrameOutput, McpToolError> {
    check_png("image", &args.image)?;
    let frame = read_frame("image", root, &args.image).map_err(|error| {
        McpToolError::new(
            error.code,
            format!(
                "{}; re-run with a capture PNG under the project root",
                error.message
            ),
        )
    })?;
    let mask = subject_mask(&frame);
    let width = usize::try_from(frame.width()).unwrap_or(0);
    let height = usize::try_from(frame.height()).unwrap_or(0);
    let luma = frame.luma_plane();
    let energy = luma_row_energy(&luma, width, height);
    Ok(framing_of(&mask, frame.width(), frame.height(), &energy))
}

/// `view.frame` tool: one PNG scored as a framing composition.
pub struct ViewFrameTool;

impl ToolBase for ViewFrameTool {
    type Parameter = ViewFrameArgs;
    type Output = ViewFrameOutput;
    type Error = McpToolError;

    fn name() -> std::borrow::Cow<'static, str> {
        TOOL_VIEW_FRAME.into()
    }

    fn description() -> Option<std::borrow::Cow<'static, str>> {
        Some(
            "Score one capture PNG's composition in integer permille: coverage (subject share), \
             headroom (top row of the subject mass), thirds (centroid distance from the nearest \
             third intersection, lower is better) and horizon (strongest near-horizontal row and \
             its tilt), plus a weighted total and the worst part. The subject mask and the weight \
             table are documented on the output; the same file twice gives the same numbers. \
             Read-only."
                .into(),
        )
    }

    fn annotations() -> Option<rmcp::model::ToolAnnotations> {
        crate::mcp::read_only_annotations()
    }
}

impl SyncTool<McpServer> for ViewFrameTool {
    fn invoke(_service: &McpServer, param: Self::Parameter) -> Result<Self::Output, Self::Error> {
        run_view_frame(&crate::packs::codex::project_root(None), &param)
    }
}

#[cfg(test)]
mod tests {
    use super::super::ViewCompareArgs;
    use super::*;

    /// A fresh directory under the system temp dir.
    fn scratch(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("e2040-art-{tag}-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch dir");
        dir
    }

    /// One 32x32 frame of one flat colour.
    fn flat(rgb: [u8; 3]) -> Frame {
        let mut rgba = Vec::with_capacity(32 * 32 * 4);
        for _ in 0..32 * 32 {
            rgba.extend_from_slice(&rgb);
            rgba.push(255);
        }
        Frame::from_rgba(32, 32, rgba).expect("32x32 frame")
    }

    /// Write a frame as PNG at `dir/name`.
    fn write_png(dir: &Path, name: &str, frame: &Frame) {
        let mut rgb = Vec::with_capacity(frame.pixels() * 3);
        for at in 0..frame.pixels() {
            rgb.extend_from_slice(&frame.rgb_at(at));
        }
        std::fs::write(
            dir.join(name),
            encode_png_rgb(frame.width(), frame.height(), &rgb),
        )
        .expect("png written");
    }

    fn tile(name: &str, image: &str, view: Option<&str>) -> SheetTile {
        SheetTile {
            name: name.to_owned(),
            image: image.to_owned(),
            view: view.map(str::to_owned),
        }
    }

    #[test]
    fn sheet_is_byte_equal_twice_and_lists_the_dark_tile_first() {
        let dir = scratch("sheet");
        write_png(&dir, "light.png", &flat([200, 200, 200]));
        write_png(&dir, "dark.png", &flat([10, 10, 10]));
        let args = ViewSheetArgs {
            tiles: vec![
                tile("light", "light.png", Some("front")),
                tile("dark", "dark.png", None),
            ],
            target_width: 128,
            dir: ".".to_owned(),
            out: "sheet.png".to_owned(),
            reference: None,
        };
        let first = run_sheet(&dir, &args).expect("sheet builds");
        let bytes = std::fs::read(dir.join("sheet.png")).expect("sheet read");
        let second = run_sheet(&dir, &args).expect("sheet builds twice");
        assert_eq!(first, second);
        assert_eq!(
            std::fs::read(dir.join("sheet.png")).expect("read again"),
            bytes
        );
        assert_eq!((first.cols, first.rows), (3, 1));
        assert!(first.width <= args.target_width, "{}", first.width);
        assert_eq!(first.worst, vec!["dark".to_owned(), "light".to_owned()]);
        assert_eq!(first.tiles[0].view.as_deref(), Some("front"));
        assert!(first
            .tiles
            .iter()
            .all(|row| row.ssimulacra2_x10000.is_none()));
        assert!(first.tiles[1].worst_luma < first.tiles[0].worst_luma);
        assert_eq!(first.digest.len(), 16);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sheet_scores_every_tile_against_a_reference() {
        let dir = scratch("sheet-ref");
        write_png(&dir, "a.png", &flat([40, 40, 40]));
        write_png(&dir, "b.png", &flat([200, 200, 200]));
        let args = ViewSheetArgs {
            tiles: vec![tile("a", "a.png", None), tile("b", "b.png", None)],
            target_width: 64,
            dir: ".".to_owned(),
            out: "sheet.png".to_owned(),
            reference: Some("b.png".to_owned()),
        };
        let out = run_sheet(&dir, &args).expect("scored sheet");
        assert!(out.tiles.iter().all(|row| row.ssimulacra2_x10000.is_some()));
        assert_eq!(out.worst[0], "a");
        assert_eq!(out.worst[1], "b");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn sheet_refuses_a_bad_out_path_and_a_missing_tile() {
        let dir = scratch("sheet-bad");
        let mut args = ViewSheetArgs {
            tiles: vec![tile("gone", "gone.png", None)],
            target_width: 128,
            dir: ".".to_owned(),
            out: "sheet.png".to_owned(),
            reference: None,
        };
        let error = run_sheet(&dir, &args).expect_err("missing tile");
        assert_eq!(error.code, "bad_args");
        assert!(error.message.contains("gone.png"), "{}", error.message);
        assert!(error.message.contains("re-run"), "{}", error.message);
        for bad in ["..", "/abs", "a/../b", "C:\\x"] {
            args.dir = bad.to_owned();
            let error = run_sheet(&dir, &args).expect_err("bad dir");
            assert_eq!(error.code, "bad_args", "{bad}");
            assert!(error.message.contains("re-run"), "{bad}");
        }
        args.dir = ".".to_owned();
        args.target_width = 8;
        assert_eq!(run_sheet(&dir, &args).expect_err("narrow").code, "bad_args");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn propose_returns_three_rows_with_both_numbers_twice() {
        let dir = scratch("propose");
        write_png(&dir, "reference.png", &flat([180, 120, 90]));
        write_png(&dir, "capture.png", &flat([60, 40, 30]));
        let args = LookProposeArgs {
            capture: "capture.png".to_owned(),
            reference: "reference.png".to_owned(),
            knobs: None,
            target: None,
        };
        let first = run_propose(&dir, &args).expect("proposes");
        let second = run_propose(&dir, &args).expect("proposes twice");
        assert_eq!(first, second);
        let knobs: Vec<&str> = first.candidates.iter().map(|c| c.knob.as_str()).collect();
        assert_eq!(knobs, PROPOSE_KNOBS);
        assert!(first.candidates.iter().all(|c| c.ops.is_empty()));
        assert!(first.candidates.iter().all(|c| c.measured_x10000 <= 10_000));
        // Every prediction carries a measurement, and they are separate numbers.
        assert!(
            first
                .candidates
                .iter()
                .any(|c| c.measured_x10000 != c.predicted_x10000),
            "prediction and measurement must differ somewhere: {:?}",
            first.candidates
        );
        assert!(
            first.candidates[0].measured_x10000 >= first.baseline_x10000,
            "exposure measured {} against baseline {}",
            first.candidates[0].measured_x10000,
            first.baseline_x10000
        );
        let one = LookProposeArgs {
            knobs: Some(vec!["palette_shift".to_owned()]),
            target: Some("00000000-0000-0000-0000-000000000001".to_owned()),
            ..args.clone()
        };
        let out = run_propose(&dir, &one).expect("one knob");
        assert_eq!(out.candidates.len(), 1);
        assert_eq!(out.candidates[0].ops[0]["op"], "set_material");
        let bad = LookProposeArgs {
            knobs: Some(vec!["exposure".to_owned()]),
            ..args
        };
        let error = run_propose(&dir, &bad).expect_err("unknown knob");
        assert_eq!(error.code, "bad_args");
        assert!(error.message.contains("exposure_ev"), "{}", error.message);
        assert!(error.message.contains("re-run"), "{}", error.message);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A 32x32 grey frame with a brighter 16x16 patch at (8, 8).
    fn patched() -> Frame {
        let mut rgba = Vec::with_capacity(32 * 32 * 4);
        for at in 0..32 * 32 {
            let (x, y) = (at % 32, at / 32);
            let inside = (8..24).contains(&x) && (8..24).contains(&y);
            let value = if inside { 220 } else { 90 };
            rgba.extend_from_slice(&[value, value, value, 255]);
        }
        Frame::from_rgba(32, 32, rgba).expect("frame")
    }

    #[test]
    fn view_compare_ignore_scores_the_pixels_it_keeps() {
        let dir = scratch("compare-ignore");
        write_png(&dir, "same.png", &flat([90, 90, 90]));
        write_png(&dir, "changed.png", &patched());
        let with_ignore = ViewCompareArgs {
            left: "same.png".to_owned(),
            right: "changed.png".to_owned(),
            diff: None,
            region: None,
            ignore: Some(vec![CompareRegion::Rect {
                x: 8,
                y: 8,
                w: 16,
                h: 16,
            }]),
        };
        let out = super::super::run_view_compare(&dir, &with_ignore).expect("compare");
        assert_eq!(out.score_x10000, Some(10_000), "{out:?}");
        assert_eq!(out.ssim_x10000, metric::SSIM_ONE);
        assert_eq!(out.ssimulacra2_x10000, None);
        let plain = ViewCompareArgs {
            ignore: None,
            ..with_ignore
        };
        let out = super::super::run_view_compare(&dir, &plain).expect("plain compare");
        assert_eq!(out.score_x10000, None);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn frame_prefers_a_third_over_the_centre() {
        let dir = scratch("frame-thirds");
        write_png(&dir, "centre.png", &square_frame(18, 18));
        write_png(&dir, "third.png", &square_frame(10, 10));
        let read = |name: &str| {
            run_view_frame(
                &dir,
                &ViewFrameArgs {
                    image: name.to_owned(),
                },
            )
            .expect("frame scores")
        };
        let centre = read("centre.png");
        let third = read("third.png");
        assert!(
            third.thirds_permille < centre.thirds_permille,
            "third {} vs centre {}",
            third.thirds_permille,
            centre.thirds_permille
        );
        assert!(
            third.total_permille > centre.total_permille,
            "third {third:?} vs centre {centre:?}"
        );
        assert_eq!(read("centre.png"), centre);
        assert_eq!(read("third.png"), third);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn frame_loses_headroom_when_the_subject_touches_the_top() {
        let dir = scratch("frame-headroom");
        write_png(&dir, "touching.png", &square_frame(18, 0));
        write_png(&dir, "inside.png", &square_frame(18, 6));
        let touching = run_view_frame(
            &dir,
            &ViewFrameArgs {
                image: "touching.png".to_owned(),
            },
        )
        .expect("touching frame");
        let inside = run_view_frame(
            &dir,
            &ViewFrameArgs {
                image: "inside.png".to_owned(),
            },
        )
        .expect("inside frame");
        assert_eq!(touching.headroom_permille, 0);
        assert!(inside.headroom_permille <= HEADROOM_BAND.1, "{inside:?}");
        assert_eq!(touching.worst_part, "headroom");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn frame_refuses_a_missing_or_non_png_file() {
        let dir = scratch("frame-missing");
        let missing = run_view_frame(
            &dir,
            &ViewFrameArgs {
                image: "gone.png".to_owned(),
            },
        )
        .expect_err("missing file");
        assert_eq!(missing.code, "bad_args");
        assert!(missing.message.contains("gone.png"), "{}", missing.message);
        assert!(missing.message.contains("re-run"), "{}", missing.message);
        let not_png = run_view_frame(
            &dir,
            &ViewFrameArgs {
                image: "frame.jpg".to_owned(),
            },
        )
        .expect_err("not a png");
        assert_eq!(not_png.code, "bad_args");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A 48x48 frame: a 12x12 light square at `(x0, y0)` on a 40-grey field.
    fn square_frame(x0: u32, y0: u32) -> Frame {
        let mut rgba = Vec::with_capacity(48 * 48 * 4);
        for y in 0..48u32 {
            for x in 0..48u32 {
                let inside = x >= x0 && x < x0 + 12 && y >= y0 && y < y0 + 12;
                let value = if inside { 220 } else { 40 };
                rgba.extend_from_slice(&[value, value, value, 255]);
            }
        }
        Frame::from_rgba(48, 48, rgba).expect("48x48 frame")
    }

    #[test]
    fn ignored_rect_holds_the_only_difference_and_scores_one() {
        let same = flat([90, 90, 90]);
        let mut rgba = Vec::with_capacity(32 * 32 * 4);
        for at in 0..32 * 32 {
            let (x, y) = (at % 32, at / 32);
            let inside = (8..24).contains(&x) && (8..24).contains(&y);
            let value = if inside { 220 } else { 90 };
            rgba.extend_from_slice(&[value, value, value, 255]);
        }
        let changed = Frame::from_rgba(32, 32, rgba).expect("frame");
        let ignore = [CompareRegion::Rect {
            x: 8,
            y: 8,
            w: 16,
            h: 16,
        }];
        assert_eq!(
            compare_with_ignore(&same, &changed, None, &ignore)
                .map(|(_, score)| score)
                .expect("scored"),
            10_000
        );
        // The scored metrics ignore the rect: SSIM reads identical and deltaE
        // zero. FLIP pools over neighbourhoods, so it is not asserted here.
        let (compared, _) = compare_with_ignore(&same, &changed, None, &ignore).expect("masked");
        assert_eq!(compared.ssim_x10000, metric::SSIM_ONE);
        assert_eq!(compared.mean_de_x100, 0);
        assert!(
            compare_with_ignore(&same, &changed, None, &[])
                .map(|(_, score)| score)
                .expect("all")
                < 10_000
        );
        // A full-frame ignore, or one outside the picture, refuses.
        assert_eq!(
            compare_with_ignore(&same, &changed, None, &[CompareRegion::Full])
                .expect_err("full ignore")
                .code,
            "bad_args"
        );
        assert_eq!(
            compare_with_ignore(
                &same,
                &changed,
                None,
                &[CompareRegion::Rect {
                    x: 30,
                    y: 0,
                    w: 8,
                    h: 8
                }]
            )
            .expect_err("outside")
            .code,
            "bad_args"
        );
        // Region and ignore together: the region shrinks the scored set.
        let region = Some(CompareRegion::Rect {
            x: 0,
            y: 0,
            w: 8,
            h: 8,
        });
        assert_eq!(
            compare_with_ignore(&same, &changed, region, &ignore)
                .map(|(_, score)| score)
                .expect("region"),
            10_000
        );
    }
}
