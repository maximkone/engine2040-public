//! The wasm-bindgen face the page calls. Every rule lives in [`crate::shell`];
//! this module only moves values across the JS boundary.
//!
//! The page calls `advance(dtMs)` once per animation frame with the delta of the
//! page's own clock, reads `frame()` into one `Float32Array`, and forwards the
//! two keys of the milestone: `togglePause()` for Space and `pan(dx, dz)` for the
//! arrow keys. The sidebar of the milestone asks the same face: `houseKinds()`
//! lists the five build cells, `placement(kind, x, z)` asks the sim whether a
//! house would stand there, `build(kind, x, z)` places one and `stockOf(side)`
//! counts the wares. The page never decides a rule of its own.

use js_sys::Float32Array;
use wasm_bindgen::prelude::*;

use crate::shell::Shell;

/// One browser match: the start village with the AI mayor in the side-1 seat.
#[wasm_bindgen]
pub struct Kam2Web {
    shell: Shell,
}

#[wasm_bindgen]
impl Kam2Web {
    /// Build the start village; a refusal is a page error, never a silent miss.
    ///
    /// # Errors
    ///
    /// Returns the refusal text when the fixed start village no longer fits.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Result<Kam2Web, JsValue> {
        Shell::new().map(|shell| Self { shell }).map_err(|refusal| {
            JsValue::from_str(&format!("the start village was refused: {refusal:?}"))
        })
    }

    /// Hand in one frame's delta in milliseconds; returns the ticks it played.
    pub fn advance(&mut self, dt_ms: f64) -> u32 {
        self.shell.advance_ms(dt_ms)
    }

    /// The flat 2.5D frame, layout in [`crate::scene_flat`].
    #[wasm_bindgen(js_name = frame)]
    pub fn flat_frame(&self) -> Float32Array {
        Float32Array::from(self.shell.frame().as_slice())
    }

    /// Shift the view by a world-space delta on the ground plane.
    pub fn pan(&mut self, dx: f32, dz: f32) {
        self.shell.pan(dx, dz);
    }

    /// Hold or release the match; Space calls this.
    #[wasm_bindgen(js_name = togglePause)]
    pub fn toggle_pause(&mut self) -> bool {
        self.shell.toggle_pause()
    }

    /// True while the match is held.
    #[wasm_bindgen(getter)]
    pub fn paused(&self) -> bool {
        self.shell.paused()
    }

    /// Ticks played so far.
    #[wasm_bindgen(getter)]
    pub fn ticks(&self) -> u32 {
        self.shell.ticks()
    }

    /// State digest of the match, sixteen lowercase hex digits.
    #[wasm_bindgen(getter)]
    pub fn hash(&self) -> String {
        self.shell.hash()
    }

    /// The five-ware stock line.
    #[wasm_bindgen(getter)]
    pub fn stock(&self) -> String {
        self.shell.stock()
    }

    /// Houses on the field.
    #[wasm_bindgen(getter)]
    pub fn houses(&self) -> u32 {
        self.shell.houses()
    }

    /// Units on the field.
    #[wasm_bindgen(getter)]
    pub fn units(&self) -> u32 {
        self.shell.units()
    }

    /// The pan the camera holds, as `[x, z]` world units.
    #[wasm_bindgen(getter, js_name = panOffset)]
    pub fn pan_offset(&self) -> Float32Array {
        Float32Array::from(self.shell.pan_offset().as_slice())
    }

    /// The five build cells as one JSON line: id, label, footprint and cost.
    #[wasm_bindgen(js_name = houseKinds)]
    pub fn house_kinds() -> String {
        crate::build_menu::house_kinds()
    }

    /// What the sim says about a house standing at `(x, z)`: an empty string
    /// when the sim would take it, the sim's own refusal words otherwise.
    #[wasm_bindgen]
    pub fn placement(&self, kind: &str, x: i32, z: i32) -> String {
        let Some(kind) = crate::build_menu::kind_of(kind) else {
            return crate::build_menu::UNKNOWN_KIND.to_string();
        };
        crate::build_menu::answer_line(self.shell.placement(kind, x, z))
    }

    /// Place the armed house at `(x, z)` for the player: an empty string when
    /// the sim took it, the sim's own refusal words otherwise.
    #[wasm_bindgen]
    pub fn build(&mut self, kind: &str, x: i32, z: i32) -> String {
        let Some(kind) = crate::build_menu::kind_of(kind) else {
            return crate::build_menu::UNKNOWN_KIND.to_string();
        };
        crate::build_menu::answer_line(self.shell.build(kind, x, z))
    }

    /// Place a house for one side; the same words as [`Kam2Web::build`].
    #[wasm_bindgen(js_name = buildFor)]
    pub fn build_for(&mut self, side: u8, kind: &str, x: i32, z: i32) -> String {
        let Some(kind) = crate::build_menu::kind_of(kind) else {
            return crate::build_menu::UNKNOWN_KIND.to_string();
        };
        crate::build_menu::answer_line(self.shell.build_for(side, kind, x, z))
    }

    /// How many of each ware one side holds, in ware order.
    #[wasm_bindgen(js_name = stockOf)]
    pub fn stock_of(&self, side: u8) -> Vec<u32> {
        self.shell.stock_counts(side)
    }
}
