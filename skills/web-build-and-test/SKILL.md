---
name: web-build-and-test
description: "Use when you build, run, prove or publish the browser (wasm) target of engine2040: dist/web, headless Chrome, the page's golden line, the web-build and pages workflows, or the public mirror export."
---

# Web build and test

The browser shell is `crates/web` (package `engine-web`). It runs the golden tape in the page and draws with WebGPU. Spec: `docs/plan/web-and-realism.md`; rows: `docs/SPRINT-22-web-realism.md`.

## Check the PC first
- `node tools/web-doctor.mjs`: one line per piece (`OK` or `MISSING` plus `fix:`), last line `web target: READY` or `NOT READY n missing`. It installs nothing.

## Build dist/web
- `node tools/web-build.mjs [--out dist/web] [--commit <hash>] [--no-opt]`.
- Steps: `cargo build --locked --profile web --target wasm32-unknown-unknown -p engine-web`, then the wasm-bindgen CLI at the exact version `Cargo.lock` locks (fix line: `cargo install wasm-bindgen-cli --version <v> --locked`), then `wasm-opt -Oz` when installed.
- Output: `index.html`, `engine.js`, `engine_bg.wasm`, `build.json` (sizes, gzip, SHA-256, commit). Exit 0 = complete and under 8 MiB, 1 = a step failed and says which.
- `node tools/wasm-info.mjs dist/web/engine_bg.wasm` prints imports, tables and memories as JSON.
- Budgets (WEB11): `node tools/web-budget.mjs --dir dist/web [--receipt out/web/receipt.json --require-receipt]` against `tools/web-budget.json`.

## Look at it yourself
- `python -m http.server 8080 --bind 127.0.0.1 -d dist/web`, then open `http://localhost:8080/`. WebGPU needs https or localhost, and `.wasm` must be served as `application/wasm`.

## Headless Chrome proof
- `node tools/web-browse.mjs --dir dist/web --chrome <chrome path> --out out/web [--require-adapter] [--only <n>] [--all]`.
- It serves loopback only, tries software-adapter flag sets (`FLAG_SETS` in the tool) until one gives an adapter, screenshots the canvas (must not be blank), checks frames keep coming and that the page survives `device.destroy()`, then reruns with `navigator.gpu` removed (must show the loud message and `ok:false reason no-webgpu`).
- Exit: 0 proven, 1 a proof failed, 2 could not run (no Chrome), 3 no adapter with `--require-adapter`. Every failed attempt is written to `attempts.json`.
- The flag line that works on ubuntu-latest (Chrome 155, no GPU):
  `google-chrome --headless=new --no-sandbox --disable-dev-shm-usage --no-first-run --no-default-browser-check --hide-scrollbars --mute-audio --window-size=1280,720 --enable-unsafe-webgpu --enable-features=Vulkan --use-angle=swiftshader --use-webgpu-adapter=swiftshader`
- `--use-angle=swiftshader` is what makes presentation work. `--use-vulkan=...` gives a blank canvas; `--enable-unsafe-webgpu` alone gives an adapter whose device is lost again and again.
- A runner needs `sudo apt-get install -y libvulkan1 mesa-vulkan-drivers`, or the canvas never presents.

## Read the golden line
- The page prints `golden <16 hex> state <16 hex> corpus <16 hex> OK <n> ms` in `#golden` and sets `data-golden` on body to `ok`, `mismatch` or `error`. The tool matches the shape `GOLDEN_OK` in `tools/web-browse.mjs`.
- `golden MISMATCH got ... expected ...` means the wasm engine diverged: go to skill `golden-matrix`. Expected golden is `d57b5c8ae3096614`.

## Workflows (public mirror only, repo `maximkone/engine2040-public`)
- `web-build.yml`: jobs `build` (unit tests of the web tools, `cargo test -p engine-web --lib`, clippy for wasm32, build, 8 MB and MIME check) then `browser` (headless Chrome, budgets). Start it by pushing a throwaway branch `web-build-*` of the mirror, or dispatch.
- `pages.yml`: same build, plus the mdBook handbook at `/docs/` and rustdoc at `/docs/api/`; `tools/book-links.mjs` fails the build on a broken internal link. It deploys on mirror `main` pushes only.
- `wasm-check.yml` and `golden-matrix.yml`: skills `wasm-compat` and `golden-matrix`.
- `wasm-opt`: never apt's binaryen 108 (its output fails in every browser: `Table.grow ... failed to grow table by 4`). Workflows pin release `version_133`. Rust 1.97 needs the `--enable-*` flags in `WASM_OPT_FLAGS` or wasm-opt rejects the module.

## Get your tree to the mirror
- `node tools/public-export.mjs` first (builds and scans `target/public-export/tree` from `git archive HEAD`, writes a verify receipt). Then `node tools/public-export.mjs --commit --push`; `--commit` refuses without a fresh receipt for this HEAD. First time only: `--remote <url>`.
- Read the result: `node tools/public-export.mjs --ci` (last mirror runs and the failed step).
- It exports committed `HEAD`, not your dirty files. For one cargo line use `node tools/gh-check.mjs` instead (skill `github-check`).

## Pitfalls we hit
- Reading Actions logs while a run is still going gives partial text: wait for the run to end.
- A browser cannot use `std::fs`, `Instant::now()`, threads or raw sockets: skill `wasm-compat`.
- A `Blocking waiting for file lock` on a wasm build is another cargo run: wait.
