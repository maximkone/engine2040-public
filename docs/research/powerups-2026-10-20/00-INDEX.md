# Powerup wave index (2026-10-20): every finding, ranked by what it closes

The lead's index over 15 lanes. One row per find, ranked: what closes a finish-line gap or a live gate
first, then cheap. "State" says how ready it is: LANDED (code in the tree), SPEC (patch-ready spec written,
waiting for a crate), ROW (proposed row written, no code), RESEARCH (read it, no bytes).

Finish-line gaps: A1-A6 (one game, two targets), B7-B9 (looks good, proven with numbers), C10-C15 (hard
systems in one call), D16-D20 (the AI builds, tests and fixes), E21-E23 (engine core).

## Ranked queue

| # | What | Closes | Cost | Donor (licence) | Crate | State |
|---|---|---|---|---|---|---|
| 1 | The lit frame, cut 1: GPU motion vectors (a velocity target the TAA reads) | LF5, B7 | S-M | Falcor VBuffer (BSD-3) | render | SPEC |
| 2 | The lit frame, cut 2: bins with a transform, GPU cull, indirect draws | RL60, RL32, GS18b, B7 | M | renderling, Rust and wgpu (MIT OR Apache-2.0) | render | SPEC |
| 3 | The lit frame, cut 3: depth and normal prepass, AO on ambient only | RL41, B7 | M | Falcor GBuffer (BSD-3) | render | SPEC |
| 4 | The lit frame, cut 4: TAA as a real pass | RL38, B7 | M | Falcor TAA (BSD-3) | render | SPEC |
| 5 | The lit frame, cut 5: sun shadows, static arena map plus fighter map, PCF, 2 cascades | RL35, QA4 | M | TheRealMJP/Shadows (MIT), sirhco/verve (Apache-2.0) | render | SPEC |
| 6 | One-call physical effects: body.toml, fx.ragdoll / fx.limb / fx.break / fx.blast | C10, RZ rows | M | our own physfx | physfx, endpoint | ROW |
| 7 | Cloth collisions: collider layer plus max_distance, intercollision particle set, swept particle test, CLOTH and SOFT layer bits | PX27, PX28, PX29, PX35 | M | Bullet btSoftBody clusters, btBroadphaseProxy (zlib) | physics | SPEC + landing |
| 8 | The four MCP resources, the five scenario prompts, progress notifications | D16 | M | dcc-mcp-core (MIT) | endpoint | landing |
| 9 | Verifier output in every build verb, per-instance programmatic grading | D19, D18 | S-M | GamePhanes (MIT), SWE-bench harness (MIT) | endpoint | SPEC |
| 10 | The job pool in the sim: fork_join_on over the shared pool, golden equal at 1 and 8 workers, a loom model | E21, RC36 | M | our own ecs | ecs, arena | landing |
| 11 | Runtime state injection plus keypoint verification | D19, D18 | M | GameGen-Verifier (MIT) | playtest | landing |
| 12 | Every edit re-runs the recorded session from tick 0; every file format as JSON Schema | D19, D16 | M | loom-engine (MIT OR Apache-2.0) | endpoint | ROW |
| 13 | The auto-exposure GPU histogram: 64 bins, one-frame-late readback, EV into the chain | GS44, G3 | M | bevy auto_exposure (already read) | render | landing |
| 14 | The page draws the real cooked fight, not stand-ins; parry and block cues; SSIM frame gate | A1, WEB16, WEB17, WEB5 | M | our own web | web, assets | ROW |
| 15 | Content by fetch: AssetSource seam, LoadQueue with priority and byte budget, generational handle with unload | A2, WEB6 | M | our own assets | assets, game | landing |
| 16 | Heals that prove themselves: digest equality, attempt budget, VerifiedRecovery, rollback, per-minute cap, frozen frame is never green | AD14, E75, G1, A9 | M | resilience4j, Polly (shape only) | observe | landing |
| 17 | The plan trace: one brain, BT per tick, GOAP on a goal change, blackboard and goal stack feed it | GA11, GA12, AD5 | M | our own ai | ai | landing + spec |
| 18 | Rotation-additive hit flinch: delta = t * conj(r), applied base * delta, weight through graph::nlerp | AM23, C14 | S | three.js AnimationUtils (MIT) | anim | SPEC |
| 19 | The machine look score: reference captures per tier, view.compare returning SSIMULACRA2 | B8, G4 | M | rust-av/ssimulacra2 | render, playtest | ROW |
| 20 | Web audio on an AudioWorklet, with a degrade path when cross-origin isolation is off | A5, WEB14 | M | RustAudio/cpal (Apache-2.0) | web, audio | landing |
| 21 | BVH motion intake as a std-only Rust parser (no C++ dependency) | NS19 | M | assimp BVHLoader as the format spec only (BSD-3) | assets | SPEC |
| 22 | Weapons and shields on hand bones: a socket under a bone, world transform kept | EG40, LF12 | M | three.js Object3D.attach (MIT) | game, anim | SPEC |
| 23 | One feature table for the ladder: quality.rs and tier.rs become one, E2040_LADDER | GS35 | S | Dawn Features.h (BSD-3) | render | SPEC |
| 24 | The pass report validates the plan: a read with no writer fails with the pass and the texture named | GS45 | S | zenfg, Granite | render | SPEC |
| 25 | The input layer reaches the tape: ActionSetStack gets a consumer and the layer is hashed | IN-L1 | S | our own input | input | SPEC |
| 26 | Colour: ciede2000 in the material compare, one ACES already gated | RL34 | S | our own material | material | landing |
| 27 | KTX2 with Basis for the web bundle; count draw calls and CPU ms in the web receipt | QW5, QW10 | M | marcogomez/basisu (Apache-2.0), Babylon instrumentation (Apache-2.0) | assets, web | SPEC |
| 28 | The one-call project door: project.new, project.build, project.run; scene diff; open-prompt eval | C12, FU2, FU3 | M | our own endpoint | endpoint | SPEC |
| 29 | Part sets cook to one union pose with a per-part triangle budget, then into destruction chunks | GN5, GN6 | M | our own geo; PartCrafter is research-only (licence) | geo, assets | LANDED (geo) + SPEC |
| 30 | A licence gate that reads the model card and fails closed, plus a job runner with a run id | GN7, GN8 | M | kaggle-cli (Apache-2.0) | center | LANDED (center/make.mjs) |
| 31 | Path-traced reference on WebGPU, and the probe-grid GI shape (donors unlicensed, ideas only) | RT7, RT13, RT4, RT8 | M | strahl (MIT); Split Radiance Cascades, ideas only | render | SPEC |
| 32 | Radiance cache with light and no bake; SVGF denoiser | RT4, RT8, RT6 | M | AMD FSR SDK (MIT), Falcor SVGFPass (BSD-3) | render | RESEARCH |
| 33 | Hair and skin: LTC strand shading for the hair half | VX3 | M | selfshadow/ltc_code (BSD-3) | render | RESEARCH |
| 34 | Volumetric clouds | VX2 | M | Meteoros (MIT) | render | RESEARCH |
| 35 | Frame generation | FG1 | M | AMD FSR3 frame interpolation (MIT) | render | RESEARCH |
| 36 | Spatiotemporal sampling, the reference for the TAA pass | RP1, RL38 | S-M | electronicarts/fastnoise (BSD-3) | render | ROW |
| 37 | Bounded agent memory: a retriever with a write gate | D18, LAI7 | S-M | AgenticSTS (AGPL mod flagged, shape only) | provider | SPEC |
| 38 | VRAM budget to layer count for the arbiter | OPT27, LAI7 | S | llama.cpp GGUF constants (MIT) | provider | ROW |
| 39 | ORCA gets a production caller, so the crowd moves | NV4, NV10 | M | snape/RVO2 (Apache-2.0) | nav | SPEC |
| 40 | The playtest crate reaches the door as a task suite with programmatic referees | D18, HS20 | M | GamePhanes (MIT) | playtest, endpoint | ROW |
| 41 | Layers: the crate layering rule gains sim-may-not-import-presentation, GPU-only-under-present, per-module IO | CQ20 | S | our own workspace-check | tools | SPEC |
| 42 | engine-anim is data, not presentation: move it a layer down so physics may depend on it | CQ20 | S | our own | crates | ROW |
| 43 | Lip sync: phoneme segments to viseme curves, and a mouth-shape table per phoneme | LAI3 | S-M | OpenFaceFX (MIT), Rhubarb Lip Sync (MIT) | anim | ROW |
| 44 | The state machine and blend tree as JSON data | EG18b | S | Piston skeletal_animation | anim | ROW |
| 45 | The debug kit: cvars, an in-game console, pause, step, time scale, one Perfetto trace | finish-line item 13 | M | our own | editor, game | ROW |
| 46 | Dead code lands: about 4,334 lines with no production caller, 7 dead feature flags | AD5 | S-M | our own | several | ROW + landing |
| 47 | One crate per agent: a per-crate packet under 6,000 tokens; modmap is 36,154 tokens; docs/archive is 76.7% of docs | OPT29 | S | our own | tools, docs | ROW |
| 48 | Split the three heavy crates with numbers before and after | OPT11 | L | cargo-llvm-lines (MIT OR Apache-2.0) | render, endpoint, game | ROW |
| 49 | Feature unification: workspace-hack gets a dependent, then the 5 worst multi-version names | OPT8 | M | cargo-hakari (MIT OR Apache-2.0) | workspace | ROW (speedup lane) |
| 50 | A compile cache across the lane pools | OPT9 | M | sccache | tools | ROW (speedup lane) |

## What the lanes found already in our tree (do not re-mine)

Motion matching, springs, inertialization, IK and FABRIK, the quaternion pack lanes, the clip edit verbs,
lipsync and viseme, crowd scaling, the JSON anim graph (anim). Strict WebGPU CI, browser netcode,
cross-origin isolation, the wasm size tools, the quality ladder and the potato gate (web). A keypoint
verifier (playtest). The auto-exposure CPU half and the 64-bin histogram kernel (render). The retarget
cook, the CC0 intake, the provenance rows and the LOD bake (assets). The LOD chain: 4 meshes, LOD0 1448 to
2232 tris, ratios 1 / .5 / .25 / .125, cook x2 (content). A load-time validator on the BT (ai). The rule
tooling in workspace-check (tools).

## Decisions taken (00-DECISIONS.md)

D1 the duel brain is GA11 BT per tick plus GA12 GOAP on a goal change, one consumer, nothing deleted.
D2 forge's unreachable shelf feeds the Rome arena, no new project.
D3 the center arsenal is a contract, so a tool with no run file is a bug to fix, not a choice.

## The corrections that must land in the board (00-CORRECTIONS.md)

nav::crowd does not exist. import_repair is wired. engine-playtest has a consumer. ai::utility is live but
internal. Cloth does take colliders. The SMOOTH-BUILD dependency numbers were build artifacts. EG24's .e2nn
is on disk. content/reference/ does not exist. GN4/GN5/GN7/GN8 point at a deleted file. WD3/WD12/WD14/WD15
are mostly in the tree with stale row text. GS45, UW12/UW40 and the input layer are landed with no consumer.
