STEAL-WDE scout, 2026-10-18. Wave 2026-10-18/1, block 1: github.com/topics/game-engine (l=rust, o=desc, s=updated), position 11. Area: debug-line buffer, cascaded shadow settings, sim/render boundary.
Read only, ideas only; no code copied, nothing vendored. Every donor file below was fetched live on 2026-10-18.

== 1. WHAT IT IS ==
WaterDropEngine (explorablescience, 3 stars, updated 2026-10-18) is a small 3D engine in Rust built on Bevy's ECS, wgpu (custom renderer, deferred PBR, cascaded shadows, atmosphere) and Rapier. It is explicitly one person's playground: the README says it is not meant for other users and issues/PRs are not accepted, and the main consumer is a separate node-based terrain generator. Layout is `crates/core/*` (wde, wde-renderer, wde-editor, wde-scene), `crates/render/*` (wde-pbr, wde-gizmos, wde-camera, wde-animation, wde-gltf, wde-pbr-outline) and `crates/terrain/*`. Our interest is the render-side shapes: how gizmo lines are stored and drawn, how cascade settings are packed, and where the sim/render world boundary sits.

== 2. LICENCE ==
GPL-3.0 (SPDX `GPL-3.0` via GitHub API 2026-10-18; LICENSE read live 2026-10-18: https://raw.githubusercontent.com/explorablescience/WaterDropEngine/main/LICENSE — GNU General Public License Version 3, 29 June 2007). Copyleft: ideas only, no code copied.

== 3. STEALS (ranked) ==
1. Fixed-capacity gizmo line buffer, appended by the sim side, taken and uploaded by the render side | explorablescience/WaterDropEngine@main:crates/render/wde-gizmos/src/data.rs:5-15,24-38; extract.rs:30-39; pipeline.rs:39-50 | PRESENTATION | engine2040 new `render/src/debug_draw.rs` (EG26 / XX3) | S-M
   Idea: `MAX_GIZMO_LINES` (:7) caps how many lines a frame may record; past the cap, lines are **dropped with a warning rather than growing the GPU buffer** (:5-7). One storage buffer sized `GizmoVertex * MAX * 2` is created once and filled each frame, two vertices per line, vertex-pulled by index in the shader (:12-38). The main world records into a `Gizmos` resource; the render world's `extract_gizmo_lines` calls `take_lines()`/`take_quads()`, which clears the source so the next frame records fresh (:30-39). The pipeline is depth-tested against the scene but writes no depth, no cull, alpha-blended line list (:39-50).
   Maps: engine2040 has no debug-draw at all (`BOOK-GAPS-2026-10-12.md:102`); this gives EG26 a shape that cannot blow VRAM per frame — a cap, a warning, a reused buffer, and a "take" that keeps the sim side append-only until the render side copies it.
   Kill: if the cap drops lines the vision judge needs for one capture, or any debug line pixel can enter the beauty frame/golden -> keep the id/pass out of the hashed path and raise the cap.

2. One uniform and one resource that fully describe the cascade set, with the depth bias packed into a used component | explorablescience/WaterDropEngine@main:crates/render/wde-pbr/src/passes/shadow/shadow_map_data.rs:16-35,37-55 | PRESENTATION | engine2040 `render` shadow/cascade path (render-lighting) | S
   Idea: `ShadowParamsUniform` (:16-35) is `view_proj` plus `light_dir` where `w` is the **depth bias** (not padding), then PCF `kernel_half_size`, `kernel_scale`, `shadow_intensity` and explicit `_padding`; the default has `light_dir.w = 0.005` (:28). `CascadedShadowSettings` (:37-55) is one reflected resource holding three parallel arrays — `split_distances`, `ortho_sizes`, `ortho_depths`, and per-cascade `kernel_half_size`, `kernel_scale`, `shadow_intensity` — so every cascade knob is edited and extracted as one unit.
   Maps: engine2040's light/shadow code is hunting for the same three things: bias, PCF kernel and per-cascade intensity. Packing bias into a free component and keeping the cascade set in one resource makes the exposure/lighting tuning a single edit rather than three scattered literals.
   Kill: any per-cascade uniform that bloats above 3x the single-shadow size with no visible pixel change, or a bias packed where WGSL alignment actually needs padding -> keep it explicit.

3. Sim world handed to the render world for one extract pass, then swapped back | explorablescience/WaterDropEngine@main:crates/core/wde-renderer/src/core/extract.rs:14-36 | PRESENTATION (boundary) | engine2040 tick-and-runtime: render reads copies taken after the tick | M
   Idea: `main_extract` first runs `sync_worlds` while the real main world is still available (:16-19), then *temporarily* swaps an empty world into the main world's slot, inserts the real main world as a `MainWorld` resource on the render world, runs the `Extract` schedule, and moves everything back (:21-35). Extract commands registered during the schedule are applied only afterwards (`apply_extract_commands`, :39-47).
   Maps: engine2040 wants the render side to read a snapshot, never the live sim; the swap-in-resource trick is a formal boundary that makes "the render schedule cannot touch the sim world" structural rather than a comment.
   Kill: if any render pass ends up holding a `&mut` sim reference past the extract, or the swap costs a measurable frame hit at 1080p.

== 4. NOT WORTH STEALING ==
- `crates/core/wde-editor/src/panels/rendergraph/mod.rs:44-48`: the pass-name listing that would populate the render-graph panel is commented out (`get_sorted_passes_OLD`), so the panel registers no live pass introspection; engine2040's XX1 pass ring is the better shape.
- `crates/core/wde-editor/src/panels/profiler.rs:7-24`: embedding `puffin_egui` in the editor panel; the 2026-10-12 card already records the egui-version trap, so only the idea survives.
- Bevy's `Extract`/`Render` sub-app machinery as a whole: engine2040 keeps its own executor and does not want a second schedule world; only the swap boundary above transfers.

== 5. FILES READ ==
README.md, LICENSE, crates/core/wde-renderer/src/core/extract.rs, render_multithread.rs, crates/render/wde-pbr/src/passes/shadow/shadow_map_data.rs, crates/render/wde-gizmos/src/data.rs, extract.rs, pipeline.rs, crates/core/wde-editor/src/panels/rendergraph/mod.rs, panels/profiler.rs (all @main, 2026-10-18).
