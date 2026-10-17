# Web load and WebGPU performance: 14 techniques (QW rows)

Date: 2026-10-17. Question: how a heavy browser 3D build (engine2040: Rust to wasm32, WebGPU via wgpu, fetched asset bundle, Chrome desktop first, potato tier) gets a fast first load and a smooth frame.

Method and labels used below:
- "fetched" = a page read in this pass, with the claim quoted or measured from it.
- "scout 0N" = a claim from `docs/research/web-2026-10-17/0N-*.md`, read there, not re-fetched here.
- "search snippet" = only search result text, no page read.
- "unverified" = not confirmed by any source read.
- "repo" = a file or budget number in this checkout, read in this pass.
- Arithmetic is marked as such.

## 1. Where the repo stands

- `Cargo.toml` `[profile.web]` (lines 307-312): opt-level "z", lto true, codegen-units 1, panic "abort", inherits release, overflow-checks on. Only `tools/web-build.mjs` uses it.
- `tools/web-build.mjs`: `PROFILE = "web"` (line 20); `WASM_OPT_FLAGS` = `-Oz` plus the five `--enable-*` feature flags (line 27); SIZE_LIMIT 8 MiB for `dist/web` (line 23). It writes gzip sizes at zlib level 9 (line 155). A grep found no brotli producer in this file.
- `tools/web-budget.json`: wasm 4,250,000; gzip 2,375,000; brotli 2,020,000; first_bundle 2,040,000; mem 20,250,000; ttff_ms 8000.
- Measured in web-build run 39054155155 (budget note, repo): wasm 3,399,353; gzip 1,898,936; brotli 1,613,379; first_bundle 1,628,433; mem 16,187,392 (247 pages); ttff 4,652 ms on SwiftShader (a software adapter, not a GPU number).
- `mem` in the budget is wasm linear memory at first frame. It is not the whole tab. The WEB11 "peak under 1.5 GB" bar in scout 01 is a different number; keep both labelled.
- The budget note says the 1.6 MB studio HDRI sits inside the wasm (repo). That is the largest single known item in the 3.4 MB.
- `crates/web/index.html` line 70 calls `await init();` with no Response passed (repo). Whether wasm-bindgen's default init streams is unverified.
- `crates/web/src/shell.rs`: `requestAdapter` (line 96); `request_animation_frame` (line 231); the frame loop runs until the device is lost (line 235); a `DESTROY_REQUESTED` flag separate from `frame.lost` (lines 242-246); `recover` respawned on loss (line 250). `crates/web/src/recovery.rs` is the storm guard. `crates/web/src/limits.rs` requests `Limits::defaults()` (line 12).
- `crates/render/src`: 79 lines match `create_render_pipeline`, `create_compute_pipeline` or `create_bind_group` (grep, two directory levels, not exhaustive). The probe's READ counts for render: `pollster::block_on` 34, `device.poll(wait)` 21, `map_async` 17 (scout 07, `target/reports/wasm-probe-2026-10-17.md`).
- `crates/geo/Cargo.toml`: `lod = ["dep:meshopt"]` (line 22), meshopt 0.6.2 (line 36). The probe did not build `lod` for wasm32 (scout 07, section 2).
- The SwiftShader adapter reports features `texture-compression-bc`, `-astc`, `-etc2` (scout 07, section 3, Chrome 155).

## 2. The 14 techniques

### QW1 wasm size: sweep opt-level z, s and 3 on the web profile
- What: build the same `[profile.web]` at opt-level "z", "s" and 3 (LTO and codegen-units as now), keep the smallest raw wasm that still passes the web golden receipt. The sources say the choice must be measured.
- Source: Cargo profiles, doc.rust-lang.org/cargo/reference/profiles.html (fetched): "s" optimizes for size; "z" also turns off loop vectorization; the page warns "level 3 being slower than 2, or the s and z levels not being necessarily smaller". Rust wasm book, rustwasm.github.io/book/reference/code-size.html (fetched): "Always measure!"
- Numbers: current 3,399,353 raw / 1,898,936 gzip / 1,613,379 brotli (repo). No per-setting delta is published in the pages read. Unverified: how much "s" beats "z" for this code.
- Maps onto: `Cargo.toml` `[profile.web]`; `tools/web-build.mjs` (`PROFILE`); `tools/web-budget.json` `wasm` line only if the winner moves it.
- Cost: S.
- Row: `- [ ] QW1 web: size sweep of the web profile - build `[profile.web]` at opt-level "z", "s" and 3 with the same LTO and one codegen unit, print raw, gzip and brotli bytes per level, keep the smallest raw wasm that passes the web golden receipt. Diff: regular. Tier A. Size S. Owns the `[profile.web]` lines of `Cargo.toml`, a `--size-sweep` option in `tools/web-build.mjs` and its test. Accept: a receipt table with three rows (z, s, 3) for raw, gzip and brotli; the chosen level has the smallest raw wasm and the web golden receipt is unchanged. F2P: no sweep exists today. P2P: the native release profile and the golden hash `d57b5c8ae3096614` unchanged. needs: none.`

### QW2 wasm-opt pass and a twiggy receipt
- What: after `wasm-opt`, print `twiggy top -n 20` so the largest items are named (HDRI bytes, fmt, panic strings, the allocator); A/B `-Oz` against `-Os` and `-O3` on the same module; keep the names section only where a page needs it.
- Source: Rust wasm book (fetched): `wasm-opt` with `-Os` or `-Oz` "can often save another 15-20% on code size"; `twiggy top -n 20` names the largest contributors; the default dlmalloc allocator is "somewhere around ten kilobytes"; `wasm-snip` plus `--dce`; avoid `format!`/`to_string` and panic paths in release. wasm-opt flag names: scout 01 read the Binaryen manpage (not re-fetched).
- Numbers: 15-20% from the book (its own example, not ours). Repo: the HDRI is 1.6 MB of the 3.40 MB raw (repo budget note). Unverified: the twiggy split for our build.
- Maps onto: `tools/web-build.mjs` `WASM_OPT_FLAGS` (line 27) and the step at line 126; `crates/render` (embedded HDRI, shaders); `crates/web` (panic hook).
- Cost: S.
- Row: `- [ ] QW2 web: name the bytes - print twiggy top -n 20 for the built wasm in the web receipt and A/B -Oz against -Os and -O3 on the same module. Diff: regular. Tier A. Size S. Owns `tools/web-build.mjs` (the step after line 126 and its test) and `tools/web-build.test.mjs`. Accept: the receipt names the top 20 items with bytes; the A/B table prints three raw and gzip sizes; the chosen flag is the smallest raw size that still passes the web golden receipt. F2P: receipt has no twiggy line. P2P: `WASM_OPT_FLAGS` feature list unchanged (wasm-opt rejects a module that uses a feature it was not told about). needs: twiggy on the build machine (Cargo install or a pinned binary; unverified which one CI uses).`

### QW3 streaming compile through instantiateStreaming
- What: hand the fetch Response to `init` (or `WebAssembly.instantiateStreaming`) so the engine compiles while bytes arrive. Keep `application/wasm` on the server. Split the receipt into download ms, compile ms and first-frame ms. A wrong MIME type rejects with a TypeError, so catch it and show the fatal page with the reason.
- Source: MDN `WebAssembly.instantiateStreaming` (fetched): accepts a Response or a promise for one; the server "must" return `application/wasm`; a wrong MIME type makes the promise reject with a `TypeError`; no fallback described. V8 Liftoff post, v8.dev/blog/liftoff (fetched, undated in the read): Liftoff generates code in a single decoding pass, "which, combined with WebAssembly's streaming APIs" lets V8 compile while the module downloads; on desktop, eager tier-up starts TurboFan after Liftoff finishes; Liftoff code ran "around 50% slower" on a Unity test (about 70% on the MacBook) and 18-54% slower on PSPDFKit; on EpicZenGarden TurboFan finished "several more seconds" after Liftoff. The page does not name the streaming API calls; the MDN page does.
- Numbers: compile speed is not quantified in the post (charts only). Arithmetic: a 1.6 MB download on a 10 Mbit link is about 1.3 s of transfer (unverified link speed), so compile hidden under download can save the compile time that follows.
- Maps onto: `crates/web/index.html` line 70 (`await init()`); `crates/web/src/shell.rs` (fatal page, `set-text fatal-reason`); `tools/web-build.mjs` (writes `engine.js` and `engine_bg.wasm`); `tools/web-budget.json` `ttff_ms` 8000.
- Cost: S.
- Row: `- [ ] QW3 web: stream the module into compile - pass the fetch Response to init so the browser compiles while the wasm downloads; split the web receipt into download ms, compile ms and first-frame ms; a wrong MIME type shows the fatal page with the reason. Diff: regular. Tier A. Size S. Owns `crates/web/index.html` (the init line), `crates/web/src/shell.rs` (fatal path), `crates/web/src/report.rs` (receipt fields). Accept: the receipt prints three numbers; a test server sending text/plain shows the fatal reason line, not a blank page; ttff stays under 8000 ms on the SwiftShader run. F2P: the receipt has one ttff number today. P2P: first frame unchanged in the web golden receipt. needs: check whether wasm-bindgen's default init already streams (unverified; the row decides).`

### QW4 coarse-first bundle and the HDRI out of the wasm
- What: split the content bundle into a first bundle (what frame 1 needs: kit at coarse LOD, fighters at low LOD, the procedural fallback) and later blobs (finer LOD, full-res textures). Order blobs coarse first. Move the 1.6 MB studio HDRI out of the wasm and into the fetched bundle. Show progress from the response stream.
- Source: PlayCanvas engine v2.23.2 release notes (scout 02, read there): "load the lowest splat LOD across the whole view first, then refine". MDN Response.body progress (scout 01): `getReader()` chunks drive a bar; percent only with Content-Length (not re-fetched). Repo: the bundle APIs take bytes (`ArchivedBundle::open(Arc<[u8]>)`, `BundleLoader::load(&[u8])`), per scout 07, section 4.
- Numbers: first_bundle now 1,628,433 bytes against a 2,040,000 budget (repo). Arithmetic: if the 1.6 MB HDRI leaves the wasm, raw wasm falls from 3.40 MB to about 1.8 MB (assumes the embedded HDRI is the whole 1.6 MB; unverified with twiggy, QW2).
- Maps onto: `crates/assets` (bundle open and load); `crates/render` (the embedded studio HDRI and where it is included); `tools/web-build.mjs` (the file list and order the page loads); `tools/web-budget.json` `first_bundle`.
- Cost: M.
- Row: `- [ ] QW4 web: coarse first, HDRI out of the wasm - move the 1.6 MB studio HDRI into the fetched bundle, order bundle blobs coarse LOD first, print the first-bundle bytes and a progress bar from the response reader. Diff: regular. Tier A. Size M. Owns `crates/render` (the HDRI include and its load path), `crates/assets` bundle order, `tools/web-build.mjs` file order and `tools/web-build.test.mjs`. Accept: raw wasm drops by at least the HDRI size printed by twiggy; first_bundle stays under 2,040,000; the arena's first frame renders the same hash with the HDRI from the bundle. F2P: HDRI is inside the wasm today. P2P: native build still loads the HDRI from disk; golden unchanged. needs: QW2 (names the bytes) first.`

### QW5 pick texture formats at load (Basis or KTX2, transcode to the device)
- What: cook kit textures once to Basis Universal (KTX2 container) and transcode at load to the device's compressed format. Choose by the adapter's texture-compression features, not by guess.
- Source: PlayCanvas texture compression manual, developer.playcanvas.com/user-manual/optimization/texture-compression/ (fetched): a 512x512 JPG of 202 KB "to as little as 46KB using the Default quality setting and ETC Mode", and 1.05 MB of VRAM with mip levels; transcode formats "ASTC, DXT, ETC2, ETC, PVR and ATC (selected in that order where available)"; non-square or non-power-of-two images cannot transcode to PVR and fall back to 16-bit 565. The page names DXT, not BC, and does not name WebGPU feature strings (unverified mapping). Repo: the probe's adapter lists bc, astc and etc2 (scout 07, section 3). Scout 02 gives the BC/ETC2/ASTC optional WebGPU feature point (not re-fetched).
- Numbers: 202 KB to 46 KB, about 4.4x on one texture (arithmetic from the manual); kit texture totals are not measured.
- Maps onto: `crates/assets` (cook step); `crates/render` (texture upload path, format choice; wgpu's KTX2 path unverified); `tools/web-build.mjs` (bundle contents); `tools/web-budget.json` `first_bundle`; the adapter-features line already in `crates/web/src/limits.rs` (`feature_names`).
- Cost: M.
- Row: `- [ ] QW5 web: kit textures in KTX2 for the browser - cook the kit to KTX2 with Basis for the web bundle only and transcode at load to the first compressed format the adapter reports (astc, bc, etc2, else RGBA); print the chosen format. Diff: regular. Tier A. Size M. Owns the texture cook step in `crates/assets`, the transcode path in `crates/render`, and the web bundle list in `tools/web-build.mjs`. Accept: the first_bundle bytes for kit textures fall by the printed ratio; the receipt names the format chosen; forcing a feature off picks the next format and still renders a non-blank frame. F2P: web bundle holds PNG or raw textures today (unverified; check the cook). P2P: native textures unchanged. needs: a KTX2/Basis crate choice (unverified; not searched in this pass).`

### QW6 meshopt compression and LOD chains for the kit
- What: store kit vertex and index buffers with meshopt compression (decoded at load); build LOD chains with meshopt simplify, keeping normals and UV seams with the attribute-aware variant.
- Source: meshoptimizer, github.com/zeux/meshoptimizer (fetched): vertex and index decoders "3-6 GB/s on modern desktop CPUs"; meshlet decoder "7-10 GB/s"; `meshopt_simplifyWithAttributes` weights normals, UVs and colours; `meshopt_simplifySloppy` ignores topology; the LOD error is converted with `meshopt_simplifyScale`; MIT; "the library source is C++" (a C++ compiler is needed to build).
- Numbers: decode speed above (desktop CPU, not a browser). Repo: no wasm build of the `lod` feature yet (scout 07, section 2: `lod` not probed because it needs a C/C++ compiler for wasm). This is a blocker, not an optimization.
- Maps onto: `crates/geo/Cargo.toml` (line 22 `lod`, line 36 meshopt); `crates/assets` (decode at load); `tools/web-build.mjs` (the toolchain check, which must know a wasm C/C++ sysroot).
- Cost: M.
- Row: `- [ ] QW6 web: kit meshes compressed and in LOD chains - build geo `lod` for wasm32-unknown-unknown (pinned clang and sysroot in CI), cook kit meshes with meshopt compression and simplified LOD levels, decode in assets at load. Diff: regular. Tier A. Size M. Owns `crates/geo/Cargo.toml` (the lod line and any build script), the cook in `crates/assets`, and the toolchain line in `tools/web-build.mjs`. Accept: `cargo check -p engine-geo --features lod --target wasm32-unknown-unknown` passes; decoded vertex and index bytes equal native for the same cook (a hash line in the receipt); bundle bytes for kit meshes fall by the printed ratio. F2P: the lod feature does not build for wasm32 today (probe). P2P: native lod output unchanged. needs: a wasm C/C++ toolchain on the runner (unverified which one).`

### QW7 build every pipeline behind the loader
- What: create every render and compute pipeline the arena uses while the loader shows, with async creation where the API allows; print the count and the ms; after the loader closes the counter must stay at zero, so no first-hit compile happens mid-fight.
- Source: MDN `GPUDevice.createRenderPipelineAsync` (fetched): "prevents blocking of GPU operation execution on pipeline compilation"; secure context only (HTTPS or localhost); rejects with `GPUPipelineError` whose `reason` is "internal" or "validation". Scout 02 cites three.js `compileAsync` and Godot's stutter guide (not re-fetched).
- Numbers: no compile time is given in the MDN page. Repo: 79 matching create lines in `crates/render/src` (grep, not exhaustive). Unverified: wgpu's exposure of the async call on the web target.
- Maps onto: `crates/render` (pipeline construction sites, the present path); `crates/web/src/shell.rs` (loader and first frame); `crates/web/src/report.rs` (the count and ms line).
- Cost: M.
- Row: `- [ ] QW7 web: pipelines before frame 1 - create every render and compute pipeline the arena needs behind the loader, print count and ms, and assert no pipeline is created after the loader closes. Diff: regular. Tier A. Size M. Owns the pipeline construction in `crates/render` (the present and post paths) and `crates/web/src/report.rs`. Accept: the receipt prints the count and ms before the first frame; a counter of pipelines created after the first frame is 0 over a 600-frame run; native frame hash unchanged. F2P: no warm-up exists today. P2P: golden unchanged. needs: check whether wgpu exposes an async pipeline call on wasm32 (unverified here).`

### QW8 device loss: keep the storm guard, add the reason and a forced-loss test
- What: distinguish a requested `destroy()` (no retry) from a real loss (new adapter, rebuild every GPU object from the cooked data); record the loss reason in the receipt; add a forced-loss path to the arena test page.
- Source: MDN `GPUDevice.lost` (fetched): the promise "stays pending for the device's lifetime" and resolves with `GPUDeviceLostInfo`; devices can be lost "at any time" (driver updates, browser resource management); `reason === "destroyed"` means the app called `destroy()` and should not retry; recreate all resources with the new device; if the adapter is gone, it "will only return already-lost devices". The `"unknown"` reason value is not in the text read (unverified).
- Numbers: the storm guard gives up after three losses in ten seconds (repo: `crates/web/src/recovery.rs` and the WEB5 board text).
- Maps onto: `crates/web/src/shell.rs` (lines 235-250: `DESTROY_REQUESTED`, `frame.lost`, `recover`); `crates/web/src/recovery.rs` (storm guard tests); `crates/web/src/report.rs` (reason line).
- Cost: S.
- Row: `- [ ] QW8 web: device loss, proved - print the GPUDeviceLostInfo reason in the web receipt, keep the requested-destroy path separate from a real loss, and add a forced-loss path that rebuilds the arena and keeps the frame counter moving. Diff: regular. Tier A. Size S. Owns `crates/web/src/shell.rs` (the loss branch), `crates/web/src/report.rs` and `crates/web/src/recovery.rs` tests. Accept: a forced loss rebuilds and the frame counter increases after it; a requested destroy does not retry; three losses in ten seconds show the fatal page with the reason. F2P: the receipt has no reason field today. P2P: existing recovery.rs tests still pass. needs: none.`

### QW9 uploads and readbacks: writeBuffer, a 2-3 buffer ring, and no blocking map
- What: small per-frame uploads with `writeBuffer`; data written once with `mappedAtCreation`; frame readbacks (the WEB5 capture hash) through a ring of 2-3 staging buffers with `mapAsync` and a frame-later read. No `block_on` and no `device.poll(wait)` on the browser path.
- Source: toji.dev "WebGPU Buffer Uploads" (fetched; page dated Feb 3, 2023): "the `writeBuffer()` method is always a safe fallback that doesn't have many downsides"; `mappedAtCreation` "Only works for newly created buffers"; a staging ring "typically results in a list of 2-3 staging buffers that are cycled through". The page does not say whether `mapAsync` stalls (unverified). Scout 02 and 07 give the same blocking concern.
- Numbers: ring of 2-3 (page). Repo (probe READ): 34 `pollster::block_on`, 21 `device.poll(wait)`, 17 `map_async` in render. A `block_on` on the browser main thread never returns (scout 07).
- Maps onto: `crates/render` (the presenter capture, `request_capture` and `take_capture` per the WEB5 landing; the 17 `map_async` sites); `crates/web/src/lib.rs` (hash of the readback); `tools/web-browse.mjs` (reads the bytes).
- Cost: M.
- Row: `- [ ] QW9 web: no blocking GPU calls - replace pollster::block_on, device.poll(wait) and synchronous map reads on the wasm path with writeBuffer for per-frame data and a 3-buffer readback ring read one frame later; the capture hash comes from the ring. Diff: regular. Tier A. Size M. Owns the wasm32 path in `crates/render` (presenter capture and the map_async sites), the capture consumer in `crates/web/src/lib.rs`. Accept: `tools/wasm-gate.mjs` (WEB2) reports zero `pollster::block_on` and zero `device.poll(` with a wait on the wasm path; the capture hash equals the one from the ring; the arena frame still matches the WEB5 hash. F2P: 34 block_on sites today (probe). P2P: native capture and golden unchanged. needs: WEB2's wasm-gate in place.`

### QW10 render bundles for static draws
- What: record the static arena draws (kit walls, floor, torches, static prop meshes, shadow casters that do not move) into one `GPURenderBundle`, replay with `executeBundles` each frame; per-fighter draws stay outside the bundle.
- Source: MDN `GPURenderBundleEncoder` (fetched): record once with `finish()`, replay with `executeBundles()`; benefit is "reducing JavaScript draw call overhead" ("significantly", no figure given); not recorded: occlusion queries, viewport, scissor rect, blend constant, stencil reference (inherited from the pass); bound state is cleared before and after bundle execution; "Limited availability", not Baseline; secure context. Scout 02 cites Babylon and three.js paths (not re-fetched).
- Numbers: none in the source. Unverified: any draw-call saving on our arena. The receipt must count draw calls first.
- Maps onto: `crates/render` forward and shadow passes (`present/**`, which the VR rows also touch: queue behind LF1-LF3 per the plan); `crates/web/src/arena_view.rs`; wgpu bundle API (unverified).
- Cost: M.
- Row: `- [ ] QW10 web: static draws in a render bundle - count draw calls and CPU ms per frame in the web receipt, then record the static arena draws into a render bundle and replay it each frame, only if the count shows the CPU cost. Diff: regular. Tier A. Size M. Owns the static draw list in `crates/render/src/present/**` and `crates/web/src/arena_view.rs`. Accept: the receipt prints draw calls and CPU ms per frame with bundles on and off in the same run; the pixel hash is equal; merge only if CPU ms falls by the printed amount. F2P: no bundle path today. P2P: native frames and golden unchanged. needs: the LF1-LF3 render writer finished (one writer on `present/**`).`

### QW11 frame pacing: fixed steps from the rAF timestamp, and the hidden-tab rules
- What: drive the fixed 60 Hz sim from an accumulator fed by the `requestAnimationFrame` timestamp, capped per frame, not from setTimeout and not from a per-frame constant; decide the hidden-tab rule (rAF pauses, so a single-player sim pauses; a net host keeps ticking on the server).
- Source: MDN `requestAnimationFrame` (fetched): callbacks "will generally match the display refresh rate" (60, 75, 120, 144 Hz); "paused in most browsers when running in background tabs or hidden iframes"; the timestamp must be used "otherwise, the animation will run faster on high refresh-rate screens". Chrome timer throttling, developer.chrome.com/blog/timer-throttling-in-chrome-88 (fetched): a chained setTimeout at chain count 5 or more, hidden over 5 minutes and silent 30 seconds, is checked "once per minute"; the 4 ms floor is unchanged; audio in the last 30 s or WebRTC exempts the page.
- Numbers: 60 Hz is 16.7 ms per frame (arithmetic); throttled timers once per minute (Chrome blog). Repo: `shell.rs` line 231 uses `request_animation_frame`; line 235 runs the loop until loss. Unverified: whether that loop reads the timestamp argument.
- Maps onto: `crates/web/src/shell.rs` (the loop); the fixed-step tick in `crates/game` and `crates/arena` (RZ2 note: the 60 Hz step); `crates/net` (server tick rule for the browser host).
- Cost: S.
- Row: `- [ ] QW11 web: fixed steps from the frame clock - feed the 60 Hz sim accumulator from the requestAnimationFrame timestamp with a cap per frame, and document that a hidden tab pauses a single-player sim; print steps per frame in the receipt. Diff: regular. Tier A. Size S. Owns `crates/web/src/shell.rs` (the loop) and a pure accumulator in `crates/web/src/anim.rs` with its tests. Accept: a fake-clock test gives exactly 60 steps per second of timestamp at 144 Hz and 60 Hz input; the receipt prints max steps per frame; a hidden-tab test (no rAF) produces no steps. F2P: the loop is not driven by a fixed-step accumulator (check; unverified). P2P: golden tape unchanged. needs: none.`

### QW12 cache the bundle by its hash, and ask for persistence
- What: keep the fetched bundle in the Cache API under a key that is the build's SHA-256 (already in `build.json`), so a repeat visit reads it locally; ask `navigator.storage.persist()`; treat `QuotaExceededError` as "stream without cache".
- Source: MDN storage quotas (fetched): Chromium lets an origin store "up to 60% of the total disk size" with a browser-wide "at most 80%"; best-effort data is evicted least-recently-used under pressure; `navigator.storage.persist()` is opt-in, and Chromium "automatically approve or deny[s]" without a prompt; Cache and IndexedDB over quota fail with `QuotaExceededError`; an evicted origin loses all its data together.
- Numbers: first_bundle 1,628,433 bytes (repo) is about 1.6 MB per build on disk (arithmetic). Quotas are a share of total disk, so a 1.6 MB object is far below them (arithmetic from the quota rule, not a measured case).
- Maps onto: `tools/web-build.mjs` (`build.json` per-file `sha256`, line ~155); `crates/web` (fetch path, new small cache module); `tools/web-budget.json` `first_bundle`.
- Cost: S.
- Row: `- [ ] QW12 web: the bundle is cached by hash - store the fetched first bundle in the Cache API keyed by its sha256 from build.json, call navigator.storage.persist(), and fall back to streaming on QuotaExceededError. Diff: regular. Tier A. Size S. Owns `crates/web` (a cache module and its call in the fetch path) and the `build.json` hash line in `tools/web-build.mjs`. Accept: a second load in the same profile reports bytes fetched 0 and `bundle=cache` in the receipt; with the cache forced to throw QuotaExceededError, the page still loads and says so. F2P: every load downloads the bundle today (the fetch path has no cache). P2P: first load byte-identical. needs: none.`

### QW13 Pages: check what the server actually sends
- What: the WEB10 deploy checks, does not assume: the wasm MIME type, the content encoding actually served (gzip or brotli), and the size; the brotli budget is kept only if the server sends brotli.
- Source: GitHub Pages limits (fetched): a published site "may be no larger than 1 GB"; a "soft bandwidth limit of 100 GB per month"; "soft limit of 10 builds per hour", not applied to a custom Actions workflow; the page says nothing about headers, MIME types or compression. Search snippet (2020 and 2022 project notes): GitHub Pages did not serve pre-compressed `.br` files (unverified for today). Scout 05 (read): GitHub Pages cannot set COOP/COEP headers.
- Numbers: wasm brotli 1,613,379 vs gzip 1,898,936 (repo): brotli is 15.0% smaller than gzip (arithmetic). Site limit 1 GB (page) against dist/web 8 MiB (repo). Bandwidth soft limit 100 GB per month (page).
- Maps onto: `tools/web-build.mjs` (gzip at zlib level 9; no brotli producer found); `tools/web-budget.json` `brotli` line; `crates/web/index.html`; the WEB10 Pages workflow (not yet on the board as landed).
- Cost: S.
- Row: `- [ ] QW13 web: the server's real headers in the receipt - after the Pages deploy, curl the wasm and the bundle and write content-type, content-encoding and content-length into the web receipt; keep the brotli budget only if brotli is served. Diff: regular. Tier A. Size S. Owns the deploy check step in the Pages workflow and the receipt line in `tools/web-build.mjs`. Accept: the receipt shows `application/wasm` for `engine_bg.wasm` and the content-encoding the server returns; a run with no application/wasm fails the deploy check. F2P: no header check exists. P2P: deploy unchanged for the page. needs: the deployed Pages URL (owner enables Pages once, per scout 07).`

### QW14 memory: print pages, grow safely, budget from the printed peak
- What: print linear memory pages at first frame and every 60 s in a 10-minute soak; refresh every typed-array view after `memory.grow`; set the `mem` budget from the printed peak, with a reason in the commit (the budget note rule).
- Source: MDN `WebAssembly.Memory` (fetched): memory grows in "64KiB" pages; `initial` and `maximum` are page counts; `memory.grow()` "Detaches the previous buffer", so old views are invalid. The 32-bit ceiling is 4 GiB = 65,536 pages (arithmetic; the page does not state it). Memory64 is in search snippets only, with conflicting dates (unverified; not used).
- Numbers: 16,187,392 bytes (247 pages) at first frame (repo budget note), against `mem` 20,250,000. Scout 01's "peak under 1.5 GB" is an external-to-page target, not measured.
- Maps onto: `crates/web` (any cached `Float32Array` or `Uint8Array` views after grow); `tools/web-budget.json` `mem`; `crates/render` (GPU buffers are not wasm memory, so VRAM is not in this number).
- Cost: S.
- Row: `- [ ] QW14 web: memory, printed and bounded - print wasm linear memory pages at first frame and every 60 s in a 10-minute arena soak, re-create typed-array views after memory.grow, and set the mem budget from the printed peak with a reason. Diff: regular. Tier A. Size S. Owns `crates/web/src/report.rs`, any cached view in `crates/web` (re-create on grow), and `tools/web-budget.json` only with a reason. Accept: the soak prints the pages every 60 s; a forced grow leaves the frame hash unchanged; the mem line in the budget equals the printed peak plus the 25% headroom the note describes. F2P: receipt prints first-frame memory only. P2P: first-frame memory unchanged. needs: none.`

## 3. Precedents (what is verified and what is not)

- PlayCanvas: the KTX2/Basis manual figures in QW5 are fetched. Its coarse-first splat loading is scout 02 (release notes read, not re-fetched).
- Babylon.js: the ordered quality ladder (merge meshes, shadows, post, texture size, hardware scaling last) is scout 02 (its SceneOptimizer page read there). Used in WEB8 and RZ9, not repeated here.
- Godot: its web export is WebGL 2 only in the docs read (scout 02). It is not a WebGPU precedent.
- Unity: recommends Brotli over gzip (scout 02, secondary). Not re-verified.
- Bevy: no official wasm size numbers found in one search. A jam project sets opt-level "s" with a note to try "z" (search snippet). Not a measured precedent.
- Our own numbers (the repo budget note, SwiftShader, one run) are the only measured first-load numbers in this file. No GPU load numbers exist yet. The work PC is the first real GPU sample (scout 07 and the plan).

## 4. Checked and not picked (for the next pass)

- Threads and COOP/COEP: GitHub Pages cannot set headers (scout 05); coi-serviceworker reloads on the first visit (scout 05, search). Keep single-thread (WEB10 path). Stays parked (WEB14).
- Splitting the wasm into lazy modules: Binaryen has a `wasm-split` tool (search snippet, dokk manpage); the wasm-bindgen integration is an open issue (search snippet). Unverified and not fetched. Park until QW1 and QW4 show what is left.
- GPU timestamps: the Chrome developer-features page (fetched) says timestamp queries are "quantized with a resolution of 100 microseconds", and the quantization "is automatically disabled" when the "WebGPU Developer Features" flag is on, with the warning that this "may raise privacy concerns". Use the flag only on the dev PC, never in the shipped page. A link to cross-origin isolation was searched and not found (unverified).
- Memory64: conflicting search snippets; not used.

## 5. Unverified items (do not build on these until a row checks them)

- wgpu's async pipeline call on the wasm target (QW7).
- Whether wasm-bindgen's default `init()` already streams (QW3).
- The `"unknown"` device-loss reason value (QW8).
- Whether `mapAsync` stalls on Chrome (QW9; the toji page does not say).
- Whether the current Pages deploy serves brotli or sends `application/wasm` (QW13; the 2020 and 2022 notes are old).
- Twiggy's breakdown of our wasm (QW2, QW4).
- Any draw-call saving from render bundles on our arena (QW10).
- The GPU cost of any technique here: every number above is either a source's own machine or the SwiftShader software adapter (repo). The RTX 3050 and the work PC are the real sample.
