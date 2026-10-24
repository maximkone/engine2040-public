//! `anim.retarget.preview`: the mapped skeleton as one real PNG frame.
//!
//! The bone map comes from `super::retarget`, unchanged. The frame is a
//! software raster: 16 humanoid bones as two-pixel-wide segments, coloured by
//! how each row was built, plus a 3x3 joint block. Integer pixel writes only,
//! so the same map always gives the same PNG bytes.

use engine_anim::contract::BoneRole;
use rmcp::schemars::JsonSchema;
use rmcp::serde::{Deserialize, Serialize};

use super::{retarget, BoneMapDto, RetargetArgs};
use crate::mcp::{McpServer, McpToolError};
use crate::play::play_session::encode_png_rgb;

/// The verb name: one scored bone map drawn as one real frame.
pub const TOOL_ANIM_RETARGET_PREVIEW: &str = "anim.retarget.preview";
/// Frame side when the call names none, in pixels.
const PREVIEW_DEFAULT_SIZE: u32 = 256;
/// Largest frame side the software raster draws: 512x512 is the software cap.
pub(super) const PREVIEW_MAX_SIZE: u32 = 512;
/// Frame background, so every lit pixel is a bone pixel.
const PREVIEW_BACK: [u8; 3] = [12, 14, 20];
/// Colour of a row read straight from a role name.
const PREVIEW_EXACT: [u8; 3] = [70, 200, 120];
/// Colour of a row the family alias table guessed.
const PREVIEW_ALIAS: [u8; 3] = [230, 170, 60];
/// Colour of a row a caller wrote by hand.
const PREVIEW_MANUAL: [u8; 3] = [80, 150, 240];

/// The humanoid tree: 16 bone edges, parent then child, over the 17 roles.
const SKELETON_EDGES: [(BoneRole, BoneRole); 16] = [
    (BoneRole::Root, BoneRole::Pelvis),
    (BoneRole::Pelvis, BoneRole::SpineLower),
    (BoneRole::SpineLower, BoneRole::SpineUpper),
    (BoneRole::SpineUpper, BoneRole::Head),
    (BoneRole::SpineUpper, BoneRole::UpperArmLeft),
    (BoneRole::UpperArmLeft, BoneRole::LowerArmLeft),
    (BoneRole::LowerArmLeft, BoneRole::HandLeft),
    (BoneRole::SpineUpper, BoneRole::UpperArmRight),
    (BoneRole::UpperArmRight, BoneRole::LowerArmRight),
    (BoneRole::LowerArmRight, BoneRole::HandRight),
    (BoneRole::Pelvis, BoneRole::UpperLegLeft),
    (BoneRole::UpperLegLeft, BoneRole::LowerLegLeft),
    (BoneRole::LowerLegLeft, BoneRole::FootLeft),
    (BoneRole::Pelvis, BoneRole::UpperLegRight),
    (BoneRole::UpperLegRight, BoneRole::LowerLegRight),
    (BoneRole::LowerLegRight, BoneRole::FootRight),
];

/// Where one role sits in a reference T-pose, in per-mille of the frame.
/// Left limbs land on the high-x side, so the figure faces the viewer.
fn role_anchor(role: BoneRole) -> (i64, i64) {
    match role {
        BoneRole::Root => (500, 470),
        BoneRole::Pelvis => (500, 400),
        BoneRole::SpineLower => (500, 330),
        BoneRole::SpineUpper => (500, 230),
        BoneRole::Head => (500, 90),
        BoneRole::UpperArmLeft => (690, 240),
        BoneRole::UpperArmRight => (310, 240),
        BoneRole::LowerArmLeft => (830, 240),
        BoneRole::LowerArmRight => (170, 240),
        BoneRole::HandLeft => (930, 240),
        BoneRole::HandRight => (70, 240),
        BoneRole::UpperLegLeft => (620, 520),
        BoneRole::UpperLegRight => (380, 520),
        BoneRole::LowerLegLeft => (620, 720),
        BoneRole::LowerLegRight => (380, 720),
        BoneRole::FootLeft => (640, 920),
        BoneRole::FootRight => (360, 920),
    }
}

/// A role's anchor scaled into frame pixels; integer math only, so two runs
/// write the same bytes.
fn anchor_pixel(anchor: (i64, i64), width: u32, height: u32) -> (i64, i64) {
    let span_x = i64::from(width).max(1) - 1;
    let span_y = i64::from(height).max(1) - 1;
    (anchor.0 * span_x / 1000, anchor.1 * span_y / 1000)
}

/// The colour a row draws in, decided by how the row was built.
fn preview_color(method: &str) -> [u8; 3] {
    match method {
        "exact" => PREVIEW_EXACT,
        "manual" => PREVIEW_MANUAL,
        _ => PREVIEW_ALIAS,
    }
}

/// One RGB frame. Integer pixel writes only, so two runs agree byte for byte.
struct PreviewCanvas {
    width: u32,
    height: u32,
    rgb: Vec<u8>,
}

impl PreviewCanvas {
    /// A `width` x `height` frame filled with the background.
    fn new(width: u32, height: u32) -> Self {
        let pixels = usize::try_from(width)
            .unwrap_or(0)
            .saturating_mul(usize::try_from(height).unwrap_or(0));
        let mut rgb = vec![0u8; pixels * 3];
        for chunk in rgb.chunks_exact_mut(3) {
            chunk.copy_from_slice(&PREVIEW_BACK);
        }
        Self { width, height, rgb }
    }

    /// Write one pixel; anything outside the frame is dropped.
    fn put(&mut self, x: i64, y: i64, rgb: [u8; 3]) {
        let (Ok(col), Ok(row)) = (usize::try_from(x), usize::try_from(y)) else {
            return;
        };
        let (Ok(width), Ok(height)) = (usize::try_from(self.width), usize::try_from(self.height))
        else {
            return;
        };
        if col >= width || row >= height {
            return;
        }
        let at = (row * width + col) * 3;
        self.rgb[at..at + 3].copy_from_slice(&rgb);
    }

    /// Draw one bone as a two-pixel-wide segment between two pixel points.
    fn segment(&mut self, from: (i64, i64), to: (i64, i64), rgb: [u8; 3]) {
        let dx = to.0 - from.0;
        let dy = to.1 - from.1;
        let steps = dx.abs().max(dy.abs()).max(1);
        for step in 0..=steps {
            let x = from.0 + dx * step / steps;
            let y = from.1 + dy * step / steps;
            for (ox, oy) in [(0i64, 0i64), (1, 0), (0, 1), (1, 1)] {
                self.put(x + ox, y + oy, rgb);
            }
        }
    }

    /// Paint a 3x3 joint block centred on one pixel.
    fn dot(&mut self, at: (i64, i64), rgb: [u8; 3]) {
        for oy in -1..=1 {
            for ox in -1..=1 {
                self.put(at.0 + ox, at.1 + oy, rgb);
            }
        }
    }

    /// Pixels that are not the background.
    fn lit_pixels(&self) -> u32 {
        let mut lit = 0u32;
        for chunk in self.rgb.chunks_exact(3) {
            if !chunk.starts_with(&PREVIEW_BACK) {
                lit = lit.saturating_add(1);
            }
        }
        lit
    }
}

/// Draw one map: every edge whose parent and child both carry a row. The
/// child's method colours the bone, so a guessed arm reads differently from a
/// hand-written one.
fn draw_map(rows: &[BoneMapDto], width: u32, height: u32) -> (PreviewCanvas, u32) {
    let color_of = |role: BoneRole| {
        rows.iter()
            .find(|row| row.role == role.name())
            .map(|row| preview_color(&row.method))
    };
    let mut canvas = PreviewCanvas::new(width, height);
    let mut edges = 0u32;
    for (parent, child) in SKELETON_EDGES {
        let Some(child_color) = color_of(child).filter(|_| color_of(parent).is_some()) else {
            continue;
        };
        let from = anchor_pixel(role_anchor(parent), width, height);
        let to = anchor_pixel(role_anchor(child), width, height);
        canvas.segment(from, to, child_color);
        canvas.dot(to, child_color);
        edges = edges.saturating_add(1);
    }
    (canvas, edges)
}

/// Args for `anim.retarget.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RetargetPreviewArgs {
    /// The bone map to draw, exactly what `anim.retarget` takes.
    pub map: RetargetArgs,
    /// Directory under the project root, relative, such as `reports/anim`.
    pub dir: String,
    /// File inside that directory, relative, such as `retarget.png`.
    pub out: String,
    /// Frame width in pixels, 1..=512; default 256.
    #[serde(default)]
    pub width: Option<u32>,
    /// Frame height in pixels, 1..=512; default 256.
    #[serde(default)]
    pub height: Option<u32>,
}

/// Result of `anim.retarget.preview`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
#[serde(crate = "rmcp::serde")]
#[schemars(crate = "rmcp::schemars")]
pub struct RetargetPreviewOutput {
    /// PNG path, relative to the project root.
    pub png: String,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// Bones drawn: 16 when the map covers the whole humanoid tree.
    pub edges: u32,
    /// Pixels that are not the background: a real frame always has some.
    pub lit_pixels: u32,
    /// PNG size in bytes.
    pub bytes: u32,
    /// PNG digest (16 hex digits): equal on every replay of the same map.
    pub digest: String,
    /// Coverage of the 17 roles in per-mille, from the map step.
    pub coverage_permille: u16,
    /// Map digest (16 hex digits) this frame was drawn from.
    pub map_digest: String,
}

/// One frame side, or a refusal that names it.
fn preview_size(got: Option<u32>, name: &str) -> Result<u32, McpToolError> {
    let side = got.unwrap_or(PREVIEW_DEFAULT_SIZE);
    if side == 0 || side > PREVIEW_MAX_SIZE {
        return Err(McpToolError::new(
            "bad_size",
            format!(
                "`{name}` {side} is outside 1..={PREVIEW_MAX_SIZE}; recovery: ask for a side in \
                 that range (default {PREVIEW_DEFAULT_SIZE})"
            ),
        ));
    }
    Ok(side)
}

/// Join `dir` and `out` under the project root, refusing a path that leaves it.
fn preview_path(
    root: &std::path::Path,
    dir: &str,
    out: &str,
) -> Result<std::path::PathBuf, McpToolError> {
    for (field, text) in [("dir", dir), ("out", out)] {
        let path = std::path::Path::new(text);
        let plain = !text.is_empty()
            && !path.is_absolute()
            && path
                .components()
                .all(|part| matches!(part, std::path::Component::Normal(_)));
        if !plain {
            return Err(McpToolError::new(
                "bad_args",
                format!(
                    "`{field}` `{text}` is not a plain relative path; recovery: keep the frame \
                     under the project root, such as dir `reports/anim` and out `retarget.png`"
                ),
            ));
        }
    }
    if !out.to_ascii_lowercase().ends_with(".png") {
        return Err(McpToolError::new(
            "bad_args",
            format!("`out` `{out}` is not a `.png` name; recovery: end the frame file with `.png`"),
        ));
    }
    Ok(root.join(dir).join(out))
}

/// Write the frame and answer its size, pixel count and digest.
pub(super) fn preview_frame(
    root: &std::path::Path,
    server: &McpServer,
    args: &RetargetPreviewArgs,
) -> Result<RetargetPreviewOutput, McpToolError> {
    let mapped = retarget(server, &args.map)?;
    let width = preview_size(args.width, "width")?;
    let height = preview_size(args.height, "height")?;
    let path = preview_path(root, &args.dir, &args.out)?;
    let (canvas, edges) = draw_map(&mapped.mappings, width, height);
    let png = encode_png_rgb(width, height, &canvas.rgb);
    let bytes = u32::try_from(png.len()).map_err(|_| {
        McpToolError::new(
            "internal",
            "the retarget frame is larger than 4 GiB; report this preview",
        )
    })?;
    let failed = |why: std::io::Error| {
        McpToolError::new(
            "write_failed",
            format!(
                "could not write the retarget frame `{}/{}` under the project root ({why}); \
                 re-run with a writable dir such as `reports/anim`",
                args.dir, args.out
            ),
        )
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(failed)?;
    }
    std::fs::write(&path, &png).map_err(failed)?;
    Ok(RetargetPreviewOutput {
        png: format!("{}/{}", args.dir, args.out),
        width,
        height,
        edges,
        lit_pixels: canvas.lit_pixels(),
        bytes,
        digest: format!("{:016x}", engine_core::hash_bytes(&png)),
        coverage_permille: mapped.coverage_permille,
        map_digest: mapped.digest,
    })
}

fn retarget_preview(
    server: &McpServer,
    args: &RetargetPreviewArgs,
) -> Result<RetargetPreviewOutput, McpToolError> {
    preview_frame(&crate::packs::codex::project_root(None), server, args)
}

crate::e2_tool!(
    /// `anim.retarget.preview`: draw one mapped skeleton as a real PNG frame.
    RetargetPreviewTool,
    name = TOOL_ANIM_RETARGET_PREVIEW,
    description = "Draw a retargeted skeleton: score the same bone map as `anim.retarget`, then write one PNG under the project root (dir and out are plain relative paths, sides 1..=512, default 256). Each of the 16 humanoid bones is two pixels wide and coloured by how its row was built: green exact, amber alias, blue manual, with a 3x3 joint block. The answer gives the path, size, bones drawn, lit pixel count, PNG bytes and a PNG digest that is equal on every replay of the same map.",
    readonly = true,
    args = RetargetPreviewArgs,
    output = RetargetPreviewOutput,
    invoke = retarget_preview
);
