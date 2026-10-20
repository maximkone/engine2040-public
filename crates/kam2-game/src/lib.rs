//! kam2-game: KAM6 first playable match - 16x12 window village and the AI mayor on side 1.
//!
//! Consumer: the `kam2play` binary (feature `window`), which opens one 1280x720
//! window, ticks the match at a fixed 10 Hz, draws the field, houses and units
//! in the locked 2.5D view, and saves one frame as a PNG receipt; and the
//! headless `kam2match` binary, which plays the same loop with no window.
//! Data owner: `kam2-sim` owns the match; this crate owns the match loop, the
//! side-1 AI seat and presentation. It never feeds a frame or a clock reading
//! back into the sim.
//! Units: tiles for the sim, framebuffer pixels for the view and the HUD.
//! Determinism: [`script`] and [`view`] are pure, and [`ai_match::AiMatch`]
//! reads no clock, so the tests pin the mapping, one scripted fight and the
//! 3000-tick AI match hash without a window.
//!
//! The window leg (bin) keeps GPU types out of this library: every quad is
//! computed here or in [`scene`], and the binary only uploads the triangles.

pub mod ai_match;
pub mod house;
pub mod hud;
pub mod scene;
pub mod script;
pub mod soldier;
pub mod view;

pub use ai_match::{AiMatch, AI_PERIOD, AI_SIDE, DEFENCE_RANGE, RECEIPT_TICKS};
pub use house::{HouseManifest, HouseState};
pub use hud::{hud_frame, stock_line, title_line};
pub use scene::{build_with, Part, Scene, ShapeQuad, SpriteQuad, Units};
pub use script::{match_start, nearest_enemy, scripted_fight, spawn_tile};
pub use soldier::{
    frame_index, ManifestError, SoldierAnim, SoldierManifest, UnitClip, FACING_NAMES, SIM_HZ,
};
pub use view::{
    footprint_center_world, footprint_screen, screen_to_tile, tile_center_world, tile_diamond,
    tile_screen, MAP_H, MAP_W,
};
