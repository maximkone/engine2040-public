# Landscape ideas — the backlog from 74 scouts

Plain summary: 74 Haiku scouts (2026-10-11) read other engines and tools on GitHub at any star count. 33 ideas became rows (`potato.md`, `docs/SPRINT-18-steals.md`). This file keeps the rest, so a planner run can promote them. Nothing here is a row yet. The rule from `frontier.md` applies: every idea earns a spike (one test, one baseline, a kill rule) before it becomes a row.
Tag LATER. Prefix for promoted rows: GS (continue after the highest ID in `docs/SPRINT-18-steals.md`). Full sources: `docs/research/steals-2026-10-11/engine-landscape.md`.
Tier: P = potato fit, C = comfortable only. Size: S under 1 day, M days, L a week or more. Verified items were opened by a scout; U = search result only.

How a planner run promotes an idea: read the donor file, check `.opencode/modmap.md` for an existing module, write one row (one crate, at most 200 changed lines, a named donor file, a test that fails before), append it to the shard, add the ID to FOCUS group 7, delete the line here. Kill rule for every cluster: if the spike shows no change in the number it targets, or costs more than 1 ms / 50 MB on the potato tier, delete the idea.

## A. Render and look

| ID | Idea | Donor | Gate | Tier / size |
|---|---|---|---|---|
| LI1 | Temporal upscaler in WGSL (EASU/RCAS plus temporal path) to render 720p and upscale | github.com/pmndrs/upscaler ; SnapdragonGameStudios/snapdragon-gsr `sgsr/v1` | G2 | P, M |
| LI2 | Port Babylon.js WGSL shaders (clustered lights, OpenPBR set, ssao2, ssr2, tonemap, volumetric light) | BabylonJS/Babylon.js `ShadersWGSL` | G3 G4 | C, M each |
| LI3 | Port PlayCanvas WGSL chunks (aniso GGX, sheen, LTC area light, VSM) | playcanvas/engine `shader-lib/wgsl/chunks/lit/frag` | G4 | C, M |
| LI4 | Hair strands Kajiya-Kay plus Marschner in WGSL | Scthe/frostbitten-hair-webgpu | G4 | C, M |
| LI5 | Clear coat and iridescence WGSL for lacquered armor | patriciogonzalezvivo/lygia `lighting` | G4 | P, S |
| LI6 | Pre-integrated skin (curvature LUT) and eye shader | Penner SIGGRAPH 2011; larsbertram69/Lux_2.0 (U) | G4 | C, M |
| LI7 | Auto exposure by 64-bin histogram | bevy `auto_exposure.wesl` | G3 | P, S |
| LI8 | FXAA single pass as the cheapest AA; SMAA already in `aa_scale.rs` | bevy `fxaa.wesl` | G2 | P, S |
| LI9 | Chromatic aberration, vignette, grain in one pass | AMD FidelityFX Lens | G3 | P, S |
| LI10 | Bokeh depth of field by fast filter spreading | GPUOpen-Effects/DepthOfFieldFX | G3 | C, M |
| LI11 | Parallax-corrected cubemap for armor and walls | Lagarde 2012 | G3 | P, S |
| LI12 | Baked cubemap plus one planar mirror for wet sand | ARM combined reflections | G3 | P, M |
| LI13 | Interior mapping for stand windows | mohsenheydari/three-interior-mapping | G3 | P, S |
| LI14 | Ambient cubes / SH probes for fighters | bevy `irradiance_volume` | G3 | P, S |
| LI15 | GPU IBL prefilter pass plan (move the CPU bake to the GPU) | bevy `light_probe/generate.rs` | G2 | P, M |
| LI16 | GPU BC1-BC7 encoder for the cook | hasenbanck/block_compression | G2 | P, S |
| LI17 | Cascaded shadow cost tricks: 1 cascade plus blob shadows on potato | Unreal sg level 0 | G9 | P, S |
| LI18 | Half-res AO with temporal filter | Battlefield 3 stable SSAO (U) | G9 | P, M |
| LI19 | Radiance cascades (3D probes) for dynamic bounce | alexmalyutindev/unity-urp-radiance-cascades | G3 | C, L |
| LI20 | Cloud raymarch with reprojection | AmanSachan1/Meteoros | G3 | C, M |
| LI21 | CDLOD terrain, floating origin (big cities) | fintelia/terra ; aevyrie/big_space | later | C, M |
| LI22 | Meshlet cluster LOD cook | zeux/meshoptimizer `clusterlod.h` | G2 | C, L |
| LI23 | Raster occlusion culling | nvpro-samples/gl_occlusion_culling | G2 | C, M |
| LI24 | Depth pyramid build (one mip per dispatch) | zeux/niagara `depthreduce.comp.glsl` | G2 | P, S |
| LI25 | 16-bit varyings to save bandwidth (check wgpu f16 support first) | Vulkan 16bit_storage sample | G9 | P, M |
| LI26 | Octahedral probes and half-res SSR as the first fallback | Godot 4.6 PRs 107902, 111210 | G9 | P, M |
| LI27 | Dithered clustered decals with a texture-array slot per decal | bevy `decal/clustered.wesl` | G3 | C, M |
| LI28 | GPU radix sort for particles | KeKsBoTer/wgpu_sort ; samjsui/wgpu-primitives | G3 | C, M |
| LI29 | Flipbook smoke and fire sheets (CC0) | Unity free VFX image sequences | G3 | P, S |
| LI30 | Skin-shader dismemberment: clip the limb, cap the hole | Gutbox devlog (U) | G4 | P, M |
| LI31 | Voxel destruction: two-level DDA, binary greedy meshing, floating-chunk contract | NCrashed/roxlap ; cgerikj/binary-greedy-meshing ; Zylann `separate_floating_chunks` | Bodies | C, L |
| LI32 | Gaussian-splat backdrops stored as SPZ (offline train with Brush) | nianticlabs/spz ; LioQing/wgpu-3dgs-viewer | later | C, M |
| LI33 | Virtual-texture page cache | shlomnissan/virtual-textures ; Barrett SVT write-up | G9 | P, M |
| LI34 | Texture mip-bias fallback when VRAM is tight | Unreal `r.Streaming.MipBias` | G9 | P, S |
| LI35 | Offline GPU lightmap and probe bake (needs Vulkan ray query) | clienthax/glim | G3 | C, L |

## B. Motion, combat, feel

| ID | Idea | Donor | Gate | Tier / size |
|---|---|---|---|---|
| LI40 | Inertialization as a post step (163 lines) | Remi123/MotionMatching `PPInertialization3D.hpp` | G5 | P, S |
| LI41 | Quintic inertializer and additive hit layer | Force67/kinema `blend.cc` | G5 | P, S |
| LI42 | Layer mask with sorted excluded bones | Fyrox `fyrox-animation/src/machine/mask.rs` | G5 | P, S |
| LI43 | Animation events: "time to next footstep" query | Remi123 `MFEvents.hpp` | G5 | P, S |
| LI44 | Two-bone IK with pole target | TheComet/ik ; Kurble `solver.rs` | G5 | P, S |
| LI45 | Damped least squares IK with joint limits | WouterApts/openik-unity | G5 | P, M |
| LI46 | Godot 4.6 IK post-solve modifiers (limit angular velocity, twist disperser) | godotengine.org IK article | G5 | P, M |
| LI47 | Physics-based fighter: PD drives and balance tuning set | matieme/Active-Ragdoll-Character | Bodies | C, L |
| LI48 | In-game policy tracker via ONNX (22 MB example) | ProtoMotions G1 tracker | later | C, L |
| LI49 | Hit-freeze from damage, integer (Smash formulas, cap 20) | ssbwiki Hitlag | G5 | P, S |
| LI50 | Hit-stop tiers: weak 0-0.03 s, strong 0.05-0.08 s, critical 0.1-0.15 s; stop only first and last hit of a combo | uhiyama-lab hit feedback | G5 | P, S |
| LI51 | Trauma shake: squared trauma, decay 1.2-1.5 per s, 5-frame flash | m2-md/game-feel-hitstop-juice ; CodingQuests/godot-game-feel-lab | G5 | P, S |
| LI52 | Parry window 4 ticks, consecutive 3 ticks, deflect at 75% speed | Luk's Perfect Parry | G5 | P, S |
| LI53 | Five-phase attack telegraph with floors 340 / 240 / 170 ms | Anatomy of an Enemy Attack in DS3 | G5 | P, S |
| LI54 | Cancel windows per move: `cancel_on`, `cancel_off`, buffered or instant | Combat Canceled (God of War) | G5 | P, S |
| LI55 | Guard-break vulnerability 200 ms and feint rules | For Honor patch 2.63.1 | G5 | P, S |
| LI56 | Mordhau-style stamina: feint 10, morph 7, drain only on a miss | magicgameworld Mordhau guide | design | P, S |
| LI57 | Camera priority stack and kill-cam by priority | ramokz/phantom-camera | G5 | P, M |
| LI58 | Camera rig crate for the third-person follow | h3r2tic/dolly | G5 | P, S |
| LI59 | Shot scoring for an auto-director (5 distances x 8 angles, Markov transitions) | US patent 9367942 | later | P, M |
| LI60 | Highlight picks from sim events, not vision | HeroesReplay ; CGodiksen/highlightly | later | P, M |
| LI61 | Offline frame writer with locked audio block | Godot `movie_writer_pngwav.cpp` | later | P, S |

## C. AI, audio, UI

| ID | Idea | Donor | Gate | Tier / size |
|---|---|---|---|---|
| LI70 | Behavior trees with Timeout decorators for telegraph timers | Sollimann/bonsai | AI door | P, S |
| LI71 | Utility scorers as data: personality = weights | zkat/big-brain | AI door | P, S |
| LI72 | i16 quantised learned evaluator in the tick | nicotina04/noru | AI door | P, M |
| LI73 | Bot training harness with action delay and frame skip | como-research/FootsiesGym | AI door | P, M |
| LI74 | Skill-to-aim-error curve on one axis | Xonotic `aim.qc` | AI door | P, S |
| LI75 | Cover points scored on six terms, exclusive claim with expiry | SimulatedFlow/ue-plugin-CoverPoints (read only) | later | P, M |
| LI76 | Dynamic navmesh obstacles for doors and breaches | wowemulation-dev/recast-rs `waymark-tilecache` | later | P, M |
| LI77 | Wall penetration by material | Lakamfo/PenetrationSystem | later | P, S |
| LI78 | Flow field with bounded steps per tick | Anuken/Mindustry `Pathfinder.java` | Bodies | P, S |
| LI79 | Formation offsets and ORCA avoidance | 0ad `Formation.js` ; dodgy | later | P, M |
| LI80 | Dialogue as text | YarnSpinner-Rust ; bladecoder/blade-ink-rs | later | P, M |
| LI81 | Quest, objective, task data model | sinbad/SUQS | later | P, S |
| LI82 | Attribute and effect layer (stacking, tags, cues) | gamesmiths-guild/forge-godot (read the design) | later | P, L |
| LI83 | Stats as expressions with modifier categories | DEMIURGE-studio/bevy_gauge | later | P, M |
| LI84 | Modal impact synthesis from one recorded clash | Raghuvanshi and Lloyd (Crackdown II) | G5 | P, M |
| LI85 | Crowd murmur from a fixed grain pool | fundsp `granular.rs` (rewrite without the heap) | later | P, M |
| LI86 | Wwise-MCP style audio events as data through the door | BilkentAudio/Wwise-MCP | AI door | P, M |
| LI87 | Pooled damage numbers with a spawn budget and merged DOT ticks | SimulatedFlow/ue-plugin-HitTicker (U) | G5 | P, S |
| LI88 | MSDF outlined text for the HUD | klyff_msdf ; grovesNL/glyphon | later | P, M |
| LI89 | Flex/grid layout for HUD and menus | DioxusLabs/taffy | later | P, S |
| LI90 | Localisation with plurals | projectfluent/fluent-rs | ship | P, S |

## D. Tools, loop, data

| ID | Idea | Donor | Gate | Tier / size |
|---|---|---|---|---|
| LI100 | Repo map by PageRank under a token budget for each packet | Aider `repomap.py` | loop | P, M |
| LI101 | Three-stage localisation: file, item, line | OpenAutoCoder/Agentless `localize.py` | loop | P, S |
| LI102 | Seat kill-switches: 3 format errors or a cost cap stops it and requeues | SWE-agent/mini-swe-agent `default.py` | loop | P, S |
| LI103 | `cargo check --message-format=json` as the red/green gate | cargo external-tools doc | loop | P, S |
| LI104 | Claims in one SQLite file plus a landing queue | alwh1te/agent-semaphore (U) | loop | P, M |
| LI105 | Unused-pub across the workspace with a SCIP index | cpg314/cargo-workspace-unused-pub | wiring | P, M |
| LI106 | `unreachable_pub` and `dead_code_pub_in_binary` lints | rustc lint list | wiring | P, S |
| LI107 | Orphan files and unused deps | cargo-modules `orphans` ; cargo-shear | wiring | P, S |
| LI108 | Same-seed trace diff in CI (no default hasher) | s2.dev/blog/dst | G6 | P, S |
| LI109 | Cross-compiler determinism CI (`-ffp-contract=off`, own trig) | box2d.org determinism post | G6 | P, M |
| LI110 | Fixed-point trig with frozen hashes | mas-bandwidth/fixed | G6 | P, M |
| LI111 | Gameplay-only checksum with an exclusion list | Ikemen GO `rollback.go` | G6 | P, S |
| LI112 | Heavy-mode desync check: save and reload every tick | wiki.factorio.com/Desync | G6 | P, S |
| LI113 | Record/replay file with `--json` and `schema_version` | renew-engine/renew | G6 | P, S |
| LI114 | Schema from types, validate on load, error path and fix hint | schemars ; boon ; serde_path_to_error ; miette | AI door | P, M |
| LI115 | Defs with inheritance and patch ops (zero matches is an error) | RimWorld PatchOperations ; Factorio data lifecycle | AI door | P, M |
| LI116 | Door verb set: `get_state`, `step_frames`, `describe_game`, `run_tests` | jsvd/arcane | AI door | P, S |
| LI117 | Tool tiers by flag so small models see fewer tools | youichi-uda/godot-mcp-pro | AI door | P, S |
| LI118 | Hot patch gameplay code | subsecond (Dioxus) | dev | P, M |
| LI119 | Sandboxed scripts: op budget (Rhai), fuel (wasmtime) | rhai.rs ; docs.wasmtime.dev | later | P, M |
| LI120 | Level text: portal PVS bake, CSG blockouts | ericwa/ericw-tools `vis.cc` ; timschmidt/csgrs | later | P, M |
| LI121 | Hash-skip incremental cook | Bevy AssetProcessor ; AssetCooker USN journal | dev | P, M |

## E. Content, ship, embedded AI

| ID | Idea | Donor | Gate | Tier / size |
|---|---|---|---|---|
| LI130 | Quaternius Universal Animation Library as the first clip set | quaternius.com | content | P, S |
| LI131 | 100STYLE locomotion (CC-BY) for blends | zenodo 8127870 | content | P, M |
| LI132 | VRM 15-bone map as the retarget standard (use the existing `assets/retarget.rs`) | vrm-c/vrm-specification | content | P, S |
| LI133 | Parametric crowd bodies at cook time | naver/anny | content | C, M |
| LI134 | Auto-rig imported meshes (8 GB floor stated; test on the 3050) | VAST-AI-Research/UniRig | content | C, M |
| LI135 | Batch SFX from prompts | stabilityai/stable-audio-open-small | content | C, M |
| LI136 | Offline TTS for barks | Kokoro-82M ; Supertonic | content | P, M |
| LI137 | CC0 sword/impact audio and GDC bundles | Sonniss GameAudioGDC ; Freesound CC0 | content | P, S |
| LI138 | Steam page items: achievements, cloud, Input (`steamworks` crate behind the `steam` feature) | Noxime/steamworks-rs | ship | P, M |
| LI139 | Installer and delta updates | velopack/velopack | ship | P, M |
| LI140 | Crash dumps from a monitor process | EmbarkStudios/crash-handling `minidumper` | ship | P, S |
| LI141 | Borderless flip-model default, DPI manifest, battery poll | Microsoft DXGI and HiDPI docs | ship | P, S |
| LI142 | Raw mouse from `DeviceEvent::MouseMotion` | winit docs | ship | P, S |
| LI143 | Small text model: LFM2-350M GGUF 229 MB for NPC lines | huggingface.co/LiquidAI/LFM2-350M-GGUF | AI door | P, S |
| LI144 | Grammar-constrained output at 6.37 ms per token | guidance-ai/llguidance | AI door | P, S |
| LI145 | Announcer authored once as data with triggers and a last-played throttle | abadugu13/football_llm_commentary | AI door | P, S |
| LI146 | Offline look judge | Qwen3-VL-2B GGUF (1.11 GB) ; SmolVLM-256M (under 1 GB) | G4 | P, M |
| LI147 | STT for voice commands | UsefulSensors/moonshine-streaming-tiny | later | P, M |
| LI148 | Class and matchup table shape for a ludus loop | todorivanov/ludus-magnus `combat.ts` | design | P, S |
| LI149 | Injury-ended duels and multi-angle replay | Gladio Mori README | design | P, M |

## F. Do not take (kept so nobody re-researches them)

Solari full path, SDFGI, Blade, DirectStorage, XeSS (binary), TressFX, Slint, RmlUi, ASE and AMASS clips for shipping (non-commercial), Hunyuan3D 2.1 and HY-Motion (too big), RTX neural shaders (tensor hardware), Practical-RIFE, GlitchTip, Steam Datagram Relay, Agones, Nakama, SpacetimeDB, cargo-mutants as a wiring gate, Cranelift dev builds on Windows, `-Zthreads`.
