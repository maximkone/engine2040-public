---
name: msaa-resolve-checks
description: Use when a builder or judge adds 4x MSAA in crates/render and must prove thin edges blend with a resolve before the blit.
---
# MSAA resolve checks
Eight checks for a 4x MSAA diff. Name the crate in each finding (render, game, perf). PASS needs an edge-pixel test plus frame-split ms, never a picture alone.
1. **Four samples, not full supersample.** Ask: is the target 4x multisample with one shading run per pixel. Flag: a 4x wider frame or per-sample shading cost (render) `crates/render/src/present/gpu_present/presenter.rs::sample_count`.
2. **Coverage plus depth per subsample.** Ask: do coverage and depth run per subsample while color shades once. Flag: single-sample depth with a 4x color target (render) `crates/render/src/v1_gpu.rs::sample_count`.
3. **Color depth and stencil grow 4x.** Ask: are all three multisample buffers sized N times with slots per pixel. Flag: a 4x color buffer beside a 1x depth buffer (render) `crates/render/src/present/gpu_present/submit.rs::sample_count`.
4. **Resolve before post and blit.** Ask: does the multisample buffer average down to screen size before tone map and present. Flag: a resolve after present or no `resolve_target` on the pass (render) `crates/render/src/v1_skin.rs::resolve_target`.
5. **Edges blend, interiors hold.** Ask: do edge pixels show in-between values with MSAA on and flat steps with it off. Flag: whole-frame softening or unchanged edges (game) `crates/render/src/present/tests_unit.rs`.
6. **Off switch kept.** Ask: is there one setting that turns MSAA off and picks the fallback path. Flag: MSAA always on with no A/B capture (render) `crates/render/src/present/aa.rs::AaChoice`.
7. **Cost is bandwidth, not shading.** Ask: does frame-split p50 rise at most 2 ms with MSAA on at 1080p. Flag: a large shading jump or no before/after numbers (perf) `crates/game/src/bin/arena_frame/mod.rs::frame_split_line`.
8. **MSAA stays apart from SMAA.** Ask: is the MSAA resolve separate from the temporal and SMAA fallback. Flag: MSAA wired through `taa_resolve` or the scale fallback (render) `crates/render/src/post.rs::taa_resolve`.
## Do / Don't
- Do: set 4x targets plus a resolve before the blit and show edge pixels plus p50. Don't: ship 4x buffers with no resolve or a blur that softens interiors too.
source: Gregory, Game Engine Architecture (2nd ed.), ch. 10.2.7
