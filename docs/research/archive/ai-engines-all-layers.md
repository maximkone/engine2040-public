# AI-engines all-layers survey: 28 repos COMPARE.md does not score (RES-AIENG-5, 2026-10-01)

Scope: delta rows to EXTEND `docs/research/COMPARE.md` — this file proposes, it does not edit
COMPARE.md, freeze denominators, or score cells. Rubric reused unchanged: R-CMP-2
(same items/weights/fixture per row, DOCS-CLAIM vs verified split, UNKNOWN stays UNKNOWN,
no percentage from a partial sample). Every cell below is UNKNOWN until a frozen-denominator
run sources it; "lead" means publisher claim or code shape read, never a measured verdict here.
Researcher ran no builds, no tests, no GPU/WAN runs.

Already scored in COMPARE.md (not repeated): Unreal 5, Unity 6, Godot 4 (docs cells),
Bevy/O3DE/Flax/Stride (UNKNOWN columns), Jolt BodyInterface.h, Rapier determinism guide,
Filament/meshoptimizer/glTF-IO/glTF-Transform/ozz-animation (primary leads), ahujasid
mcp-for-blender, blend-ai, Coding-Solo + tugcantopaloglu godot-mcp, promptfoo, objaverse-xl.
License key: `v` = LICENSE text read live 2026-10-01 via raw.githubusercontent;
`~` = widely-published SPDX, NOT re-read this run (verify at promotion); `!` = non-permissive
(EULA/GPL: ideas only, never port bytes — packet rule + `deny.toml` allow-list).
Stars are memory-approx, 2026-10-01, never live-counted (re-count at promotion).

## Delta table: 28 new/under-covered donor repos (rows to add to COMPARE.md)

| # | Repo + URL | Stars~ | SPDX | Best layer | Stealable technique + file path in repo |
|---|---|---|---|---|---|
| 1 | Bevy https://github.com/bevyengine/bevy | ~40k | MIT/Apache-2.0 v | ECS + schedule | ambiguity + schedule-order audit for system conflicts: crates/bevy_ecs/src/schedule/schedule.rs (upstream Bevy file) |
| 2 | Fyrox https://github.com/FyroxEngine/Fyrox | ~10k | MIT v | editor + scene | reflection-driven inspectors + saveload Visitors (snapshot-friendly serde): `fyrox-core/src/visitor.rs` |
| 3 | Macroquad https://github.com/not-fl3/macroquad | ~8k | MIT/Apache-2.0 ~ | slimness | miniquad single-backend window loop, zero-setup native+wasm: `src/lib.rs` + `miniquad` backend |
| 4 | Comfy https://github.com/darthdeus/comfy | ~2.5k | MIT ~ | slim game loop | jam-scale global-context teardown that keeps hot-reload simple: `comfy-core/src/lib.rs` |
| 5 | Kajiya https://github.com/EmbarkStudios/kajiya | ~4k | Apache-2.0/MIT ~ | render GI | ReSTIR-DI reservoir + render-graph pass culling (verify path at promotion): `crates/lib/kajiya-rg` + `assets/shaders/restir_gi.hlsl` |
| 6 | Rend3 https://github.com/BVE-Reborn/rend3 | ~1.5k | MIT/Apache-2.0 ~ | render graph | automatic barrier + routine-driven culling graph: `rend3/src/util/graph.rs` (verify at promotion) |
| 7 | Piston https://github.com/PistonDevelopers/piston | ~7k | MIT/Apache-2.0 ~ | event loop | pistoncore-input state-machine event fold (Update/Render split): `crates/input/src/lib.rs` (pistoncore-input) |
| 8 | Ambient https://github.com/AmbientRun/Ambient | ~3.5k | MIT/Apache-2.0 ~ | netcode runtime | WASM component runtime + content-addressed asset cache: `shared_crates/asset_cache/src/lib.rs` (verify) |
| 9 | Veloren https://github.com/veloren/veloren | ~6k | GPL-3.0-only ~ ! | voxels + crowds | greedy-meshed voxel LOD + large-agent sim split (IDEAS ONLY, no bytes): `common/src/volumes/` (verify) |
| 10 | Rapier https://github.com/dimforge/rapier | ~3.5k | Apache-2.0 v | physics determinism | enhanced-determinism (parallel-solver-identical-to-sequential claim) + fixed-step pipeline: `src/pipeline/physics_pipeline.rs` |
| 11 | Lightyear https://github.com/cBournhonesque/lightyear | ~2.5k | MIT v | netcode | input-buffer rollback + server-authoritative replication ticks: `lightyear/src/client/input_leaf.rs` (verify) |
| 12 | Renet https://github.com/lucaspoffo/renet | ~2.5k | MIT/Apache-2.0 ~ | netcode transport | per-channel reliability config (reliable/unreliable/block) over one UDP socket: `renet/src/channel.rs` (verify) |
| 13 | GGRS https://github.com/gschup/ggrs | ~1.5k | MIT/Apache-2.0 v | rollback | sync-test session + sparse-input checksum discipline: `src/sessions/sync_test_session.rs` |
| 14 | bevy_replicon https://github.com/lifescapegame/bevy_replicon | ~900 | MIT/Apache-2.0 ~ | replication | server-tick mutate ack + client mutate-ack ordering: `src/server.rs` (verify) |
| 15 | bevy_ggrs https://github.com/johanhelsing/bevy_ggrs | ~400 | MIT/Apache-2.0 ~ | rollback | GGRS session runner folded into Bevy schedule + checksum saves: `src/lib.rs` (verify) |
| 16 | Godot https://github.com/godotengine/godot | ~95k | MIT v | animation + editor | AnimationNodeBlendTree state-machine authoring (ideas + MIT-safe port): `scene/animation/animation_blend_tree.cpp` |
| 17 | O3DE https://github.com/o3de/o3de | ~11k | Apache-2.0/MIT v | render RHI | Atom RHI transient-attachment pooling + FrameScheduler pass order: `Gems/Atom/RHI/Code/Source/RHI/FrameScheduler.cpp` (verify) |
| 18 | Stride https://github.com/stride3d/stride | ~6k | MIT ~ | render compositor | GraphicsCompositor permutation cache for feature mix-and-match: `sources/engine/Stride.Rendering/Rendering/GraphicsCompositor.cs` (verify) |
| 19 | Flax https://github.com/FlaxEngine/Flax | ~8k | Flax-EULA ! | editor undo | scoped Undo objects per edit op (IDEAS ONLY, EULA source): `Source/Editor/Undo/Undo.cs` |
| 20 | WickedEngine https://github.com/turanszkij/WickedEngine | ~10k | MIT ~ | render GI + slim | render-path switch (forward/deferred/tiled + VXGI/surfels) in one renderer: `WickedEngine/wiRenderer.h` |
| 21 | Falcor https://github.com/NVIDIAGameWorks/Falcor | ~3k | BSD-3-Clause ~ | RT reference | RenderGraph auto resource-lifetime + ScriptableRenderPass authoring: `Source/Framework/RenderGraph/RenderGraph.cpp` (verify) |
| 22 | bgfx https://github.com/bkaradzic/bgfx | ~16k | BSD-2-Clause v | slim RHI | transient-buffer frame protocol + shaderc cross-compile to all backends: `src/bgfx.cpp`, `tools/shaderc/shaderc.cpp` |
| 23 | sokol https://github.com/floooh/sokol | ~10k | Zlib v | slim RHI | single-header sokol_gfx command model + sokol_app zero-dep loop: `sokol_gfx.h` |
| 24 | raylib https://github.com/raysan5/raylib | ~24k | Zlib v | build speed + slim | dependency-free single-platform rcore, seconds-scale builds (DOCS-CLAIM): `src/rcore.c` |
| 25 | Jolt https://github.com/jrouwe/JoltPhysics | ~13k | MIT v | physics character | CharacterVirtual swept controller + motion-quality CCD switch: `Jolt/Physics/Character/CharacterVirtual.h` |
| 26 | Box2D https://github.com/erincatto/box2d | ~9k | MIT ~ | physics determinism | deterministic island solver + versioned world serialization: `src/dynamics/b2_world.cpp` (verify) |
| 27 | Recast https://github.com/recastnavigation/recastnavigation | ~9k | Zlib ~ | crowds + nav | TileCache runtime rebake + DetourCrowd local avoidance (D-CROWDS-1 donor): `DetourCrowd/Source/DetourCrowd.cpp` |
| 28 | Diligent https://github.com/DiligentGraphics/DiligentEngine | ~4k | Apache-2.0 v | RHI + RT | backend-agnostic RenderDevice with one ray-tracing tutorial path: `Tutorials/Tutorial21_RayTracing/` (verify) |
| 29 | Lumix https://github.com/nem0/LumixEngine | ~5k | MIT ~ | editor + hot-reload | plugin-DLL hot-reload + Lua studio embedded in the editor: `src/studio/` (verify) |
| 30 | Piccolo https://github.com/BoomingTech/Piccolo | ~8k | MIT ~ | full-engine teaching | one-course engine: render-system + pilot math in reviewable slice: `engine/source/runtime/function/render/render_system.cpp` (verify) |

Notes: 30 rows (>=25 required). Measured numbers: UNKNOWN for all 30 — no donor publishes a
same-fixture number this survey reuses, and per R-CMP-2 a publisher claim is DOCS-CLAIM, never
our runtime result (raylib seconds-builds and Rapier parallel-identical stay claims until a
frozen run replays them). The Forge (ConfettiFX/The-Forge, custom EULA) and Hazel/Overload
were triaged OUT this run: Forge duplicates the Diligent/bgfx RHI lesson under a worse license,
Hazel duplicates the Lumix/Flax editor lesson with an unverified root license, Overload
duplicates Piccolo's teaching slice at lower coverage — re-open any with a LICENSE live-read +
a file path the delta table lacks.

## Ranked top-10 layers where engine2040 trails the best OPEN engine (donor + bounded next task)

Rank = (gap size x steal clarity), cells stay UNKNOWN until frozen. Owner crates use COMPARE.md
plan homes; `(verify)` = confirm home at dispatch with `node tools/board-slice.mjs <ID>`.

| Rank | Layer (COMPARE row) | Best open donor (why) | Bounded next task: owner crate + acceptance |
|---|---|---|---|
| 1 | Render lighting/GI (UNKNOWN) | Kajiya ReSTIR-DI + RG culling (#5) | `crates/render`: spike ReSTIR-DI-shape probe vs current path on one fixture; accept hash x2 + p95 frame delta published, kill on any drift |
| 2 | Render post/upscale/FG (UNKNOWN) | Falcor RenderGraph + SDK sample passes (#21) | `crates/render`: spike single upscaler pass behind the graph; accept capture pair + 1.0 ms budget hold, kill on ghosting over bar |
| 3 | Render particles/VFX fluids (D-FLUIDS-1 1/9 shapes) | WickedEngine render-path switch + volumetric tier (#20) | `crates/render` fluids_gpu.rs (VX5): accept identical sim bytes x2 from a seed + 1.0 ms budget, kill on byte drift |
| 4 | Netcode/rollback (MP/ON rows UNKNOWN) | Lightyear input rollback (#11) + GGRS sync-test (#13) | `crates/net` (verify): spike input-buffer rollback on loopback tape; accept replay x2 + checksum-equal resim, kill on any divergence |
| 5 | Crowds/avoidance (D-CROWDS-1, ORCA missing) | Recast DetourCrowd (#27) | `crates/nav` orca.rs (NV4): accept steering <=0.1 ms/agent flat 50->500 agents + 0 stuck in 1000-seed drill + replay x2 |
| 6 | Rigid determinism/islands (D-RIGID-1, PX6 open) | Box2D island solver (#26) + Jolt CreateBodyWithID | `crates/physics` islands.rs (PX6): accept same hash under 1/2/4/8 threads + >=2x on 64-fighter scene, kill on thread-count drift |
| 7 | Skeletal events (D-SKEL-1 1/9 unknown) | Godot AnimationNodeBlendTree (#16) | `crates/anim`: read events.rs + run `cargo test -p engine-anim --lib events`; accept green + Combat-tape 0 missed/late, kill on any late fire |
| 8 | Editor undo/panels (ED/UIX rows UNKNOWN) | Lumix hot-reload studio (#29) + Flax Undo scope (#19, ideas) | `crates/editor` (verify): spike scoped-undo one panel op; accept preview/undo round-trip + replay x2, kill on second write path |
| 9 | AI-door tool-text (AX rows, 20 tools) | tug godot-mcp 80-char cap (COMPARE e-blendermcp follow-up) | `crates/endpoint` collapse.rs (API4): accept >=50% fewer tool tokens on 20 fixtures, 0 success loss, p95 within +10% |
| 10 | Build speed/slimness (no row) | sokol (#23) + raylib (#24) single-header/zero-dep shape | workspace: publish clean-build seconds + binary bytes bars before/after one dep cut; accept both bars down, kill on feature loss |

Rejections (this run, re-open with license + path + fixture): The-Forge (EULA, dup lesson),
Hazel (license UNVERIFIED, dup lesson), Overload (dup lesson), Veloren bytes (GPL-3.0 — ideas
only, voxel LOD line stays an idea cite), Flax bytes (EULA — ideas only).
