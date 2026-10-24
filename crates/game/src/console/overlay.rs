//! Console overlay staging for the windowed presenter.
//!
//! Consumer: `crates/game/src/bin/arena.rs`, through [`stage_hud`]. Drawing
//! is presentation only and never hashed: the sim sees no console state.
//! Units: 0..1 HUD space. Errors: none; a full overlay ring refuses a quad
//! and leaves the frame as staged. Budget: rows are clipped in
//! `console.rs`, so the fixed 128-quad ring keeps room for the HP bars.

use crate::console::training_overlay::TrainingOverlay;
use crate::console::{
    DevConsole, OVERLAY_LINE_STEP, OVERLAY_PANEL, OVERLAY_TEXT_COLOR, OVERLAY_TEXT_HEIGHT,
    OVERLAY_TEXT_X,
};
use engine_render::present::Presenter;
use engine_ui::DuelOverlay;

/// Height of a guard bar under its hp bar, HUD space: a fifth of the hp bar's
/// 0.04, so it reads as a meter under it and not a second health bar.
pub const GUARD_BAR_H: f32 = 0.008;
/// Gap between an hp bar and its guard bar: with [`GUARD_BAR_H`] it fills the
/// hp bar's 0.01 slot gap exactly.
pub const GUARD_BAR_GAP: f32 = 0.002;

/// Stage one window HUD frame: optional HP bars, then the open console.
/// `show_bars` is false during the speed demo; the console still draws.
/// Presentation only; never hashed.
pub fn stage_hud(
    presenter: &mut Presenter,
    bars: &DuelOverlay,
    console: &DevConsole,
    show_bars: bool,
) {
    if show_bars {
        crate::windowed::stage_hp_bars(presenter, bars);
    }
    stage_console(presenter, console);
}

/// The guard bar's band under its hp bar, in HUD space: the hp bar's own width,
/// as tall as [`GUARD_BAR_H`], in the slot gap below it so the two never touch.
#[must_use]
pub fn guard_bar_rect(hp: [f32; 4]) -> [f32; 4] {
    let top = hp[3] + GUARD_BAR_GAP;
    [hp[0], top, hp[2], top + GUARD_BAR_H]
}

/// Stage one guard bar per fighter under its hp bar (row SW81), a
/// [`crate::guard_pressure::GuardPressure`] fill per slot. Stops at the first
/// refused quad, so a full ring leaves the frame as staged. Presentation only;
/// never hashed.
pub fn stage_guard_bars(presenter: &mut Presenter, pressures: &[u16]) {
    for (slot, pressure) in pressures.iter().enumerate() {
        let rect = guard_bar_rect(engine_render::overlay::hp_bar_rect(slot));
        let fill = engine_render::overlay::hp_fraction(*pressure);
        if !presenter.push_hp_bar(rect[0], rect[1], rect[2], rect[3], fill) {
            break;
        }
    }
}

/// Stage the training readout: one text row per remembered press and per
/// exchange, from the panel top down, each row showing its age or the answer's
/// frame advantage. A closed overlay stages nothing. Presentation only; never
/// hashed.
pub fn stage_training(presenter: &mut Presenter, training: &TrainingOverlay, now: u32) {
    if !training.is_enabled() {
        return;
    }
    let mut y = OVERLAY_PANEL[1] + OVERLAY_LINE_STEP;
    for row in training.rows(now) {
        if y > OVERLAY_PANEL[3] {
            break;
        }
        let _ = presenter.push_text(
            OVERLAY_TEXT_X,
            y,
            OVERLAY_TEXT_HEIGHT,
            OVERLAY_TEXT_COLOR,
            &row.text,
        );
        y += OVERLAY_LINE_STEP;
    }
}

/// Stage the open dev console into the presenter CPU ring: one dark panel
/// plus the input line and transcript tail. A closed console stages nothing.
/// Presentation only; never hashed.
pub fn stage_console(presenter: &mut Presenter, console: &DevConsole) {
    if !console.is_open() {
        return;
    }
    let [x0, y0, x1, y1] = OVERLAY_PANEL;
    // A zero fill leaves the bar's dark back quad as the console panel.
    let _ = presenter.push_hp_bar(x0, y0, x1, y1, 0.0);
    let advance = OVERLAY_TEXT_HEIGHT * engine_render::overlay::text::advance_ratio();
    for row in console.overlay_rows() {
        let mut x = OVERLAY_TEXT_X;
        if row.prompt {
            let _ = presenter.push_text(x, row.y, OVERLAY_TEXT_HEIGHT, row.color, "> ");
            x += advance * 2.0;
        }
        let _ = presenter.push_text(x, row.y, OVERLAY_TEXT_HEIGHT, row.color, row.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_bars_sit_under_the_hp_bars() {
        // SW81: one meter under each hp bar, inside the slot gap, never over
        // the bar above it and never into the next slot.
        for slot in 0..4 {
            let hp = engine_render::overlay::hp_bar_rect(slot);
            let guard = guard_bar_rect(hp);
            assert_eq!((guard[0], guard[2]), (hp[0], hp[2]), "one width");
            assert!(guard[1] > hp[3], "below the hp bar");
            assert!((guard[1] - hp[3] - GUARD_BAR_GAP).abs() < 1e-6);
            assert!(guard[1] < guard[3], "top above bottom");
            let next = engine_render::overlay::hp_bar_rect(slot + 1);
            // The guard band fills the slot gap exactly, so the boundary is an
            // equality in the constants; compare with one ULP of slack rather
            // than requiring the float sums to land on the same bit pattern.
            assert!(guard[3] <= next[1] + 1e-6, "never touches the next hp bar");
        }
    }
}
