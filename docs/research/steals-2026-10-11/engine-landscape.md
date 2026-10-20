# Engine landscape steals (research, 2026-10-11)

Ask (owner, 2026-10-11): look at other GitHub engines, at any star count, for ideas that finish us faster or make us richer, and fill the gaps.

Method: 38 Haiku scouts, one topic each, about 20 tool calls each. Each opened real pages on the live web. "Opened" means the scout read a repo, directory or file page. It does not mean the file body was read. The top six claims were re-opened by hand on 2026-10-11 and held: unlit3d, pmndrs/upscaler, zenfg, renew-engine, undoredo, Ikemen GO rollback.go. Rows marked U were seen in search results only. Before you port a file, open it. License and last-push dates were often not visible; the table says so.

Cost: S under 1 day, M days, L a week or more. Win: 1 to 5.

Already mined earlier (not repeated here): `render-steals.md`, `anim-feel-steals.md`, `arxiv-neural.md`, `ai-native.md`.

## The ten-line answer

1. Our worst habit is modules that land and never reach the screen. Four finds attack that directly: a pass graph that prints what ran and what it culled (zenfg, Granite), a golden frame per pass (unlit3d), self-registering systems (linkme), and a schedule that refuses unordered writes (Bevy ambiguity check).
2. Speed: render at 720p and upscale. Two WGSL-ready upscalers exist (pmndrs/upscaler, SGSR). This is the cheapest road to G2.
3. Light: the arena is static, so bake it. A Rust wgpu lightmap baker exists (scathanna2). Probes and radiance cascades are for later.
4. Shaders: Babylon.js and PlayCanvas ship WGSL, our own language. Port their files; do not translate HLSL by hand.
5. Content is the biggest "richer" lever and it is free: Poly Haven, ambientCG, Quaternius, Sonniss and Freesound all have APIs or stable zips. One fetch script fills the empty content folder.
6. The editor is mostly a shopping list: transform-gizmo, egui_dock, an undo crate, facet. Only the viewport shell is ours to write.
7. Determinism has a recipe from Box2D, Rapier, Ikemen GO and the renew engine. Most of it is CI and flags, not new code.
8. AI door: llguidance makes model output always parse. A small vision model (Qwen3-VL-2B, 1.11 GB) can be the look judge. An announcer can be authored once and replayed with no model.
9. Replay to video: our input tape plus a fixed-tick re-sim plus an ffmpeg pipe is a trailer machine (Crimson flow, bevy_capture pipe).
10. Nobody has open code for some of our gaps. Section "Gaps nobody filled" lists them. We build those.

## Do these first (ordered by the finish-line gates)

| # | Gate | Steal | Source | Cost / Win |
|---|---|---|---|---|
| 1 | G3 | Pass graph: each pass declares reads and writes; the frame report lists every pass as ran, culled or failed; a test asserts sky, IBL, shadow, GTAO, bloom, TAA are in the plan. Port the idea, not Vulkan barriers (wgpu tracks them). | zenfg (Rust, wgpu, MIT) https://github.com/uinosoft/zenfg ; Granite `validate_passes` https://github.com/Themaister/Granite/blob/master/renderer/render_graph.cpp | M / 4 |
| 2 | G1 G3 | Golden frame per pass. Harness: headless device, readback, SSIMULACRA2 floor plus outlier fraction, `SNAPSHOT_UPDATE=1` to re-bless, lavapipe in CI. Add wgpu's own writer of `-actual` and `-difference` PNGs, and `WGPU_ADAPTER_NAME` pinning so a run never lands on the iGPU. | unlit3d https://github.com/beicause/unlit3d (crate `unlit_wgpu_test_util`); https://github.com/gfx-rs/wgpu/blob/trunk/tests/src/image.rs ; https://github.com/gfx-rs/wgpu/blob/trunk/wgpu/src/util/init.rs | S / 5 |
| 3 | G2 | Render 720p, upscale to 1080p. pmndrs/upscaler has WGSL EASU/RCAS plus a temporal path (MIT). SGSR v1 is one pass, 12 taps (BSD-3). | https://github.com/pmndrs/upscaler ; https://github.com/SnapdragonGameStudios/snapdragon-gsr/tree/main/sgsr | M / 5 |
| 4 | G2 | Per-pass GPU ms on the 3050. Add the crate. Also try `PresentMode::Mailbox` (a lead, not confirmed for wgpu). | https://github.com/Wumpf/wgpu-profiler | S / 5 |
| 5 | G3 | Bake the arena light. Rust baker with Halton sampling, runtime only samples lightmaps. Pair with xatlas for lightmap UVs. Fighters get ambient cubes or SH probes. | https://github.com/barnex/scathanna2 ; https://github.com/jpcy/xatlas | M / 5 |
| 6 | G3 G4 | Port WGSL that exists: Babylon.js `ShadersWGSL` (clustered lights, OpenPBR set, ssao2, ssr2, taa, tonemap, shadowMap, volumetric light, subsurface with diffusion profile). PlayCanvas `lit/frag` WGSL chunks (aniso GGX, sheen, LTC, clustered, VSM). For GLSL: `naga` CLI. For modules and `#ifdef` variants: `naga_oil`. | https://github.com/BabylonJS/Babylon.js/tree/master/packages/dev/core/src/ShadersWGSL ; https://github.com/playcanvas/engine/tree/main/src/scene/shader-lib/wgsl/chunks/lit/frag ; https://github.com/gfx-rs/wgpu/tree/trunk/naga ; https://github.com/bevyengine/naga_oil | M / 4 |
| 7 | G4 | Bodies: hair strands with Kajiya-Kay plus Marschner in WGSL; clear coat for lacquered armor; pre-integrated skin; eyes. | https://github.com/Scthe/frostbitten-hair-webgpu ; https://github.com/patriciogonzalezvivo/lygia/tree/main/lighting ; U https://github.com/larsbertram69/Lux_2.0 | M / 4 |
| 8 | Content | One `fetch-content` script: Poly Haven API (CC0 HDRI, PBR, models), ambientCG API, Quaternius Universal Animation Library (120+ clips, glTF), Sonniss GDC audio bundles, Freesound API with the CC0 filter, StarNinjas sword clashes, Unity free flipbooks. | see "Content" below | S / 5 |
| 9 | G5 | Hit and feel: sweep the weapon from last frame to this frame, one hit per target per swing (Ember). Camera rig crate `dolly`. Trauma shake model. Priority stack for cameras (Phantom Camera). Two-bone IK (TheComet/ik) plus pole-target math. | https://github.com/JVFDevelopment/ember ; https://github.com/h3r2tic/dolly ; https://github.com/ramokz/phantom-camera ; https://github.com/TheComet/ik | S each / 4 |
| 10 | Editor | Gizmo, dock, undo, reflection: `transform-gizmo` (core crate, no UI lock-in), `egui_dock` or `egui_tiles`, `undoredo` (`#[derive(Delta)]`, each AI door command is one commit with its tool and args), `facet` for the inspector and text scenes. Shape from BevyForge: editor in one process, headless runtime in another, one socket. | https://github.com/urholaukkarinen/transform-gizmo ; https://github.com/mikwielgus/undoredo ; https://github.com/facet-rs/facet ; https://github.com/salom600/BevyForge | S to M / 5 |
| 11 | G6 | Determinism kit: renew-engine record/replay and per-target digests (`--json` with `schema_version`); Box2D recipe (own trig, no fast-math, `-ffp-contract=off`, OR-merge of contacts, CI hash on three compilers); Rapier `enhanced-determinism`; Ikemen GO `LiveChecksum` with its exclusion list; Factorio desync bundle; s2 same-seed trace diff; `mas-bandwidth/fixed` trig. | see "Determinism" below | S each / 4 |
| 12 | G7 | AI door: `llguidance` so output always parses. Announcer authored once by a model as data (text plus trigger, throttled by last-played time), replayed with no model. Qwen3-VL-2B GGUF as the offline look judge. | https://github.com/guidance-ai/llguidance ; https://github.com/abadugu13/football_llm_commentary ; https://huggingface.co/Qwen/Qwen3-VL-2B-Instruct-GGUF | S / 5 |
| 13 | Rich | Replay to MP4: input tape, fixed-tick headless re-sim, raw frames into ffmpeg stdin, `--max-ticks` for a clip. | https://mintlify.wiki/banteg/crimson/cli/replay/render ; https://docs.rs/bevy_capture | S / 5 |
| 14 | Rich | Crowd stands: vertex-animation-texture crowd with per-seat offset and tint, octahedral impostors for far rows. Siege fields: Mindustry flow-field steps (run inside the fixed tick with a fixed step count), 0 A.D. formation offsets, ORCA avoidance (`dodgy`). | https://github.com/Mefodei/Render-Crowd-Of-Animated-Characters ; https://github.com/SIsilicon/Godot-Octahedral-Impostors ; https://github.com/Anuken/Mindustry/blob/master/core/src/mindustry/ai/Pathfinder.java ; https://github.com/0ad/0ad/blob/master/binaries/data/mods/public/simulation/components/Formation.js | M / 5 |
| 15 | Ship | `steamworks` crate (achievements, cloud, Steam Input, lobbies), Velopack, `crash-handler` + `minidumper`, and the hybrid-GPU export symbols so the laptop does not render on the iGPU. | see "Shipping" below | S each / 4 |

Note on row 9: the Ember repo is a Roblox/Luau project. The scout opened https://github.com/JVFDevelopment/ember. Use the rule (per-frame swept hit, once per swing), not the code.

## Wiring traps and fixes (the recurring pain)

- Pass graph with report (row 1). A culled pass is reported, never dropped silently. Granite throws when a history input has no writer; that is exactly the TAA case.
- `linkme` `distributed_slice` (https://github.com/dtolnay/linkme): each module appends its own system or pass from its own file. No central list to forget. S / 4.
- Startup self-check and "ran after N headless frames" diff: no engine found that ships it. Design from the scout: run counter per system, snapshot at start, diff after N frames, allowlist for run conditions. We build it.
- Bevy `ScheduleBuildSettings.ambiguity_detection` (https://docs.rs/bevy_ecs/latest/bevy_ecs/schedule/struct.ScheduleBuildSettings.html): conflicting access with no order becomes an error. Copy the check. S / 4.
- Safe-schedule rule, arXiv 2508.15264 Def 4.3 and Thm 4.7: co-schedule only when entity-component influences are disjoint. M / 4.
- `moirai` build-time validation (https://github.com/bnomei/moirai): missing dependency or cycle rejected before tick 0. Pattern only. S / 3.
- bevy_skein idea (https://github.com/rust-adventure/skein): put component data in glTF extras so hitboxes, sockets and spawns attach by data when a scene loads. Wiring becomes a data step. M / 4.
- Infernux idea (https://github.com/ChenlizheMe/Infernux): editor panels generated from declared fields, so a new module shows up with no hand wiring. M / 2.

## Gaps nobody filled (we build these, or pick a weaker substitute)

No open code was found for:
- inertialization or dead blending (only Unreal docs and orangeduck, already mined)
- projected or deferred blood decals in wgpu or WGSL
- 3D sword parry or clash deflection; stamina, poise, hit-stop or cancel-window files
- stack-up and breach-and-clear AI; suspect surrender and morale; sight-cone, hearing and alert packages in Rust
- an auto-director that picks shots from sim events
- crowd roar driven by excitement
- Teardown-style voxel destruction on wgpu; integer or hash-friendly support graphs
- sparse virtual texturing in Rust or wgpu; a dynamic-resolution controller; froxel fog in WGSL; LUT grading in WGSL
- a compute-culling repo for wgpu (only a draft Bevy PR #12673, U)
- an engine-agnostic Rust ability and effect crate (only Bevy ones); a Rust save-migration crate
- a Rust scene-diff tool; an engine that diffs registered systems against systems that ran
- tensor-field streets and Parish-Muller roads; Roman amphitheater, aqueduct and ruin-damage generators
- Windows lavapipe or WARP recipe for wgpu DX12 golden tests (a Mesa 21.1 note was seen, U)

## By area

### Rendering and look

| Item | Take | URL | Cost / Win |
|---|---|---|---|
| Spartan Engine | Uber shaders plus bindless to cut pipeline permutations; persistent per-draw buffers, one indirect draw per pass. Landing page only read. License looked restricted. | https://github.com/PanosK92/SpartanEngine | M / 4 |
| Adria, KWA | Render graph with barrier and transient tracking, aliasing of non-overlapping attachments (matters on 6 GB). | https://github.com/mateeeeeee/Adria ; https://github.com/suVrik/KWA | M / 3 |
| nightshade-renderer | `rendergraph` feature: reads and writes, dead-pass culling, transient aliasing, in a wgpu-native renderer. | https://docs.rs/crate/nightshade-renderer/0.57.0/source/docs/ARCHITECTURE.md | M / 3 |
| TruvisRenderer | Inspectable execution plan; CPU world to GPU world sync docs named but unopened. | https://github.com/acccoco/TruvisRenderer | M / 3 |
| cuneus | Multipass WGSL builder: each pass reads earlier outputs, self-reference for feedback. Good template for bloom, TAA, GTAO. | https://github.com/altunenes/cuneus | S / 3 |
| Godot ShaderPreprocessor | Port `#include` with cycle check, `#define` with args, `#if` stack, file and line regions. WGSL has none. | https://raw.githubusercontent.com/godotengine/godot/master/servers/rendering/shader_preprocessor.h | S / 4 |
| Godot SDFGI | One cascade, scroll with camera, spread convergence over frames. Full SDFGI is over budget. | https://raw.githubusercontent.com/godotengine/godot/master/servers/rendering/renderer_rd/environment/gi.h | L / 2 |
| Solari world cache | Spatial hash of quantized position and normal, luminance blend, expiry. Cache update fell from 1.42 ms to 0.09 ms on Bistro. Take only the cache. | https://jms55.github.io/posts/2025-12-27-solari-bevy-0-18 | L / 4 |
| Bevy contact shadows | Screen-space march over depth; 16 steps, thickness 0.1, length 0.3. Shader body unread. | https://github.com/bevyengine/bevy/blob/main/crates/bevy_pbr/src/contact_shadows.rs | S / 3 |
| Bevy LTC, parallax, SSR | Rect area lights, box-projected probes, SSR. Names only. | https://github.com/bevyengine/bevy/tree/main/crates/bevy_pbr/src | M / 2 |
| Radiance cascades | 3D probes in HLSL (MIT); screen-space probes (no license, approach only); sparse 2D store on wgpu for memory. | https://github.com/alexmalyutindev/unity-urp-radiance-cascades ; https://github.com/CodyJasonBennett/three-rc ; https://github.com/IceKontroI/sparse_rc_2d | M / 3 |
| strolle | wgpu ReSTIR with SVGF denoise; reference only, tuned near 800x600. | https://github.com/Patryk27/strolle | L / 2 |
| FidelityFX Lens | Chromatic aberration, vignette, film grain in one pass. | https://gpuopen.com/manuals/fidelityfx_sdk/techniques/lens/ | S / 4 |
| AMD DepthOfFieldFX | Fast filter spreading DOF; cost does not grow with kernel. HLSL. | https://github.com/GPUOpen-Effects/DepthOfFieldFX | M / 4 |
| G3D motion blur | Tile-max, neighbor-max, gather. U | https://casual-effects.com/g3d/G3D10/build/manual/_motion_blur__neighbor_min_max_8pix.html | M / 3 |
| Meteoros clouds | Nubis-style raymarch with reprojection; under 3 ms at 1080p on a GTX 1070 laptop (README). | https://github.com/AmanSachan1/Meteoros | M / 4 |
| terra (CDLOD) | Rust wgpu terrain LOD and morph math; drop the sphere mapping. Archived Jun 2026, moved to Codeberg. | https://github.com/fintelia/terra | M / 4 |
| big_space | Floating origin as integer cells plus local f32. Needed for big cities. | https://github.com/aevyrie/big_space | S / 3 |
| nanite-webgpu | WGSL meshlet LOD, depth-pyramid occlusion, 12-view impostors. | https://github.com/Scthe/nanite-webgpu | L / 3 |
| meshoptimizer cluster LOD | Group clusters by shared vertices, simplify with locked borders, 1 px error test. | https://zeux.io/2025/09/30/billions-of-triangles-in-minutes/ | M / 4 |
| nvpro occlusion culling | Raster boxes against last depth, bit-pack, indirect draws. | https://github.com/nvpro-samples/gl_occlusion_culling | M / 4 |
| Bevy GPU preprocess | GPU writes indirect args; HZB sample picks mip by box size. | https://github.com/bevyengine/bevy/tree/main/crates/bevy_pbr/src/render | M / 4 |
| wgpu features | `MULTI_DRAW_INDIRECT_COUNT` and `TEXTURE_BINDING_ARRAY` are native-only; check the 3050 at startup. | https://docs.rs/wgpu/latest/wgpu/struct.Features.html | S / 4 |
| wgpu PipelineCache | Vulkan only. On DX12 the stutter fix is to create all pipelines at load. | https://wgpu.rs/doc/wgpu/struct.PipelineCache.html | S / 3 |
| BeamNG texture streaming | Policy: keep high mips under 50% of budget, stop near 90%. A rule to copy. | https://documentation.beamng.com/modding/materials/texture_streaming/ | M / 4 |
| Particle kit | `wgpu_sort` WGSL radix sort (BSD-2); Kenney Particle Pack (CC0); Unity free flipbooks (CC0, EXR or TGA). | https://github.com/KeKsBoTer/wgpu_sort ; https://kenney.nl/assets/particle-pack ; https://unity.com/blog/engine-platform/free-vfx-image-sequences-flipbooks | M / 4 |
| Gore | Dismember by clipping the limb in the skin shader plus a gore cap mesh (Gutbox devlog, U). MCut for mesh cuts (LGPL-3). | https://luzemrik.itch.io/gutbox/devlog/475023/some-info-on-the-gore-tech ; https://github.com/Liudeke/mcut | M / 4 |
| Skin and eyes | Penner pre-integrated skin (Lux port, U); procedural eye (godotshaders, U). | https://godotshaders.com/shader/eye/ | S to M / 3 |
| Material Maker | Offline node graph for worn metal, dirt and cloth masks; bake to PNG. | https://github.com/RodZill4/material-maker | M / 3 |

### Motion, combat, camera

| Item | Take | URL | Cost / Win |
|---|---|---|---|
| Kurble IK | `solver.rs`: pole-target bend plane for knees and elbows. Math only. | https://raw.githubusercontent.com/Kurble/bevy_mod_inverse_kinematics/master/src/solver.rs | S / 3 |
| openik-unity | Damped least squares, swing and twist limits, per-joint speed limit. | https://github.com/WouterApts/openik-unity | M / 3 |
| bevy_animation_graph | State machine whose transitions carry their own graphs. Design only. | https://github.com/mbrea-c/bevy_animation_graph | M / 3 |
| Foot placement | Lower the root by the larger foot offset; gate IK per foot with a plant weight. U | https://docs.vulkan.org/tutorial/latest/Advanced_glTF/Procedural_Animation_IK/04_foot_placement.html | S / 3 |
| MeleeTrace | Sphere traces between socket positions of two frames. | https://github.com/rlewicki/MeleeTrace | S / 3 |
| Sakuga Engine | Frame-locked states, stances, explicit cancel rules, rollback (2D, Godot .NET). | https://github.com/NoisyChain/Sakuga-Engine | M / 3 |
| veil-of-ash | Lock-on framing and poise break rules (0 stars; verify). | https://github.com/XalMorak/veil-of-ash | M / 3 |
| HoloFight | Frame data as JSON, editor draws hitboxes per frame. U | https://omergosh.itch.io/holofight | M / 3 |
| Active ragdoll | Joint targets driven by the animated pose; an impact sets new targets. U | https://github.com/sergioabreu-g/active-ragdolls ; https://github.com/matieme/Active-Ragdoll-Character | M / 3 |
| FootsiesGym | Frame skip, action-delay queue, 6-action set, batched self-play server. Use as a bot test harness (GPL-3). | https://github.com/como-research/FootsiesGym | M / 4 |
| bevy_trauma_shake | `add_trauma`, power 3, decay per second, octaves. 2D crate; port the model. | https://github.com/sjmueller/bevy_trauma_shake | S / 4 |
| Godot SpringArm3D | Sphere motion-cast from pivot each frame; stops at pillars. | https://docs.godotengine.org/en/stable/tutorials/3d/spring_arm.html | S / 3 |
| bevy_director | Shot = rig + eased key + lens; arc-length spline dolly; sequences as RON. | https://docs.rs/bevy_director/latest/bevy_director/ | M / 4 |

### Content (richer)

| Source | What | URL | License |
|---|---|---|---|
| Poly Haven API | HDRIs, PBR sets, models. `/assets`, `/info/{id}`, `/files/{id}`. Send a User-Agent. Use the API, not scraping. | https://polyhaven.com/llms.txt | CC0 |
| ambientCG API v2 | `full_json`, `downloads_csv`. v3 exists; check it for decals. | https://docs.ambientcg.com/api/v2/ | CC0 |
| Quaternius Universal Animation Library | 120+ clips, one humanoid rig, glTF/FBX free. Download buttons go through itch.io; fetch once by hand. | https://quaternius.com/packs/universalanimationlibrary.html | CC0 |
| 100STYLE | 100 locomotion styles, BVH, 1.5 GB, no combat. Needs a retarget. | https://zenodo.org/record/8127870 | CC-BY |
| KayKit | Adventurers, Skeletons, animations, Fantasy Weapons Bits, Medieval Hexagon. Check each pack page. | https://kaylousberg.itch.io/ | check |
| Sonniss GameAudioGDC | Yearly bundles 2015 to 2024, direct zips, mirror and torrent. No AI training. | https://sonniss.com/gameaudiogdc/ | royalty-free |
| Freesound API v2 | Search with the CC0 filter; download needs OAuth2. | https://freesound.org/docs/api/resources_apiv2.html | per sound |
| StarNinjas swords | 10 clashes, 10 attacks. | https://opengameart.org/content/20-sword-sound-effects-attacks-and-clashes | CC0 |
| CMU mocap (4TU FBX) | Martial-arts clips; prototype only. | https://data.4tu.nl/datasets/0448aab2-3332-449f-a8e2-d208cb58c7df/1 | CC-BY-NC |
| Kiln fetch script | Pattern: `assets/models/fetch.sh` downloads CC0 meshes, then bakes them. | https://github.com/zexk/kiln | MIT |
| Retarget | BVH to Mixamo, UE5 and VRM via Blender add-on (U). | https://github.com/topics/motion-retargeting | not seen |

Not found: Smithsonian 3D, Sketchfab CC0, Poly Pizza API, Kevin Iglesias packs, MakeHuman or VRM bases, free music, blood decal sets.

### AI content that fits 6 GB

| Item | Fit | URL |
|---|---|---|
| Stable Audio Open Small | 0.5B, 8 steps, 44.1 kHz stereo, up to 11 s. Gated on HF. Measure VRAM. | https://huggingface.co/stabilityai/stable-audio-open-small |
| UniRig | Auto-skeleton and skin, GLB out. Stated floor 8 GB; test on the 3050. MIT. | https://github.com/VAST-AI-Research/UniRig |
| Hunyuan3D-2mini | Shape only; for 6 GB try the 2GP fork (U). | https://huggingface.co/tencent/Hunyuan3D-2mini |
| Kokoro-82M | Offline TTS via fastkokoro or pykokoro (U); third-party guide says under 2 GB. | https://pypi.org/project/fastkokoro/0.6.1/ |
| MMAudio | Video or text to audio (U). | https://github.com/hkchengrex/MMAudio |
| TexGaussian | Feed-forward PBR from a mesh (U; code unconfirmed). | https://arxiv.org/abs/2411.19654 |

Too big for 6 GB (skip): HY-Motion 1.0 (26 GB), Hunyuan3D-2.1 (10 to 29 GB), SkinTokens (14 GB), Step1X-3D.

### Editor and data

| Item | Take | URL | Cost / Win |
|---|---|---|---|
| Godot UndoRedo | Action = do and undo ops; merge modes fold a gizmo drag into one step. | https://raw.githubusercontent.com/godotengine/godot/master/core/object/undo_redo.h | S / 5 |
| evenorog/undo | `Record` linear, `History` tree; one `Edit` per action; merge for drags. | https://github.com/evenorog/undo | S / 4 |
| ByroRedux | One typed `StudioCommand` enum shared by GUI, CLI and tool clients; stable object IDs. Undo is future work there. | https://github.com/matiaszanolli/ByroRedux/blob/main/crates/sdk/README.md | M / 4 |
| bevy_workbench | egui_tiles layout, game view as texture, Edit/Play/Pause/Stop bar. Port panels. | https://github.com/Bli-AIk/bevy_workbench | M / 3 |
| ID-buffer picking | Separate small target, crop to the cursor pixel, object ID in red and green. U | https://ref.rerun.io/docs/rust/head/re_renderer/draw_phases/picking_layer/index.html | S / 3 |
| egui-snarl | Node graph for VFX or material graphs, serde save. | https://github.com/zakarumych/egui-snarl | S / 3 |
| openusd | Copy the layer rule: every edit goes to a named layer; revert by removing it. Adopt only after 1.0. | https://github.com/mxpv/openusd | S copy / 3 |
| subsecond | Hot patch the per-frame update. Experimental; MSVC support unknown. | https://docs.rs/subsecond/latest/subsecond/ | M / 4 |
| LimboAI | Behavior tree as one state of a state machine; blackboard scopes. Design only. | https://github.com/limbonaut/limboai | M / 4 |
| Cyclops Level Builder | Grid block-out of arenas and streets. U | search listing | M / 3 |
| ProtonScatter | Modifier stack, negative shapes, project-to-hit for debris. | https://github.com/HungryProton/scatter | M / 3 |

### Asset pipeline

| Item | Take | URL | Cost / Win |
|---|---|---|---|
| meshoptimizer + gltfpack | Meshlets, LOD (`-si`), `-c` compression, `-tc` KTX2. Crate `meshopt` 0.6.2. | https://github.com/zeux/meshoptimizer/tree/master/gltf | S / 5 |
| glTF-Transform | optimize, meshopt, uastc, etc1s, resize, prune, dedup. Adds Node to the cook. | https://github.com/donmccurdy/glTF-Transform | S / 4 |
| basisu 0.1.0 | Pure-Rust KTX2 transcoder for runtime: reads a `.ktx2` or `.basis` and transcodes it to BC7, the other BCn, ASTC, ETC, PVRTC, ATC, FXT1 or RGBA at load. Decode side only (the encoder is the C++ `basisu` tool). Repo `marcogomez/basisu`, files `basisu/src/ktx2.rs`, `basisu/src/transcoder.rs`, `basisu/src/dispatch.rs`; Apache-2.0; read live 2026-10-20. | https://github.com/marcogomez/basisu | S / 4 |
| intel_tex_2, block_compression | BC7 on CPU at cook; GPU port in WGSL. U | https://docs.rs/block_compression | S / 4 |
| Symphonia | Pure-Rust decode of Ogg, FLAC, WAV, MP3. | https://github.com/pdeljanov/Symphonia | S / 4 |
| AssetCooker | USN-journal incremental cook driver for Windows. | https://github.com/jlaumon/AssetCooker | S / 4 |
| ufbx | FBX and OBJ loader, two files. | https://github.com/bqqbarbhg/ufbx | M / 3 |
| Byte-Engine | Input intents; authored vs baked resources; `beld` bake CLI. | https://github.com/Game-Tek/Byte-Engine | M / 3 |
| rafx binding codegen | Generate Rust bind-group structs from WGSL reflection (naga) so a mismatch fails at compile time. Idea only. | https://github.com/aclysma/rafx | M / 2 |

### Determinism, replay, netcode

| Item | Take | URL |
|---|---|---|
| renew-engine | `frame` fixed timestep, `trace` input file, `renew determinism --emit` digests on five targets, `--json` with `schema_version`. | https://github.com/renew-engine/renew |
| Box2D determinism | OR-merge of per-worker bit arrays, own trig, `-ffp-contract=off`, CI hash on MSVC, Clang, GCC, x64 and ARM. | https://box2d.org/posts/2024/08/determinism/ |
| Rapier | `enhanced-determinism` excludes `simd8`; trig from nalgebra traits; same insertion order. | https://rapier.rs/docs/user_guides/templates/determinism/ |
| Ikemen GO | `LiveChecksum` (CRC32 over gameplay fields only), `AdvanceFrame`, hand-written clone with shared immutable data. | https://github.com/ikemen-engine/Ikemen-GO/blob/develop/src/rollback.go ; https://github.com/ikemen-engine/Ikemen-GO/blob/develop/src/state_clone.go |
| Factorio desync | Keep local state, reference copy, tagged dump and log; diff dumps. "Heavy mode": save and reload every tick. | https://wiki.factorio.com/Desync |
| s2 DST | Seeded RNG, no default hasher, rerun a seed and byte-diff traces. | https://s2.dev/blog/dst |
| mas-bandwidth/fixed | Q48.16 trig with frozen-hash tests on four toolchains. | https://github.com/mas-bandwidth/fixed |
| WinDbg TTD | Record a failing run, query writes to the diverging field. Windows only. | https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/time-travel-debugging-overview |
| matchbox | WebRTC peer link with a `ggrs` feature; signaling server. Confirm TURN before trusting. | https://github.com/johanhelsing/matchbox |
| netcode.io | Connect tokens, encrypted packets, replay protection. | https://github.com/mas-bandwidth/netcode |
| ioquake3 | `net_chan.c` sequence, fragments, qport; `msg.c` delta fields; `sv_snapshot.c` per-client view. | https://raw.githubusercontent.com/ioquake/ioq3/master/code/qcommon/net_chan.c |
| Lag compensation | Rewind = interpolation delay + half RTT, clamp 500 ms; test at 50, 150, 500 ms. | https://docs.colyseus.io/netcode/lag-compensation |
| GameNetworkingSockets | Reliable lanes, P2P over WebRTC ICE, built-in fake lag. | https://github.com/ValveSoftware/GameNetworkingSockets |
| DarkPlaces demo | Record shape: length, view angles, bytes; playback feeds the live parser. Ours is smaller (input log plus seed). | https://raw.githubusercontent.com/DarkPlacesEngine/Darkplaces/master/cl_demo.c |

Skip: Steam Datagram Relay (Steam only), Agones (needs Kubernetes), SpacetimeDB (logic in the database), Nakama (extra database).

### Physics, destruction, cloth

| Item | Take | URL |
|---|---|---|
| Rapier character controller | `move_shape` returns corrected move plus grounded and sliding flags; autostep; snap to ground. | https://rapier.rs/docs/user_guides/templates/character_controller/ |
| Ten-Minute-Physics-Unity | XPBD rigid body, spatial hash. MIT. | https://github.com/Habrador/Ten-Minute-Physics-Unity |
| jspdown/cloth | WebGPU XPBD with graph-coloured constraints. Port to WGSL. | https://github.com/jspdown/cloth |
| bevy_silk | Pin by vertex id, vertex colour or predicate; paint pins in Blender. Float Verlet needs a fixed order to fit rollback. | https://github.com/ManevilleF/bevy_silk |
| Libre Fracture | Chunk graph with Connected, Broken, Detached, Anchored; BFS from anchors on each break. | https://github.com/HunterProduction/unity-libre-fracture-2.0 |
| boolmesh, manifold3d | Bake-time pre-fracture by intersecting Voronoi cells with the mesh. U | https://docs.rs/crate/boolmesh/latest |
| Voxelis | Rust sparse voxel DAG store with batch edits; no GPU path. U | https://lib.rs/crates/voxelis |
| FallingSandEngine | Chunk tick order reference in Rust. | https://github.com/PieKing1215/FallingSandEngine |
| obvhs | Rust BVH builders for sweeps and crowd broad phase. U | https://rust-digger.code-maven.com/crates/obvhs |
| box3d-rust | Pure-Rust port of Box3D, "port starting". Watch only. | https://docs.rs/crate/box3d-rust/0.0.1 |

### AI: fighters, squads, embedded models

| Item | Take | URL |
|---|---|---|
| FightingICE timing | Fixed 60 fps; AI sees state 15 frames late; a late decision repeats the last input. Reaction time becomes data. | https://ice.ci.ritsumei.ac.jp/~ftgaic/index-3.html |
| bonsai-bt | Tick-driven tree with Timeout, While, Race. | https://github.com/Sollimann/bonsai |
| big-brain | Scorers + first-to-score thinker; personality = weights as data. Archived Oct 2025; port the pattern. | https://github.com/zkat/big-brain |
| noru | i16 quantised inference, offline Adam training, incremental accumulator. Test SIMD vs scalar yourself. | https://docs.rs/crate/noru/latest |
| cppGOAP | Action model and A* planner for squads. | https://github.com/cpowell/cppGOAP |
| recast-rs tilecache | Add and remove obstacles, rebuild touched tiles only. For doors and breaches. | https://github.com/wowemulation-dev/recast-rs |
| PenetrationSystem | Per-material thickness and cost, penetration cap. | https://github.com/Lakamfo/PenetrationSystem |
| CoverGenerator | Cover point tags: crouched or standing, lean flags. | https://github.com/Deams51/CoverGenerator-UE4 |
| geqo | EQS-style generators and tests, time-sliced. Slice by tick for determinism. | https://github.com/geqo-godot/geqo |
| ioq3 botlib | Goal stack and weighted inputs; skip AAS. | https://github.com/ioquake/ioq3/tree/master/code/botlib |
| gemma4-dungeon-master | Turn shape: read-only tools, narration pass, write tools; arguments naming missing entities are rejected. | https://github.com/nickc672/gemma4-dungeon-master |
| NobodyWho | Grammar generated from tool signatures; off-thread chat (EUPL). | https://github.com/nobodywho-ooo/nobodywho |
| llama.cpp router | Stop the server before a play session; sleep mode kept the GPU at 99% (issue 19318, U). | https://github.com/ggml-org/llama.cpp/issues/19318 |
| Yarn Spinner Rust | Dialogue as text files; use the non-Bevy path. | https://github.com/YarnSpinnerTool/YarnSpinner-Rust |
| blade-ink-rs | Pure-Rust Ink runtime. Pick this or Yarn. | https://github.com/bladecoder/blade-ink-rs |
| SUQS | Quest, objective, task data model; ours in RON with a validator. | https://github.com/sinbad/SUQS |
| Forge for Godot | Reference for attribute sets, stacking effects, tags, cues. | https://github.com/gamesmiths-guild/forge-godot |
| bevy_gauge | Stats as expressions, modifier categories, tag queries. Port the model. | https://github.com/DEMIURGE-studio/bevy_gauge |

### Audio, UI, procedural, shipping

| Item | Take | URL |
|---|---|---|
| garjan | Procedural impact (10 materials), friction, footsteps; seeded. GPL-3.0, so a dependency would trip `deny.toml`; port the modal idea. | https://docs.rs/crate/garjan/latest |
| Kira | Mixer, sub-tracks, clocks, tweens (ducking is a tween). | https://docs.rs/kira/latest/kira/ |
| Modal impact synthesis | One recorded clash gives endless varied hits. Method only. | https://www.microsoft.com/en-us/research/?p=186041 |
| Wwise-MCP | Pattern for audio events authored through our MCP door. | https://github.com/BilkentAudio/Wwise-MCP |
| glyphon | wgpu text over cosmic-text; HUD text. | https://github.com/grovesNL/glyphon |
| Taffy | Flex and grid layout; no text. | https://github.com/DioxusLabs/taffy |
| klyff | Pure-Rust GPU MSDF, outline and glow text. Pins wgpu 30. | https://codeberg.org/SnailBionicLab/klyff |
| egui | Editor panels and quick menus. | https://github.com/emilk/egui |
| ghx_proc_gen | Rust 3D WFC; pin a cell and propagate as a constraint check an AI can drive. | https://github.com/Henauxg/ghx_proc_gen |
| shapeml, edifice | Split grammars for facades (GPL-3, PolyForm-NC). Read for operators. | https://github.com/stefalie/shapeml ; https://github.com/DonBeleren/edifice |
| OSM2World | Real street extracts to glTF, offline (Java, MIT). | https://github.com/tordanik/OSM2World |
| fast_poisson | Seeded Poisson-disk scatter. | https://github.com/Kromey/fast_poisson |
| steamworks-rs | Cloud, Input, matchmaking, stats, UGC. Skip its networking. | https://docs.rs/steamworks/latest/steamworks/ |
| Velopack | Installer plus delta updates in one CLI. | https://github.com/velopack/velopack |
| crash-handling | `minidumper` writes the dump from a monitor process. | https://github.com/EmbarkStudios/crash-handling |
| GLFW hybrid-GPU exports | Export `NvOptimusEnablement = 1` and `AmdPowerXpressRequestHighPerformance = 1` from the exe. Rust export path untested. | https://github.com/glfw/glfw/blob/master/src/win32_init.c |
| Steam Deck rules | Controller config reaches all functions, on-screen keyboard, offline single player. | https://partner.steamgames.com/doc/steamdeck/recommendations |
| fluent-rs | Localisation with plurals and pseudo-locale for tests. | https://github.com/projectfluent/fluent-rs |

### Dev speed and QA

| Item | Take | URL |
|---|---|---|
| Bevy fast-builds config | Windows `rust-lld` block; keep `-Zshare-generics` off. | https://github.com/bevyengine/bevy/blob/main/.cargo/config_fast_builds.toml |
| `live` crate | Debounced watcher, TOML tuning files, wait-free reads. | https://docs.rs/crate/live/0.4.3/source/README.md |
| iqa-cli | SSIMULACRA2, Butteraugli, SSIM as JSON. Needs a C++ toolchain. | https://docs.rs/crate/iqa/0.2.0 |
| Gungraun (iai-callgrind) | Instruction-count benches for the sim. Needs Valgrind: Linux or WSL. | https://github.com/iai-callgrind/iai-callgrind |
| Bolero | One harness: `cargo test` and fuzzer. Replay-hash equality as the oracle. Assume WSL. | https://github.com/camshaft/bolero |
| Rerun | Per-tick state to `.rrd`, timeline scrubbing. | https://github.com/rerun-io/rerun |
| RenderDoc Python | Replay a `.rdc` and save a texture for a pixel diff. | https://renderdoc.org/docs/python_api/index.html |
| Rhai ops budget | `set_max_operations`; meter host calls yourself. | https://rhai.rs/book/safety/max-operations.html |
| wasmtime determinism | `cranelift_nan_canonicalization`, `consume_fuel`. | https://docs.wasmtime.dev/api/wasmtime/struct.Config.html |

## Skip list (looks useful, is not)

Blade (raw Vulkan and Metal; wgpu hides barriers), full Solari (needs DLSS-RR, 7 to 14 ms on a 3080), full SDFGI, Bevy BSN (macros, no file loader yet), TressFX (heavy framework, no hair BRDF found), Slint (no wgpu renderer), RmlUi (no wgpu backend, large), DirectStorage (wgpu hides the device), Intel XeSS (binary DLLs), Terrain3D, Beehave (LimboAI covers it), Godot Jolt and Rapier wrappers, Armory3D, Ebiten, Orillusion, SpacetimeDB, Agones, Nakama, Intel MaskedOcclusionCulling (CPU, archived), Mordhau-like games (closed), CMU mocap for shipping (non-commercial), Bandai Namco move set (non-commercial).

## What to board next

Cut S rows first, sized for the Muse loop: one crate, at most 200 changed lines, a named steal file, one test that fails before. Suggested first six: row 2 (golden harness), row 4 (wgpu-profiler), row 8 (fetch-content script), `linkme` registration, GLFW hybrid-GPU exports, row 12 (`llguidance`). Hard rows for a Claude session: row 1 (pass graph), row 3 (upscaler temporal path), row 5 (bake).

# Wave 2 (2026-10-11, 36 more scouts)

Owner added: the target is a potato (weak GPU, RAM and CPU), and every game must still look great. This PC is an i5-13420H (4 P + 4 E cores), 16 GB RAM, RTX 3050 6 GB plus an Intel UHD. The UHD is a real potato test bed. Boarded as `docs/SPRINT-18-steals.md` rows GS1 to GS33; they sit in FOCUS "Now" group 7, after the older groups.

## Correction after reading the code

Wave 1 listed things the engine already has as modules. Checked against `.opencode/modmap.md`: `render/graph.rs` and `executor.rs` (frame graph), `aa_scale.rs` (SMAA plus dyn-res), `baked_gi.rs`, `decals.rs`, `cache.rs` (pipeline cache), `timing.rs` (GPU pass stamps), `tier.rs`, `heal.rs`, `variants.rs`; `game/steam.rs`, `crash.rs`, `director.rs`, `capture.rs`, `camera.rs`; `combat/sweep.rs`, `exchange.rs`, `feel.rs`; `ai/eqs.rs`, `legion.rs`, `utility.rs`, `behavior.rs`; `audio/hrtf.rs`, `occlude.rs`, `reverb.rs`, `crowd.rs`; `assets/bc.rs`, `retarget.rs`; `spatial/sight.rs`, `sample.rs`; the editor already depends on `transform-gizmo` and `egui_dock`. So the job is wiring and proof, not building. That is why GS1 to GS5 come first, and why most wave-1 "build X" items became donor notes instead of rows. The "What to board next" paragraph above is superseded by the shard.

## New facts worth keeping

| Area | Find | URL |
|---|---|---|
| Hybrid GPU | wgpu #257: HighPerformance returned the Intel adapter on Optimus; recipe to export `NvOptimusEnablement` from a Rust exe; DXGI `EnumAdapterByGpuPreference` | https://github.com/gfx-rs/wgpu/issues/257 |
| DX12 safety | wgpu #10547: FXC blob refcount race, 3 of 221 runs corrupt the heap; serialize pipeline creation | https://github.com/gfx-rs/wgpu/issues/10547 |
| DX12 stutter | wgpu #8526: no PIPELINE_CACHE on DX12, so build every pipeline during loading | https://github.com/gfx-rs/wgpu/issues/8526 |
| Software adapter trap | Bevy #13113: an Intel HD silently got the CPU adapter | https://github.com/bevyengine/bevy/issues/13113 |
| Potato floor | `Limits::downlevel_defaults()`: 4 storage buffers, 16 KiB uniform, 2048 px textures; GLES has no storage buffers | https://docs.rs/wgpu-types/latest/wgpu_types/struct.Limits.html |
| Tier tables | Unreal scalability level 0 values; Chromium GPU control list shape; Android device-profile matcher | https://dev.epicgames.com/documentation/unreal-engine/scalability-reference-for-unreal-engine |
| Dyn-res law | deadband, cooldown, panic drop, slow recovery | https://dev.epicgames.com/documentation/en-us/unreal-engine/dynamic-resolution-in-unreal-engine |
| VRAM budget | Dawn residency: reservation = min(budget/2, requested), budget x 0.95 | https://learn.microsoft.com/en-us/windows/win32/api/dxgi1_4/nf-dxgi1_4-idxgiadapter3-queryvideomemoryinfo |
| Asset caps | Unity hero 1500-4000 tris desktop; Unreal mobile 75 bones and 4 influences; Quest 1 busy scene 50-150 draws | https://developers.meta.com/horizon/documentation/unity/unity-perf/ |
| Memory | `dhat-rs` peak gate; `lz4_flex` decompress_into; rpmalloc keeps oversized Windows mappings (measure first); `bumpalo` reset; `memory-stats` for working set | https://github.com/nnethercote/dhat-rs |
| Build speed | rust-lld and `debug = "line-tables-only"` are already in `.cargo/config.toml`; sccache wants one target dir per slot; Defender exclusions for CARGO_HOME, repo and target; Cranelift has no unwinding on Windows; `-Zthreads` gave no gain at 8 threads | https://github.com/mozilla/sccache |
| Wiring | cargo-llvm-cov on the real binary; `cargo-workspace-unused-pub`; `unreachable_pub`; `cargo-bloat --crates`; cargo-modules orphans | https://github.com/taiki-e/cargo-llvm-cov |
| Loop for weak models | Aider repo-map (PageRank under a token budget); Agentless 3-stage localize; mini-swe-agent step and cost limits; cargo JSON as the red/green gate; EvilGenie fake-pass signals | https://github.com/Aider-AI/aider/blob/main/aider/repomap.py |
| Agent doors | Arcane (`get_state`, `step_frames`, `describe_game`, `run_tests`); Hearth (`probe_sweep`, `probe_capabilities`); godot-mcp-pro tool tiers (35 to 187); Blender MCP safe mode | https://github.com/jsvd/arcane |
| GPU offload | niagara `drawcull.comp.glsl` + `depthreduce.comp.glsl`; vkguide cull without a count buffer; Bevy `mesh_preprocess`; `wgpu-primitives` scan and sort | https://raw.githubusercontent.com/zeux/niagara/master/src/shaders/drawcull.comp.glsl |
| GPU cook | `block_compression` (wgpu BC1-BC7 in WGSL); Bevy env-map prefilter pass plan; Betsy; glim Vulkan lightmapper (needs ray query) | https://github.com/hasenbanck/block_compression |
| Animation | Remi123 `PPInertialization3D.hpp` (163 lines, MIT); kinema `blend.cc` (quintic inertializer, additive layer); Fyrox `mask.rs`; Remi123 `MFEvents.hpp` | https://github.com/Remi123/MotionMatching |
| Motion data | ASE: 87 sword-and-shield clips (36 attacks, 9 parries, 6 blocks, no hit-reacts or deaths), NON-COMMERCIAL; ProtoMotions tracker ONNX is 22 MB | https://github.com/nv-tlabs/ASE/tree/main/ase/data/motions/reallusion_sword_shield |
| Wounds | naelstrof SkinnedMeshDecals: per-character UV splat map, per-frame paint cap, evict oldest over the VRAM budget | https://github.com/naelstrof/SkinnedMeshDecals |
| Melee numbers | parry window 4 ticks (0.20 s); hit freeze weak 0-0.03 s, strong 0.05-0.08 s, critical 0.1-0.15 s; Dark Souls 3 telegraph floors 340 / 240 / 170 ms; For Honor guard-break 200 ms; God of War buffered cancel frames 0-21 of 60 | https://www.gamedeveloper.com/game-platforms/anatomy-of-an-enemy-attack-in-dark-souls-3 |
| Feel numbers | trauma^2 shake, decay 1.2-1.5 per s, flash 5 frames, hitstop 0.08 s (5 ticks) | https://github.com/m2-md/game-feel-hitstop-juice |
| Post | Bevy `auto_exposure` histogram (64 bins), SMAA, FXAA; wgpu HDR10 needs `color_space` and your own PQ encode; Tony LUT | https://raw.githubusercontent.com/bevyengine/bevy/main/crates/bevy_post_process/src/auto_exposure/auto_exposure.wesl |
| Cheap tricks | parallax-corrected cubemap; baked cubemap plus one planar mirror; interior mapping in closed form; dithered LOD cross-fade; Superhot three-colour rule; Valheim: light carries low-res textures | https://seblagarde.wordpress.com/2012/09/29/image-based-lighting-approaches-and-parallax-corrected-cubemap/ |
| Voxels | roxlap WGSL two-level DDA; binary greedy meshing 50-200 us per chunk; Zylann floating-chunk contract | https://github.com/cgerikj/binary-greedy-meshing |
| Roman content | Sketchfab CC-BY gladius and Colosseum uids; Poly Pizza Coliseum, Centurion, Arch; Freesound CC0 crowd ids; OGA StarNinjas swords | see row GS23 |
| Bodies | Anny parametric bodies (Apache); VRM 15 required bones as the retarget standard; Blender `-b -P` cook | https://github.com/naver/anny |
| Embedded AI | LFM2-350M GGUF 229 MB; llguidance 6.37 ms per token vs 29.98 for the llama.cpp grammar (A100, JSONSchemaBench); SmolVLM-256M under 1 GB; Supertonic TTS RTF 0.012-0.015 on an M4 Pro | https://huggingface.co/LiquidAI/LFM2-350M-GGUF |
| Highlights | camera-planning patent: 5 distances x 8 angles, Markov transitions, 30 degree continuity; Virtual Cinematographer idioms as a state machine | https://patents.google.com/patent/US9367942 |
| CQB | Xonotic bot aim curve; esyfxy awareness ladder (5 states, 300-tick memory); yapb hearing table | https://raw.githubusercontent.com/yapb/yapb/master/src/sounds.cpp |
| Level tools | ericw `vis.cc` portal PVS; visibility polygon (CC0); csgrs Rust booleans | https://github.com/ericwa/ericw-tools/blob/master/vis/vis.cc |
| Data | schemars + boon + serde_path_to_error + miette; Factorio staged data lifecycle; RimWorld patch ops (make zero matches an error) | https://github.com/GREsau/schemars |
| Virtual textures | Sean Barrett SVT write-up; shlomnissan virtual-textures (page cache first) | https://silverspaceship.com/src/svt/ |
| Design | Ludus Magnus class table (Retiarius 0.5x + 80% net, Samnite 2.2x + 30% stun, Thraex 1.8x + 50% bleed); Nidhogg tug-of-war pacing; Mordhau stamina: feint 10, morph 7 | https://github.com/todorivanov/ludus-magnus/blob/main/src/data/combat.ts |
| Splats | SPZ format is about 10x smaller than PLY; `wgpu-3dgs-viewer`; RTX neural texture compression is NOT potato (sample-time wants Ada) | https://github.com/nianticlabs/spz |

## Wave 2 skip list

Cranelift for dev builds (no unwinding on Windows), `-Zthreads`, Dev Drive (Windows 11 only), mimalloc and snmalloc (no Windows numbers), HY-Motion and Hunyuan3D 2.1 (too big), ASE and AMASS clips for shipping (non-commercial), RTX neural shaders and sample-time NTC (need tensor hardware), Practical-RIFE frame interpolation (adds latency), Steam Audio (heavy C library), GlitchTip (needs a server), SmolVLA, `cargo-mutants` as a wiring gate (counts unwired code as caught).

## Gaps wave 2 could not fill

The search quota ran out in many scouts, so these stay open: Intel iGPU bandwidth numbers, MSAA vs FXAA cost on weak GPUs, a published sub-2B tool-call benchmark on this CPU, rayon thread pinning on P/E cores, Mordhau and Chivalry parry frames, a commercial-safe sword set with hit reactions and deaths, SWAT-style breach order and surrender rules, shot-length and cut-timing numbers for auto-editing, VRAM per million splats.
