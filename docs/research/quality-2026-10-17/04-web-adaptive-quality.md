# Web adaptive quality: 11 techniques (QT rows)

Date: 2026-10-17. Question: how shipping web and WebGPU games and engines pick a quality tier on weak and strong GPUs and adapt it while running, and what that means for the wgpu web build (`crates/render/src/quality.rs`, tiers potato < web < comfort < high).

Labels used below:
- "fetched" = page or file read in this pass.
- "snippet" = search result text only, page not opened.
- "repo" = read in this checkout.
- "unverified" = not confirmed by any source read.
- "proposal" = our own number, not sourced.

## 1. Where the repo stands

- `crates/render/src/quality.rs`: `choose_web_tier(query, max_texture_2d)` takes `?tier=` first, then the limit floor `WEB_MIN_TEXTURE_2D` = 4096 (below it: potato), else web. No timing input. A grep of `crates/web/src` found no startup fps probe yet.
- `WEB_CUT_ORDER` (quality.rs): merge_draws, shadows, post_and_particles, texture_size, render_targets, render_scale.
- `crates/web/src/tier_view.rs`: `WINDOW_MS` = 1000, mean frame ms per window; `MISSES_BEFORE_CUT` = 2 windows in a row over `budget_ms` takes the next cut.
- Restore path: a grep for restore/recover/raise in `tier_view.rs` and `arena.rs` found none. The ladder only goes down.
- `crates/web/src/arena.rs` lines 208-210: a hidden tab skips the window (`document().hidden()`).
- WEB13 pipeline warm-up is ticked (`docs/SPRINT-22-web-realism.md` line 19), so a probe can start after compiles are done.
- WEB8 landing note (`docs/SPRINT-22-web-realism.md` line 14): only render scale and bloom follow the tier; `texture_max_px` and the other knobs print but draw nothing yet.
- Target GPUs: RTX 3050 and Intel UHD iGPU.

## 2. The 11 techniques

### QT1 Startup probe that the 60 Hz cap cannot hide
- What: after warm-up, draw a fixed workload for a fixed number of frames at the candidate scale, drop the first frames, take the median. A plain frame-time probe cannot separate a 3050 from an iGPU when both keep up with the display, because callbacks "will generally match the display refresh rate" (MDN, fetched). Add work per frame until the median is off the cap, or use GPU timestamp queries if the device offers them (availability unverified here).
- Source: detect-gpu, https://github.com/pmndrs/detect-gpu (README read at https://cdn.jsdelivr.net/npm/@pmndrs/detect-gpu@6.0.15/README.md, fetched). PlayCanvas names the same two options, a startup benchmark or a GPU-name list (snippet, https://developer.playcanvas.com/user-manual/optimization/runtime-devicepixelratio/).
- Numbers: detect-gpu tiers at 15, 30 and 60 fps (defaults `[0, 15, 30, 60]`), tier 0 below 15 fps or with no WebGL (fetched). Its data is from gfxbench.com and "stopped updating in December 2025" (fetched). Babylon SceneOptimizer target is 60 fps (fetched). Proposal: median 16.7 ms or less picks comfort, 33.3 ms or less picks web, else potato. These are detect-gpu's fps lines as frame times, not our measurements.
- Maps onto: `choose_web_tier` gains a probe argument and keeps the limit floor as its first test; `arena.rs` `pick_tier` runs the probe after the warm-up.
- Cost: M.
- Row: `- [ ] QT1 web: startup probe - after the WEB13 warm-up, draw a fixed workload at the Web scale for 120 frames, drop the first 30, take the median frame ms; 16.7 ms or less picks comfort, 33.3 ms or less picks web, else potato; the workload is sized so the median is off the 60 Hz cap. Diff: regular. Tier A. Size M. Owns the probe in `crates/web/src/arena.rs` (pick_tier), `choose_web_tier` in `crates/render/src/quality.rs` and their tests. Accept: \`cargo test -p engine-render --lib quality\` (probe picks the tier, the limit floor still caps it, ?tier= still wins); the web receipt prints \`probe_ms=\` and \`tier=\`. F2P: no probe, tier from limits only. P2P: golden d57b5c8ae3096614 unchanged. needs: WEB13 (landed).`

### QT2 Probe waits for the compiles
- What: a probe that overlaps a shader or pipeline compile reads the compile as slow GPU work. Start timing only after the warm-up and a few settled frames.
- Source: Babylon SceneOptimizer runs its checks after `scene.executeWhenReady` (fetched, source at https://raw.githubusercontent.com/BabylonJS/Babylon.js/master/packages/dev/core/src/Misc/sceneOptimizer.ts). Repo: WEB13 landed (`docs/SPRINT-22-web-realism.md` line 19).
- Numbers: none sourced. Proposal: discard the first 30 frames after the warm-up, and repeat the probe once if any `late_compile` line appears during it.
- Maps onto: `arena.rs` ordering (warm-up receipt, then probe); `tier_view.rs` window reset.
- Cost: S.
- Row: `- [ ] QT2 web: probe waits for the compiles - start the QT1 probe only after the warm-up receipt and 30 settled frames; a probe window with any late_compile is repeated once; the receipt prints late_compile=<n> before probe_ms. Diff: regular. Tier A. Size S. Owns \`crates/web/src/arena.rs\` (probe start) and the window reset in \`crates/web/src/tier_view.rs\`. Accept: a fake-clock test where a late compile repeats the probe once; the receipt line shows late_compile=0 before probe_ms. F2P: probe starts at frame 1. P2P: golden d57b5c8ae3096614 unchanged. needs: QT1.`

### QT3 Adapter name is a hint, never the tier
- What: WebGPU adapter identifiers should be "as broad as possible while still being useful", and a browser may report only part or none of them. Brave 1.93 empties WebGPU adapter descriptors (snippet). A name table therefore cannot be the tier key on WebGPU. Keep it for the denylist (QT10).
- Source: WebGPU spec section 2.2.6 "Adapter Identifiers", https://www.w3.org/TR/2026/CRD-webgpu-20260915/ (Candidate Recommendation Draft dated 15 September 2026, fetched). Brave: https://brave.com/privacy-updates/38-webgl-webgpu-fingerprinting-protections (snippet).
- Numbers: none beyond the quotes.
- Maps onto: `arena.rs` `pick_tier` reads limits only today; `crates/web/src/limits.rs` requests `Limits::defaults()` (repo).
- Cost: S.
- Row: `- [ ] QT3 web: adapter info as a hint - read the adapter vendor, architecture and description when the browser gives them and print adapter=<text or none> in the receipt; the tier never reads them. Diff: regular. Tier A. Size S. Owns \`crates/web/src/limits.rs\` (adapter info read) and the receipt line in \`crates/web/src/arena.rs\`. Accept: receipt prints adapter=none when the browser blanks the info; a unit test that \`choose_web_tier\` takes no adapter-name argument. F2P: no adapter line. P2P: golden d57b5c8ae3096614 unchanged. needs: none.`

### QT4 Limits can only lower the tier
- What: browsers may report limits in buckets, not real values, so a limit can rule a GPU out but cannot prove a GPU is strong.
- Source: WebGPU spec section 2.2.1 "Machine-specific features and limits" (fetched): "User agents are not obligated to expose the real hardware limits", and suggests binning. Sections 2.2 and 3.6 (fetched excerpt): a user agent "must not reveal more than 32 distinguishable configurations or buckets". Snippet only: a mirror says a real 16384 may report 8192 (tiers 2048, 8192, 32768); not checked against the spec text.
- Numbers: 32 buckets maximum (fetched). `WEB_MIN_TEXTURE_2D` = 4096 (repo).
- Maps onto: the limit branch of `choose_web_tier` stays a floor. Its test `quality_web_pick_reads_the_query_first_then_the_limits` (quality.rs, line ~482) pins it.
- Cost: S.
- Row: `- [ ] QT4 web: limits only lower the tier - a probe result never raises the tier above what the limit floor allows, and the doc comment on choose_web_tier says limits are bucketed. Diff: regular. Tier A. Size S. Owns \`crates/render/src/quality.rs\` (choose_web_tier doc and test). Accept: \`cargo test -p engine-render --lib quality\` covers a probe-above-floor and a probe-below-floor case. F2P: no probe-vs-floor test. P2P: golden d57b5c8ae3096614 unchanged. needs: QT1.`

### QT5 A ladder that comes back, with hysteresis
- What: Babylon's SceneOptimizer only steps down (levels rise, never fall, except a hardware-scale reversal rule). A Babylon forum user reported that hardware scaling never came back in improvement mode (snippet). Unreal uses a dead band and a minimum gap between changes. A three.js user reported a loop: low fps lowers the ratio, the higher fps restores it, and the cycle repeats, with black frames on switching; the advice in that thread is separate up and down thresholds plus a cooldown (snippet, https://discourse.threejs.org/t/changing-pixelratio-based-on-fps-good-or-bad-idea/34563).
- Source: Babylon SceneOptimizer source (fetched). Unreal dynamic resolution, https://dev.epicgames.com/documentation/en-us/unreal-engine/dynamic-resolution-in-unreal-engine (fetched).
- Numbers (fetched): Babylon target 60 fps, one rounded fps reading per check, checks 2000 ms apart (`trackerDuration`). Moderate preset priorities: merge meshes, shadows, lens flares at 0; post processes and particles at 1; texture (512) at 2; render targets at 3; hardware scaling (maximumScale 2) at 4. Unreal: `r.DynamicRes.FrameTimeBudget` 33.3 ms; `MinScreenPercentage` 50; `MaxScreenPercentage` 100; the dead band (`ChangePercentageThreshold`), the minimum frames between changes (`MinResolutionChangePeriod`) and the panic count (`MaxConsecutiveOverbudgetGPUFrameCount`) are named but their values are not listed on the page. Repo: 2 one-second windows, mean not percentile, no restore.
- Proposal: keep the 2-window cut; restore one cut after 10 s of mean frame ms under 80 percent of budget; a restore that misses twice within 30 s is not tried again in that run.
- Maps onto: `tier_view.rs` `Ladder` (add restore state). `WEB_CUT_ORDER` stays.
- Cost: M.
- Row: `- [ ] QT5 web: the ladder comes back - keep the 2-window cut; restore one cut after 10 seconds under 80 percent of the frame budget; a restore that misses twice within 30 seconds is not tried again in that run; the receipt prints restore=<n>. Diff: regular. Tier A. Size M. Owns \`crates/web/src/tier_view.rs\` (Ladder), its unit tests and the cut print in \`crates/web/src/arena.rs\`. Accept: fake-clock unit tests: a cut then 10 s of headroom restores one step; a miss after restore reverts it. F2P: the ladder never restores. P2P: golden d57b5c8ae3096614 unchanged; the WEB_CUT_ORDER test unchanged. needs: QT1.`

### QT6 Scale floor and pixel-ratio cap
- What: the scale changes only the main pass; Unreal says materials must not read the screen percentage, and keeps the secondary scale fixed for TAA history. PlayCanvas caps `maxPixelRatio` at 1 on lower-tier devices (snippet). A forum report says on a phone with ratio 3, values below 2 looked blurry (snippet). three.js users use fractional ratios such as 1.1 and 1.5 (snippet, https://sbcode.net/tsl/adaptive-dpr).
- Source: Unreal (fetched, as QT5). PlayCanvas runtime device pixel ratio page (snippet). Forum thread https://forum.playcanvas.com/t/how-could-we-manage-device-pixel-ratio-for-the-low-end-vs-high-end-devices/29380 (snippet).
- Numbers: Unreal floor 50, cap 100 (fetched). Repo: potato scale 67 (`knobs`). Proposal: floor 50 percent on the web build, backing size never above CSS box times devicePixelRatio.
- Maps onto: `WEB_CUT_ORDER` last step (render_scale); `arena.rs` `scale_after_cut` and the canvas size.
- Cost: S.
- Row: `- [ ] QT6 web: scale floor and DPR cap - the render_scale cut stops at 50 percent, and the canvas backing size never exceeds the CSS box times devicePixelRatio; the receipt prints scale=<pct> and dpr=<x>. Diff: regular. Tier A. Size S. Owns \`crates/web/src/arena.rs\` (scale_after_cut, canvas size). Accept: a test that repeated render_scale cuts stop at 50; a test at dpr 1 and dpr 3 that the backing size is the scale times the CSS box and never above it. F2P: the scale cut has no floor. P2P: golden d57b5c8ae3096614 unchanged. needs: QT5.`

### QT7 Hidden tab and tab return
- What: rAF callbacks "are paused in most browsers when running in background tabs or hidden iframes" (MDN). Chrome throttles background timers to once per second, starts a budget after 10 seconds in the background, and exempts audible audio and WebSocket or WebRTC from the budget (Chrome 57 post). The first frame after a long hidden spell spans the whole gap, so it must not count as a miss.
- Source: MDN https://developer.mozilla.org/en-US/docs/Web/API/Window/requestAnimationFrame (fetched). Chrome https://developer.chrome.com/blog/background_tabs (fetched, last updated 2017-03-09). IndexedDB exemption: snippet only, not on the Chrome page; unverified.
- Numbers (fetched, 2017 post): budget starts after 10 s; regenerates 0.01 s per second; timers once per second. Current Chrome may differ.
- Maps onto: `arena.rs` lines 208-210 (hidden skip) and the rAF loop in `crates/web/src/shell.rs` (line ~231); the QT1 probe must abort on hide and restart on show.
- Cost: S.
- Row: `- [ ] QT7 web: hidden gap is not a miss - a visibilitychange to visible drops the frame that spans the gap and restarts a running QT1 probe; the receipt prints hidden_skips=<n>. Diff: regular. Tier A. Size S. Owns \`crates/web/src/arena.rs\` (hidden check, probe restart) and \`crates/web/src/shell.rs\` (visibility listener). Accept: a test with a 30 s frame gap expects no cut and a probe restart; the receipt prints hidden_skips. F2P: a long first frame after show may count as a miss (unverified). P2P: golden d57b5c8ae3096614 unchanged. needs: QT1.`

### QT8 Laptop and iGPU thermal drift
- What: a laptop starts fast and slows as heat builds, so a one-minute probe misses it. Compare the first minute with later minutes. RTINGS measures the drop as the first score against the average of later ones (snippet). PCPer found higher frame-time variance on a thermally limited laptop than on the same silicon in a desktop (snippet).
- Source: RTINGS https://www.rtings.com/laptop/tests/performance/performance-over-time (snippet). PCPer https://pcper.com/2017/07/asus-rog-zephyrus-gx501-gtx-1080-max-q-gaming-notebook/2/ (snippet). Temperature thresholds conflict: 90-95 C and 85 C (snippets, low-authority sources). A web page exposing GPU temperature: not checked here (unverified).
- Numbers: none trusted. Proposal: 60-second rolling mean 20 percent over the first minute's mean for 3 minutes takes one cut; no restore for 5 minutes.
- Maps onto: `tier_view.rs` windows; the WEB8 receipt already prints a frame split every second.
- Cost: M.
- Row: `- [ ] QT8 web: long-run drift check - keep a 60-second rolling mean after the first minute and print drift=<pct> in the split line; one extra cut when drift stays over 20 percent for 3 minutes; no restore for 5 minutes. Diff: regular. Tier A. Size M. Owns \`crates/web/src/tier_view.rs\` (drift window) and the split line in \`crates/web/src/arena.rs\`. Accept: unit test on a synthetic 10-minute frame series; the receipt carries drift=. F2P: no drift field. P2P: golden d57b5c8ae3096614 unchanged. needs: QT5.`

### QT9 Texture cap from adapter limits, applied for real
- What: the web gives no VRAM size (unverified here; the spec exposes limits, not memory). Unreal's texture streaming page describes a pool whose mips go up and down, but the page read has no pool numbers (fetched). Use the bucketed limits (QT4) as a proxy and wire `texture_max_px` to the upload path, which the WEB8 note says it is not yet.
- Source: Unreal texture streaming, https://dev.epicgames.com/documentation/en-us/unreal-engine/texture-streaming-in-unreal-engine (fetched; no numbers). Repo knobs: potato `stream_pool_mb` 200, `texture_max_px` 512.
- Numbers: repo values only. No sourced pool size for a 3050 or a shared-memory UHD.
- Maps onto: `quality.rs` (a helper beside `knobs`); the texture upload or mip pick in `crates/web`.
- Cost: S.
- Row: `- [ ] QT9 web: texture cap from limits - the web tier's texture_max_px becomes the smaller of the tier value and the adapter's maxTextureDimension2D, applied to uploads; the receipt prints tex_max=<px>. Diff: regular. Tier A. Size S. Owns \`crates/render/src/quality.rs\` (helper beside knobs) and the mip pick in \`crates/web\`. Accept: a test where a 2048 adapter limit caps a 4096 tier value to 2048; the receipt prints tex_max. F2P: texture_max_px prints but is not applied (WEB8 note, docs/SPRINT-22-web-realism.md line 14). P2P: golden d57b5c8ae3096614 unchanged. needs: QT4.`

### QT10 Denylist that forces the fallback
- What: known-bad GPU or driver strings go to potato whatever the probe says. detect-gpu has a BLOCKLISTED result type (fetched). Unity 6.6 adds Graphics Device Filtering that forces the fallback for given browsers, GPUs or drivers (snippet from third-party news, not Unity's notes).
- Source: detect-gpu README (fetched, MIT). Unity: https://app.cinevva.com/news/2026-09-01-unity-6-6-webgpu-production (snippet).
- Numbers: none beyond the fps threshold above.
- Maps onto: `choose_web_tier` (a denylist argument) or `arena.rs` `pick_tier`.
- Cost: S.
- Row: `- [ ] QT10 web: denylist - a short list of adapter name fragments, empty by default, sends a match to potato with reason=denylist; the list is one const whose comment names its source. Diff: regular. Tier A. Size S. Owns \`crates/render/src/quality.rs\` (const and reason string) and its test. Accept: a listed fragment gives (Potato, "denylist"); an empty list changes nothing. F2P: no denylist. P2P: golden d57b5c8ae3096614 unchanged. needs: QT3.`

### QT11 Named fallbacks, never a blank canvas
- What: Unity can drop from WebGPU to WebGL 2 without an obvious error when the page is not a secure context (snippet). Godot 4's web export has no WebGPU renderer and runs the Compatibility renderer on WebGL 2 only (snippet from the 4.3 and 4.4 web export docs, not fetched). Whatever path lands below web should say why.
- Source: Godot web export docs, https://docs.godotengine.org/en/4.3/tutorials/export/exporting_for_web.html (snippet). Unity: as QT10. Repo: `crates/web/src/shell.rs` already has a fatal-reason path (`set-text fatal-reason`, from doc 03 in this folder).
- Numbers: none.
- Maps onto: `shell.rs` fatal path; the reason strings from `choose_web_tier`.
- Cost: S.
- Row: `- [ ] QT11 web: named fallbacks - every path below the web tier (no WebGPU, adapter request failed, limits under floor, denylist) prints fallback=<reason> in the receipt and on the page; a missing adapter never leaves a blank canvas. Diff: regular. Tier A. Size S. Owns \`crates/web/src/shell.rs\` (fatal path) and the text in \`crates/web/index.html\`. Accept: a headless run with WebGPU disabled prints fallback=no-webgpu and shows the message, not a blank canvas. F2P: blank canvas or generic error today. P2P: golden d57b5c8ae3096614 unchanged. needs: QT10.`

## 3. Order

Start with the no-timing rows (QT3, QT4, QT10, QT11, QT9), then the probe chain (QT1, QT2, QT7), then the ladder (QT5, QT6), then QT8 last because it needs a long run to check. QT1 is the largest unknown; its thresholds are proposals until measured on the 3050 and the UHD.

## 4. What this does not settle

- Every threshold marked proposal (QT1, QT2, QT5, QT6, QT8) is ours. No source here measured frame costs on a 3050 or an Intel UHD.
- detect-gpu's data stopped updating in December 2025 (fetched). Treat its tiers as a rough hint only.
- Babylon's source disagrees with itself on the hardware-scale step: code default 0.25, doc comment 0.5 (fetched).
- Chrome's background budget numbers come from a 2017 post (fetched). Current Chrome may differ.
- The IndexedDB background exemption and the default `maxTextureDimension2D` of 8192 are snippet only.
- Unity and Godot claims are snippets; neither vendor's current page was read.
- The web GPU temperature question and the VRAM question were not checked against any web API page.
- Licences: detect-gpu is MIT (fetched). The fetched Babylon file had no licence header; the repository licence was not checked. The other sources are documentation; no code was taken from them.
