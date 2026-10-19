# Build, run and publish the web target

The web target is the same game in a browser tab: WebAssembly for the code, WebGPU for the pixels. This page shows how to build it, run it, test it in a headless browser and publish it, with the flags and limits we measured. Sources: `docs/plan/web-and-realism.md` (the plan and the headless-Chrome run of 2026-10-17) and `docs/research/web-2026-10-17/07-wasm-probe.md` (the compiler and run probe).

## What works today, and what does not

Measured in the probe (mirror run 39036560799):

- The golden tape, the 20-block break corpus and the Rapier ragdoll digest are bit-identical on wasm32: golden `d57b5c8ae3096614`, tape `last_state 2aca9338670aa0cb`, corpus `b37fc6d66df2d417`, ragdoll digest `5d7aedd4c6095b7c`.
- 34 of 37 crate legs type-check for `wasm32-unknown-unknown` with no change. The three that do not: `engine-scene` and `engine-app` (uuid needs a randomness source) and `engine-net --features quic` (quinn needs mio; a browser has no raw UDP).
- The whole `arena` game links to a 2,552,864 byte wasm (638,281 gzip, 433,950 brotli) before wasm-opt. That is compile and link only.
- Rapier costs 1.7 to 1.85 times native in scalar wasm and 1.4 to 1.55 times with simd128 (shared runners, noisy).
- What stops the full game from running is behaviour, not the compiler: `Instant::now()` traps, `std::fs` and threads fail, `pollster::block_on` and `device.poll(wait)` cannot block (about 76 sites), the net crate has UDP and QUIC only.

The browser shell in `crates/web` is the first thing that does run: it opens a WebGPU device, draws a moving square, runs the 600-tick golden tape and the cut corpus in wasm, and prints the commit and the result.

## Prerequisites

`node tools/web-doctor.mjs` lists what is missing, one line each, with the fix command. It checks Node 24, the Rust targets `wasm32-unknown-unknown` and `wasm32-wasip1`, the wasm-bindgen CLI at the exact version `Cargo.lock` locks, `wasm-opt`, a Chrome or Edge 113 or newer, and `gh`.

```sh
node tools/web-doctor.mjs
```

`tools/wasm-gate.mjs` counts the calls in the web-path crates that type-check for wasm32 but break in a browser (clock, files, threads, blocking GPU waits) and ratchets them against `tools/wasm-gate.baseline.json`:

```sh
node tools/wasm-gate.mjs --help
```

Use binaryen release `version_133` for `wasm-opt`. Do not use the apt package (version 108): its `-Oz` output fails in every browser at load with `WebAssembly.Table.grow(): failed to grow table by 4`. Rust 1.97 needs `--enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext` (plus multivalue, reference-types and mutable-globals) or wasm-opt rejects the module; `tools/web-build.mjs` passes them.

## Build

```sh
node tools/web-build.mjs
```

The tool runs `cargo build --locked --profile web --target wasm32-unknown-unknown -p engine-web`, then the wasm-bindgen CLI, then wasm-opt, and writes `dist/web`: `index.html`, `loader.js`, `coi-serviceworker.js` (the WEB14 isolation worker), `engine.js`, `engine_bg.wasm` and `build.json` (sizes, gzip sizes, SHA-256, tool versions, the commit). It exits 1 and says which step failed. Flags: `--out <dir>`, `--commit <hash>` (the page prints it) and `--no-opt` (skip wasm-opt).

```sh
node tools/web-build.mjs --no-opt
```

The size limit for `dist/web` is 8 MiB before game content. The first green run measured 237,384 bytes: `engine_bg.wasm` 166,387 (72,223 gzip), `engine.js` 68,283, `index.html` about 4 KB.

## Run it

WebGPU needs https or `localhost`. Serve `dist/web` from loopback and open the page:

```sh
python -m http.server 8080 --bind 127.0.0.1 -d dist/web
```

Open `http://localhost:8080/`. The server must type `engine_bg.wasm` as `application/wasm`; check it:

```sh
curl -sI http://localhost:8080/engine_bg.wasm
```

## Test it in a headless browser

`tools/web-browse.mjs` serves `dist/web` on loopback and drives headless Chrome twice, once with WebGPU and once with `navigator.gpu` removed.

```sh
node tools/web-browse.mjs --dir dist/web --chrome google-chrome --out out/web
```

Add `--require-adapter` to make a missing adapter a failure (exit 3) and `--only <n>` to try one flag set. The WebGPU run reads the JSON line from `#report`, takes a screenshot, tests that the canvas is not blank, that frames keep coming and that the page survives a destroyed device. The no-WebGPU run must show the loud message with the browser versions, `data-state=fatal` and `"ok":false,"reason":"no-webgpu"`, and nothing may throw.

Measured on a GitHub runner (ubuntu-latest, Chrome 155.0.8059.39, no GPU), the line that gives a software WebGPU adapter and a canvas that presents is:

```text
google-chrome --headless=new --no-sandbox --disable-dev-shm-usage --no-first-run --no-default-browser-check --hide-scrollbars --mute-audio --window-size=1280,720 --enable-unsafe-webgpu --enable-features=Vulkan --use-angle=swiftshader --use-webgpu-adapter=swiftshader
```

- The runner needs the Vulkan loader and Mesa's software driver: `sudo apt-get install -y libvulkan1 mesa-vulkan-drivers`. Without them the flags still give an adapter, but the swap chain cannot be created and the device is lost over and over.
- `--use-angle=swiftshader` is the flag that makes presentation work. `--use-vulkan=swiftshader` or `--use-vulkan=native` give an adapter and a blank canvas.
- The adapter reports vendor `google`, architecture `swiftshader`, `maxStorageBuffersPerShaderStage` 10, `maxBindGroups` 4, `maxTextureDimension2D` 8192, `maxBufferSize` 1 GiB.
- A headless canvas screenshot can come out black. Do not trust a screenshot alone; read the frame back with `mapAsync` and hash the bytes.

The unit tests of the tools run without a browser:

```sh
node --test tools/web-doctor.test.mjs
```

## Publish

`.github/workflows/pages.yml` runs on every push of the public mirror's main and on demand. It builds `dist/web` exactly like the steps above, then assembles one site: the game at the root, this handbook at `/docs/`, rustdoc at `/docs/api/`. The deploy job runs on main only. After a deploy the module must be typed `application/wasm`:

```sh
curl -sI https://maximkone.github.io/engine2040-public/engine_bg.wasm
```

## Limits to plan around

- WebGPU only. Our bloom, TAA, GTAO and skinning passes use compute shaders and storage buffers; WebGL2 has none. A browser without WebGPU gets the fatal page, not a crash.
- One thread today. GitHub Pages cannot set the COOP and COEP headers that threads need; `coi-serviceworker.js` (WEB14) adds them from a service worker and the page reloads once, so `crossOriginIsolated` can be true. No Rust code uses threads yet, so the build stays single-threaded.
- wasm32 memory tops out at 4 GiB; the plan's bar for the web build is a peak under 1.5 GB.
- Audio starts only after a user gesture.
- Browsers with WebGPU on by default (stated by the plan, not tested here): Chrome and Edge 113, Firefox 141 on Windows, Safari 26.
- Everything in the published site is public.

If something fails, see [Troubleshooting](troubleshooting.md).
