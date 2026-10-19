# Web reference (browser build)

Detail for humans and AIs. Names, strings, numbers and commands were read from the code on 2026-10-17. A measured value carries its file. A plan value is marked as a target. Plan of record: `docs/plan/web-and-realism.md`. Board rows: `docs/SPRINT-22-web-realism.md` (WEB1-WEB14, RZ, VR, DOC). Probe: `docs/research/web-2026-10-17/07-wasm-probe.md`.

## 1. Status at a glance

| Piece | Code | State | Proof |
|---|---|---|---|
| Simulation on wasm32 (golden tape, corpus) | `engine-proof`, `engine-core` | Equal to native | probe sections 1 and 3; `golden-matrix.yml` wasm legs |
| Rapier ragdoll on wasm32 | `engine-physics --features rapier` | Equal digest; cost 1.4 to 1.85 times native | probe section 3 |
| Crate legs that type-check for wasm32 | `wasm-check.yml` (38 legs) | 34 of 37 in the probe; `engine-net --features quic`, `engine-app` and `engine-scene` did not (fixed by feature or by design) | probe section 2 |
| Browser shell: WebGPU device, moving square, JSON line, golden in page | `crates/web` | Built and checked headless (WEB4) | `web-build.yml`, `tools/web-browse.mjs` |
| Cross-origin isolation host (WEB14) | `crates/web/coi-serviceworker.js`, `index.html` | Worker stamps COOP `same-origin` and COEP `require-corp`; the page reloads once, prints `coi: isolated=<bool> reloads=<n>` and the ready line carries `crossOriginIsolated` | unit tests (`tools/web-build.test.mjs`); the live Pages proof of `crossOriginIsolated=true` is still open |
| Clock seam | `crates/clock` | Used by shell, assets, app and parts of render | `tools/wasm-gate.mjs` ratchet |
| Fetched-bytes asset reader | `crates/assets/src/asset_source/bundle.rs` | Library. The shell loads no content yet | unit tests |
| WebTransport server | `crates/net/src/webtransport*` | Library. No caller, no browser client | unit tests |
| Window renderer on wasm | `crates/render`, feature `window` | Type-checks. Blocking GPU calls remain | `wasm-gate` (render: 124 hits) |
| Input and audio on the web | none | Not started (WEB7) | |
| Pages URL | `.github/workflows/pages.yml` | Deploy job exists; Pages must be enabled on the mirror (owner action) | probe section 4 |

## 2. `crates/clock` (`engine-clock`)

One monotonic time source for shell and tool crates. No dependencies (`Cargo.toml`), so the headless graph stays free of web crates. A reading is diagnostic and never enters a simulation hash.

| Item | Signature | Behaviour |
|---|---|---|
| `now` | `pub fn now() -> Duration` | Time on the process-wide clock |
| `since` | `pub fn since(start: Duration) -> Duration` | `now() - start`, saturating: a future `start` gives zero |
| `install` | `pub fn install(source: fn() -> Duration) -> bool` | Sets the source once; `false` and no change when one is set |
| `Clock` | `const fn new()`, `install(&self, ..)`, `now(&self)` | Per-instance form used by the tests |

Without an installed source, native targets read `Instant` from a `OnceLock` origin. On `wasm32-unknown-unknown` the platform reading panics with `engine-clock: the browser has no std clock; the shell must call engine_clock::install before any read`. std has no clock there and `Instant::now()` / `SystemTime::now()` trap (proved: `RuntimeError: unreachable`, probe section 3). The shell installs `performance.now()` first thing in `run`.

Callers (grep of `crates/`): `app/src/app.rs`, `assets/src/archive.rs`, `assets/src/bundle.rs`, `render/src/{ibl,sky}.rs`, `render/src/present/gpu_present/{live,submit,sun_shadow}.rs`, `web/src/shell.rs`.

Tests: `crates/clock/src/lib.rs` (5 unit tests) and `crates/clock/tests/it/install.rs` (the installed source drives `now` and `since`; one process). Command: `node tools/gh-check.mjs cargo test -p engine-clock --lib` and `--test it`.

## 3. `crates/web` (`engine-web`, the browser shell)

Crate types `cdylib` (wasm-bindgen input) and `rlib` (native build and pure-module tests). The browser glue is `#[cfg(target_arch = "wasm32")] mod shell`. Dependencies: `wgpu 30.0.1` (`std`, `wgsl`, `webgpu`), `engine-proof`, `engine-clock`; on wasm32: `wasm-bindgen 0.2`, `wasm-bindgen-futures 0.4`, `js-sys 0.3`, `web-sys 0.3`. Release profile `[profile.web]` (root `Cargo.toml`): inherits release, `opt-level = "z"`, `lto = true`, `codegen-units = 1`, `panic = "abort"`, overflow checks kept.

| File | Role |
|---|---|
| `src/lib.rs` | Module list; `COMMIT` (from `ENGINE_WEB_COMMIT`, else `unknown`); `SHADER_WGSL` |
| `src/shell.rs` | wasm32 only: `run`, `lose_device`, frame loop, recovery, golden call |
| `src/anim.rs` | Pure: `clear_colour(t_ms)`, `Square::at(t_ms, aspect)`, `to_bytes() -> [u8; 32]`, `FrameStats`, `backing_size`. Constants `HUE_PERIOD_S = 12.0`, `ORBIT_PERIOD_S = 5.0`, `HALF_HEIGHT = 0.12` |
| `src/golden.rs` | Pure: the golden check and its line format (3.4) |
| `src/limits.rs` | Pure: `required_limits()` = `Limits::defaults()`; `wanted_features` = adapter AND `TIMESTAMP_QUERY` |
| `src/recovery.rs` | Pure: device-loss budget (3.3) |
| `src/report.rs` | Pure: hand-written JSON, `Ready::line()`, `Failure`, `failed_line` |
| `src/shader.wgsl` | `vs`, `fs`; 32-byte uniform `square` (test checks field order) |
| `index.html` | Canvas, HUD, golden box, report box, fatal box; `{{COMMIT}}` replaced at build |
| `coi-serviceworker.js` | WEB14: stamps `Cross-Origin-Opener-Policy: same-origin` and `Cross-Origin-Embedder-Policy: require-corp` on every response it controls; `install` calls `skipWaiting`, `activate` calls `clients.claim` |

Page contract. `index.html` imports `engine.js`, calls `init()` then `run("canvas")`, sets `window.loseDevice = lose_device`. Any throw there sets `data-state="fatal"` and a line with `reason: "load-failed"`. Attributes the headless check reads: `<body data-state>` (`starting`, `running`, `recovering`, `fatal`), `data-generation`, `data-golden` (`ok`, `mismatch`, `error`). Elements: `#report` (JSON line), `#hud` (`frame N | X ms | Y fps`, or `frame N | -- ms`), `#golden`, `#fatal` and `#fatal-reason`. The loud box lists Chrome 113, Edge 113, Safari 26 and Firefox 141 (Windows).

WEB14 isolation. When `window.crossOriginIsolated` is false the module first registers `./coi-serviceworker.js` (relative to the page, so the worker's scope is the site root), waits for `navigator.serviceWorker.ready`, then reloads once. It reloads only when the page is a secure context, the worker is not already controlling, and the `sessionStorage` key `e2040-coi-reloaded` is absent, so a worker that never delivers isolation cannot loop the page. Any failure keeps the page single-threaded. The line `#coi` always exists in the HTML and always ends as `coi: isolated=<true|false> reloads=<n>`, with a short note in parentheses when isolation did not happen; `<body data-coi>` is `isolated` or `single`, and `window.coi` holds `{ isolated, reloads, note }`.

### 3.1 Ready line (`"ok":true`)

Printed to `#report`, the console, and returned from `run`. Keys in order: `ok`, `commit` (12 hex from `--commit`), `backend` (for example `BrowserWebGpu`), `adapter` (`name`, `deviceType`, `vendor`, `architecture`, `device`, `description`; the last four are raw `GPUAdapterInfo` strings read again through `requestAdapter`), `features` (WebGPU names), `limits` (every WebGPU limit by WebGPU name, for example `maxStorageBuffersPerShaderStage`), `wasmMemoryPages`, `wasmMemoryBytes`, `deviceGeneration` (1, then +1 per recovered loss), `canvas` (`width`, `height`), `surfaceFormat` (for example `bgra8unorm`), `userAgent`, `crossOriginIsolated` (WEB14: `window.crossOriginIsolated` when the line was printed; the page's `#coi` line adds the reload count).

### 3.2 Failed line and reasons

Keys: `ok` (false), `reason`, `message`, `detail`, `commit`, `wasmMemoryPages`, `userAgent`.

| `reason` | When | `detail` |
|---|---|---|
| `no-webgpu` | `navigator.gpu` undefined | `navigator.gpu is undefined: an older browser, or a page that is neither https nor localhost.` |
| `no-canvas` | no element with the id | ``expected <canvas id="...">`` |
| `no-adapter` | `request_adapter` fails | wgpu error text |
| `no-device` | `request_device` fails, or a loss storm | wgpu text, or `the device was lost again and again; the browser cannot keep one.` |
| `no-surface` | `create_surface` fails, or the adapter cannot present | wgpu text, or `the adapter cannot present to this canvas` |
| `load-failed` | `index.html` only | `String(e)` |

Each `reason` has a fixed page sentence (`Failure::headline`), for example `This browser has no WebGPU (navigator.gpu is missing).`

### 3.3 Device loss

The device's lost callback sets a per-device flag; the loop then calls `recover`. Constants (`recovery.rs`): `ATTEMPTS = 4`; pause before attempt k is `PAUSE_MS * k` with `PAUSE_MS = 500`; `MAX_LOSSES = 3` inside `WINDOW_MS = 10_000.0`. The third loss in the window ends the loop with `no-device`. Log lines: `device lost: starting device generation N, attempt K`, `wgpu device lost (<reason>): <message>`, `wgpu: <error>`, `panic: <info>`.

### 3.4 Golden check (in the page)

After the first line, `run` calls `golden::check`: records the 600-tick seed-2040 tape (`engine_proof::record(Config::default(), &[])`) and runs `engine_proof::matrix::collect_local_entry()` (20 corpus blocks). The three numbers must equal the constants in `golden.rs`:

| Number | Expected |
|---|---|
| golden (last legacy checkpoint) | `d57b5c8ae3096614` |
| state (last tick) | `2aca9338670aa0cb` |
| corpus digest (20 blocks) | `b37fc6d66df2d417` |

| Case | Line (first line of `#golden`) |
|---|---|
| OK | `golden d57b5c8ae3096614 state 2aca9338670aa0cb corpus b37fc6d66df2d417 OK <ms> ms` |
| mismatch | `golden MISMATCH got <g> state <s> corpus <c>, expected d57b5c8ae3096614 state 2aca9338670aa0cb corpus b37fc6d66df2d417 (<ms> ms)` |
| error | `golden ERROR tape: <error>` or `golden ERROR matrix row: <error>` |
| tick line (second line) | `tape 600 ticks in <ms .1> ms, <per tick .4> ms per tick` |

`data-golden` gets `ok`, `mismatch` or `error`. The check runs even when there is no WebGPU. Tests: `this_machine_computes_the_pinned_values` and five more in `golden.rs`.

### 3.5 Frame loop, tests

Each frame: stats tick, HUD, canvas fit (`backing_size`: CSS size times devicePixelRatio, clamped to the device max texture size, never zero), acquire, one clear pass with `clear_colour(now)`, `draw(0..6)`, submit, present. `Outdated`, `Lost` and timeouts skip the frame. Smoothed frame time: `0.9 old + 0.1 new`.

Tests: `crates/web/src/{lib,anim,golden,limits,recovery,report}.rs` (1, 11, 6, 5, 4, 6 tests). Command: `node tools/gh-check.mjs cargo test -p engine-web --lib`. Glue: `cargo clippy --locked --target wasm32-unknown-unknown -p engine-web -- -D warnings` (CI).

## 4. `tools/web-build.mjs`

Usage: `node tools/web-build.mjs [--out dist/web] [--commit <hash>] [--no-opt]`. Default commit: `ENGINE_WEB_COMMIT`, else `git rev-parse --short=12 HEAD`, else `unknown`. Exports for the test: `TARGET = wasm32-unknown-unknown`, `PROFILE = web`, `PACKAGE = engine-web`, `OUT_NAME = engine`, `COI_WORKER = coi-serviceworker.js` (WEB14), `SIZE_LIMIT_BYTES = 8388608`, `WASM_OPT_FLAGS`, `DIST_FILES`, `lockedVersion`, `cargoArgs`, `bindgenArgs`, `renderHtml`, `readCoiWorker`, `build`, `parseArgs`.

| Step | Command | Check |
|---|---|---|
| 1 | read `Cargo.lock`; `wasm-bindgen` must have exactly one version | |
| 2 | `wasm-bindgen --version` equals that version | |
| 3 | `cargo build --locked --profile web --target wasm32-unknown-unknown -p engine-web` with `ENGINE_WEB_COMMIT` | `target/wasm32-unknown-unknown/web/engine_web.wasm` exists |
| 4 | `wasm-bindgen --target web --no-typescript --out-dir <tmp> --out-name engine <wasm>` | `engine.js`, `engine_bg.wasm` exist |
| 5 | if `wasm-opt --version` works: `wasm-opt <wasm> -o <wasm> -Oz --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext --enable-multivalue --enable-reference-types --enable-mutable-globals` | absent: a message, not an error |
| 6 | `twiggy top -n 8 <wasm>` if installed | first 12 lines go to `build.json` |
| 7 | copy files (including `coi-serviceworker.js`), write `index.html` and `build.json`; total must be under the limit | else the output is removed |

Success transcript: `[web-build] cargo build --profile web --target wasm32-unknown-unknown (commit <hash>)`, `[web-build] wasm-bindgen --target web`, `[web-build] wasm-opt -Oz ...` (or `not installed`), `[web-build] dist/web <N> bytes (limit 8388608); wasm <N> bytes, <N> gzip`. Exit 0.

`build.json`: `schema` 1, `commit`, `target`, `profile`, `files` (`bytes`, `gzipBytes` at level 9, `sha256` per file), `totalBytes`, `limitBytes`, `underLimit`, `rawWasmBytes` (before wasm-opt), `tools` (`rustc`, `wasmBindgen`, `wasmOpt`), `wasmOptFlags`, `twiggy`.

| Failure message (after `web-build: `) | Cause |
|---|---|
| `Cargo.lock has N versions of <name> ...; the build needs exactly one` | lockfile |
| `the wasm-bindgen CLI is not installed; Cargo.lock locks <v>: cargo install wasm-bindgen-cli --version <v> --locked` | CLI missing |
| `wasm-bindgen CLI is <a> but Cargo.lock locks <b>; they must match exactly: <fix>` | CLI version |
| `<label> failed (exit N): <cmd> <args>` plus the last 25 lines | cargo, wasm-bindgen or wasm-opt failed |
| `cargo reported success but <path> does not exist` / `wasm-bindgen wrote <files>; expected engine.js and engine_bg.wasm` | unexpected output |
| `crates/web/index.html lost its {{COMMIT}} placeholder` | template |
| `crates/web/coi-serviceworker.js is missing (WEB14): the page would never be cross-origin isolated` | WEB14 worker file |
| `dist/web is <N> bytes, over the 8388608 byte limit` | size |

Exit 1 for these; exit 2 for usage (`web-build: unknown argument <x>`). Tests: `node --test tools/web-build.test.mjs` (26 tests).

## 5. `tools/web-browse.mjs`

Headless Chrome check of `dist/web`. It serves the directory on `127.0.0.1` (`.wasm` as `application/wasm`, `no-store`), starts Chrome with `--remote-debugging-port=0` and a temp profile, reads the page over CDP, takes a screenshot, closes Chrome.

Usage: `node tools/web-browse.mjs --dir dist/web --chrome <path> --out out/web [--require-adapter] [--all] [--only <n>]`. `--chrome` is required. `WEB_CHROME_EXTRA` adds a flag set named `extra`.

Base flags: `--headless=new --no-sandbox --disable-dev-shm-usage --no-first-run --no-default-browser-check --hide-scrollbars --mute-audio --window-size=1280,720`. The launcher adds `--remote-debugging-port=0 --user-data-dir=<tmp> --enable-logging=stderr about:blank`.

| n | Name | Extra flags (after the base) |
|---|---|---|
| 0 | `swiftshader-angle` | `--enable-unsafe-webgpu --enable-features=Vulkan --use-angle=swiftshader --use-webgpu-adapter=swiftshader` |
| 1 | `swiftshader-vulkan` | `--enable-unsafe-webgpu --enable-features=Vulkan --use-vulkan=swiftshader --use-angle=vulkan --disable-vulkan-surface --use-webgpu-adapter=swiftshader` |
| 2 | `adapter-only` | `--enable-unsafe-webgpu --use-webgpu-adapter=swiftshader` |
| 3 | `swiftshader-unsafe` | `--enable-unsafe-webgpu --enable-unsafe-swiftshader --enable-features=Vulkan --use-angle=swiftshader --use-webgpu-adapter=swiftshader --ignore-gpu-blocklist --disable-gpu-sandbox` |
| 4 | `lavapipe` | `--enable-unsafe-webgpu --enable-features=Vulkan --use-vulkan=native --use-angle=vulkan --disable-vulkan-surface --ignore-gpu-blocklist --disable-gpu-sandbox` |
| 5 | `vulkan-feature-only` | `--enable-unsafe-webgpu --enable-features=Vulkan --ignore-gpu-blocklist` |
| 6 | `bare` | `--enable-unsafe-webgpu` |

The proven line (set 0, printed as `FLAG LINE`): `google-chrome --headless=new --no-sandbox --disable-dev-shm-usage --no-first-run --no-default-browser-check --hide-scrollbars --mute-audio --window-size=1280,720 --enable-unsafe-webgpu --enable-features=Vulkan --use-angle=swiftshader --use-webgpu-adapter=swiftshader`.

Measured (`web-and-realism.md` section 2; run 39040803910, Chrome 155.0.8059.39, GitHub ubuntu-latest, no GPU): sets 0 and 3 give the SwiftShader adapter and the canvas presents. Without `libvulkan1 mesa-vulkan-drivers` every set gets an adapter but the swap-chain shared image fails (`Could not find SharedImageBackingFactory ... WebgpuSwapChainTexture`) and the device is lost repeatedly. `--use-vulkan=*` gives a blank canvas; `--use-webgpu-adapter` alone gives a lost device.

Verdict codes (`verdict`): `adapter-ok`; `timeout` (`the page never left data-state=starting within <n> ms`); `no-report`; `bad-line`; `blank-canvas` (`isDrawn`: mean luma at least 20, at least 4 colours, largest colour under 98 percent); `frozen` (frame counter static for 1 s); `no-recovery` (no `data-generation="2"` running within 30 s after `window.loseDevice()`); `run-error` (`Chrome did not open a debugging port ...; see <log>`); or a page reason (`no-webgpu`, `no-adapter`, `no-device`, `no-canvas`, `no-surface`).

Stdout: `[<i>] <set>: ADAPTER OK - <vendor> <architecture> <description>` or `[<i>] <set>: <code> - <why>`; on failure also `navigator.gpu probe: {...}`, console, exception and Chrome-log lines. On success: `FLAG LINE`, `READY LINE <json>`, `CANVAS {...}`, `GOLDEN <line>` (or `GOLDEN FAILED`), `WASM TICK <line>`. Always: `NO-WEBGPU GOLDEN ...` and `no-WebGPU page: OK (loud message shown, reason no-webgpu, nothing thrown)` or `no-WebGPU page: FAILED - <problems>`. No set: `NO ADAPTER: every flag set was tried; see attempts.json for each reason.` Then `GOLDEN FAILED on the adapter page: <why>` when the golden line is wrong.

No-WebGPU run: `Navigator.prototype.gpu` is redefined to return `undefined` before the page loads. Problems named: wrong `data-state`, message not shown, a missing browser version, a line that is not `ok:false` with `reason no-webgpu`, uncaught exceptions, a golden problem.

Files in `--out`: `ready.png`, `report.json`, `flag-line.txt` (winning set); `receipt.json` (winning set: `schema` 1, `set`, `wasmMemoryBytes`, `ttffMs` (ms from navigation to the first frame), `commit`); `attempt-<set>.png`; `chrome-<set>.log`; `attempts.json` (verdicts, flags, console, served requests); `no-webgpu.png`, `chrome-no-webgpu.log`. Exit: 0 proven (or no adapter without `--require-adapter`); 1 a proof failed; 2 could not run (bad arguments, `web-browse: <dir> has no index.html (run node tools/web-build.mjs)`); 3 no adapter with `--require-adapter`. Tests: `node --test tools/web-browse.test.mjs` (14; one reads a real Chrome and is skipped without one).

### 5.1 Budgets (`tools/web-budget.mjs`, WEB11)

Usage: `node tools/web-budget.mjs [--dir dist/web] [--receipt out/web/receipt.json] [--budget tools/web-budget.json] [--require-receipt]`.

| Number | Meaning |
|---|---|
| `wasm` | `engine_bg.wasm` as shipped (bytes) |
| `gzip` / `brotli` | that file, gzip level 9 / brotli quality 11 |
| `first_bundle` | brotli size of every file in `dist/web` except `build.json` (the wasm counted as brotli) |
| `mem` | `wasmMemoryBytes` from the receipt; `na` without one |
| `ttff_ms` | `ttffMs` from the receipt; `na` without one |

Output: `web-budget: wasm=<b> gzip=<b> brotli=<b> first_bundle=<b> mem=<b|na> ttff=<ms|na>`, then `web-budget: twiggy <line>` for each twiggy line in `build.json`. Over budget: `web-budget: FAIL <key> <v> is over its budget <b> by <d> (<p>%)`. With `--require-receipt` a missing number fails: `<key> is missing from the receipt (the browse step did not record it)`. Exit 0 inside every budget; 1 over budget or a required number missing; 2 cannot run (for example `engine_bg.wasm does not exist (run node tools/web-build.mjs first)`). With `GITHUB_STEP_SUMMARY` set, a table of measured and budget values is appended. `web-build.yml` runs it after the browser step with `--require-receipt`.

Budgets in `tools/web-budget.json` (schema 1): `wasm` 269800, `gzip` 119600, `brotli` 99500, `first_bundle` 114400, `mem` 1474560, `ttff_ms` 8000. The file says they were set from web-build run 39042934511 (measured: wasm 215789, gzip 95654, brotli 79566, first_bundle 91478, mem 1179648 = 18 pages) plus about 25 percent headroom; `ttff_ms` is the board bar because no run had recorded it. Tests: `node --test tools/web-budget.test.mjs` (15).

## 6. Workflows (`.github/workflows/`)

Every job is guarded by `github.repository == 'maximkone/engine2040-public'`: the public mirror only.

| File | Trigger | Jobs | Proves |
|---|---|---|---|
| `web-build.yml` | dispatch; push `web-build-*` | `build`: node tests (`web-build`, `web-browse`, `wasm-info`, `web-budget`), `cargo test -p engine-web --lib`, clippy for wasm32, `web-build.mjs --commit`, `wasm-info`, size under 8 MiB, MIME check; upload `web-dist`. `browser`: Chrome, `--only 0` (never fails), Vulkan and Mesa packages, `--require-adapter` run, `web-budget.mjs --require-receipt`, `--all` run; upload `web-browse` | Build, headless proof, budgets |
| `pages.yml` | dispatch; push `main` | `build` (wasm-opt pinned to binaryen `version_133`, handbook, rustdoc), `deploy` (`actions/deploy-pages@v4`, then the published MIME is checked 6 times at 10 s). The game is at the site root, the handbook at `/docs/`, rustdoc at `/docs/api/` (DOC1) | The URL the work PC opens |
| `wasm-check.yml` | dispatch; PR; push `main`, `wasm-check-*` | `check`: 38 legs, `cargo check --locked --keep-going --target wasm32-unknown-unknown <args>`; `gate`: `node --test tools/wasm-gate.test.mjs`, then `wasm-gate.mjs --list` | Type-check and hazard ratchet |
| `wasm-probe.yml` | push `wasm-probe-*` only | `check`, `wasi-golden`, `wasi-tests`, `unknown-run`, `wasm-size`, `webgpu-adapter`, `table` | The probe; nothing ships |
| `golden-matrix.yml` | dispatch; push `golden-matrix-*` | `record` (linux-x86_64, windows-x86_64, linux-aarch64), `record-wasm` (`wasm32-wasip1` and `-simd128` under wasmtime v49.0.2), `physics-tests`, `compare` (`tools/golden-matrix.mjs`; a missing file fails) | Same digests on five machines |

The probe ran the wasm legs under Node's WASI; the matrix runs them under wasmtime. Native rows of run 39032745322 are cited in `web-and-realism.md`; wasm rows are in probe run 39036560799 (`07-wasm-probe.md` section 9).

## 7. Hazard ratchet: `tools/wasm-gate.mjs`

Counts calls that type-check for wasm32 but fail or trap in a browser, in `WEB_CRATES`, and fails when a file rises above `tools/wasm-gate.baseline.json`. Usage: `node tools/wasm-gate.mjs [--list] [--tighten] [--init] [--help]`. Exit 0 ok, 1 regression, 2 usage.

| Id | Pattern | Why |
|---|---|---|
| `fs` | `std::fs` | every call returns `Err` (proved) |
| `clock` | `Instant::now`, `SystemTime::now` | traps (proved) |
| `thread` | `thread::spawn`, `thread::scope`, `Builder::new().spawn` | `spawn` returns `Err` (proved) |
| `socket` | `UdpSocket`, `TcpStream`, `TcpListener` | no raw sockets in a tab |
| `exe` | `current_exe`, `current_dir` | no executable path or working directory |
| `block` | `pollster::block_on`, `.poll(...wait...)` | the main thread cannot block on the GPU |

Scope: `crates/<c>/src/**/*.rs` for `core ecs geo spatial rules input combat anim ai nn physics destruction nav arena scene material world save observe ui audio assets net game render proof app clock`. Skipped: `tests/`, `benches/`, `examples/`, `tests.rs`, `*_tests.rs`, `src/bin` and `main.rs` (unless in `WEB_BINS`, empty), and items whose `#[cfg]` is false for wasm32 (features count as unknown and do not skip). Not in the list: `web` and `physfx` (section 12).

| crate | fs | clock | thread | socket | exe | block | total |
|---|---|---|---|---|---|---|---|
| arena | 0 | 0 | 3 | 0 | 0 | 0 | 3 |
| assets | 14 | 0 | 0 | 0 | 0 | 0 | 14 |
| audio | 1 | 0 | 0 | 0 | 0 | 0 | 1 |
| ecs | 0 | 0 | 1 | 0 | 0 | 0 | 1 |
| game | 21 | 8 | 0 | 4 | 2 | 0 | 35 |
| net | 2 | 12 | 5 | 25 | 0 | 0 | 44 |
| proof | 15 | 0 | 0 | 0 | 0 | 0 | 15 |
| render | 7 | 65 | 1 | 0 | 0 | 51 | 124 |
| save | 6 | 0 | 0 | 0 | 0 | 0 | 6 |
| **all** | **66** | **85** | **10** | **29** | **2** | **51** | **243** |

Measured 2026-10-17 (`node tools/wasm-gate.mjs`): `RESULT PASS: 0 ratchet regressions over 65 file(s) with hazards`.

| Output | Meaning |
|---|---|
| `[FAIL] ratchet count rose <file>: <id> <was> -> <now> (<what>)` plus `<file>:<line>: <snippet>` | count above baseline (`new file with hazards` for an unlisted file) |
| `[IMPROVED] <file>: <id> <was> -> <now>` and `[INFO] ... run --tighten` | count fell |
| `RESULT FAIL: n ratchet regression(s)` / `RESULT PASS: ...` | verdict |

The baseline only goes down: `--tighten` lowers counts and never raises one; `--init` writes it once. Tests: `node --test tools/wasm-gate.test.mjs` (20), including `the committed baseline holds for this tree`.

## 8. Assets and net on the browser path

Asset source (`crates/assets/src/asset_source.rs`). Callers name content by logical path (`/`-separated, relative). `validate_path` refuses empty names, names over `MAX_SOURCE_PATH_BYTES = 512`, empty parts, `.` or `..`, and backslash, colon or control characters. `trait AssetSource`: `read`, `exists`, `list` (sorted bytewise). `content_digest(source, dir)` is FNV-1a over path, length and byte hash of each file, in path order, plus the count. `FsSource` on wasm32 refuses every read with `asset "<p>" io failure (Unsupported)` (commit `86e97544`). `SourceError` texts: `asset "<p>" not found`, `asset path "<p>" refused: <reason>`, `asset "<p>" hash mismatch: bundle has <hex>, bytes hash to <hex>`, `bad bundle: <detail>`, `bad pack request: <detail>`.

`.e2bundle` version 1 (`asset_source/bundle.rs`, little-endian):

| Part | Bytes | Content |
|---|---|---|
| header | 32 | magic `E2BUNDLE`, `u32` version 1, `u32` count (max 65536), `u64` index length, `u64` FNV-1a over version, count, index length and index |
| index | per entry | `u16` path length, path, `u8` priority, `u64` offset, `u64` length, `u64` FNV-1a of the blob; sorted by path |
| blobs | back to back | in (priority, path) order; tiers `0..=p` are a file prefix |

`BundleSource::open(bytes)` refuses with: `magic is not E2BUNDLE`, `version <v> unreadable (this reader reads 1)`, `<n> entries is over the 65536 cap`, `truncated: need <a> bytes, got <b>`, `index length is out of range`, `blob "<p>" left the bundle`, `blob lengths overflow`. Each blob is checked against its hash on read; one bad blob fails by name.

Packing (native only): `bundle_pack(root, out, &PackRules)`; `pack_to(source, rules, writer)` for any source. `PackRules::new().include(prefix).priority(prefix, level)`; `priority_spec("<prefix>=<0-255>")` refuses with `priority "<spec>" is not <prefix>=<0-255>`. Errors: `no file selected to pack`, `output lies inside the content root`. A failed pack deletes its output. `PackReport { entries, bytes, digest }`.

`e2bundle` (`crates/assets/src/bin/e2bundle.rs`, native only): `e2bundle pack <root> <out> [--include <prefix>]... [--priority <prefix>=<tier>]...` prints `e2bundle pack OK entries=<n> bytes=<n> digest=<hex16>`. `e2bundle verify <bundle> [<root>]` prints `e2bundle verify OK entries=<n> bytes=<n> digest=<hex16>`, one `  tier <t>: <n> files, ends at byte <e>` line per tier, and `  every file matches <root>` when given. Failures print `e2bundle FAIL: <message>` and exit 1 (for example `<path> differs between the bundle and <root>`). Tests: 12 in `asset_source/bundle.rs` and `pack_then_verify_round_trips_and_names_bad_input` in `e2bundle.rs`. Command: `node tools/gh-check.mjs cargo test -p engine-assets --lib asset_source`.

`crates/assets/src/bundle.rs` is a different format: the content bundle v1 for the game and editor (`BundleLoader::load(&[u8])`). The browser shell loads no content, so neither format is on the browser path yet. The game still reads about 8 content files with `std::fs::read` (`arena_kit.rs`, `fighter_bind.rs`, `fighter_look.rs`, `audio/decode.rs`; probe section 4). WEB6 owns the seam.

## 9. WebTransport (`crates/net/src/webtransport*`)

Feature `webtransport` (= `quic`, `dep:wtransport`, `dep:tokio`); the module compiles only with it (`crates/net/src/lib.rs`). Default `engine-net` has no quinn and compiles for wasm32. `--features quic` does not (mio has no wasm backend), so browsers use WebTransport.

Status: `WtServer` has no production caller (only its module and `webtransport/tests.rs`). The shell has no network code.

| Item | Behaviour |
|---|---|
| `WtServer::bind(addr, &PinnedCert) -> io::Result<Self>` | port 0 picks a free port; one thread `wt-server` runs a single-thread tokio runtime |
| `local_addr`, `peers`, `take_departures`, `dropped_datagrams` | address; live sessions; `Departure { peer, reason }` oldest first; drops |
| `impl Transport` | `send_unreliable` checks the 1200-byte budget and sends a datagram; `send_reliable` frames then `try_send`s, so a full queue returns `BadInput` |

Constants: `SESSION_PATH = "/e2040"` (other paths 404); `MAX_SESSIONS = 16` (next request 429); `INBOX_FRAMES = 1024`; `RELIABLE_FRAMES = 65536 / 1200 = 54` per session; `MAX_DEPARTURES = 64`; `DRAIN_WAIT = 500 ms`; `QUIC_KEEPALIVE = 5 s`; `MAX_PACKET_BYTES = 1200`. Close codes: `0` shutdown, `1` protocol, `2` replaced (newer session, same address), `3` full. One session per remote address.

Frame (`frame.rs`): `kind u8` (`0` snapshot, `1` lobby), `length u16` little-endian, payload (the exact codec bytes). Unknown kind: `NetError::BadKind(<byte>)`. Length over 1200: `NetError::TooLarge`, also on encode.

Pinned certificate (`cert.rs`): browsers accept a self-signed cert through `serverCertificateHashes` when it is X.509 v3, ECDSA P-256, valid at most 14 days, and current. The pin is SHA-256 of the leaf DER. `MAX_VALIDITY_DAYS = 14`; `notBefore` is backdated 1 hour. `PinnedCert::mint(sans, days)` refuses 0 or over 14 before making a key; `from_identity`, `check_pinnable(der, now)`, `leaf_hash(der)`, `hash_hex()`. `CertError` texts: `validity of <n> days is outside 1..=14`, `invalid subject alternative name`, `public key is not ECDSA P-256`, `validity of <s> s exceeds the 14-day browser limit`, `certificate is not valid yet`, `certificate has expired`.

Tests: 11 in `cert.rs` (DER fixtures under `crates/net/testdata/webtransport/`), 3 in `frame.rs`, 6 in `tests.rs` (a 600-tick duel with a pinned client, wrong-hash refusal, wrong path, broken framing, no stream, oversize and unknown peers). Command: `node tools/gh-check.mjs cargo test -p engine-net --features webtransport --lib webtransport`.

## 10. Measured numbers and targets

| Quantity | Value | Source |
|---|---|---|
| Golden tape on wasm32-wasip1 | `final_hash=d57b5c8ae3096614 state_hash=2aca9338670aa0cb verified=true`; byte-identical to native Linux | `07-wasm-probe.md` section 3 |
| Corpus row on wasm | `golden=d57b5c8ae3096614 corpus=b37fc6d66df2d417 blocks=20`; 20 of 20 block digests equal | same |
| Lib tests, 15 crates, wasip1 | 2269 passed, 0 failed, 35 killed (threads, `current_dir`, deliberate panics, OS label) | same |
| Rapier ragdoll digest | `5d7aedd4c6095b7c` on native and wasm (50 falls, 19,320 ticks, three runs) | same |
| Rapier cost, wasm vs native | scalar 1.82 to 1.84 times; SIMD128 1.45 to 1.54 times; about 22 microseconds per 4-body step | same |
| Arena game linked for wasm32 (no wasm-opt) | 2,552,864 bytes; 638,281 gzip; 433,950 brotli; 333 imports | same |
| Headless Chrome 155, set 0 | adapter `google` / `swiftshader`; 21 features; `maxStorageBuffersPerShaderStage` 10; `maxBufferSize` 1 GiB | same |
| `dist/web` (run 39040803910, before the golden check) | 237,384 bytes (limit 8 MiB); `engine_bg.wasm` 166,387 (72,223 gzip); `engine.js` 68,283 | `web-and-realism.md` section 2 |
| `dist/web` (run 39042934511, with the golden check) | `engine_bg.wasm` 215,789; gzip 95,654; brotli 79,566; first bundle 91,478; wasm memory 1,179,648 bytes (18 pages) | `tools/web-budget.json` note |
| Target: first bundle | under 60 MB compressed; wasm under 25 MB; peak memory under 1.5 GB; first frame under 8 s (targets) | `03-browser-physics.md` line 42 |

## 11. Golden matrix tool (`tools/golden-matrix.mjs`)

Usage: `node tools/golden-matrix.mjs <a.e2040> <b.e2040> [...] [--rows <a.txt> <b.txt> ...] [--ragdoll <a.txt> <b.txt> ...]`. Each group needs two or more files. Exit 0 all agree, 1 divergence, 2 usage or unreadable file.

| Output | Meaning |
|---|---|
| `MATCH <name> vs <first>` | equal |
| `DIVERGE <name> vs <first>: first bad tick <t> (<lane> lane) cause=<c> - <note>` | tape |
| `DIVERGE <name> vs <first> (<host> vs <host>): <note>` | corpus or ragdoll |
| `RESULT PASS|FAIL`, `CORPUS RESULT PASS|FAIL`, `RAGDOLL RESULT PASS|FAIL` | group verdicts |

Row formats: corpus `<host> golden=<16 hex> corpus=<16 hex> blocks=<n>` then `<host> every=<n> cut_checkpoints=<hex,...>`; ragdoll `<host> ragdoll=<16 hex> ticks=<n> bodies=<n> contacts=<n> final_state=<16 hex>` then `<host> tick=<t> bodies=<hex,...>` per tick. Tape: magic `E2040RP\0`, `u16` format 1, `u16` fixture 1, `u64` seed, `u32` ticks, `u32` input count, 16 bytes per input, 16 per checkpoint (`u64` legacy, `u64` state), header 28 bytes. Tests: `tools/golden-matrix.test.mjs` (24).

## 12. Not done, and suspicious

1. `physfx` is not in `WEB_CRATES` (`tools/wasm-gate.mjs`), though the plan puts it on the web path. Its hazards are not counted. `crates/web` is not listed either (wasm32 by design).
2. `wasm-check.yml` type-checks `engine-physfx-rapier`, but no hazard scan covers it (item 1).
3. The WebTransport server has no caller and there is no browser client (section 9).
4. Render has 124 hazard hits (section 7), including `pollster::block_on` and `device.poll(wait)` on the main thread (probe section 5: about 76 sites).
5. Input (WEB7, gamepad via gilrs-core) and audio (cpal `webaudio`, started on the first click) are not wired in the shell.
6. The real renderer's browser frame cost is not measured. The probe timed only CPU sim and physics (probe section 8).
7. `pages.yml` runs on push to `main` of the public mirror only. The private repo's branch is `master` (CLAUDE.md), and its repository guard excludes it.
8. The `mem` budget is 1474560 bytes (about 1.4 MiB) in `tools/web-budget.json`, but the WEB11 bar in `03-browser-physics.md` line 42 reads "peak memory under 1.5 GB". The unit or the value is wrong; decide which.
