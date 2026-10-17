# Web and realism (Look, Bodies, Multiplayer)

Owner 2026-10-17: "i want heavy games on web, multiplayer, destruction, ragdoll, dismembering and realistic graphic, renderer and collision, flexibility so ai sets it all up easy; i saw the arena, it looked meh; realistic physics in many aspects."

Outcome: the same game runs on Windows and in a browser tab (WebGPU plus WebAssembly), 8 fighters in a lit arena where bodies fall as ragdolls, limbs come off, walls break into debris, and a second machine (the work PC) joins by opening a URL. An AI sets the project up, builds both targets, opens the web build in a headless browser and reads back a frame and the view line, all through the door.
Proof: (1) the golden tape and the break corpus give one digest on x86-64 Windows, x86-64 Linux, aarch64 Linux and wasm32 (extends HS2, gate G6); (2) a browser frame of the arena beside the native frame (SSIM) and a pass-truth table with every look pass LIVE (gate G3); (3) a headless Chrome client and a native client play one duel against one server and end on one state digest (gate G8, web leg); (4) the physical layer budgets hold in a soak: 8 ragdolls, 200 debris bodies, 6 severed limbs under the frame budget (new gate G11).
Order: wave 1 below (no GPU, GitHub proves it), then wave 2, 3, 4. Rows live on the board in `docs/SPRINT-22-web-realism.md`; this file says why and in what order.
Crates: physics (new `physfx` beside it), destruction, combat, arena, render, game, assets, net, input, audio, proof, endpoint, perf.
Open board IDs: WEB1-WEB14, RZ1-RZ10, VR1-VR4, AIS1-AIS4, DOC1-DOC4.

Tag NOW. Prefixes: WEB (browser build and net), RZ (realistic physics, "reality"), VR (visual reality), AIS (AI setup), DOC (handbook, guides, generated references). Existing rows are not copied: this plan orders them and adds only what is missing.

## 1. Where it stands (checked 2026-10-17)

| Fact | Evidence |
|---|---|
| The playable arena has no rigid-body physics. The window game runs the kinematic AR2 match (capsule controller, integer balance, stagger, cut events); Rapier is off in it on purpose. No ragdoll tumbles, no debris flies, no limb flies. | `crates/game/Cargo.toml:12-17` ("`app` intentionally does NOT enable `engine-arena/sim`"), `:50-52` (game "never depends on engine-physics directly"); no read of `shards3d`, `fracture`, `BondedCollection` or `BreakEvent` in `crates/game/src` or `crates/render/src` |
| Dismemberment is decided but never drawn. Combat decides a cut and a limb loss (16-byte event, hashed); the presentation cue `DismemberLite` is a knockdown. | `crates/combat/src/cut.rs`, `limbloss.rs`, `crates/arena/src/limbs.rs`; `crates/combat/src/gore.rs:46` |
| Destruction exists as authority: bonded collections, Voronoi fracture, shards, snapshot. The window draws none of it. | `crates/destruction/src/{bonded,fracture,shards3d,snapshot}.rs` |
| Rapier ragdoll and powered ragdoll exist as libraries with tests (`--features rapier`). Cloth, grapple, vehicles, army LOD were cut as dead code on 2026-10-16 and can be restored. | `crates/physics/src/{ragdoll,powered,rapier}.rs`; `status-2026-10-17.md` |
| The authority layer is already the same on three architectures: golden tape and 20 corpus blocks equal on linux-x86_64, windows-x86_64 and linux-aarch64. Rapier determinism across architectures is not shown yet. | HS2, `golden-matrix.yml`, run 39032745322 |
| Why the arena looks meh: the live frame draws forward PBR plus GPU skinning and little else. Sky, IBL, sun shadow, ambient occlusion, TAA and decals are built but off or probe-only; the arena kit is boxes with flat colour; no reference captures exist (Q1, 0 of 10). | `render-look.md` (EG8 LF1 LF2 LF3 EG11-EG14), `finish-line.md` G3 and G4 |
| The wasm probe (2026-10-17, `docs/research/web-2026-10-17/07-wasm-probe.md`, mirror run 39036560799) says the compiler is not the problem: 34 of 37 crate legs type-check for wasm32 unchanged, `engine-render --features window` and `engine-game --features app` included, the whole `arena` game links to a 2.55 MB wasm (434 KB brotli, before wasm-opt), the golden tape, the corpus and the Rapier ragdoll digest are equal on wasm, Rapier costs 1.8x native (1.5x with simd128), and a headless Chrome gets a software WebGPU adapter. What stops it running is behaviour: `std::fs`, `Instant::now()` (traps), threads, blocking wgpu calls (about 76 sites), UDP and QUIC. | `target/reports/wasm-probe-2026-10-17.md` |
| Nothing is built for the browser. `WASM ship` is on the CUT list; no wasm target is installed; `engine-render` `window` pulls `winit`, `pollster` and the Windows-only `windows` crate; assets are read with `std::fs`; the net crate is `quinn` QUIC. | `docs/FOCUS.md` CUT list, `crates/render/Cargo.toml:20`; the wasm probe report `target/reports/wasm-probe-2026-10-17.md` has the compiler's answer per crate |

## 2. The design in one page

**Two physics layers.** This is what lets us have heavy, real-looking physics and the hash law and a browser.

- Authority layer (exists, hashed, integers): who hit whom, whether it cut, which limb is gone, who is floored, which bond broke. It decides every game outcome. It is the same on every PC (proved on three architectures; wasm is WEB1). It never reads a float from the physical layer.
- Physical layer (new, `crates/physfx`, floats, not hashed): a Rapier world on each client, seeded by authority events. It owns what only has to look and feel heavy: ragdolls after a knockout or fall, severed limbs, debris from a break, props that tumble, casings, corpses, capes. It may differ between machines by a few centimeters; the verdict (who fell, what broke) never does. It has hard caps (bodies, contacts, substeps per tier), sleeps bodies, recycles the oldest, and cannot call back into the sim (physfx depends on physics and combat types; arena and combat never depend on physfx).
- When a game needs a physical outcome to matter for rules (a boulder that crushes, a collapse that traps), the outcome goes through an authority row first (a break event, a damage volume) and physfx only paints it. That keeps rollback and web multiplayer honest.

**Web.** wasm32-unknown-unknown plus wasm-bindgen, WebGPU only (our bloom, downsample, GTAO and skinning passes are compute shaders; WebGL2 has none). Single thread in v1 (threads need cross-origin isolation headers that GitHub Pages cannot set; a service-worker shim comes later). Assets arrive by `fetch` into one bundle (no `std::fs` on the web path). Input from DOM events and the Gamepad API, audio through WebAudio after a user gesture. Net by WebTransport (WebSocket fallback) to the same dedicated server the native clients use. The build is hosted on GitHub Pages from the public mirror so the work PC opens a URL. Honest limits: wasm runs at roughly half to two thirds of native speed, a tab has a few GB of memory, WebGPU has no hardware ray tracing and no bindless textures, default limits allow 8 storage buffers per shader stage (row VR3 checks our shaders against that), and the path-traced rows (RT) stay native.

**Proofs run on GitHub.** wasm32-wasip1 under wasmtime runs the tape and the physics tests on a runner with no browser (WEB1, WEB3). Headless Chrome on a runner loads the build, runs the net duel against a server started in the same job (WEB4, WEB9), and takes a frame (WEB5; software WebGPU through the SwiftShader adapter flag, to be confirmed by WEB4, with the owner's GPU as the fallback witness). GPU-speed numbers need a real GPU: the RTX 3050 here, and the work PC through the Pages URL.

**Headless Chrome on a runner (WEB4, measured 2026-10-17, workflow `web-build.yml`, branch `web-build-a` of the mirror).** ubuntu-latest, Google Chrome 155.0.8059.39, no GPU. `navigator.gpu.requestAdapter()` returns the SwiftShader adapter (vendor `google`, architecture `swiftshader`, 21 features incl. `timestamp-query`, default WebGPU limits) with exactly this flag line, and the shell opens a device, clears the canvas every frame, draws its moving square (screenshot: 1280x720, 150 colours, not blank), counts frames, and comes back after `device.destroy()` (device generation 2):

`google-chrome --headless=new --no-sandbox --disable-dev-shm-usage --no-first-run --no-default-browser-check --hide-scrollbars --mute-audio --window-size=1280,720 --enable-unsafe-webgpu --enable-features=Vulkan --use-angle=swiftshader --use-webgpu-adapter=swiftshader`

What else the runs showed (all in `tools/web-browse.mjs`, `FLAG_SETS`, tried in this order; the log of each attempt names the reason):
- The runner needs the Vulkan loader and Mesa's software driver (`sudo apt-get install -y libvulkan1 mesa-vulkan-drivers`). Without them the same flags still give an adapter, but Chrome cannot create the swap-chain shared image (`Could not find SharedImageBackingFactory ... WebgpuSwapChainTexture` in the Chrome log), the canvas never presents and the device is destroyed over and over; the shell stops after three losses in ten seconds and shows the fatal page (`the device was lost again and again`).
- With the packages (run 39040803910, every set tried): `swiftshader-angle` (the line above) and `swiftshader-unsafe` (adds `--enable-unsafe-swiftshader --ignore-gpu-blocklist --disable-gpu-sandbox`) work. `--use-vulkan=swiftshader` / `--use-vulkan=native` with `--use-angle=vulkan` give an adapter but a blank canvas; `--use-webgpu-adapter=swiftshader` alone, and `--enable-unsafe-webgpu` alone (with or without `--enable-features=Vulkan`), give an adapter whose device is lost again and again. So `--use-angle=swiftshader` is the flag that makes presentation work.
- Do not use apt's `binaryen` (version 108): its `wasm-opt -Oz` output fails in every browser at load (`WebAssembly.Table.grow(): failed to grow table by 4`). The workflow pins release `version_133`; Rust 1.97 needs `--enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext` (plus multivalue, reference-types, mutable-globals) or wasm-opt rejects the module.
- Sizes: `dist/web` 237,384 bytes (limit 8 MiB): `engine_bg.wasm` 166,387 bytes (72,223 gzip), `engine.js` 68,283, `index.html` about 4 KB, before the golden check was added.
- Without WebGPU (the check removes `navigator.gpu` before the page starts) the loud message with Chrome/Edge 113, Safari 26 and Firefox 141 shows, `data-state=fatal`, the line says `"ok":false,"reason":"no-webgpu"`, nothing throws.
- To look at the page yourself: `node tools/web-build.mjs`, then any static server on localhost (`python -m http.server 8080 --bind 127.0.0.1 -d dist/web`) and open `http://localhost:8080/`. WebGPU needs https or localhost; serve `.wasm` as `application/wasm`.

## 3. Waves (what lands first)

| Wave | Rows | Proves | GPU needed |
|---|---|---|---|
| 1 (days) | WEB1 WEB2 WEB3 RZ1 VR3 AIS3 | wasm hash parity, headless crates build for wasm, rapier ragdoll under wasm, the physical layer exists with caps and budgets, our shaders fit WebGPU limits, the doctor tells an AI what is missing | no |
| 2 | WEB4 WEB6 WEB10 RZ2 RZ4 RZ5 | a browser shell with a WebGPU device and a Pages URL; content by fetch; ragdoll on knockout and visible debris in the native window; collision cooked from art | native window |
| 3 | WEB5 WEB7 WEB8 RZ3 VR1 VR2 | the arena frame in the browser, input and audio on the web, the web quality tier, limbs that come off, the lit arena and its art | yes |
| 4 | WEB9 WEB11 WEB12 WEB13 VR4 RZ6 RZ7 RZ8 RZ9 AIS1 AIS2 AIS4 (WEB14 and RZ10 are LATER) | browser multiplayer, explosions and projectiles, capes and banners, material table, physics ladder, project templates and the web verbs on the door | yes |

Order inside a wave is free; rows in different crates run side by side. RZ rows touch `crates/physfx`, `crates/game` and `crates/arena` only through events; VR rows share `crates/render/src/present/**` with the Look chain, so they queue behind LF1-LF3 and EG11-EG13 (one render writer at a time).

## 4. Hard problems

1. Rapier determinism on wasm is a claim until WEB3 prints it (`enhanced-determinism` forbids SIMD and fused multiply-add differences, which is why it can match). If wasm diverges, the authority layer is unaffected (it does not use Rapier) and physfx stays cosmetic; the cost is that Rapier cannot become an authority engine on the web.
2. A browser frame budget: 8 fighters, shadows, GTAO, bloom, TAA, 200 debris bodies, wasm physics. The web tier (WEB8) cuts render scale, shadow resolution and physfx caps first; budgets are printed in the page, not guessed.
3. WebGPU limits and features differ per browser and GPU. VR3 pins the default-limits fit; texture compression (BC on desktop, ASTC on mobile) is chosen at load (VR2).
4. Download size: the arena must start from a small first bundle (target 60 MB compressed) and stream the rest. WEB6 owns the bundle format.
5. Content realism is art, not only code: a lit engine with box walls still looks meh. VR2 replaces the kit with PBR-textured stone, sand and wood and uses the off-tick asset jobs (decision D3) to make textures; Q1 reference captures decide when it is good.
6. Corporate networks: WebTransport needs UDP and many offices block it, and TLS-inspecting proxies can break `wss`; the browser client races both (WEB12) and a failure names the blocked leg. A pinned certificate lives at most 14 days and is hashed over the full DER leaf (WEB9 regenerates it every run). Safari has WebTransport only from 26.4, so the WebSocket leg is required, not optional.
7. Browser multiplayer cannot use raw UDP or QUIC. The server speaks WebTransport (datagrams for snapshots, a stream for reliable events) and WebSocket as fallback; rollback and the snapshot codec stay as they are (`multiplayer.md`).

## 5. Donors and references

wgpu web examples and its WebGPU backend notes; wasm-bindgen and wasm-opt; Rapier's own wasm builds (`rapier.js` shows the wasm cost profile); Jolt and Box3D for ragdoll and debris budgeting (`docs/research/steals-2026-10-12/`); wtransport (server side WebTransport); Bevy's web examples for the shell shape. Licence and cite rules: owner D5 (port freely, no EVIDENCE rows).

### Scouts 2026-10-17 (six reads, notes in `docs/research/web-2026-10-17/`)

Gold, in the order it pays: (1) WEB3 gets a simd128 digest leg, because Rapier says wasm meets IEEE 754 but does not say it tests it; relaxed-simd stays off. (2) The WebGPU headless line and a canvas readback by `mapAsync` (a headless canvas screenshot comes out black), so no frame gate trusts a screenshot (WEB4, WEB5). (3) The connect race of WebTransport against WebSocket, and a certificate helper (ECDSA P-256, 14 days, SHA-256 of the DER leaf), because offices block UDP (WEB9, WEB12). (4) Pipelines compiled behind the loader (WEB13) and Babylon's fixed degrade order for the quality ladder (WEB8). (5) KTX2 plus meshopt cooks with a coarse-first bundle (VR2, VR4, WEB6). (6) Rapier motors in AccelerationBased mode for ragdoll muscles, sleep defaults and debris lifetime with variance (RZ1, RZ2). (7) CC0 texture sets and trim sheets first, then the cheap lights: hue-keeping tonemap (RL34), contact shadows (RL7), a box-projected cube for metal and stone (LF2), half-res GTAO at a lower preset on the web tier (LF3); SMAA or FXAA as the web default because TAA needs motion vectors (LF5). (8) Playwright MCP verb names for the door (AIS2). Parked on purpose: Jolt as a second backend (RZ10), threads (WEB14), Gaussian splats (no WebGPU path found), virtual shadow maps (no browser path found).

Corrections the scouts found in our own text: WebGPU default `maxUniformBufferBindingSize` is 64 KB and `maxVertexBufferArrayStride` 2048 (VR3 fixed); Godot's web export is WebGL 2 only, so it is no WebGPU precedent; Unreal has no HTML5 export; Needle and Wonderland licences are unread, so no code from them.

## 6. Quality gates

Every row: a test that fails before; the golden `d57b5c8ae3096614` and the HS2 matrix unchanged; no `std::fs`, `std::time::Instant`, `std::thread` or `std::net` reachable from a crate on the wasm list (WEB2's gate prints the line); physfx never imported by arena, combat or proof (a grep gate in row RZ1); every budget printed as a number in the receipt line the page and the window both show.

## 7. Board tasks

The rows are in `docs/SPRINT-22-web-realism.md` (the board shard), one section per prefix. Wave 1 first.
