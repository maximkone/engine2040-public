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
//! `engine-input` records, [`audio`] decides when the WebAudio context may start, and
//! [`audio_worklet`] is the sink the engine's mixer writes (an `AudioWorklet` ring with the
//! deprecated `ScriptProcessorNode` as the printed fallback); `browser` (wasm only) wires both to
//! the page and prints `input: kb=.. mouse=.. pad=..` and `audio: state=.. underruns=..`.
//!
//! Net duel (WEB9, WEB12): `?server=ws://host:port` makes the page join a native dedicated server
//! through the connect race (`netplay`, wasm only), play a scripted 600-tick duel and print
//! `net: transport=<webtransport|websocket> ticks=<n> hash=<hex> peers=<n>`; [`netquery`] reads the address.
//!
//! Device loss, time and pacing (QW8, QW11, QT2 to QT11): [`recovery`] tells a real or forced loss (start over)
//! from a requested destroy (stop) and keeps the browser's own loss reason for the receipt; [`anim::StepClock`]
//! turns the animation-frame timestamp into 60 fixed steps a second; [`tier_view`] holds the frame pacing
//! (window, ladder, drift, hidden gaps) and the startup probe's bookkeeping, all as plain data.
//!
//! The pure parts ([`anim`], [`audio`], [`audio_worklet`], [`content`], [`golden`], [`input`], [`limits`], [`netquery`], [`recovery`], [`report`]) build and
//! test on every target. The browser glue (`shell`) exists on `wasm32` only, so the native
//! workspace builds this crate as a library with no web dependencies. It depends on
//! `engine-proof` (std-only) for the golden check. On `wasm32` it also draws the arena frame
//! through the render crate's presenter (WEB5, `arena`): a device the page opened with `await`s,
//! a procedural lit arena with eight GPU-skinned fighters, no blocking call anywhere on the path,
//! and a GPU copy of the canvas that the headless check hashes and turns into a PNG.

pub mod anim;
pub mod arena_view;
pub mod audio;
pub mod audio_worklet;
pub mod content;
pub mod exchange;
pub mod frame_bus;
pub mod golden;
pub mod input;
pub mod input_tape;
pub mod limits;
pub mod netquery;
pub mod perf_window;
pub mod recovery;
pub mod report;
pub mod tier_view;

#[cfg(target_arch = "wasm32")]
mod arena;
#[cfg(target_arch = "wasm32")]
mod browser;
#[cfg(target_arch = "wasm32")]
mod fight;
#[cfg(target_arch = "wasm32")]
mod netplay;
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

    /// The page's per-seat arrays must be exactly as long as the match has seats. `SEATS_LEN`
    /// is written out because `usize::from` is not const-callable yet; this test is what keeps
    /// the two equal, so a seat added to `page_duel` fails here instead of silently dropping a
    /// seat's HUD. Both constants sit behind the wasm target (`fight` and `engine-game`), so the
    /// test compiles and runs on the wasm build only.
    #[cfg(target_arch = "wasm32")]
    #[test]
    fn seats_len_matches_the_match_seats() {
        assert_eq!(
            fight::SEATS_LEN,
            usize::from(fight::SEATS),
            "SEATS_LEN must equal the page match's seat count"
        );
    }

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
