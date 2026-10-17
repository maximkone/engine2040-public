---
name: wasm-compat
description: "Use when a crate on the browser path must also compile and run for wasm32: cfg(target_arch = wasm32) patterns, engine_clock::install, the FsSource stub, tools/wasm-gate.mjs and its baseline, and the wasm-check.yml legs."
---

# Wasm compat

`cargo check --target wasm32-unknown-unknown` proves a crate type-checks for the browser, not that it runs there. On that target `std::fs` returns errors, `Instant::now()` and `SystemTime::now()` panic, threads and raw sockets fail, and a tab cannot block on the GPU. Spec: `docs/plan/web-and-realism.md`; rows WEB2, WEB6.

## The patterns used here
- **Clock.** Never `Instant::now()` on the web path. Read `engine_clock::now()` / `engine_clock::since(start)` (`crates/clock`). Native reads `Instant`; on `wasm32-unknown-unknown` a read panics until the shell calls `engine_clock::install(browser_clock)` once (`crates/web/src/shell.rs`, the page's `performance.now()`). The first source wins; a second install returns `false`. Cross-check: tests in `crates/clock/tests/it/install.rs`.
- **Files.** `crates/assets/src/asset_source.rs`: the `AssetSource` trait has `FsSource` (native: directory reads, byte for byte) and, under `#[cfg(target_arch = "wasm32")]`, a stub of the same name whose reads return `ErrorKind::Unsupported` by name. Callers compile on every target. A browser build reads a fetched bundle through `BundleSource` (`.e2bundle`, `asset_source/bundle.rs`). New file reads on the web path go through `AssetSource`, never `std::fs`.
- **Same function, two bodies.** Pair `#[cfg(target_arch = "wasm32")]` with `#[cfg(not(target_arch = "wasm32"))]` inside the function (see `deferred_validation_errors` in `crates/render/src/present/gpu_present/presenter.rs`: a browser answers wgpu error scopes late, so it counts them; native fails the frame at once and returns 0).
- **Native-only code.** Put `#[cfg(not(target_arch = "wasm32"))]` on the item or `mod` (bundle writer, `e2bundle` bin, thread pools) and give wasm a stub or leave it out. Tests that need files: `#[cfg(all(test, not(target_arch = "wasm32")))]`.
- **Browser-only glue.** Dependencies in `[target.'cfg(target_arch = "wasm32")'.dependencies]` (`crates/web/Cargo.toml`, `crates/net/Cargo.toml`), so native builds never see `wasm-bindgen`, `web-sys`, `js-sys`.
- **Blocking on the GPU.** No `pollster::block_on` or `device.poll(wait)` on the web path: use async and `spawn_local`.
- **Net.** QUIC (`quinn`, needs `mio`) stays native; the browser uses WebTransport with a WebSocket fallback. `engine-net --features quic` is not a wasm leg.
- `engine-core` and the sim crates stay std-only: nothing above enters a hash.

## The gate: tools/wasm-gate.mjs
- `node tools/wasm-gate.mjs` prints hazard totals per crate (`fs`, `clock`, `thread`, `socket`, `exe`, `block`) and fails (exit 1) when a file has more than `tools/wasm-gate.baseline.json` allows. `--list` adds every hit as `file:line`.
- The baseline only goes down: after you remove hazards run `node tools/wasm-gate.mjs --tighten` and commit the new baseline with your change. `--init` refuses when one exists. There is no way to raise a count; fix the code instead.
- It scans non-test source of the crates in `WEB_CRATES`, with strings and comments blanked, and skips items whose `cfg` is false for wasm32 (`not(target_arch = "wasm32")`, `windows`, `test`). A hit behind a feature flag still counts. Its tests: `node --test tools/wasm-gate.test.mjs`.

## The legs: .github/workflows/wasm-check.yml (public mirror)
- `check` runs `cargo check --locked --keep-going --target wasm32-unknown-unknown <args>` once per matrix leg: each headless crate, plus `engine-physics --features rapier`, `engine-physfx --features rapier`, `engine-arena --features sim`, `engine-assets --features import`, `engine-render` (default, `v1`, `window`), `engine-game --features app`, `engine-audio --features device`, `engine-net`, `engine-app`, `engine2040`. No `continue-on-error`.
- A new crate or feature on the web path gets its own leg in the same commit. `gate` runs the wasm-gate tests and `node tools/wasm-gate.mjs --list`.
- Start it by pushing a throwaway branch `wasm-check-*` of the mirror, or a pull request; for one leg locally use `cargo check --locked --target wasm32-unknown-unknown -p <crate>` (needs `rustup target add wasm32-unknown-unknown`; `node tools/web-doctor.mjs` says what is missing).
- Running, not just compiling: golden tape, corpus and Rapier ragdoll on `wasm32-wasip1` are in `golden-matrix.yml` (skill `golden-matrix`), the page in `web-build.yml` (skill `web-build-and-test`).

## Pitfalls we hit
- A browser has no OS random source: `uuid` needs `features = ["js"]` under a wasm32 target table (`crates/scene/Cargo.toml`) or the crate fails to build for the browser. Same for any crate that reads randomness from the OS.
- A crate that type-checks for wasm can still trap at startup (`Instant::now`, `std::fs`): trust the gate and a browser run, not the check.
- Leaving a wasm-bindgen CLI of another version than `Cargo.lock` locks gives a load error: `tools/web-build.mjs` prints the exact `cargo install wasm-bindgen-cli --version <v> --locked` fix.
- Heavy wasm builds wait on the cargo lock like any build: `Blocking waiting for file lock` is another run, wait.
