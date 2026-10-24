//! hud (SD10): the speed demo's own line on screen. While the demo runs the arena's fight
//! overlay (the fighters' health bars) is off, so the demo states its speed, mode and cuts in
//! that place. Presentation only: the text never enters the sim, the tape or the hash.

use engine_render::present::Presenter;

use super::scene::SpeedDemo;

/// Screen pixels: the first row's origin and the row pitch.
const ORIGIN: [f32; 2] = [24.0, 24.0];
const ROW_H: f32 = 22.0;
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
        let y = ORIGIN[1] + ROW_H * row as f32;
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
}
