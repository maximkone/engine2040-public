//! Console overlay staging for the windowed presenter.
//!
//! Consumer: `crates/game/src/bin/arena.rs`, through [`stage_hud`]. Drawing
//! is presentation only and never hashed: the sim sees no console state.
//! Units: 0..1 HUD space. Errors: none; a full overlay ring refuses a quad
//! and leaves the frame as staged. Budget: rows are clipped in
//! `console.rs`, so the fixed 128-quad ring keeps room for the HP bars.

use crate::console::{DevConsole, OVERLAY_PANEL, OVERLAY_TEXT_HEIGHT, OVERLAY_TEXT_X};
use engine_render::present::Presenter;
use engine_ui::DuelOverlay;

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
