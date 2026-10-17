# Rust and WASM web engines and tooling

Scout, 2026-10-17. Read: Bevy, wgpu, wasm-bindgen, rustc, getrandom, binaryen, web-time, cpal, Chrome, MDN, W3C WebGPU. GUESS = not read.

| Idea | Source and URL | Licence | Copy or do | Row | Effort |
|---|---|---|---|---|---|
| Web size profile | Bevy `[profile.wasm-release]`, github.com/bevyengine/bevy; min-sized-rust, github.com/johnthagen/min-sized-rust | Bevy GUESS MIT/Apache; min-sized-rust MIT | New `wasm-release` profile: inherits release, `opt-level = "z"`. Keep fat LTO, one codegen unit, `overflow-checks = true`. Measure z, s and 3 on the tape. | WEB11 | S |
| wasm-opt flags | binaryen wasm-opt manpage, manpages.debian.org | Binaryen GUESS Apache-2.0 | Rust 1.97.1 emits bulk-memory and nontrapping-fptoint by default. Pass `--enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext` (or `--all-features`) with `-Oz`. | WEB4 | S |
| Size measure | github.com/rustwasm/twiggy; cargo-bloat README | twiggy not stated; cargo-bloat MIT | cargo-bloat says "WASM is not supported". Print twiggy `top` (subcommand GUESS) for `engine_bg.wasm`. Strip after measuring: `--strip-debug` also drops names. | WEB11 | S |
| Init path | wgpu `examples/features/src/framework.rs`; wasm-bindgen `crates/cli-support/src/js/mod.rs` | GUESS MIT/Apache | Create the surface before the adapter (WebGL needs the canvas). Adapter and device via `spawn_local`, never `pollster`. Call `console_error_panic_hook::set_once()` first. `init` given a fetch Response uses `instantiateStreaming`. | WEB5, WEB4 | M |
| Adapter retry | developer.chrome.com/blog/new-in-webgpu-140 | Chrome docs | Chrome 140: a successful `requestDevice()` consumes the adapter; a second call rejects. On `device.lost`, request a new adapter. | WEB5 | S |
| Limits | docs.rs/wgpu `Limits` (30.0.1) | GUESS MIT/Apache | Do not copy `downlevel_webgl2_defaults` from the example: storage buffers, storage textures and workgroup storage are 0. Request `Limits::defaults()`. | WEB4, WEB5 | S |
| VR3 table | W3C WebGPU spec, w3.org/TR/webgpu | W3C GUESS | Defaults: uniform binding 65536 B (16384 is downlevel), vertex stride 2048 B (not 256). Add `maxUniformBuffersPerShaderStage` 12 and `maxComputeWorkgroupStorageSize` 16384. | VR3 | S |
| Browser RNG | getrandom README and CHANGELOG, github.com/rust-random/getrandom | MIT or Apache-2.0 | Lock has getrandom 0.2.17, 0.3.4, 0.4.3. Enable `js` (0.2) and `wasm_js` (0.3) on the binary crate only; 0.3.4+ needs no cfg; 0.4 name GUESS. | WEB2 | S |
| Clock seam | docs.rs/web-time | MIT or Apache-2.0 | `Instant::now()` and `SystemTime::now()` "will simply panic" on wasm32-unknown-unknown. Use `web-time` behind a cfg. | WEB2 | S |
| Headless and tests | developer.chrome.com/blog/supercharge-web-ai-testing; wasm-bindgen `wasm-bindgen-test/browsers.md` | Chrome docs; GUESS MIT/Apache | Chrome: `--headless=new --enable-unsafe-webgpu --use-angle=vulkan --enable-features=Vulkan --disable-vulkan-surface`; adapter at chrome://gpu. Tests: `CHROMEDRIVER=... WASM_BINDGEN_USE_BROWSER=1 cargo test --target wasm32-unknown-unknown`. | WEB4, WEB7 | M |
| Pointer and audio | MDN Pointer Lock and Autoplay; github.com/RustAudio/cpal | MDN GUESS; cpal Apache-2.0 | Pointer lock from a click (sandboxed iframes block it). Read `navigator.getAutoplayPolicy("audiocontext")`; `resume()` in the click. cpal `wasm-bindgen` feature; skip `audioworklet` (atomics, COOP/COEP). | WEB7 | S |
| Download progress | MDN Response.body | MDN GUESS | `body.getReader()` chunks drive the bundle bar; percent only with Content-Length. | WEB6 | S |
| WebGPU only | Bevy `webgpu` feature, its Cargo.toml | Bevy GUESS | Bevy keeps WebGL2 default; `webgpu` overrides it. We keep WebGPU only: wgpu fixes `BROWSER_WEBGPU` at `Instance` creation, with no documented fallback. Show a "no WebGPU" page. | WEB4, WEB8 | S |
| Dev hot reload | github.com/trunk-rs/trunk README; DioxusLabs subsecond | Trunk MIT or Apache-2.0; subsecond not stated | `trunk serve` rebuilds and reloads; dev only. Subsecond hot-patches Rust functions; its web support was not read. | NEW: web dev reload loop | M |

## Top 5 to do this week

1. Pin the wasm-bindgen CLI to the locked 0.2.128; add the wasm-opt flags and a twiggy line to `tools/web-build.mjs` (WEB4, WEB11).
2. Correct VR3's limits table before trusting its test (VR3).
3. Enable the browser RNG on the binary crate; add the clock seam (WEB2).
4. WebGPU init via `spawn_local`, a fresh adapter per retry, `Limits::defaults()` (WEB5).
5. Headless run prints which init path ran and the adapter line (WEB4, WEB10).

## Traps that cost others days

- `Instant::now()` panics at run time on wasm32-unknown-unknown; `cargo check` will not catch a dependency that calls it (our reasoning).
- The getrandom README does not say whether a missing backend fails at compile or run time. Check in a browser.
- `crates/web` has `wasm-bindgen = "0.2"`; the CLI must match the locked crate. The rustwasm org is being archived (September 2025); wasm-pack's status is unverified.
- On wasm32, `std::fs` returns errors, `std::thread::spawn` panics, `println!` does nothing. WEB2 and WEB6 must avoid them.
- Nightly `-Zfmt-debug=none` strips the Debug text that asserts print. Not for us.
- WebGPU needs a secure context: serve on 127.0.0.1 or localhost; 0.0.0.0 fails (wgpu docs).
- Dropping `overflow-checks` in the web profile lets wasm wrap where native panics, breaking parity (our reasoning).
- A wrong `application/wasm` MIME does not fail; it warns and takes the buffer path.
- Emscripten's module split cannot load lazily on the main thread. Park splitting until measured.
