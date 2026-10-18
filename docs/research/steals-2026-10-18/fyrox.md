STEAL-SCOUT block 9 card, read 2026-10-18. Donor: FyroxEngine/Fyrox (3D and 2D game engine in Rust, 9.6k stars).
Donor sha 729d7b7c3f59ca7de643fa7f0cf6748a1259baf8 (master, 2026-10-18). Read: file bodies fetched live today (camera.rs, pool/mod.rs, pool/multiborrow.rs, hdr/adaptation.rs, hdr/luminance/*, occlusion/optimizer.rs, tree, LICENSE.md). No build, no run. No bytes vendored: ideas only.
Gates: golden d57b5c8ae3096614 / 2aca9338670aa0cb are P2P on every row below; nothing here touches a tick or a hash.
Already taken from this donor, do not re-take: `scene/ragdoll.rs` momentum handoff (00-combined L5, ALREADY HAVE), `blendspace.rs` event strategy (00-combined L9, landed 1c0ab6be), `plugin/dylib.rs` copy-then-load reload (steals-2026-10-12/arch.md rank 10), `scene/visitor` save versioning (00-combined T9).

## What it is

The largest mature Rust engine with a real editor. It is also the best donor in this block for *plumbing*: handles, pools, buffers, cameras and render targets, all written to be used by an editor that mutates a live world. That editor pressure is what our single-window arena lacks.

## Licence

`LICENSE.md` root, first line (read live): `Copyright (c) 2019-present Dmitry Stepanov and Fyrox Engine contributors.` followed by the MIT permission text. GitHub licence API: SPDX `MIT`.

## What we have (checked, not guessed)

- One frame camera per draw list: `crates/render/src/present.rs:449-457` (`FrameDraws { camera, .. }`).
- No render target anywhere in the renderer: grep `render_target|RenderTarget` in `crates/render/src` -> only a string inside a quality-table name (`quality.rs:255`, `:652`).
- Sketch: `crates/render` has no mirror, planar reflection or reflection probe (grep `mirror|planar|reflection_probe|Enviro` -> only prose in comments).
- CPU exposure exists; GPU exposure is a plan row (steals-2026-10-12/post.md rank 7, RL18).

## Steals (ranked by value per hour)

1. **A camera that renders into a texture, with a resolution-independent viewport** | Fyrox@729d7b7c3f59:fyrox-impl/src/scene/camera.rs:359-402 (`Camera { .. }`), :366 `viewport: InheritableVariable<Rect<f32>>`, :386-388 `render_target: Option<TextureResource>` with a reflect setter, :463-480 `set_viewport` clamps all four numbers to `0.0..=1.0`, :483-499 `viewport_pixels` scales by frame size and floors each edge at 1 px (a 0-size viewport panics elsewhere) | MIT | our file: `crates/render/src/present.rs` (`FrameDraws`), `crates/render/src/present/gpu_present/submit.rs` |
   Our draw list has exactly one camera and one surface. The steal is small and general: a camera carries an optional target texture and a fractional viewport. That one idea pays for a minimap, a looks-different photo/replay frame, a second eye, and the editor preview, with no second window. Kill: if it forces a second frame-graph and more than ~150 lines of fork, keep the single camera and revisit when a real consumer exists.

2. **Borrow two or three items from one pool at once** | Fyrox@729d7b7c3f59:fyrox-core/src/pool/mod.rs:21-41 (generational arena: handle validity = generation match), :879 `borrow_two_mut((Handle, Handle)) -> (&mut T, &mut T)`, :911 `borrow_three_mut`; fyrox-core/src/pool/multiborrow.rs:126 `MultiBorrowContext`, :206 `try_get`, :275 `try_get_mut`, :295 `free`, with a runtime `RefCounter` so a second borrow is refused, not aliased | MIT | our file: `crates/ecs` (spine slots), `crates/combat` (the two fighters in one system) |
   Every two-fighter step in the duel wants two mutable rows at once. We work around it with copies or index juggling. Fyrox's answer is a checked multi-borrow: disjoint handles borrow, the same handle twice refuses. It also gives us the discipline for 4 and 8 threads later (RC36). Kill: if a two-phase read/copy already reads clearly at the call site, do not add a context type for it.

3. **Eye adaptation by a ping-pong luminance chain, no readback** | Fyrox@729d7b7c3f59:fyrox-impl/src/renderer/hdr/adaptation.rs:27-45 (`AdaptationChain { lum_framebuffers: [LumBuffer; 2], swap: Cell<bool> }`, `begin()` hands the previous frame's average to this frame), :67 `avg_lum_texture()`, with `hdr/luminance/histogram_luminance_evaluator.rs` as the bigger variant | MIT | our file: `crates/render/src/post.rs` (exposure), plan row RL18 (`docs/plan/render-look.md`) |
   Our exposure is a CPU log-luma histogram that would need a readback on the live frame. Fyrox keeps the average on the GPU and swaps two buffers per frame, so the value is always one frame old and never stalls. Take the cheap potato-tier shape (two 1x1 buffers, up and down slew rates) and leave the histogram to Bevy's steal (steals-2026-10-12/post.md rank 7) if that lands first. Kill: if the two-buffer chain flickers on the torch fixture more than the CPU twin, keep the CPU value and expose it as a fixed EV.

Not worth it: their occlusion `VisibilityBufferOptimizer` (`occlusion/optimizer.rs:41-44`) reads the visibility buffer back through `GpuAsyncReadBuffer`, which the 00-combined "skipped" list already rejected as a readback stall. Their `GraphicsServer` trait with GL and WGPU backends is a second device layer we do not need; we have one backend and one adapter choice (`crates/render/src/adapter.rs`).
