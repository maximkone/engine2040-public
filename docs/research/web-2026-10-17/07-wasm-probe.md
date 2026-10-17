# WebAssembly probe, 2026-10-17

Question: how far is engine2040 from running in a browser (WebAssembly), with physics and ragdoll, and from a URL the work PC can open?

Method: `.github/workflows/wasm-probe.yml` (new, uncommitted) ran on throwaway branches `wasm-probe-*` of the public mirror (all deleted afterwards). The snapshot was the mirror tree of HEAD `66111b0f` plus only that one file, so nobody's half-edited crate was probed. No crate source was changed. Final run: https://github.com/maximkone/engine2040-public/actions/runs/39036560799 (58 jobs, all green as jobs; the compile result of each crate is in its artifact). Earlier runs: 39033689553, 39034412244, 39034873827, 39035755526. Artifacts hold every log, JSON compiler message list and crate graph.

Three kinds of evidence, never mixed:
- COMPILED = `cargo check --locked --keep-going --target wasm32-unknown-unknown -p <crate>` (the browser target). Type-checks only. Says nothing about running. Each job also lists the units the compiler really checked (JSON `compiler-artifact`), so "wgpu compiled" means `wgpu@30.0.1 lib` is in that list.
- RAN = executed: the same code built for `wasm32-wasip1` and run under Node 24's built-in WASI (a 32-bit, no-thread machine), and a generated cdylib shim built for `wasm32-unknown-unknown` and run under plain Node with zero imports. Not a browser, but the same V8 wasm engine and the same instruction set.
- READ = from the code (grep of non-test source). Marked as such.

## 1. Verdict

- The simulation side is already there. The golden tape and the whole break corpus come out bit-identical on wasm32: golden `d57b5c8ae3096614`, tape `last_state 2aca9338670aa0cb`, corpus `b37fc6d66df2d417` over 20 blocks. Tape file byte-identical to the native Linux tape (SHA-256 `C5DF1670...93EA`), `tools/golden-matrix.mjs` says MATCH and CORPUS RESULT PASS. Done on wasm32-wasip1 (scalar and with SIMD128) and on wasm32-unknown-unknown (scalar).
- Rapier (workspace pin, `enhanced-determinism`) builds and runs on wasm32 (run on wasip1, compiled for unknown-unknown): 156 of 156 physics lib tests pass (native 156), the 50-fall ragdoll tape digest `5d7aedd4c6095b7c` is identical in wasm (scalar and SIMD128) and native. Cost: 1.7 to 1.85 times native for scalar wasm, 1.4 to 1.55 times with SIMD128 (shared CI runners, noisy). The legacy ECS/physics fixture is 1.06 to 1.10 times native.
- 34 of 37 crate/feature legs type-check for the browser target with no change. The three that do not: `engine-scene` and `engine-app` (uuid needs a randomness source, one feature fixes both) and `engine-net --features quic` (quinn needs mio, no wasm; by design a browser cannot do raw UDP/QUIC).
- The GPU side type-checks too: `engine-render --features window` and `engine-game --features app` (winit, wgpu 30.0.1, cpal, gilrs-core all really checked). The whole `arena` game binary even links for wasm32-unknown-unknown in release: 2.55 MB raw, 638 KB gzip, 434 KB brotli, before wasm-opt. That is compile-and-link only. It would not run today.
- What stops it running is not the compiler but behaviour: `Instant::now()` and `SystemTime::now()` trap (proved), `std::fs` and threads fail (proved), `pollster::block_on` and `device.poll(wait)` cannot block in a browser (READ, 72 sites in render), the content is read from files (READ), the net crate has UDP and QUIC only.
- Compute passes (bloom, TAA, GTAO, fog, cull, cluster) and storage buffers in the skinning vertex shader mean WebGPU only. wgpu 30.0.1 with our features on wasm32 is exactly WebGPU, no WebGL2. A headless Chrome on a GitHub runner does get a software WebGPU adapter with the right flags, so a browser frame can be proven without a GPU.

## 2. Per-crate table: `cargo check --target wasm32-unknown-unknown`

Units = units the compiler checked in that leg (crate plus every dependency). Graph = crates in the wasm32 dependency graph (external crates in brackets). "Runtime notes" are READ unless they say proved.

| Leg (cargo line) | Compiles | Units | Graph | Why not, or what to know at run time |
|---|---|---|---|---|
| engine-core | yes | 1 | 1 (0) | none. std-only. No clock, thread or file in non-test code. |
| engine-ecs | yes | 2 | 2 (0) | shared job pool: `available_parallelism()` is Err on wasm so it makes 0 workers and runs inline (proved: returns 0). 10 tests that need real threads die and 2 that panic on purpose (section 3). |
| engine-geo | yes | 3 | 2 (0) | default features only. `uv` (C build script, xatlas FFI), `lod` (meshopt), `boolean` not probed: they need a C/C++ compiler for wasm. |
| engine-geo --features collision | yes | 49 | 40 (38) | parry3d 0.30.2 builds (libm, wide). |
| engine-spatial | yes | 2 | 2 (0) | none. 301 lib tests pass on wasi. |
| engine-rules | yes | 54 | 38 (36) | rhai pulls getrandom 0.2 with `js` and wasm-bindgen: the module gets JS imports, so it needs the wasm-bindgen CLI step anyway. |
| engine-input | yes | 2 | 2 (0) | none. |
| engine-combat | yes | 3 | 3 (0) | none. 261 tests pass. |
| engine-anim | yes | 5 | 5 (0) | none. 381 tests pass. |
| engine-ai | yes | 6 | 6 (0) | none. 264 tests pass. |
| engine-nn | yes | 2 | 2 (0) | none. 30 pass, 1 killed (WASI has an empty OS label; the test asserts it is not empty). |
| engine-physics | yes | 2 | 2 (0) | none. 79 pass. |
| engine-physics --features rapier | yes | 63 | 54 (48) | rapier3d 0.36.0, parry3d 0.31.1, nalgebra 0.35.0, simba 0.10.2 (libm), wide. No rayon (feature `parallel` is off). 156 tests pass, ragdoll digest equal. |
| engine-arena | yes | 10 | 10 (0) | `fork.rs` (2 sites) and `gym.rs` (1 site) spawn threads: fail at run time (proved: spawn returns Err, 15 tests killed). `parity.rs` has one `Instant::now()`. Not on the match path. |
| engine-arena --features sim | yes | 67 | 58 (48) | same, plus rapier. 169 tests pass, 15 killed (all fork/gym threads or panics). |
| engine-destruction | yes | 2 | 2 (0) | none at run time. 146 pass, 1 killed (a test that itself runs two threads). |
| engine-nav | yes | 3 | 3 (0) | none. 67 pass. |
| engine-scene | NO | 20 | 20 (18) | `error: to use uuid on wasm32-unknown-unknown, specify a source of randomness using one of the js, rng-getrandom, or rng-rand features` (uuid-1.26.1/src/rng.rs:104). Root cause: `uuid` with `v4`. With feature `js` (patched in the runner only) it compiles. The getrandom cfg flag was NOT needed and the compiler never asked for it. |
| engine-material | yes | 3 | 3 (0) | none (`validate`/naga not probed). |
| engine-world | yes | 2 | 2 (0) | none. 34 pass. |
| engine-save | yes | 3 | 3 (0) | 6 `std::fs` sites (save files): need a storage seam (localStorage or IndexedDB). |
| engine-game | yes | 29 | 26 (7) | default features. 6 `Instant::now()` and 2 `SystemTime::now()` (crash.rs) in lib (trap), `current_exe` in arena_kit.rs, about 24 `std::fs` sites (8 on the content path). |
| engine-proof | yes | 14 | 13 (0) | 20 `std::fs` sites in test/tool modules. The golden and corpus code (`record`, `replay`, `matrix`) runs clean (proved on unknown-unknown). 135 pass, 6 killed (`current_dir` unsupported on WASI, file writes). |
| engine-observe | yes | 2 | 2 (0) | none. |
| engine-assets | yes | 3 | 2 (0) | default features. 2 `Instant::now()` in `archive.rs:369` and `bundle.rs:368` (trap when a bundle is opened). Bundle APIs take bytes (`ArchivedBundle::open(Arc<[u8]>)`, `BundleLoader::load(&[u8])`): good for fetch. About 28 `std::fs` sites, mostly tooling (recook, gen_intake, bundle_cook). |
| engine-assets --features import | yes | 145 | 116 (110) | gltf 1.4.1, image 0.25.10 (pulls rayon; on a single-thread wasm it must fall back to the caller thread: not verified). |
| engine-ui | yes | 3 | 3 (0) | none. |
| engine-audio | yes | 3 | 3 (0) | 1 `std::fs` in `decode.rs`. |
| engine-audio --features device | yes | 35 | 28 (25) | cpal 0.18.2 really checked; it has a `webaudio` host for wasm32-unknown-unknown (and an `audioworklet` feature). Needs a user gesture to start. |
| engine-net | yes | 13 | 12 (0) | default features have no quinn. 8 `UdpSocket::bind` sites and 12 `Instant::now()` (trap) at run time; `Transport` trait (`send_unreliable`, `send_reliable`, `recv`) is the seam for a browser transport. |
| engine-net --features quic | NO | 80 | 65 (53) | `error: This wasm target is unsupported by mio. If using Tokio, disable the net feature.` (mio-1.2.3/src/lib.rs:44; "could not compile mio due to 36 previous errors"). Root cause: quinn needs tokio net and mio (quinn-udp and ring are in the same graph and were not reached). By design: use WebTransport in the browser. |
| engine-app | NO | 29 | 30 (18) | same uuid error, through engine-scene. With `js` it compiles. Also 1 `Instant::now()` in `startup()` (trap). Nothing in game or the golden bin depends on engine-app. |
| engine2040 (root bin, `--record`) | yes | 19 | 19 (0) | zero external crates. Uses `std::fs`/`std::env::args` in `src/cli.rs`, so it is run as a WASI program, not in a page. |
| engine-render (default) | yes | 3 | 3 (0) | CPU side only. 65 `Instant::now()` in lib (trap), a thread in `sky.rs`, a `Condvar` in `cache.rs`, 8 `std::fs` sites. |
| engine-render --features v1 | yes | 147 | 119 (111) | glam, image, serde_json, assets import. |
| engine-render --features window | yes | 186 | 154 (146) | wgpu 30.0.1 lib, winit 0.30.13 lib, pollster 1.0.1, web-sys, wasm-bindgen all checked. The Windows-only `windows` crate is `cfg(windows)` only, so it is not in the wasm graph. Run time: 34 `pollster::block_on`, 21 `device.poll(wait_indefinitely)`, 17 `map_async` (READ). |
| engine-game --features app | yes | 219 | 183 (163) | includes the `arena` and `staged_fight` bins: winit, wgpu, cpal, gilrs 0.11.2 (gilrs-core uses the browser Gamepad API), image, gltf, sha2. |

Not a leg: `engine-editor`, `engine-endpoint`, `engine-provider`, `engine-perf`, `engine-playtest`, `engine-distill`, `engine-chaos` (tools, not shipped to a browser), and the root `window` feature.

## 3. What was executed on wasm

| Run | Target | Result |
|---|---|---|
| Golden tape, seed 2040, 600 ticks, `--record --verify` | wasm32-wasip1, Node WASI, release | `final_hash=d57b5c8ae3096614 state_hash=2aca9338670aa0cb verified=true`. Tape byte-identical to the native Linux tape of mirror run 39032745322. `golden-matrix.mjs`: MATCH. |
| Corpus row (`matrix_line`) | wasm32-wasip1 | `golden=d57b5c8ae3096614 corpus=b37fc6d66df2d417 blocks=20` and all 20 block digests equal the Linux row: CORPUS RESULT PASS. Host label printed `-wasm32` at HEAD (empty OS string); the working tree's `matrix.rs` already has a `label_of` that maps it to `wasi`. |
| Golden tape and corpus via a generated cdylib | wasm32-unknown-unknown, plain Node, 0 imports | same three numbers: `final_hash=d57b5c8ae3096614 state_hash=2aca9338670aa0cb`, corpus `b37fc6d66df2d417` 20 blocks. This is the browser target itself. |
| Same, built with `+simd128` | wasm32-wasip1 | tape byte-identical to the scalar wasm tape; corpus equal. |
| 100,000 ticks of the legacy fixture, release | wasi vs native, same runner, three CI runs | wasm vs native: 1.05 vs 0.95 s, 1.22 vs 1.15 s, 0.76 vs 0.71 s, so 1.06 to 1.10 times. Same `final_hash 34e27da4092018ae` every time. SIMD128: no change (the fixture does not use `wide`). |
| engine-physics --features rapier lib tests | wasip1 | 156 passed, 0 failed (native 156). |
| Ragdoll tape (`ragdoll_gap`: 50 seeded falls plus a clinch, 19,320 ticks, run 3 times = 57,960 steps) | wasip1 vs native | digest `5d7aedd4c6095b7c`, `inter_max 0.0000m`, `sep_min 0.0327m` identical. Scalar wasm vs native in three runs: 1.35 vs 0.74 s, 1.73 vs 0.94 s, 1.26 vs 0.69 s (1.82 to 1.84 times). SIMD128: 1.28 vs 0.83 s and 1.36 vs 0.94 s (1.45 to 1.54 times). The whole 156-test lib: 1.70 to 1.78 times scalar, 1.39 to 1.40 times SIMD128. About 22 microseconds per fixed step of at most 4 bodies in wasm. Heap bytes per tick differ (32-bit pointers), as expected. |
| Lib tests, 15 crates | wasip1, one thread | 2269 passed, 0 failed, 35 killed. |
| What the 35 killed tests are | | Not numeric: wasm32 has no unwinding, so a panic aborts the module, and each was re-run alone to read its panic line. 26 need threads (`failed to spawn thread` / `Spawn { kind: Unsupported }`: 10 in ecs pool and parallel, 15 in arena fork and gym, 1 in destruction), 6 call `current_dir` (WASI: "not supported by WASI yet"; proof), 2 panic on purpose (ecs `a_panicking_chunk_*`), 1 asserts the OS label is not empty (nn; WASI reports ""). Per crate: ecs 12, arena 15, proof 6, destruction 1, nn 1; core, combat, anim, ai, physics, rapier, input, nav, spatial, world: none. |
| What std does on wasm32-unknown-unknown (proved by running) | | `Instant::now()`: trap (`RuntimeError: unreachable`). `SystemTime::now()`: trap. `thread::Builder::spawn`: returns Err. `fs::read`: returns Err. `available_parallelism()`: Err (the ecs pool treats it as 1 core). |
| Whole `arena` game linked | wasm32-unknown-unknown, release, no wasm-opt | 2,552,864 bytes; gzip -9 638,281; brotli -q11 433,950; 333 imports (331 `__wbindgen_placeholder__`, so the wasm-bindgen CLI step is required); built in 127 s on a 2-core runner. |
| Headless Chrome 155 on ubuntu-latest | | `navigator.gpu` exists, adapter null without flags. With `--enable-unsafe-webgpu --enable-features=Vulkan --use-angle=swiftshader --use-webgpu-adapter=swiftshader --ignore-gpu-blocklist`: adapter vendor google / swiftshader (fallback true), device creation ok, features include timestamp-query, subgroups, texture-compression bc/astc/etc2; limits maxStorageBuffersPerShaderStage 10, maxComputeInvocationsPerWorkgroup 256, maxBindGroups 4, maxTextureDimension2D 8192, maxBufferSize 1 GiB. The other flag set (`--use-vulkan=swiftshader --disable-vulkan-surface`) returned no result inside the 15 s budget. |

## 4. Answers from the code

Rapier. `rapier3d = 0.36, default-features = false, features = [dim3, f32, std, enhanced-determinism]`; Cargo.lock has rapier3d 0.36.0 with parry3d 0.31.1. It builds for wasm32-unknown-unknown (63 units checked) and its tests pass on wasm32-wasip1, including the ragdoll tape digest. `enhanced-determinism` turns on `simba/libm_force` and `parry3d/enhanced-determinism` (pure-Rust libm, no hardware transcendental), which is why x86-64 and wasm agree. The 156-test count equals native; the plan's "152 today" in WEB3 is stale. Not shown: Rapier with the real game scene, a long soak, `parallel` (rayon) which stays off, and SIMD128 on a different V8 (same digest on Node 24 only).

Golden hash in wasm. Yes, reproduced, on both wasm targets (section 3). The reason it is cheap: the golden path is `engine_proof::record` and the `matrix` collector, which use no clock, thread or file; `engine-core` and the sim crates are std-only. 32-bit `usize` did not move any digest. The only `u64 as usize` before `%` that I found is in test-only shuffles in ecs (snapshot.rs:351, bridge.rs:941, archetype.rs:1072). The only width assertion is `usize::BITS >= 32` in ecs archetype.rs.

wgpu backend. Cargo.lock: wgpu 30.0.1 (the workspace pin and the only wgpu in any probed graph; the lock also lists 29.0.4 for a crate I did not probe). `cargo tree -e features` for wasm32-unknown-unknown shows wgpu features `default, std, parking_lot, wgsl, webgpu (=> web, web-sys), dx12, gles, metal, vulkan`. `webgl` is not enabled and `gles` does nothing on wasm without it. So a wasm32 build uses the browser WebGPU backend only. wgpu 30 ships its own `webgpu_sys` bindings, so the old `--cfg=web_sys_unstable_apis` RUSTFLAGS is not needed (the probe built without it).

Compute vs WebGL2. Nine WGSL files hold `@compute` (post/bright, downsample, upsample, composite, taa; ssao; cull; cluster; fog_vol) and ten Rust modules embed compute WGSL (gtao, aa_scale, fog, accel, baked_gi, decals, material_layer, particles_gpu, shadow_cascade, sky). WebGL2 has no compute shaders and no storage buffers, and our skinning reads a `var<storage, read>` joint palette in the vertex stage (`forward.wgsl:101`), so skinning needs WebGPU too. The device is requested with default limits (no `required_limits`), which every WebGPU browser meets; the only extra feature asked is timestamp-query when present. Verdict: WebGPU is required; there is no cheap WebGL2 fallback. Browsers with WebGPU on by default (from memory, not checked here): Chrome/Edge 113+, Firefox 141+ on Windows, Safari 26.

Content. Today content reaches the game by `std::fs::read` of loose files: kit glTF/textures from `content/arena` (`arena_kit.rs:967, 1309`, found from `current_exe`), fighter manifest and glb (`fighter_bind.rs:179, 215`), fighter look (`fighter_look.rs:1046, 1256`), audio (`audio/decode.rs`). About 8 sites. The reading side already has byte-in APIs: `ArchivedBundle::open(Arc<[u8]>)`, `ArchivedBundle::open_slice(&[u8])`, `BundleLoader::load(&[u8])`, `load_bundle(&[u8])`. So the web path is: cook one bundle in CI, `fetch()` it into bytes, call those. Size: `content/` is 2.2 GB on this disk but the ship set is about 12 MB of textures plus the 21 MB HDR and 37 MB kit glTF the SIZE-BUDGET note names. `content/*/bundle` is gitignored, so the web build job must cook bundles itself.

Hosting. The mirror is public and has no Pages site (GitHub API: 404, `has_pages false`). Enabling Pages (Settings, Pages, Source: GitHub Actions) is an owner action and I did not do it. URL would be `https://maximkone.github.io/engine2040-public/`. Pages serves HTTPS, which WebGPU needs, but cannot set COOP/COEP headers, so no SharedArrayBuffer threads without a service-worker shim; single thread is enough for now (the ecs pool already runs inline).

Multiplayer. `engine-net` already has the right seam: `trait Transport { send_unreliable, send_reliable, recv }` in `quic.rs:187` with Udp, Relay, Sim and quinn implementations, and a `Wire` trait in `link.rs:347`. A browser transport is one more `Transport` impl on WebTransport (datagrams plus one stream) with a WebSocket fallback, plus the matching server listener. Rollback, snapshot codec and heal stay unchanged.

## 5. Blockers, ranked by how many crates they hit

Compile-time (stop `cargo check`):

| # | Blocker | Crates hit | Fix |
|---|---|---|---|
| 1 | uuid has no randomness source on wasm32-unknown-unknown | 2 legs (scene, app) and anything that turns scene on (assets `registry`, proof `scene-museum`/`m1`, editor) | `uuid` feature `js` under `[target.'cfg(all(target_arch = "wasm32", target_os = "unknown"))'.dependencies]`: 3 manifest lines. Lockfile already has the wasm-bindgen entries (`--locked` passed). |
| 2 | quinn needs mio (no wasm) | 1 leg (net `quic`) | keep `quic` native-only; WebTransport on the web. |

Run-time (compiles, wrong in a browser), READ counts unless marked proved:

| # | Blocker | Crates | Sites | What happens |
|---|---|---|---|---|
| 1 | `std::fs` | 7 (game, assets, proof, save, render, net, audio) | about 89, 8 on the content path | Err Unsupported (proved). Needs an `AssetSource` seam and a storage seam. |
| 2 | `Instant::now()` / `SystemTime::now()` | 6 (render 65, net 12, game 8 plus 10 in bins, assets 2, arena 1, app 1) | 89 lib plus 10 bin | traps the whole module (proved). One clock seam (`web-time 1.1.0` is already in the lock through winit) fixes it. |
| 3 | threads | 4 (ecs, arena, net quic, render sky) | 1 + 3 + 4 + 1 | spawn Err (proved). ecs degrades to inline by itself; arena fork/gym and the sky bake thread need a single-thread path. |
| 4 | blocking wgpu: `pollster::block_on` (34), `device.poll(wait)` (21), `map_async` (17), `enumerate_adapters` (2), winit `run_app` (2) | 2 (render, game bin) but 23 render files | about 76 | `block_on` on the browser main thread never completes; adapter enumeration is empty on web. Needs async init and deferred readbacks. This is the largest single job. |
| 5 | `std::net::UdpSocket` | 1 (net) | 8 | Err at run time. WebTransport instead. |
| 6 | `env::current_exe/current_dir/temp_dir/args/var` | 5 (game 12, render 4, assets 2, core 1, proof 1) | 20 | unsupported or empty (proved for `current_dir` on WASI); the content root must not depend on the exe path. |
| 7 | `Condvar` in `render/cache.rs`, blocking wait | 1 | 1 | would deadlock the main thread on contention. Not verified. |

Constraints, not bugs: WebGPU only; single thread on Pages; wasm32 memory tops out at 4 GiB (a potato working set of 3 GB in G9 is too close, WEB11's 1.5 GB bar is right); audio starts only after a user gesture.

## 6. Plan, stepwise, with sizes

Sizes use the board's S/M/L; "lines" is my estimate of changed lines including tests (not counted from a diff). Rows are the existing WEB rows in `docs/SPRINT-22-web-realism.md` plus two small additions.

| Step | What | Rows | Size | Needs |
|---|---|---|---|---|
| 1. Headless sim in wasm, hash-checked | Add the wasm tape as a fourth recorder in `golden-matrix.yml` (about 45 workflow lines: wasi build, the 15-line Node runner, upload; the compare job already takes N tapes). Make the host label `wasi`/`web-wasm32` (the tree already has `label_of`). Run the rapier ragdoll digest in the same matrix. Fix uuid (3 lines). Put the clock seam in the 6 crates (about 100 lines for 99 sites, mostly `use` swaps) and `tools/wasm-gate.mjs` (about 150 lines) so nothing regresses. Turn this probe's matrix into `wasm-check.yml`. | WEB1 S, WEB2 M, WEB3 S; new WEB2b "clock seam" S | about 400 lines | nothing. Already proved by this probe; the work is making it a permanent gate. |
| 1.5. First URL for the work PC (new, cheap) | A page that loads `wasm_probe_shim`-style wasm through the wasm-bindgen CLI and prints `golden d57b5c8ae3096614 corpus b37fc6d66df2d417 OK` in the browser, deployed to Pages. Proves the wasm-bindgen step, the Pages workflow and the work PC's browser end to end before any rendering. | part of WEB4 + WEB10 | S to M, about 250 lines (crate `web` cdylib 120, build tool 80, `pages.yml` 80) | owner enables Pages once |
| 2. Rendering over WebGPU | Browser shell (`crates/web`): canvas, `winit` on the web (`spawn_app`, canvas attach) instead of `run_app`, async adapter/device request (adapter pick without `enumerate_adapters`), then the ~76 blocking sites in 23 render files: error-scope pops to async or deferred, readbacks to `map_async` with a frame-later poll, receipts kept native-only behind `cfg`. Move the `ArenaApp` handler out of the 3341-line `bin/arena.rs` into a lib module both entry points use. The `window` feature already compiles for wasm, so the feature split in WEB5 is for run time, not for the build. | WEB4 M, WEB5 L (3 to 4 landings) | about 1,000 to 1,500 lines | step 1.5 |
| 3. Physics and ragdoll cost in a browser | The numbers exist already: 22 microseconds per 4-body fixed step in wasm scalar, 1.4 to 1.55 times native with SIMD128, same digest. Next: build the web profile with `+simd128` and `wasm-opt -O3`, then measure the real physfx scene (8 ragdolls plus 200 debris bodies) in the page and print it. The authority layer is integer and unaffected; Rapier stays cosmetic (physfx) unless this digest keeps matching. | WEB3 (done in the probe), RZ1, RZ9, WEB8 | S for the measurement, the rest is the RZ rows | step 2 |
| 4. Content and input/audio | `AssetSource` seam (native: files, web: fetched bundle) at the 8 content sites, bundle writer in `web-build.mjs`; keyboard/mouse via winit web, gamepad via gilrs-core (already checked), cpal `webaudio` host started on first click. | WEB6 M (about 400 lines), WEB7 M (about 200 lines) | M + M | step 2 |
| 5. Multiplayer | `WebTransport` `Transport` impl in the page crate (about 300 lines using web-sys), server listener with WebTransport plus WebSocket fallback (about 500 lines, `wtransport` or quinn+h3), a headless-Chrome duel job (about 200 lines; the SwiftShader flags above give the browser a software adapter). | WEB9 L | L, about 1,000 lines | WEB5, MP6 |
| 6. Pages | Deploy `dist/web` from the mirror's main with `actions/deploy-pages`; page shows commit and golden hash; refuse non-public files. Step 1.5 already covers the first version. | WEB10 S, WEB11 S | S + S | owner enables Pages |

Order that gets the owner a URL fastest: 1, 1.5, 2, 4, 3, 5, 6 (6 is mostly 1.5). The rendering step is the only large unknown.

## 7. Where the plan text and rows differ from what the compiler said

- `web-and-realism.md` section 1 says `engine-render` `window` pulls the Windows-only `windows` crate. For wasm it does not: that dependency is `cfg(windows)`. `--features window` compiles for wasm32 as it stands. The split is still worth doing (winit and pollster are wrong at run time), but WEB5's "cargo check -p engine-render --features gpu --target wasm32" is already true for `window`.
- Same file says "the net crate is quinn QUIC". Default `engine-net` has no quinn and compiles for wasm; only `--features quic` does not.
- WEB1 and WEB3 say wasmtime. Node 24's built-in WASI ran the tapes and 2269 tests with zero installs; either works.
- WEB3's "152 today with rapier" is 156 now, and the wasm count equals the native count.
- WEB4 asks which SwiftShader flag works: `--enable-unsafe-webgpu --enable-features=Vulkan --use-angle=swiftshader --use-webgpu-adapter=swiftshader --ignore-gpu-blocklist` on Chrome 155 (section 3).
- G10's status cell ("nothing builds for wasm32 yet") can now say: golden tape, corpus and Rapier ragdoll digest equal on wasm32; 34 of 37 legs type-check; the arena game links to a 2.55 MB wasm.

## 8. Not proven

- Nothing ran in a real browser tab: no canvas, no WebGPU frame from our renderer, no wasm-bindgen glue. The shim and `arena.wasm` were not loaded in Chrome.
- Whether our WGSL passes validate on the browser's Tint compiler (naga is the validator here); VR3 owns that.
- Frame cost of the real renderer in wasm. Only CPU physics and the sim were timed. Those timings are Node V8 on a shared 2-core runner, noisy (ratios varied 1.7 to 1.85 scalar between runs).
- `rayon` inside `image` on a single-thread wasm, `Condvar` in `render/cache.rs`, geo features `uv`/`lod`/`boolean`, `engine-material --features validate`.
- Memory: peak wasm memory of the real game.
- Other browsers, the work PC's GPU and group policy (WebGPU can be blocked on managed Chrome).

## 9. How this was run, and how to run it again

- File: `.github/workflows/wasm-probe.yml` (mirror guard like golden-matrix.yml; push of `wasm-probe-*` or workflow_dispatch). It is not on the mirror's main yet, so only the push trigger works until the next export. Jobs: `check` (37 legs), `wasi-golden`, `wasi-tests` (16 legs), `unknown-run`, `wasm-size`, `webgpu-adapter`, `table`. The `table` job writes one summary page. The final file differs from the version that ran only in the header comment.
- I pushed the branch with a 45-line scratch script modelled on `startRemote` in `tools/gh-check.mjs` (the cached HEAD tree plus this one file, committed on `wasm-probe-<id>`, parent `origin/main`). Not committed to the repo. Branches `wasm-probe-mvce18p5`, `-mvcef73t`, `-mvceo2an`, `-mvcf4p1b`, `-mvcfhq3h`, `-mvcfjwvy` were created and deleted; none remains.
- Re-run once the file is on the mirror: `gh workflow run wasm-probe.yml -R maximkone/engine2040-public --ref <branch>`; fetch results with `gh run download <id> -R maximkone/engine2040-public`.
- The uuid retry patch (`sed` adding `"js"` to `uuid = {...features}`) is in the artifact `probe-engine-scene/uuid-js.patch`.
