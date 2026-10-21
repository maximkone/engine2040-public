//! Windowed KAM1c proof: the KAM1b 2.5D scene plus one hud_pass HUD frame.
//!
//! The HUD is a stock bar with three wares, a minimap frame placeholder and
//! two header text lines, all through `engine-ui`'s flat command list. The
//! hud pass composites it on the readback frame, so the saved PNG carries
//! both the scene and the HUD.

#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

#[cfg(not(target_arch = "wasm32"))]
use engine_render::kam2_sprite_proof::{run_proof_with_hud, SpriteProofError, WINDOW_H, WINDOW_W};
#[cfg(not(target_arch = "wasm32"))]
use engine_ui::text::hud_text_px;
#[cfg(not(target_arch = "wasm32"))]
use engine_ui::{HudCommand, HudFrame, HudRole};

/// Stock panel color: near-black HUD background, mostly opaque over the field.
#[cfg(not(target_arch = "wasm32"))]
const PANEL: [u8; 4] = [16, 18, 24, 220];
/// Text color: palette foreground.
#[cfg(not(target_arch = "wasm32"))]
const TEXT: [u8; 4] = [245, 245, 245, 255];
/// Ware fill: wood.
#[cfg(not(target_arch = "wasm32"))]
const WOOD: [u8; 4] = [166, 124, 82, 255];
/// Ware fill: stone.
#[cfg(not(target_arch = "wasm32"))]
const STONE: [u8; 4] = [150, 158, 170, 255];
/// Ware fill: iron.
#[cfg(not(target_arch = "wasm32"))]
const IRON: [u8; 4] = [205, 214, 226, 255];
/// Minimap placeholder background, translucent so the field reads through.
#[cfg(not(target_arch = "wasm32"))]
const MAP_BACK: [u8; 4] = [8, 10, 14, 180];
/// Stock slots: left edge, label, units held and fill color.
#[cfg(not(target_arch = "wasm32"))]
const SLOTS: [(i32, &str, u32, [u8; 4]); 3] = [
    (36, "WOOD", 120, WOOD),
    (206, "STONE", 80, STONE),
    (376, "IRON", 45, IRON),
];
/// Units a full slot holds; every fill scales against it.
#[cfg(not(target_arch = "wasm32"))]
const SLOT_FULL: u32 = 120;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), SpriteProofError> {
    let frame = stock_hud(WINDOW_W, WINDOW_H);
    let proof = run_proof_with_hud(target_png(), Some(frame))?;
    println!(
        "KAM1C OK hud={} png={}",
        proof.hud_draws,
        proof.png.display()
    );
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
fn target_png() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest
        .ancestors()
        .nth(2)
        .unwrap_or_else(|| Path::new("."));
    root.join("target").join("kam1c.png")
}

/// One HUD frame for the proof: viewport scissor, stock panel with three
/// wares, minimap placeholder frame and two text lines. ASCII only so the
/// 5x7 atlas covers every glyph.
#[cfg(not(target_arch = "wasm32"))]
fn stock_hud(width: u32, height: u32) -> HudFrame {
    let width_px = i32::try_from(width).unwrap_or(8192);
    let height_px = i32::try_from(height).unwrap_or(8192);
    let px = hud_text_px(height);
    let mut commands = Vec::with_capacity(24);
    commands.push(HudCommand::Scissor {
        x: 0,
        y: 0,
        w: width_px,
        h: height_px,
    });
    commands.push(HudCommand::Rect {
        role: HudRole::Panel,
        x: 24,
        y: 24,
        w: 540,
        h: 88,
        color: PANEL,
    });
    commands.push(HudCommand::Border {
        x: 24,
        y: 24,
        w: 540,
        h: 88,
        thickness: 2,
        color: TEXT,
    });
    for (left, label, units, fill) in SLOTS {
        let slot_w = 152;
        commands.push(HudCommand::Rect {
            role: HudRole::BarBack,
            x: left,
            y: 72,
            w: slot_w,
            h: 12,
            color: [56, 60, 72, 255],
        });
        let filled =
            slot_w * i32::try_from(units).unwrap_or(0) / i32::try_from(SLOT_FULL).unwrap_or(1);
        if filled > 0 {
            commands.push(HudCommand::Rect {
                role: HudRole::StaminaFill,
                x: left,
                y: 72,
                w: filled,
                h: 12,
                color: fill,
            });
        }
        commands.push(HudCommand::Text {
            x: left + 4,
            y: 40,
            px,
            color: TEXT,
            text: label.to_owned(),
        });
    }
    let map_x = width_px.saturating_sub(24).saturating_sub(160);
    let map_y = height_px.saturating_sub(24).saturating_sub(160);
    commands.push(HudCommand::Rect {
        role: HudRole::Panel,
        x: map_x.saturating_add(2),
        y: map_y.saturating_add(2),
        w: 156,
        h: 156,
        color: MAP_BACK,
    });
    commands.push(HudCommand::Border {
        x: map_x,
        y: map_y,
        w: 160,
        h: 160,
        thickness: 2,
        color: TEXT,
    });
    commands.push(HudCommand::Text {
        x: 24,
        y: 128,
        px,
        color: TEXT,
        text: "KAM1C HUD PASS".to_owned(),
    });
    commands.push(HudCommand::Text {
        x: 24,
        y: 152,
        px,
        color: TEXT,
        text: "STOCK BAR AND MINIMAP".to_owned(),
    });
    HudFrame::from_commands(width, height, px, commands)
}

/// The proof writes a PNG and blocks on the GPU adapter: native only.
#[cfg(target_arch = "wasm32")]
fn main() {}
