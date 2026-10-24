//! hud (SD10): the speed demo's own line on screen. While the demo runs the arena's fight
//! overlay (the fighters' health bars) is off, so the demo states its speed, mode and cuts in
//! that place. Presentation only: the text never enters the sim, the tape or the hash.
//!
//! Note for the render lane: the overlay's quad space is bottom-origin (`overlay_mvp` maps
//! y 0 to the bottom) while `push_text` pairs a glyph's top with the rect's low edge, so every
//! staged line draws upside down on the GPU path today (the dev console's rows too). This file
//! pins where the line sits; the v pairing in `overlay::write_vertices` is the fix.

use engine_render::present::Presenter;

use super::scene::SpeedDemo;

/// Screen space, 0..1 (the overlay's own units, not pixels): the first row's bottom edge
/// and the row pitch. `push_text` multiplies these by the frame size, so a pixel value
/// draws nothing; the space is bottom-origin, so the top-left corner is a high y.
const ORIGIN: [f32; 2] = [0.018, 0.93];
const ROW_H: f32 = 0.032;
/// Opaque warm white, legible over both sand and sky.
const INK: [u8; 4] = [240, 236, 228, 255];
/// The rows the demo stages: the speed line, then the keys.
pub(crate) const ROWS: usize = 2;

/// Stage the demo's HUD: clear the ring the fight's bars would use, then one text row per
/// line. Clearing first keeps a frame's rows from piling on the last frame's; a closed
/// console stages nothing over the line.
pub(crate) fn stage(presenter: &mut Presenter, text: &[String; ROWS]) {
    presenter.clear_overlay();
    for (row, line) in text.iter().enumerate() {
        // Rows stack downward from the top edge: the overlay's y grows upward.
        let y = ORIGIN[1] - ROW_H * row as f32;
        let _ = presenter.push_text(ORIGIN[0], y, ROW_H, INK, line);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::KeyCode;

    const DT: f32 = 1.0 / 60.0;

    /// The first row names the speed in m/s, the mover and the cuts: what SD7 asked for.
    #[test]
    fn the_speed_line_names_speed_mode_and_cuts() {
        let mut demo = SpeedDemo::new(false).expect("the dunes build");
        for _ in 0..120 {
            demo.step(DT);
        }
        let text = demo.hud();
        let line = &text[0];
        assert!(line.contains("SPEED "), "{line}");
        assert!(line.contains("m/s"), "{line}");
        assert!(line.contains("MODE fly"), "{line}");
        // The dunes start beside towers, so the cut count is whatever the run did: the row
        // must show the same number the exit receipt names.
        let receipt = demo.line();
        let cuts = receipt
            .split("cuts=")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .expect("the receipt names the cuts");
        assert!(
            line.contains(&format!("CUTS {cuts}")),
            "{line} vs {receipt}"
        );
        assert!(text[1].contains("ALWAYS FASTER on"), "{}", text[1]);
        assert!(
            line.len() <= 96 && text[1].len() <= 96,
            "short rows: {text:?}"
        );
    }

    /// The second row states always-faster, and F turns it off in the text as it does in flight.
    #[test]
    fn f_turns_the_line_off() {
        let mut demo = SpeedDemo::new(false).expect("the dunes build");
        demo.key(KeyCode::KeyF, true);
        demo.key(KeyCode::KeyF, false);
        assert!(
            demo.hud()[1].contains("ALWAYS FASTER off"),
            "{:?}",
            demo.hud()
        );
    }

    /// Tab reaches the car, and the mode word follows the mover.
    #[test]
    fn the_mode_word_follows_the_mover() {
        let mut demo = SpeedDemo::new(false).expect("the dunes build");
        demo.key(KeyCode::Tab, true);
        demo.key(KeyCode::Tab, false);
        assert!(demo.hud()[0].contains("MODE car"), "{:?}", demo.hud());
    }

    /// The overlay reads 0..1 screen space: the first version passed pixels (origin 24.0) and
    /// staged every glyph off-screen, so the frame showed no line at all. This pins the units
    /// by staging the real rows into a ring and checking every quad lands on screen.
    #[test]
    fn the_hud_origin_is_screen_space_and_its_glyphs_draw() {
        use engine_render::overlay::OverlayRing;

        let mut ring = OverlayRing::new();
        let demo = SpeedDemo::new(false).expect("the dunes build");
        for (row, line) in demo.hud().iter().enumerate() {
            let y = ORIGIN[1] - ROW_H * row as f32;
            let staged = ring.push_text(ORIGIN[0], y, ROW_H, INK, line);
            assert!(staged > 10, "row {row} stages glyphs, got {staged}: {line}");
            // Only codes 32..=126 have a glyph; a stray char would silently drop.
            for byte in line.bytes() {
                assert!((32..=126).contains(&byte), "no glyph for {byte} in {line}");
            }
        }
        let mut lowest = 1.0_f32;
        for quad in ring.quads() {
            let [x0, y0, x1, y1] = quad.rect;
            assert!(x0 >= 0.0 && x1 <= 1.0, "on screen across: {:?}", quad.rect);
            assert!(y0 >= 0.0 && y1 <= 1.0, "on screen down: {:?}", quad.rect);
            lowest = lowest.min(y0);
        }
        assert!(lowest > 0.5, "the rows sit in the top half: {lowest}");
    }
}
