//! web: the browser shell, a WebGPU device, a cleared canvas and one JSON line of facts (WEB4).
//!
//! The page loads `engine.js` (wasm-bindgen output of this crate), calls `run` with the id
//! of a canvas, and gets a WebGPU device through wgpu's browser backend. Every animation
//! frame clears the canvas to a slowly changing colour, draws one moving square, and updates
//! a frame counter. Once, it prints one JSON line (adapter, backend, limits, features, wasm
//! memory, commit) to the page and the console. No WebGPU (or no adapter) is not a crash:
//! the shell marks the page `data-state="fatal"`, fills the reason in, and prints a JSON
//! line with `"ok":false`; `index.html` owns the loud message and the browser versions.
//! Either way the shell then runs the golden tape and the cut corpus in the wasm module and
//! prints one `golden ... OK` (or `MISMATCH`) line: the engine runs here, not only the GPU.
//!
//! Input and audio (WEB7): [`input`] maps keyboard, mouse with pointer lock and the Gamepad API onto
//! `engine-input` records, [`audio`] decides when the WebAudio context may start; `browser` (wasm only)
//! wires both to the page and prints `input: kb=.. mouse=.. pad=..` and `audio: state=..`.
//!
//! The pure parts ([`anim`], [`audio`], [`golden`], [`input`], [`limits`], [`recovery`], [`report`]) build and
//! test on every target. The browser glue (`shell`) exists on `wasm32` only, so the native
//! workspace builds this crate as a library with no web dependencies. It depends on
//! `engine-proof` (std-only) for the golden check and on no render code yet: it must build
//! before the render crate itself does for the browser.

pub mod anim;
pub mod audio;
pub mod golden;
pub mod input;
pub mod limits;
pub mod recovery;
pub mod report;

#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(target_arch = "wasm32")]
mod shell;

/// The commit this build was made from, baked in by `tools/web-build.mjs` through the
/// `ENGINE_WEB_COMMIT` environment variable; `unknown` for a plain `cargo build`.
pub const COMMIT: &str = match option_env!("ENGINE_WEB_COMMIT") {
    Some(c) => c,
    None => "unknown",
};

/// The WGSL the shell draws its square with; the uniform block matches
/// [`anim::Square::to_bytes`].
pub const SHADER_WGSL: &str = include_str!("shader.wgsl");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_block_has_the_fields_square_to_bytes_writes_in_order() {
        let at = |needle: &str| {
            SHADER_WGSL
                .find(needle)
                .unwrap_or_else(|| panic!("shader.wgsl lost `{needle}`"))
        };
        let (centre, half, rgba) = (
            at("centre: vec2<f32>"),
            at("half: vec2<f32>"),
            at("rgba: vec4<f32>"),
        );
        assert!(centre < half && half < rgba, "field order drifted");
        assert!(SHADER_WGSL.contains("@group(0) @binding(0) var<uniform> square"));
        assert!(SHADER_WGSL.contains("fn vs(") && SHADER_WGSL.contains("fn fs("));
    }
}
