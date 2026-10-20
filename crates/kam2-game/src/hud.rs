//! The play HUD: the five-ware stock bar, the minimap frame and the status line.
//!
//! `engine-ui` owns the command shape and `engine-render::hud_pass` turns it into
//! pixels; this module decides what the window asks for. Pure: the same sim
//! gives the same text and the same rectangles, so the receipt and its tests
//! read one answer. ASCII only, the font covers `0x20..=0x7E`.

use engine_ui::text::hud_text_px;
use engine_ui::{HudCommand, HudFrame, HudRole};
use kam2_sim::{HouseKind, Sim, Ware};

use crate::view::{MAP_H, MAP_W};

/// Stock panel background: near-black, mostly opaque over the field.
const PANEL: [u8; 4] = [16, 18, 24, 220];
/// Panel and text foreground.
const TEXT: [u8; 4] = [245, 245, 245, 255];
/// Panel edge.
const BORDER: [u8; 4] = [58, 66, 82, 255];
/// Empty part of a stock bar.
const BAR_BACK: [u8; 4] = [56, 60, 72, 255];
/// Minimap background, translucent so the field reads through.
const MAP_BACK: [u8; 4] = [8, 10, 14, 180];
/// House marker on the minimap: style-lock gold.
const HOUSE_PIP: [u8; 4] = [217, 164, 65, 255];
/// Side 0 marker on the minimap: style-lock team blue.
const SIDE0_PIP: [u8; 4] = [43, 111, 214, 255];
/// Side 1 marker on the minimap: style-lock team red.
const SIDE1_PIP: [u8; 4] = [198, 55, 46, 255];
/// Fill colour per ware, in [`Ware::ALL`] order: wood, stone, grain, flour, bread.
const WARE_FILL: [[u8; 4]; 5] = [
    [166, 124, 82, 255],
    [150, 158, 170, 255],
    [217, 190, 134, 255],
    [216, 205, 180, 255],
    [217, 164, 65, 255],
];
/// A full stock bar holds this many of the ware.
const SLOT_FULL: [u32; 5] = [60, 40, 40, 40, 40];
/// Left edge of the stock panel, in pixels.
const PANEL_X: i32 = 16;
/// Width of the stock panel; the minimap sits to its right.
const PANEL_W: i32 = 1060;
/// Height of the stock panel.
const PANEL_H: i32 = 100;
/// Bottom margin of the panel and the minimap frame.
const MARGIN: i32 = 16;
/// Minimap frame size in pixels, matching the KAM1c placeholder.
const MINIMAP_W: i32 = 168;
const MINIMAP_H: i32 = 128;
/// Stock bar width and gap.
const BAR_W: i32 = 190;
const BAR_GAP: i32 = 14;

/// The stock line the HUD prints: every ware and its count, in stock order.
#[must_use]
pub fn stock_line(sim: &Sim) -> String {
    let mut line = String::new();
    for ware in Ware::ALL {
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&ware.name().to_ascii_uppercase());
        line.push(' ');
        line.push_str(&sim.stock(ware).to_string());
    }
    line
}

/// One status line: the sim tick, the run state, the selected house and the
/// last action's notice (`""` when there is none).
#[must_use]
pub fn title_line(sim: &Sim, paused: bool, selected: HouseKind, notice: &str) -> String {
    let state = if paused { "PAUSED" } else { "RUNNING" };
    let mut line = format!(
        "KAM3 PLAY tick={} {} BUILD={}",
        sim.ticks(),
        state,
        selected.name().to_ascii_uppercase()
    );
    if !notice.is_empty() {
        line.push(' ');
        line.push_str(notice);
    }
    line
}

/// One HUD frame for a `width` x `height` viewport: the stock panel with five
/// bars, the minimap frame with house and unit pips, and two text lines.
#[must_use]
pub fn hud_frame(
    width: u32,
    height: u32,
    sim: &Sim,
    paused: bool,
    selected: HouseKind,
    notice: &str,
) -> HudFrame {
    let width_px = i32::try_from(width).unwrap_or(1280);
    let height_px = i32::try_from(height).unwrap_or(720);
    let px = hud_text_px(height);
    let mut commands = Vec::with_capacity(32);
    commands.push(HudCommand::Scissor {
        x: 0,
        y: 0,
        w: width_px,
        h: height_px,
    });

    let panel_y = height_px.saturating_sub(MARGIN).saturating_sub(PANEL_H);
    stock_panel(&mut commands, sim, paused, selected, notice, panel_y, px);
    minimap(&mut commands, sim, width_px, height_px);

    HudFrame::from_commands(width, height, px, commands)
}

/// The bottom panel: its frame, the two status lines and one bar per ware.
fn stock_panel(
    commands: &mut Vec<HudCommand>,
    sim: &Sim,
    paused: bool,
    selected: HouseKind,
    notice: &str,
    panel_y: i32,
    px: u16,
) {
    commands.push(HudCommand::Rect {
        role: HudRole::Panel,
        x: PANEL_X,
        y: panel_y,
        w: PANEL_W,
        h: PANEL_H,
        color: PANEL,
    });
    commands.push(HudCommand::Border {
        x: PANEL_X,
        y: panel_y,
        w: PANEL_W,
        h: PANEL_H,
        thickness: 2,
        color: BORDER,
    });
    commands.push(HudCommand::Text {
        x: PANEL_X + 12,
        y: panel_y + 8,
        px,
        color: TEXT,
        text: title_line(sim, paused, selected, notice),
    });
    commands.push(HudCommand::Text {
        x: PANEL_X + 12,
        y: panel_y + 32,
        px,
        color: TEXT,
        text: stock_line(sim),
    });
    for (slot, ware) in Ware::ALL.into_iter().enumerate() {
        let bar_x = PANEL_X + 12 + i32::try_from(slot).unwrap_or(0) * (BAR_W + BAR_GAP);
        let bar_y = panel_y + PANEL_H - 34;
        commands.push(HudCommand::Rect {
            role: HudRole::BarBack,
            x: bar_x,
            y: bar_y,
            w: BAR_W,
            h: 12,
            color: BAR_BACK,
        });
        let amount = sim.stock(ware).min(SLOT_FULL[slot]);
        let filled = i32::try_from(amount).unwrap_or(0) * BAR_W
            / i32::try_from(SLOT_FULL[slot]).unwrap_or(1);
        if filled > 0 {
            commands.push(HudCommand::Rect {
                role: HudRole::StaminaFill,
                x: bar_x,
                y: bar_y,
                w: filled,
                h: 12,
                color: WARE_FILL[slot],
            });
        }
    }
}

/// The minimap frame and one pip per house and unit.
fn minimap(commands: &mut Vec<HudCommand>, sim: &Sim, width_px: i32, height_px: i32) {
    let map_x = width_px.saturating_sub(MARGIN).saturating_sub(MINIMAP_W);
    let map_y = height_px.saturating_sub(MARGIN).saturating_sub(MINIMAP_H);
    commands.push(HudCommand::Rect {
        role: HudRole::Panel,
        x: map_x + 2,
        y: map_y + 2,
        w: MINIMAP_W - 4,
        h: MINIMAP_H - 4,
        color: MAP_BACK,
    });
    commands.push(HudCommand::Border {
        x: map_x,
        y: map_y,
        w: MINIMAP_W,
        h: MINIMAP_H,
        thickness: 2,
        color: TEXT,
    });
    // HpPip is the only small-marker tag; the minimap uses it for houses and
    // units, coloured by kind and by side.
    for house in sim.houses() {
        commands.push(HudCommand::Rect {
            role: HudRole::HpPip,
            x: pip_x(map_x, house.x),
            y: pip_y(map_y, house.y),
            w: 4,
            h: 4,
            color: HOUSE_PIP,
        });
    }
    for unit in sim.units() {
        commands.push(HudCommand::Rect {
            role: HudRole::HpPip,
            x: pip_x(map_x, unit.x),
            y: pip_y(map_y, unit.y),
            w: 3,
            h: 3,
            color: if unit.side == 0 { SIDE0_PIP } else { SIDE1_PIP },
        });
    }
}

/// X pixel of a tile on the minimap: the inner map is 152 px wide.
fn pip_x(map_x: i32, tile: i32) -> i32 {
    map_x + 8 + tile * 152 / MAP_W
}

/// Y pixel of a tile on the minimap: the inner map is 96 px tall.
fn pip_y(map_y: i32, tile: i32) -> i32 {
    map_y + 8 + tile * 96 / MAP_H
}

#[cfg(test)]
mod tests {
    use super::*;
    use kam2_sim::{Command, UnitKind};

    fn start() -> Sim {
        crate::script::match_start().expect("the fixed start village places")
    }

    #[test]
    fn stock_text_names_every_ware_for_a_known_sim() {
        let mut sim = start();
        assert_eq!(
            stock_line(&sim),
            "WOOD 12 STONE 15 GRAIN 0 FLOUR 0 BREAD 0",
            "three houses cost 16 wood and 5 stone, the army 12 wood"
        );
        sim.apply(Command::SpawnUnit {
            kind: UnitKind::Swordsman,
            side: 0,
            x: 8,
            y: 2,
        })
        .expect("open ground");
        assert_eq!(
            stock_line(&sim),
            "WOOD 9 STONE 15 GRAIN 0 FLOUR 0 BREAD 0",
            "one swordsman costs three wood"
        );
    }

    #[test]
    fn the_hud_carries_the_stock_bar_and_the_minimap_frame() {
        let sim = start();
        let frame = hud_frame(1280, 720, &sim, false, HouseKind::Farm, "");
        assert_eq!(frame.width(), 1280);
        assert_eq!(frame.height(), 720);
        let mut has_stock = false;
        let mut has_title = false;
        let mut has_minimap = false;
        let mut bars = Vec::new();
        let mut pips = 0;
        for command in frame.commands() {
            match command {
                HudCommand::Text { text, .. } => {
                    if text == &stock_line(&sim) {
                        has_stock = true;
                    }
                    if text == "KAM3 PLAY tick=0 RUNNING BUILD=FARM" {
                        has_title = true;
                    }
                }
                HudCommand::Border { w, h, .. } => {
                    if *w == MINIMAP_W && *h == MINIMAP_H {
                        has_minimap = true;
                    }
                }
                HudCommand::Rect { role, color, w, .. } => {
                    if *role == HudRole::StaminaFill {
                        bars.push((*w, *color));
                    }
                    if *role == HudRole::HpPip {
                        pips += 1;
                    }
                }
                HudCommand::Scissor { .. } => {}
            }
        }
        assert!(has_stock, "the stock line is in the frame");
        assert!(has_title, "the status line is in the frame");
        assert!(has_minimap, "the minimap frame is in the frame");
        assert_eq!(
            bars,
            vec![(38, WARE_FILL[0]), (71, WARE_FILL[1])],
            "12 of 60 wood fills 38 px, 15 of 40 stone fills 71 px, no grain yet"
        );
        assert_eq!(pips, sim.houses().len() + sim.units().len(), "one pip each");
    }

    #[test]
    fn the_status_line_reports_pause_and_the_notice() {
        let sim = start();
        assert_eq!(
            title_line(&sim, true, HouseKind::Mill, "PLACED"),
            "KAM3 PLAY tick=0 PAUSED BUILD=MILL PLACED"
        );
        assert_eq!(
            title_line(&sim, false, HouseKind::Warehouse, ""),
            "KAM3 PLAY tick=0 RUNNING BUILD=WAREHOUSE"
        );
    }

    #[test]
    fn minimap_pips_stay_inside_their_frame() {
        let sim = start();
        let map_x = 1280 - MARGIN - MINIMAP_W;
        let map_y = 720 - MARGIN - MINIMAP_H;
        assert_eq!(pip_x(map_x, 0), map_x + 8);
        assert_eq!(pip_y(map_y, 0), map_y + 8);
        let houses = sim.houses().iter().map(|house| (house.x, house.y));
        let units = sim.units().iter().map(|unit| (unit.x, unit.y));
        for (x, y) in houses.chain(units) {
            assert!(
                pip_x(map_x, x) + 4 <= map_x + MINIMAP_W - 2,
                "pip at {x} leaves the frame"
            );
            assert!(
                pip_y(map_y, y) + 4 <= map_y + MINIMAP_H - 2,
                "pip at {y} leaves the frame"
            );
        }
    }
}
