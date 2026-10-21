//! kam2-web: KAM8 - the KAM2 village match in one browser tab.
//!
//! Consumer: `tools/kam2-web.mjs` builds this crate for wasm32-unknown-unknown,
//! wasm-bindgen turns it into `kam2web.js` plus `kam2web_bg.wasm`, and the page
//! under `crates/kam2-web/page/` draws the frame with Canvas2D. The module owns
//! the match ([`kam2_game::AiMatch`]) and a fixed 10 Hz tick the page drives with
//! the deltas of its own `performance.now()`; it reads no clock, no file and no
//! DOM, so the tick is tested with pinned deltas and never with real time.
//! Data owner: `kam2-sim` owns the match state, `kam2-game` owns the AI seat, the
//! 2.5D scene and the projection. This crate owns the browser clock, the pan and
//! the flat frame ([`scene_flat`]) the page reads in one typed array, the build
//! menu ([`build_menu`]) that names the five build cells of the sidebar, the
//! selection panel ([`selection`]) that answers what the cursor picked and the
//! orders the command row gives, and the minimap ([`minimap`]) that draws the
//! field and the window the camera looks at.
//! The page asks the sim for every answer: a placement probe and a build are
//! the sim's own command, never a copy of its rule.
//! Units: milliseconds in, ticks and framebuffer pixels out.
//! Determinism: the same start and the same tick count give the same hash, so a
//! page that only changes how often it draws never moves the sim.

pub mod build_menu;
pub mod minimap;
pub mod scene_flat;
pub mod selection;
pub mod shell;

#[cfg(target_arch = "wasm32")]
mod web;

pub use build_menu::{answer_line, house_kinds, kind_of, refusal_line, HOTKEYS};
pub use minimap::{minimap_json, MINIMAP_H, MINIMAP_W};
pub use scene_flat::{flatten, BILLBOARD_STRIDE, GROUND_STRIDE, HEADER, KIND_HOUSE, KIND_UNIT};
pub use selection::{
    command, select_at, selection_json, state_json, Order, Selection, NO_UNIT, UNKNOWN_ORDER,
};
pub use shell::{Shell, MAX_CATCHUP, PAN_LIMIT, PLAYER_SIDE, TICK_MS, VIEW_H, VIEW_W};
