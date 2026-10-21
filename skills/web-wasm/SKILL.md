---
name: web-wasm
description: "Use when a crate must build or run for wasm32 or the browser: cfg(target_arch) patterns, clock and file shims, wasm-gate, wasm-check legs, dist/web build, headless Chrome proof, web workflows. Real-browser check: browser-test-on-this-pc."
---

# Web and wasm32

The browser shell is `crates/web` (package `engine-web`). It runs the golden tape in the page and draws with WebGPU. Spec: `docs/plan/web-and-realism.md`. Rows: `docs/SPRINT-22-web-realism.md`. A golden mismatch on wasm goes to skill `determinism-debug`. Heavy cargo and the mirror push go through skill `github-check`.

## 1. What wasm32 breaks, and the patterns

`cargo check --target wasm32-unknown-unknown` proves a crate type-checks for the browser, not that it runs there. On that target:

- `std::fs` returns errors.
- `Instant::now()` and `SystemTime::now()` panic.
- Threads and raw sockets fail.
- A tab cannot block on the GPU.

Patterns used here:

- **Clock.** Never `Instant::now()` on the web path. Read `engine_clock::now()` or `engine_clock::since(start)` (`crates/clock/src/lib.rs`). Native reads `Instant`. On `wasm32-unknown-unknown` a read panics until the shell calls `engine_clock::install(browser_clock)` once (`crates/web/src/shell.rs`, the page's `performance.now()`). The first source wins. A second install returns `false`. Tests: `crates/clock/tests/it/install.rs`.
- **Files.** `crates/assets/src/asset_source.rs`: the `AssetSource` trait has `FsSource` (native: directory reads, byte for byte). Under `#[cfg(target_arch = "wasm32")]` a stub of the same name returns `ErrorKind::Unsupported`. Callers compile on every target. A browser build reads a fetched bundle through `BundleSource` (`.e2bundle`, `crates/assets/src/asset_source/bundle.rs`). New file reads on the web path go through `AssetSource`, never `std::fs`.
- **Same function, two bodies.** Pair `#[cfg(target_arch = "wasm32")]` with `#[cfg(not(target_arch = "wasm32"))]` inside the function. Example: `crates/render/src/present/gpu_present/device_init.rs::Presenter::deferred_validation_errors`. A browser answers wgpu error scopes late, so it counts them. Native fails the frame at once and returns 0.
- **Native-only code.** Put `#[cfg(not(target_arch = "wasm32"))]` on the item or `mod` (bundle writer, `e2bundle` bin, thread pools). Give wasm a stub or leave it out. Tests that need files: `#[cfg(all(test, not(target_arch = "wasm32")))]`.
- **Browser-only glue.** Dependencies go in `[target.'cfg(target_arch = "wasm32")'.dependencies]` (`crates/web/Cargo.toml`, `crates/net/Cargo.toml`), so native builds never see `wasm-bindgen`, `web-sys` or `js-sys`.
- **Blocking on the GPU.** No `pollster::block_on` or `device.poll(wait)` on the web path. Use async and `spawn_local`.
- **Net.** QUIC (`quinn`, needs `mio`) stays native. The browser uses WebTransport with a WebSocket fallback. `engine-net --features quic` is not a wasm leg.
- `engine-core` and the sim crates stay std-only. Nothing above enters a hash.

## 2. The gate: tools/wasm-gate.mjs

- `node tools/wasm-gate.mjs` prints hazard totals per crate (`fs`, `clock`, `thread`, `socket`, `exe`, `block`). It exits 1 when a file has more than `tools/wasm-gate.baseline.json` allows. `--list` adds every hit with its location.
- The baseline only goes down. After you remove hazards run `node tools/wasm-gate.mjs --tighten` and commit the new baseline with your change. `--init` refuses when one exists. There is no way to raise a count. Fix the code instead.
- It scans non-test source of the crates in `WEB_CRATES`, with strings and comments blanked. It skips items whose `cfg` is false for wasm32 (`not(target_arch = "wasm32")`, `windows`, `test`). A hit behind a feature flag still counts.
- Its tests: `node --test tools/wasm-gate.test.mjs`.

## 3. The legs: .github/workflows/wasm-check.yml (public mirror)

- `check` runs `cargo check --locked --keep-going --target wasm32-unknown-unknown <args>` once per matrix leg, one job each. No `continue-on-error`.
- Legs: each headless crate, plus `engine-physics --features rapier`, `engine-physfx --features rapier`, `engine-arena --features sim`, `engine-assets --features import`, `engine-geo --features collision`, `engine-render` (default, `v1`, `window`), `engine-game --features app`, `engine-audio --features device`, `engine-net`, `engine-app`, `engine2040`.
- A new crate or feature on the web path gets its own leg in the same commit.
- `gate` runs the wasm-gate tests and `node tools/wasm-gate.mjs --list`.
- Start it by pushing a throwaway branch `wasm-check-*` of the mirror, or a pull request. For one leg locally: `cargo check --locked --target wasm32-unknown-unknown -p <crate>` (needs `rustup target add wasm32-unknown-unknown`; `node tools/web-doctor.mjs` says what is missing).
- Running, not just compiling: golden tape, corpus and Rapier ragdoll on `wasm32-wasip1` are in `golden-matrix.yml` (skill `determinism-debug`). The page runs in `web-build.yml` (section 5).
- `wasm-probe.yml` (branch `wasm-probe-*`) is a feasibility probe on throwaway runs: wasi golden tape, a `wasm32-unknown-unknown` run under Node, wasm size, and which flags give a software WebGPU adapter. Nothing in it ships.

## 4. Build dist/web

1. `node tools/web-doctor.mjs`: one line per piece (`OK` or `MISSING` plus `fix:`), last line `web target: READY` or `NOT READY n missing`. It installs nothing.
2. `node tools/web-build.mjs [--out dist/web] [--commit <hash>] [--no-opt]`.
   - Steps: `cargo build --locked --profile web --target wasm32-unknown-unknown -p engine-web`, then the wasm-bindgen CLI at the exact version `Cargo.lock` locks, then `wasm-opt` (`WASM_OPT_FLAGS` in the tool: `-Oz` plus the `--enable-*` flags) when installed.
   - Output: `index.html`, `engine.js`, `engine_bg.wasm`, `build.json` (sizes, gzip, SHA-256, commit).
   - Exit 0 is complete and under 8 MiB (`SIZE_LIMIT_BYTES`). Exit 1 is a failed step, and it says which.
3. `node tools/wasm-info.mjs dist/web/engine_bg.wasm` prints imports, tables and memories as JSON.
4. Budgets (WEB11): `node tools/web-budget.mjs --dir dist/web [--receipt out/web/receipt.json --require-receipt]` against `tools/web-budget.json`.
5. Look at it yourself: `python -m http.server 8080 --bind 127.0.0.1 -d dist/web`, then open `http://localhost:8080/`. WebGPU needs https or localhost. `.wasm` must be served as `application/wasm`.

## 5. Headless Chrome proof

On real hardware, in the Claude app's browser pane, against the published Pages URL: skill `browser-test-on-this-pc`.

- `node tools/web-browse.mjs --dir dist/web --chrome <chrome path> --out out/web [--require-adapter] [--only <n>] [--all]`.
- It serves loopback only. It tries software-adapter flag sets (`FLAG_SETS` in the tool) until one gives an adapter. It screenshots the canvas (must not be blank), checks frames keep coming, and checks the page survives `device.destroy()`. Then it reruns with `navigator.gpu` removed: the page must show the loud message and `ok:false reason no-webgpu`.
- Exit: 0 proven, 1 a proof failed, 2 could not run (no Chrome), 3 no adapter with `--require-adapter`. Every failed attempt is written to `attempts.json`.
- Flag line that works on ubuntu-latest (Chrome 155, no GPU): `google-chrome --headless=new --no-sandbox --disable-dev-shm-usage --no-first-run --no-default-browser-check --hide-scrollbars --mute-audio --window-size=1280,720 --enable-unsafe-webgpu --enable-features=Vulkan --use-angle=swiftshader --use-webgpu-adapter=swiftshader`.
- `--use-angle=swiftshader` is what makes presentation work. `--use-vulkan=...` gives a blank canvas. `--enable-unsafe-webgpu` alone gives an adapter whose device is lost again and again.
- A runner needs `sudo apt-get install -y libvulkan1 mesa-vulkan-drivers`, or the canvas never presents.
- Input and audio (WEB7): `node tools/web-input-check.mjs --dir dist/web --chrome <chrome path> [--out out/web-input]` sends real key and mouse events over CDP. No audio context before a gesture. Escape is not a gesture. A key press starts it. A click leaves it running. Exit 0 proven, 1 failed, 2 could not run.
- Input tape (WEB7): the same tool replays `tools/fixtures/input-tape.script` (`key <code> down|up`, `mouse <button> down|up`, `tick`) as real events, calls `window.inputSample()` at each `tick`, and with `--native-tape <file>` (the output of `cargo run -p engine-web --bin input_tape -- <script>`) fails unless page and PC give the same bytes and digest. `web-build.yml` makes the native tape in the `build` job and hands it to the `browser` job.
- Device loss (QW8): `window.loseDevice()` is a forced loss (the page starts over, the next generation's receipt carries `losses` and `lastLoss` with the browser's reason and who ended the device). `window.destroyDevice()` is a requested destroy: the page ends in `data-state=stopped`, no new generation (`pagehide` calls it). Three losses in ten seconds end in the fatal page with the reason. `web-browse.mjs` drills both (`lossVerdict`, `destroyVerdict`).
- Fallbacks (QT11): every way below the web tier names itself. The fatal page and the failure line carry `fallback=<no-webgpu|no-adapter|no-device|no-canvas|no-surface>`; the tier line carries `fallback=<limits|denylist|probe|none>` and `adapter=<text|none>`.
- The golden line: the page prints `golden <16 hex> state <16 hex> corpus <16 hex> OK <n> ms` in `#golden` and sets `data-golden` on body to `ok`, `mismatch` or `error`. The tool matches the shape `GOLDEN_OK` in `tools/web-browse.mjs`. `golden MISMATCH got ... expected ...` means the wasm engine diverged: go to skill `determinism-debug`. Expected golden is `d57b5c8ae3096614`.

## 6. Workflows and the wasm-opt pin (public mirror only, repo `maximkone/engine2040-public`)

- `web-build.yml`: job `build` (unit tests of the web tools, `cargo test -p engine-web --lib`, clippy for wasm32, build, 8 MB and MIME check), then job `browser` (headless Chrome with `--require-adapter`, arena capture, web budgets, input check, every flag set for the record). Start it by pushing a throwaway branch `web-build-*` of the mirror, or dispatch.
- Run it on an isolated snapshot: `node tools/web-run.mjs [--budget <s>] [--file <tree path>=<local file>] <path> [<path> ...]`. The tree is committed HEAD plus only the paths you name, so another AI's half-written crate cannot turn your run red. Exit 0 succeeded, 1 failed, 2 could not run, 3 still running (not a failure: run the printed `--wait web-build-xxxx` line).
- `pages.yml`: the same build, plus the mdBook handbook at `/docs/` and rustdoc at `/docs/api/`. `tools/book-links.mjs` fails the build on a broken internal link. It deploys on mirror `main` pushes only.
- `wasm-check.yml`, `wasm-probe.yml` and `golden-matrix.yml`: sections above and skill `determinism-debug`.
- `wasm-opt`: never apt's binaryen 108. Its output fails in every browser (`Table.grow ... failed to grow table by 4`). The workflows pin release `version_133`. Rust 1.97 needs the `--enable-*` flags in `WASM_OPT_FLAGS` or wasm-opt rejects the module.
- Getting your committed tree to the mirror: `node tools/public-export.mjs` then `--commit --push`, and read it with `--ci`. The steps are in skill `github-check`. The export takes committed `HEAD`, not your dirty files.

## 7. Pitfalls

- A browser has no OS random source. `uuid` needs `features = ["js"]` under a wasm32 target table (`crates/scene/Cargo.toml`) or the crate fails to build for the browser. Same for any crate that reads randomness from the OS.
- A crate that type-checks for wasm can still trap at startup (`Instant::now`, `std::fs`). Trust the gate and a browser run, not the check.
- A wasm-bindgen CLI of another version than `Cargo.lock` gives a load error. `tools/web-build.mjs` prints the exact `cargo install wasm-bindgen-cli --version <v> --locked` fix.
- Heavy wasm builds wait on the cargo lock like any build. `Blocking waiting for file lock` is another run: wait.
- Reading Actions logs while a run is still going gives partial text. Wait for the run to end.
