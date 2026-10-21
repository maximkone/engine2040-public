---
name: web-wasm
description: "Use when a crate must build or run for wasm32 or the browser: cfg(target_arch) patterns, clock and file shims, wasm-gate, wasm-check legs, dist/web build, headless Chrome proof, web workflows, or checking the published Pages build in the Claude app's browser pane."
---

# Web and wasm32

The browser shell is `crates/web` (package `engine-web`). It runs the golden tape in the page and draws with WebGPU. Spec: `docs/plan/web-and-realism.md`. Rows: `docs/SPRINT-22-web-realism.md`. A golden mismatch on wasm goes to skill `determinism-debug`. Heavy cargo and the mirror push go through skill `github-check`.

## 1. What wasm32 breaks, and the patterns

`cargo check --target wasm32-unknown-unknown` proves a crate type-checks for the browser, not that it runs there. On that target `std::fs` errors, `Instant::now()` and `SystemTime::now()` panic, threads and raw sockets fail, and a tab cannot block on the GPU.

- **Clock.** Never `Instant::now()` on the web path. Read `engine_clock::now()` or `engine_clock::since(start)` (`crates/clock/src/lib.rs`). On wasm a read panics until the shell calls `engine_clock::install(browser_clock)` once (`crates/web/src/shell.rs`). The first source wins. Tests: `crates/clock/tests/it/install.rs`.
- **Files.** `crates/assets/src/asset_source.rs`: `FsSource` on native; under `#[cfg(target_arch = "wasm32")]` a stub of the same name returns `ErrorKind::Unsupported`. A browser reads a fetched bundle through `BundleSource` (`crates/assets/src/asset_source/bundle.rs`). New file reads on the web path go through `AssetSource`, never `std::fs`.
- **Same function, two bodies.** Pair `#[cfg(target_arch = "wasm32")]` with `#[cfg(not(target_arch = "wasm32"))]` inside it. Example: `crates/render/src/present/gpu_present/device_init.rs::Presenter::deferred_validation_errors` (a browser answers error scopes late; native fails the frame at once).
- **Native-only code.** Put `#[cfg(not(target_arch = "wasm32"))]` on the item or `mod` (bundle writer, thread pools) and give wasm a stub or nothing. File-based tests: `#[cfg(all(test, not(target_arch = "wasm32")))]`.
- **Browser-only glue.** Dependencies go in `[target.'cfg(target_arch = "wasm32")'.dependencies]` (`crates/web/Cargo.toml`, `crates/net/Cargo.toml`), so native builds never see `wasm-bindgen`, `web-sys` or `js-sys`.
- **No blocking on the GPU.** No `pollster::block_on` or `device.poll(wait)` on the web path; use async and `spawn_local`.
- **Net.** QUIC (`quinn`, needs `mio`) stays native. The browser uses WebTransport with a WebSocket fallback.
- `engine-core` and the sim crates stay std-only. Nothing above enters a hash.

## 2. The gate: tools/wasm-gate.mjs

- `node tools/wasm-gate.mjs` prints hazard totals per crate (`fs`, `clock`, `thread`, `socket`, `exe`, `block`) and exits 1 when a file has more than `tools/wasm-gate.baseline.json` allows. `--list` adds every hit.
- The baseline only goes down. After you remove hazards run `node tools/wasm-gate.mjs --tighten` and commit it. There is no way to raise a count: fix the code. Tests: `node --test tools/wasm-gate.test.mjs`.

## 3. The legs: .github/workflows/wasm-check.yml (public mirror)

- `check` runs `cargo check --locked --keep-going --target wasm32-unknown-unknown <args>`, one job per leg, no `continue-on-error`. Legs: each headless crate, plus `engine-physics` and `engine-physfx` (`--features rapier`), `engine-arena --features sim`, `engine-assets --features import`, `engine-geo --features collision`, `engine-render` (default, `v1`, `window`), `engine-game --features app`, `engine-audio --features device`, `engine-net`, `engine-app`, `engine2040`. A new crate or feature on the web path gets its own leg in the same commit.
- `gate` runs the wasm-gate tests and `--list`. Start the workflow by pushing a throwaway `wasm-check-*` branch of the mirror. One leg locally: `cargo check --locked --target wasm32-unknown-unknown -p <crate>` (`node tools/web-doctor.mjs` says what is missing).
- Running, not just compiling: golden tape and Rapier ragdoll on `wasm32-wasip1` are in `golden-matrix.yml` (skill `determinism-debug`). `wasm-probe.yml` is a throwaway feasibility probe; nothing in it ships.

## 4. Build dist/web

1. `node tools/web-doctor.mjs`: one line per piece (`OK` or `MISSING` plus `fix:`), last line `web target: READY` or `NOT READY n missing`. It installs nothing.
2. `node tools/web-build.mjs [--out dist/web] [--commit <hash>] [--no-opt]`: builds `engine-web` with `--profile web`, runs the wasm-bindgen CLI at the exact version `Cargo.lock` locks, then `wasm-opt` if installed. Output `index.html`, `engine.js`, `engine_bg.wasm`, `build.json`. Exit 0 is complete and under 8 MiB; exit 1 names the failed step.
3. `node tools/wasm-info.mjs dist/web/engine_bg.wasm` prints imports, tables and memories. Budgets (WEB11): `node tools/web-budget.mjs --dir dist/web` against `tools/web-budget.json`.
4. Look at it: `python -m http.server 8080 --bind 127.0.0.1 -d dist/web`, then `http://localhost:8080/`. WebGPU needs https or localhost; `.wasm` must be served as `application/wasm`.

## 5. Headless Chrome proof

- `node tools/web-browse.mjs --dir dist/web --chrome <chrome path> --out out/web [--require-adapter] [--only <n>] [--all]`. Loopback only. It tries software-adapter flag sets (`FLAG_SETS` in the tool) until one gives an adapter; it screenshots the canvas (not blank), checks frames keep coming, survives `device.destroy()`, then reruns with `navigator.gpu` removed (the page must show the loud message). Exit 0 proven, 1 a proof failed, 2 could not run, 3 no adapter with `--require-adapter`. Failed attempts go to `attempts.json`.
- Flag line that works on ubuntu-latest (Chrome 155, no GPU): `google-chrome --headless=new --no-sandbox --disable-dev-shm-usage --no-first-run --no-default-browser-check --hide-scrollbars --mute-audio --window-size=1280,720 --enable-unsafe-webgpu --enable-features=Vulkan --use-angle=swiftshader --use-webgpu-adapter=swiftshader`.
- `--use-angle=swiftshader` makes presentation work. `--use-vulkan=...` gives a blank canvas; `--enable-unsafe-webgpu` alone gives an adapter that keeps losing its device. A runner needs `sudo apt-get install -y libvulkan1 mesa-vulkan-drivers`.
- Input and audio (WEB7): `node tools/web-input-check.mjs --dir dist/web --chrome <chrome path>` sends real key and mouse events over CDP, replaying `tools/fixtures/input-tape.script`. No audio before a gesture; Escape is not one. With `--native-tape <file>` it fails unless page and PC give the same bytes.
- Device loss (QW8): `window.loseDevice()` forces a loss (the next receipt carries `losses` and `lastLoss`); `window.destroyDevice()` ends in `data-state=stopped`. Three losses in ten seconds end in the fatal page. Fallbacks (QT11): the fatal page carries `fallback=<no-webgpu|no-adapter|no-device|no-canvas|no-surface>`; the tier line carries `fallback=<limits|denylist|probe|none>` and `adapter=<text|none>`.

## 6. Workflows and the wasm-opt pin (public mirror `maximkone/engine2040-public`)

- `web-build.yml`: job `build` (web tool tests, clippy for wasm32, build, 8 MB and MIME check), then job `browser` (headless Chrome with `--require-adapter`, arena capture, budgets, input check, flag sets). Isolated run: `node tools/web-run.mjs` (skill `github-check`, section 5).
- `pages.yml`: the same build plus the mdBook handbook at `/docs/` and rustdoc at `/docs/api/`; `tools/book-links.mjs` fails on a broken internal link. It deploys on mirror `main` pushes only.
- `wasm-opt`: never apt's binaryen 108 (its output fails in every browser). The workflows pin release `version_133`. Rust 1.97 needs the `--enable-*` flags in `WASM_OPT_FLAGS`.

## 7. Pitfalls

- A browser has no OS random source. `uuid` needs `features = ["js"]` under a wasm32 target table (`crates/scene/Cargo.toml`); the same for any crate that reads OS randomness.
- A crate that type-checks for wasm can still trap at startup (`Instant::now`, `std::fs`). Trust the gate and a browser run, not the check.
- A wasm-bindgen CLI of another version than `Cargo.lock` gives a load error; `tools/web-build.mjs` prints the `cargo install wasm-bindgen-cli --version <v> --locked` fix.
- Reading Actions logs while a run is going gives partial text. Wait for the run to end.

## 8. The browser pane (real GPU, published page)

The Claude desktop app's browser pane has tools `mcp__Claude_Browser__*` (skill `anthropic-skills:built-in-browser`). It uses this PC's real GPU (2026-10-17: nvidia, ampere). The headless proof (section 5) uses a software adapter on a runner: the runner proves the build, the pane proves it on real hardware. The pane cannot open `file://` or a local server, so test the PUBLISHED page, not `dist/web`.

### Make sure the page is your build
1. Push the committed tree: `node tools/public-export.mjs`, then `node tools/public-export.mjs --commit --push` (skill `github-check`). Pages deploys only on a push to mirror `main`.
2. `gh run list -R maximkone/engine2040-public --workflow pages.yml -L 3`: the top row must be `completed success`, titled `snapshot <date> of <your sha7>`. `in_progress` means wait; `failure`: `gh run view <id> -R maximkone/engine2040-public --log-failed` once it ends.
3. `gh api repos/maximkone/engine2040-public/commits/main --jq '.sha[0:12]'` must equal `data-commit` on the page. If not, you see an old deploy: reload with `?r=2` or wait.

### Open and read
1. If the browser tools are deferred, load them in ONE ToolSearch call: query `mcp__Claude_Browser__`, `max_results: 64`.
2. `navigate` to `https://maximkone.github.io/engine2040-public/` (`/docs/` handbook, `/docs/api/` rustdoc; `?square` opens the plain WebGPU shell).
3. `get_page_text` (text, not a screenshot), then `javascript_tool`: `JSON.stringify({state:document.body.dataset.state,golden:document.body.dataset.golden,commit:document.body.dataset.commit,gen:document.body.dataset.generation,gpu:!!navigator.gpu})`.

### What the lines mean (`crates/web/index.html`, `crates/web/src/shell.rs`)

| line | good | bad |
|---|---|---|
| `frame N \| .. ms` | N grows between two reads | N stays 1: the pane is hidden (below) or frames stopped |
| `golden d57b5c8ae3096614 state ... OK <n> ms` | `data-golden` is `ok` (the other values are `mismatch` and `error`; `tools/web-browse.mjs` matches `GOLDEN_OK`) | `golden MISMATCH got ... expected ...`: skill `determinism-debug` |
| first JSON line | `"ok":true`, `backend` `BrowserWebGpu`, adapter vendor, `deviceGeneration` 1 | `"ok":false` with a `reason` (`no-webgpu`, `load-failed`); `data-state=fatal` |
| `arena ready in .. ms` | present | `arena: starting` forever: read the console |
| `adapter= .. validation_errors=0` | zero validation errors; a pass with 0.000 ms is not drawn yet | any validation error |
| last JSON line | `"frames"` rising; `frameErrors`, `validationErrors`, `deferredValidationErrors`, `uncapturedErrors` all 0; `draws` above 0 | a non-zero error count |
| `audio: state=suspended` | suspended until a key press | audio running before any gesture |

`data-state`: `starting`, `running`, `fatal`, `stopped`; `data-generation` rises by one after a lost device. Expected golden values are fixed; any other hash means the engine changed (never re-pin).

### When it looks stuck
- A hidden pane pauses animation frames: the HUD stays at `frame 1` though `document.visibilityState` says `visible` (seen 2026-10-17). `tabs_context` says whether the pane is hidden. Ask the person to bring it back (Ctrl+Shift+B in the Claude app), then read the text twice, a few seconds apart.
- `read_console_messages` with `onlyErrors: true` shows wasm panics and WebGPU errors.
- A screenshot is a second opinion only: a WebGPU canvas can capture black. The last JSON line is the proof.
- A site approval prompt: wait for the person. Do not work around it, and do not sign in anywhere.

### Report
Quote the URL, `data-commit` against the mirror head, the golden line, the first and last JSON line (frames, errors, draws), the adapter vendor, and what you could not see (hidden pane, no input test).
