//! windowed_hud_text: the HUD text glyphs the windowed smoke paints.
//!
//! Consumer: `windowed::hud_cells_quads` feeds the smoke log counts, and
//! `windowed::smoke_present_once` calls [`stage_hud_text`] after the HP bars,
//! so the counted glyphs and the glyphs the presenter overlay ring takes are
//! one list. Data owner: `engine-ui` (the layout and the hp/feed lines);
//! `engine-render` owns the overlay ring. Units: 0..1 overlay space, top-left
//! origin; one cell is `HUD_GLYPH_H` tall and `HUD_GLYPH_H * advance_ratio`
//! wide. Determinism: pure over the `Hud` snapshot; presentation only, never
//! hashed. Failure mode: a full ring takes fewer glyphs and the count says so;
//! nothing panics. Budget: one layout pass per HUD line on the presentation
//! path only.

use engine_render::present::Presenter;
use engine_ui::{
    text::{feed_lines, hp_text, layout, Align},
    Hud,
};

/// Overlay cell size of one HUD glyph in 0..1 space: the ring's glyph height
/// and its advance (`height * advance_ratio`, the same rule `push_text` uses).
const HUD_GLYPH_H: f32 = 0.04;
/// Left edge of the HUD text column, right of the HP bars (`hp_bar_rect`
/// stops at x 0.34), so the text never covers a bar.
const HUD_TEXT_X: f32 = 0.40;
/// Top of the first HUD text row; rows step down by `HUD_TEXT_STEP`.
const HUD_TEXT_Y0: f32 = 0.05;
const HUD_TEXT_STEP: f32 = 0.05;
/// sRGB colour of the HUD text.
const HUD_TEXT_COL: [u8; 4] = [230, 235, 240, 255];

/// One painted HUD glyph: top-left corner in 0..1 overlay space plus the
/// character. Spaces advance the layout but paint no quad, as the ring does.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct HudGlyph {
    /// Left edge in 0..1 space.
    pub x: f32,
    /// Top edge in 0..1 space.
    pub y: f32,
    /// The glyph character.
    pub ch: char,
}

/// Layout cell index as an overlay offset. Layout width is 32 cells, so the
/// `u16` bound never clips; a larger index would saturate, never wrap.
fn cell_offset(index: u32) -> f32 {
    f32::from(u16::try_from(index).unwrap_or(u16::MAX))
}

/// HUD text lines for one snapshot: the hp line, then the kill-feed lines.
#[must_use]
pub fn hud_text_lines(hud: &Hud) -> Vec<String> {
    let mut lines = vec![hp_text(hud)];
    lines.extend(feed_lines(&hud.feed));
    lines
}

/// Every painted HUD glyph for one snapshot, in row then column order. One
/// layout pass per line through the UI text path; a wrapped line takes the
/// rows its layout used before the next line starts. Pure.
#[must_use]
pub fn hud_glyphs(hud: &Hud) -> Vec<HudGlyph> {
    let advance = HUD_GLYPH_H * engine_render::overlay::text::advance_ratio();
    let mut out = Vec::new();
    let mut row_base = 0u32;
    for line in hud_text_lines(hud) {
        let laid = layout(&line, 32, Align::Left);
        let rows = laid.iter().map(|cell| cell.cell_y + 1).max().unwrap_or(1);
        for cell in laid.iter().filter(|cell| cell.codepoint != ' ') {
            out.push(HudGlyph {
                x: HUD_TEXT_X + cell_offset(cell.cell_x) * advance,
                y: HUD_TEXT_Y0 + cell_offset(row_base + cell.cell_y) * HUD_TEXT_STEP,
                ch: cell.codepoint,
            });
        }
        row_base += rows;
    }
    out
}

/// HUD text cells plus quads for one snapshot (hp line plus kill-feed lines
/// through the UI text path). Counts only painted glyphs: one quad per
/// non-space layout cell, the same quads [`stage_hud_text`] stages into the
/// presenter ring. Pure.
pub fn hud_cells_quads(hud: &Hud) -> (usize, usize) {
    let glyphs = hud_glyphs(hud).len();
    (glyphs, glyphs)
}

/// Stage the HUD text glyphs into the presenter overlay ring, after the HP
/// bars. Returns the quads the ring took; a full ring takes fewer glyphs and
/// the count says so. Presentation only, never hashed.
pub fn stage_hud_text(presenter: &mut Presenter, hud: &Hud) -> usize {
    let mut buf = [0u8; 4];
    let mut staged = 0usize;
    for glyph in hud_glyphs(hud) {
        let text = glyph.ch.encode_utf8(&mut buf);
        if presenter.push_text(glyph.x, glyph.y, HUD_GLYPH_H, HUD_TEXT_COL, text) {
            staged += 1;
        }
    }
    staged
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{App, GameConfig};

    fn config() -> GameConfig {
        GameConfig {
            seed: 2040,
            ticks: 60,
            fighters: 2,
            half_extent_m: 20.0,
        }
    }

    #[test]
    fn hud_text_counts_are_the_quads_the_ring_stages() {
        // Behavior: the HUD log counts equal the quads the overlay ring takes
        // for the same glyphs; spaces advance the layout without a quad.
        let app = App::new(config()).unwrap();
        let hud = app.hud();
        let glyphs = hud_glyphs(hud);
        let (cells, quads) = hud_cells_quads(hud);
        assert_eq!(cells, glyphs.len(), "log cells are the painted glyphs");
        assert_eq!(quads, cells, "one quad per painted glyph");
        assert!(
            cells >= "HP 1000/1000".len() - 1,
            "the hp line paints its non-space glyphs"
        );
        assert!(glyphs.iter().all(|glyph| glyph.ch != ' '));
        let mut ring = engine_render::overlay::OverlayRing::new();
        let mut buf = [0u8; 4];
        let mut taken = 0usize;
        for glyph in &glyphs {
            let text = glyph.ch.encode_utf8(&mut buf);
            if ring.push_text(glyph.x, glyph.y, HUD_GLYPH_H, HUD_TEXT_COL, text) > 0 {
                taken += 1;
            }
        }
        assert_eq!(taken, cells, "every counted glyph reaches the ring");
        assert_eq!(ring.quad_count(), quads, "ring quads equal the log quads");
        // Failure guard: the first glyph opens the hp row at the HUD column.
        assert!(glyphs
            .first()
            .is_some_and(|g| g.x == HUD_TEXT_X && g.y == HUD_TEXT_Y0));
    }
}
