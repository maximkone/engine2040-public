//! hud_pass: rasterizes one `engine-ui` [`HudFrame`] over a scene.
//!
//! Consumer: the KAM1c windowed proof (`kam1c`), which draws the KAM1b 2.5D
//! scene, composites the HUD on the readback frame and saves one PNG; the
//! `hud` graph pass reuses the same command order next.
//! Data owner: `engine-ui` owns the commands, the palette and the 5x7 text
//! atlas; this module only turns them into pixels.
//! Units: framebuffer pixels, top-left origin, +x right, +y down, straight
//! RGBA8 sRGB bytes.
//! Determinism: commands run in list order; integer channel blending with a
//! fixed rounding bias; a scissor replaces the clip, itself clipped to the
//! surface; no clock, no floats.
//! Failure mode: a pixel slice smaller than `width * height * 4` refuses with
//! [`HudPassError`]; geometry outside the surface clips, never panics.

use engine_ui::text::{self, Atlas};
use engine_ui::{HudCommand, HudFrame};

/// Widest or tallest accepted surface, in pixels. Keeps pixel maths bounded.
pub const MAX_HUD_SIDE: u32 = 8192;

/// One axis-aligned rectangle in framebuffer pixels, right and bottom edges
/// exclusive. Pure data.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ClipRect {
    /// Left edge in pixels.
    pub x: i32,
    /// Top edge in pixels.
    pub y: i32,
    /// Width in pixels; `<= 0` is empty.
    pub w: i32,
    /// Height in pixels; `<= 0` is empty.
    pub h: i32,
}

/// Intersection of two rectangles, or `None` when the result is empty.
/// Touching edges do not overlap. Pure; integer maths only.
#[must_use]
pub fn intersect(a: ClipRect, b: ClipRect) -> Option<ClipRect> {
    if a.w <= 0 || a.h <= 0 || b.w <= 0 || b.h <= 0 {
        return None;
    }
    let x0 = i64::from(a.x).max(i64::from(b.x));
    let y0 = i64::from(a.y).max(i64::from(b.y));
    let x1 = i64::from(a.x)
        .saturating_add(i64::from(a.w))
        .min(i64::from(b.x).saturating_add(i64::from(b.w)));
    let y1 = i64::from(a.y)
        .saturating_add(i64::from(a.h))
        .min(i64::from(b.y).saturating_add(i64::from(b.h)));
    if x1 <= x0 || y1 <= y0 {
        return None;
    }
    Some(ClipRect {
        // `x0` is one of two `i32` edges, so the conversion needs no clamp.
        x: i32::try_from(x0).unwrap_or(i32::MAX),
        y: i32::try_from(y0).unwrap_or(i32::MAX),
        w: i32::try_from(x1.saturating_sub(x0)).unwrap_or(i32::MAX),
        h: i32::try_from(y1.saturating_sub(y0)).unwrap_or(i32::MAX),
    })
}

/// Composite `src` over `dst` in straight sRGB bytes: each channel is
/// `(src * a + dst * (255 - a) + 127) / 255`, so the result rounds to nearest.
/// Alpha follows the same source-over rule. Pure; no floats.
#[must_use]
pub fn blend_over(dst: [u8; 4], src: [u8; 4]) -> [u8; 4] {
    let alpha = u32::from(src[3]);
    let keep = 255 - alpha;
    let mix = |source: u8, below: u8| -> u8 {
        let value = u32::from(source) * alpha + u32::from(below) * keep + 127;
        u8::try_from(value / 255).unwrap_or(255)
    };
    let out_alpha = alpha + (u32::from(dst[3]) * keep + 127) / 255;
    [
        mix(src[0], dst[0]),
        mix(src[1], dst[1]),
        mix(src[2], dst[2]),
        u8::try_from(out_alpha.min(255)).unwrap_or(255),
    ]
}

/// Why a surface could not be built.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum HudPassError {
    /// Width or height is zero or above [`MAX_HUD_SIDE`], culprit carried.
    BadSide(u32),
    /// The pixel slice holds fewer than `width * height * 4` bytes.
    ShortBuffer {
        /// Bytes the surface needs.
        need: usize,
        /// Bytes the caller supplied.
        got: usize,
    },
}

impl core::fmt::Display for HudPassError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::BadSide(side) => write!(f, "hud surface side {side} outside 1..={MAX_HUD_SIDE}"),
            Self::ShortBuffer { need, got } => {
                write!(f, "hud surface needs {need} bytes, got {got}")
            }
        }
    }
}

impl std::error::Error for HudPassError {}

/// One RGBA8 scene slice: the framebuffer the HUD draws into.
pub struct HudSurface<'a> {
    width: usize,
    height: usize,
    pixels: &'a mut [u8],
}

impl<'a> HudSurface<'a> {
    /// Borrow `pixels` as a `width` x `height` RGBA8 surface. The slice must
    /// hold at least `width * height * 4` bytes; extra bytes stay untouched.
    /// Pure.
    ///
    /// # Errors
    ///
    /// Returns [`HudPassError::BadSide`] outside `1..=MAX_HUD_SIDE` and
    /// [`HudPassError::ShortBuffer`] when the slice is too small.
    pub fn new(width: u32, height: u32, pixels: &'a mut [u8]) -> Result<Self, HudPassError> {
        if width == 0 || height == 0 || width > MAX_HUD_SIDE || height > MAX_HUD_SIDE {
            return Err(HudPassError::BadSide(width.max(height)));
        }
        let width = usize::try_from(width).map_err(|_| HudPassError::BadSide(width))?;
        let height = usize::try_from(height).map_err(|_| HudPassError::BadSide(height))?;
        let need = width
            .checked_mul(height)
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or(HudPassError::BadSide(u32::MAX))?;
        if pixels.len() < need {
            return Err(HudPassError::ShortBuffer {
                need,
                got: pixels.len(),
            });
        }
        Ok(Self {
            width,
            height,
            pixels,
        })
    }

    /// Surface width in pixels.
    #[must_use]
    pub fn width(&self) -> u32 {
        u32::try_from(self.width).unwrap_or(u32::MAX)
    }

    /// Surface height in pixels.
    #[must_use]
    pub fn height(&self) -> u32 {
        u32::try_from(self.height).unwrap_or(u32::MAX)
    }

    /// RGBA bytes at `(x, y)`, or `None` outside the surface.
    #[must_use]
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let x = usize::try_from(x).ok()?;
        let y = usize::try_from(y).ok()?;
        if x >= self.width || y >= self.height {
            return None;
        }
        let index = y.checked_mul(self.width)?.checked_add(x)?.checked_mul(4)?;
        let end = index.checked_add(4)?;
        let slice = self.pixels.get(index..end)?;
        Some([slice[0], slice[1], slice[2], slice[3]])
    }

    /// Blend one source pixel over the stored pixel; out-of-range writes drop.
    fn set(&mut self, x: usize, y: usize, color: [u8; 4]) {
        let Some(index) = y
            .checked_mul(self.width)
            .and_then(|row| row.checked_add(x))
            .and_then(|pixel| pixel.checked_mul(4))
        else {
            return;
        };
        let Some(end) = index.checked_add(4) else {
            return;
        };
        let Some(slot) = self.pixels.get_mut(index..end) else {
            return;
        };
        let mut below = [0u8; 4];
        below.copy_from_slice(slot);
        let out = blend_over(below, color);
        slot.copy_from_slice(&out);
    }
}

/// What one pass did: command counts, emitted text quads and painted pixels.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct HudPassReceipt {
    /// Rect commands applied, clipped or not.
    pub rects: usize,
    /// Border commands applied; each border is four fills.
    pub borders: usize,
    /// Text commands applied.
    pub texts: usize,
    /// Scissor commands applied; each replaces the current clip.
    pub scissors: usize,
    /// Text quads emitted (one per laid-out glyph cell), before clipping.
    pub text_quads: usize,
    /// Pixels actually blended across rects, borders and text.
    pub drawn_pixels: usize,
    /// Non-empty draw batches: rects, borders and text, so at most three.
    pub draws: usize,
}

/// Draw every command of `frame` over `surface` in list order and return the
/// receipt. Scissors replace the clip, intersected with the surface; rects,
/// borders and text blend with their alpha; one text quad is one laid-out
/// glyph cell. Pure; bounded by the surface size.
pub fn draw(frame: &HudFrame, surface: &mut HudSurface<'_>) -> HudPassReceipt {
    let atlas = Atlas::new();
    let viewport = ClipRect {
        x: 0,
        y: 0,
        w: i32::try_from(surface.width).unwrap_or(i32::MAX),
        h: i32::try_from(surface.height).unwrap_or(i32::MAX),
    };
    let mut clip = viewport;
    let mut receipt = HudPassReceipt::default();
    for command in frame.commands() {
        match command {
            HudCommand::Scissor { x, y, w, h } => {
                clip = intersect(
                    viewport,
                    ClipRect {
                        x: *x,
                        y: *y,
                        w: *w,
                        h: *h,
                    },
                )
                .unwrap_or(ClipRect {
                    x: 0,
                    y: 0,
                    w: 0,
                    h: 0,
                });
                receipt.scissors += 1;
            }
            HudCommand::Rect {
                x, y, w, h, color, ..
            } => {
                receipt.rects += 1;
                let rect = ClipRect {
                    x: *x,
                    y: *y,
                    w: *w,
                    h: *h,
                };
                receipt.drawn_pixels += fill(surface, rect, *color, clip);
            }
            HudCommand::Border {
                x,
                y,
                w,
                h,
                thickness,
                color,
            } => {
                receipt.borders += 1;
                let rect = ClipRect {
                    x: *x,
                    y: *y,
                    w: *w,
                    h: *h,
                };
                for side in border_sides(rect, *thickness) {
                    receipt.drawn_pixels += fill(surface, side, *color, clip);
                }
            }
            HudCommand::Text { .. } => {
                receipt.texts += 1;
                let (quads, painted) = draw_text(surface, atlas, command, clip);
                receipt.text_quads += quads;
                receipt.drawn_pixels += painted;
            }
        }
    }
    receipt.draws = usize::from(receipt.rects > 0)
        + usize::from(receipt.borders > 0)
        + usize::from(receipt.texts > 0);
    receipt
}

/// Fill `rect` with `color`, clipped by `clip`; returns the pixel count.
fn fill(surface: &mut HudSurface<'_>, rect: ClipRect, color: [u8; 4], clip: ClipRect) -> usize {
    let Some(area) = intersect(clip, rect) else {
        return 0;
    };
    let x0 = usize::try_from(area.x).unwrap_or(0);
    let y0 = usize::try_from(area.y).unwrap_or(0);
    let x1 = usize::try_from(area.x.saturating_add(area.w)).unwrap_or(0);
    let y1 = usize::try_from(area.y.saturating_add(area.h)).unwrap_or(0);
    for y in y0..y1 {
        for x in x0..x1 {
            surface.set(x, y, color);
        }
    }
    usize::try_from(area.w)
        .unwrap_or(0)
        .saturating_mul(usize::try_from(area.h).unwrap_or(0))
}

/// Four inside strokes of `rect`, each `thickness` pixels wide and clamped to
/// at most half the short side. Empty inputs return empty rectangles. Pure.
fn border_sides(rect: ClipRect, thickness: i32) -> [ClipRect; 4] {
    let empty = ClipRect {
        x: 0,
        y: 0,
        w: 0,
        h: 0,
    };
    if rect.w <= 0 || rect.h <= 0 || thickness <= 0 {
        return [empty; 4];
    }
    let stroke = thickness.min(rect.w.min(rect.h) / 2);
    if stroke <= 0 {
        return [empty; 4];
    }
    let inner_h = rect.h.saturating_sub(stroke.saturating_mul(2));
    [
        ClipRect {
            x: rect.x,
            y: rect.y,
            w: rect.w,
            h: stroke,
        },
        ClipRect {
            x: rect.x,
            y: rect.y.saturating_add(rect.h).saturating_sub(stroke),
            w: rect.w,
            h: stroke,
        },
        ClipRect {
            x: rect.x,
            y: rect.y.saturating_add(stroke),
            w: stroke,
            h: inner_h,
        },
        ClipRect {
            x: rect.x.saturating_add(rect.w).saturating_sub(stroke),
            y: rect.y.saturating_add(stroke),
            w: stroke,
            h: inner_h,
        },
    ]
}

/// Draw one text command: lay the run out in `engine-ui`'s atlas grid, then
/// paint one square quad per glyph cell, sampling the atlas bitmap at the
/// cell's 6x8 texel grid. Returns `(quads, painted pixels)`. Pure.
fn draw_text(
    surface: &mut HudSurface<'_>,
    atlas: Atlas,
    command: &HudCommand,
    clip: ClipRect,
) -> (usize, usize) {
    let HudCommand::Text {
        x,
        y,
        px,
        color,
        text,
    } = command
    else {
        return (0, 0);
    };
    if *px == 0 || clip.w <= 0 || clip.h <= 0 {
        return (0, 0);
    }
    let size = i32::from(*px);
    let room = clip.x.saturating_add(clip.w).saturating_sub(*x);
    if room <= 0 {
        return (0, 0);
    }
    // A clip narrower than one cell still lays out one glyph; pixels outside
    // the clip drop in the loop below.
    let cols = u32::try_from(room / size).unwrap_or(0).max(1);
    let cells = text::layout(text, cols, text::Align::Left);
    if cells.is_empty() {
        return (0, 0);
    }
    let texel_w = i32::try_from(Atlas::CELL_W).unwrap_or(6);
    let texel_h = i32::try_from(Atlas::CELL_H).unwrap_or(8);
    let mut painted = 0usize;
    for cell in &cells {
        let cell_x = i32::try_from(cell.cell_x).unwrap_or(i32::MAX);
        let cell_y = i32::try_from(cell.cell_y).unwrap_or(i32::MAX);
        let quad = ClipRect {
            x: x.saturating_add(cell_x.saturating_mul(size)),
            y: y.saturating_add(cell_y.saturating_mul(size)),
            w: size,
            h: size,
        };
        let Some(area) = intersect(clip, quad) else {
            continue;
        };
        let start_x = area.x.saturating_sub(quad.x);
        let start_y = area.y.saturating_sub(quad.y);
        let end_x = start_x.saturating_add(area.w);
        let end_y = start_y.saturating_add(area.h);
        for local_y in start_y..end_y {
            for local_x in start_x..end_x {
                let atlas_x =
                    u32::try_from(local_x.saturating_mul(texel_w) / size).unwrap_or(u32::MAX);
                let atlas_y =
                    u32::try_from(local_y.saturating_mul(texel_h) / size).unwrap_or(u32::MAX);
                if atlas.pixel(cell.codepoint, atlas_x, atlas_y) {
                    let out_x = usize::try_from(quad.x.saturating_add(local_x)).unwrap_or(0);
                    let out_y = usize::try_from(quad.y.saturating_add(local_y)).unwrap_or(0);
                    surface.set(out_x, out_y, *color);
                    painted += 1;
                }
            }
        }
    }
    (cells.len(), painted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_ui::text::hud_text_px;
    use engine_ui::{contrast_ratio, DuelOverlay, HudRole};
    use std::time::Instant;

    const BG: [u8; 4] = [0, 0, 0, 255];

    fn frame(width: u32, height: u32, commands: Vec<HudCommand>) -> HudFrame {
        HudFrame::from_commands(width, height, hud_text_px(height), commands)
    }

    fn run(
        frame: &HudFrame,
        width: u32,
        height: u32,
        background: [u8; 4],
    ) -> (Vec<u8>, HudPassReceipt) {
        let need = usize::try_from(width * height * 4).expect("test surface fits usize");
        let mut pixels = vec![0u8; need];
        for pixel in pixels.chunks_exact_mut(4) {
            pixel.copy_from_slice(&background);
        }
        let receipt = {
            let mut surface = HudSurface::new(width, height, &mut pixels).expect("surface builds");
            draw(frame, &mut surface)
        };
        (pixels, receipt)
    }

    fn at(pixels: &[u8], width: u32, x: u32, y: u32) -> [u8; 4] {
        let index = usize::try_from((y * width + x) * 4).expect("pixel offset fits usize");
        [
            pixels[index],
            pixels[index + 1],
            pixels[index + 2],
            pixels[index + 3],
        ]
    }

    #[test]
    fn hud_pass_refuses_a_short_surface() {
        let mut short = [0u8; 15];
        assert!(matches!(
            HudSurface::new(2, 2, &mut short),
            Err(HudPassError::ShortBuffer { need: 16, got: 15 })
        ));
        let mut pixels = [0u8; 16];
        assert!(matches!(
            HudSurface::new(0, 2, &mut pixels),
            Err(HudPassError::BadSide(_))
        ));
        assert!(matches!(
            HudSurface::new(MAX_HUD_SIDE + 1, 2, &mut pixels),
            Err(HudPassError::BadSide(_))
        ));
        let surface = HudSurface::new(2, 2, &mut pixels).expect("exact surface builds");
        assert_eq!((surface.width(), surface.height()), (2, 2));
        assert_eq!(surface.pixel(0, 0), Some([0, 0, 0, 0]));
        assert_eq!(surface.pixel(2, 0), None);
    }

    #[test]
    fn hud_pass_draws_commands_in_list_order() {
        let frame = frame(
            64,
            64,
            vec![
                HudCommand::Scissor {
                    x: 0,
                    y: 0,
                    w: 64,
                    h: 64,
                },
                HudCommand::Rect {
                    role: HudRole::Panel,
                    x: 4,
                    y: 4,
                    w: 20,
                    h: 20,
                    color: [255, 0, 0, 255],
                },
                HudCommand::Rect {
                    role: HudRole::HpPip,
                    x: 8,
                    y: 8,
                    w: 8,
                    h: 8,
                    color: [0, 0, 255, 255],
                },
                HudCommand::Border {
                    x: 4,
                    y: 4,
                    w: 20,
                    h: 20,
                    thickness: 2,
                    color: [0, 255, 0, 255],
                },
            ],
        );
        let (pixels, receipt) = run(&frame, 64, 64, BG);
        assert_eq!((receipt.rects, receipt.borders, receipt.texts), (2, 1, 0));
        assert_eq!(receipt.scissors, 1);
        assert_eq!(receipt.draws, 2);
        // The later pip rect covers the middle of the earlier panel fill.
        assert_eq!(at(&pixels, 64, 10, 10), [0, 0, 255, 255]);
        // The border comes last, so it owns the four inside edges.
        assert_eq!(at(&pixels, 64, 4, 10), [0, 255, 0, 255]);
        assert_eq!(at(&pixels, 64, 10, 23), [0, 255, 0, 255]);
        // The panel fill survives between the pip and the border.
        assert_eq!(at(&pixels, 64, 20, 10), [255, 0, 0, 255]);
        assert_eq!(at(&pixels, 64, 3, 3), BG);
    }

    #[test]
    fn hud_pass_scissor_intersects_and_clips_geometry() {
        let area = |x, y, w, h| ClipRect { x, y, w, h };
        assert_eq!(
            intersect(area(0, 0, 10, 10), area(-5, -5, 8, 8)),
            Some(area(0, 0, 3, 3))
        );
        assert_eq!(
            intersect(area(0, 0, 10, 10), area(9, 9, 5, 5)),
            Some(area(9, 9, 1, 1))
        );
        assert_eq!(intersect(area(0, 0, 10, 10), area(10, 0, 4, 4)), None);
        assert_eq!(intersect(area(0, 0, 10, 10), area(0, 0, 0, 10)), None);

        let clipped = frame(
            16,
            16,
            vec![
                HudCommand::Scissor {
                    x: 4,
                    y: 4,
                    w: 8,
                    h: 8,
                },
                HudCommand::Rect {
                    role: HudRole::Panel,
                    x: 0,
                    y: 0,
                    w: 16,
                    h: 16,
                    color: [255, 0, 0, 255],
                },
            ],
        );
        let (pixels, receipt) = run(&clipped, 16, 16, BG);
        assert_eq!(receipt.drawn_pixels, 64);
        assert_eq!(at(&pixels, 16, 3, 3), BG);
        assert_eq!(at(&pixels, 16, 4, 4), [255, 0, 0, 255]);
        assert_eq!(at(&pixels, 16, 11, 11), [255, 0, 0, 255]);
        assert_eq!(at(&pixels, 16, 12, 12), BG);

        // A second scissor replaces the first clip instead of nesting.
        let replaced = frame(
            16,
            16,
            vec![
                HudCommand::Scissor {
                    x: 0,
                    y: 0,
                    w: 8,
                    h: 8,
                },
                HudCommand::Scissor {
                    x: 4,
                    y: 4,
                    w: 8,
                    h: 8,
                },
                HudCommand::Rect {
                    role: HudRole::Panel,
                    x: 0,
                    y: 0,
                    w: 16,
                    h: 16,
                    color: [255, 0, 0, 255],
                },
            ],
        );
        let (pixels, receipt) = run(&replaced, 16, 16, BG);
        assert_eq!(receipt.scissors, 2);
        assert_eq!(receipt.drawn_pixels, 64);
        assert_eq!(at(&pixels, 16, 12, 12), BG);

        // A scissor entirely outside the surface clips everything after it.
        let outside = frame(
            16,
            16,
            vec![
                HudCommand::Scissor {
                    x: 40,
                    y: 40,
                    w: 8,
                    h: 8,
                },
                HudCommand::Rect {
                    role: HudRole::Panel,
                    x: 0,
                    y: 0,
                    w: 16,
                    h: 16,
                    color: [255, 0, 0, 255],
                },
            ],
        );
        let (pixels, receipt) = run(&outside, 16, 16, BG);
        assert_eq!(receipt.drawn_pixels, 0);
        assert_eq!(at(&pixels, 16, 8, 8), BG);
    }

    #[test]
    fn hud_pass_alpha_blends_known_pixels() {
        assert_eq!(
            blend_over([0, 0, 0, 255], [255, 0, 0, 128]),
            [128, 0, 0, 255]
        );
        assert_eq!(
            blend_over([255, 255, 255, 255], [255, 0, 0, 128]),
            [255, 127, 127, 255]
        );
        assert_eq!(
            blend_over([10, 20, 30, 255], [255, 255, 255, 0]),
            [10, 20, 30, 255]
        );

        let half = frame(
            4,
            4,
            vec![HudCommand::Rect {
                role: HudRole::Panel,
                x: 0,
                y: 0,
                w: 4,
                h: 4,
                color: [0, 255, 0, 128],
            }],
        );
        let (pixels, _) = run(&half, 4, 4, [0, 0, 0, 255]);
        assert_eq!(at(&pixels, 4, 1, 1), [0, 128, 0, 255]);
        let (pixels, _) = run(&half, 4, 4, [255, 255, 255, 255]);
        assert_eq!(at(&pixels, 4, 1, 1), [127, 255, 127, 255]);

        let clear = frame(
            4,
            4,
            vec![HudCommand::Rect {
                role: HudRole::Panel,
                x: 0,
                y: 0,
                w: 4,
                h: 4,
                color: [255, 255, 255, 0],
            }],
        );
        let (pixels, receipt) = run(&clear, 4, 4, [10, 20, 30, 255]);
        assert_eq!(
            receipt.drawn_pixels, 16,
            "alpha 0 still touches every pixel"
        );
        assert_eq!(at(&pixels, 4, 2, 2), [10, 20, 30, 255]);
    }

    #[test]
    fn hud_pass_text_quads_land_on_the_atlas_grid() {
        let text = frame(
            128,
            64,
            vec![
                HudCommand::Scissor {
                    x: 0,
                    y: 0,
                    w: 128,
                    h: 64,
                },
                HudCommand::Text {
                    x: 10,
                    y: 20,
                    px: 28,
                    color: [255, 255, 255, 255],
                    text: "AB".to_owned(),
                },
            ],
        );
        let (pixels, receipt) = run(&text, 128, 64, BG);
        assert_eq!((receipt.texts, receipt.text_quads), (1, 2));
        // "A" row 0 is 01110: texel column 0 stays dark, columns 1..3 light.
        assert_eq!(at(&pixels, 128, 10, 20), BG);
        assert_eq!(at(&pixels, 128, 15, 20), [255, 255, 255, 255]);
        // The second glyph starts one 28 px quad to the right; "B" starts 11110.
        assert_eq!(at(&pixels, 128, 38, 20), [255, 255, 255, 255]);
        assert_eq!(at(&pixels, 128, 66, 20), BG);
        // The quad is 28 px tall, so row 48 and the pixel before x = 10 stay bare.
        assert_eq!(at(&pixels, 128, 15, 48), BG);
        assert_eq!(at(&pixels, 128, 9, 20), BG);

        let clipped = frame(
            128,
            64,
            vec![
                HudCommand::Scissor {
                    x: 10,
                    y: 20,
                    w: 5,
                    h: 28,
                },
                HudCommand::Text {
                    x: 10,
                    y: 20,
                    px: 28,
                    color: [255, 255, 255, 255],
                    text: "A".to_owned(),
                },
            ],
        );
        let (pixels, receipt) = run(&clipped, 128, 64, BG);
        assert_eq!(receipt.text_quads, 1);
        // Row 0 of "A" is dark in texel column 0, so the 5 px slice is bare.
        assert_eq!(at(&pixels, 128, 10, 20), BG);
        assert_eq!(at(&pixels, 128, 12, 20), BG);
        // Row 3 of "A" lights every texel, so the clipped slice paints there.
        assert_eq!(at(&pixels, 128, 10, 31), [255, 255, 255, 255]);
        assert_eq!(at(&pixels, 128, 15, 31), BG);
    }

    #[test]
    fn hud_pass_duel_frame_meets_the_hud_budget() {
        // The seeded overlay carries two tracked fighters with distinct ASCII
        // names; the test then pins their health and presentation stats so the
        // pixel classes are exact.
        let mut overlay = DuelOverlay::from_seed(7);
        let rook = overlay.rows()[0].id();
        let sable = overlay.rows()[1].id();
        overlay.set_hp(rook, 750).expect("rook hp tracks");
        overlay.set_hp(sable, 250).expect("sable hp tracks");
        overlay.set_stamina(rook, 400).expect("stamina tracks");
        overlay.set_poise(rook, 500).expect("poise tracks");
        let frame = overlay.hud_frame(1920, 1080);
        assert_eq!(frame.draws(), 3, "rect, border and text batches");

        let palette = overlay.palette();
        let background = [palette.bg[0], palette.bg[1], palette.bg[2], 255];
        let (pixels, receipt) = run(&frame, 1920, 1080, background);
        assert_eq!(receipt.draws, frame.draws());
        assert!(receipt.draws <= 3);

        let accent = [palette.accent[0], palette.accent[1], palette.accent[2], 255];
        let foreground = [palette.fg[0], palette.fg[1], palette.fg[2], 255];
        let role_pixel = |role: HudRole, skip: usize| -> Option<[u8; 4]> {
            let mut seen = 0usize;
            for command in frame.commands() {
                if let HudCommand::Rect {
                    role: kind,
                    x,
                    y,
                    w,
                    h,
                    color,
                } = command
                {
                    if *kind == role && *w > 0 && *h > 0 && color[3] == 255 {
                        if seen == skip {
                            let cx = u32::try_from(x.saturating_add(w / 2)).ok()?;
                            let cy = u32::try_from(y.saturating_add(h / 2)).ok()?;
                            return Some(at(&pixels, 1920, cx, cy));
                        }
                        seen += 1;
                    }
                }
            }
            None
        };
        assert_eq!(role_pixel(HudRole::StaminaFill, 0), Some(accent));
        // Segment 0 of the poise ring sits above the panel clip; use a lit
        // segment that the panel scissor keeps.
        assert_eq!(role_pixel(HudRole::PoiseSegment, 2), Some(accent));
        assert_eq!(role_pixel(HudRole::HpPip, 0), Some(foreground));

        let nonbg_px = pixels
            .chunks_exact(4)
            .filter(|pixel| pixel[..3] != palette.bg[..])
            .count();
        assert!(nonbg_px > 0, "the frame paints over the scene");
        assert!(receipt.text_quads > 0, "text quads are emitted");
        for command in frame.commands() {
            if let HudCommand::Text { color, .. } = command {
                let ratio = contrast_ratio([color[0], color[1], color[2]], palette.bg);
                assert!(ratio >= 4.5, "text contrast {ratio} stays at or above 4.5");
            }
        }

        // Text planning (layout only) must fit the row budget at 1080p. The
        // best of 32 runs drops scheduler noise from the measurement.
        let mut text_ms = f64::MAX;
        for _ in 0..32 {
            let start = Instant::now();
            let mut cells = 0usize;
            for command in frame.commands() {
                if let HudCommand::Text { text, .. } = command {
                    cells += text::layout(text, text::MAX_LINE_WIDTH, text::Align::Left).len();
                }
            }
            assert!(cells > 0);
            text_ms = text_ms.min(start.elapsed().as_secs_f64() * 1000.0);
        }
        assert!(text_ms <= 0.3, "text plan took {text_ms:.3} ms at 1080p");
        println!(
            "hud_pass draws={} text_ms={text_ms:.3} nonbg_px={nonbg_px}",
            receipt.draws
        );
    }
}
