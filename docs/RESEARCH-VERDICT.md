# RESEARCH-VERDICT — which researched ideas go into the product

Decided 2026-09-23; later sweeps are dated by section. The full prose, tables and card text are archived at `docs/archive/2026-10-10/RESEARCH-VERDICT-prose.md`. Read that file for the reasons behind a verdict.

**IN** = on the finish-line path at the named milestone. **LATER** = good, after the named point. **DROP** = not for this product. Catalog rows not listed here default to LATER. This file, not the catalogs, decides what gets built. Nothing marked DROP lands.

Line format: `ID: VERDICT — one short reason`. The ID is the board or card ID when the source gives one. Otherwise it is a short slug of the idea, or the repo name. Source words map to: TAKE, PATTERNS, REPRODUCE, DONE, HAVE IT, GOLD → IN; OPEN, PARKED, HELD, RES-STEAL, UNVERIFIED → LATER; REJECT, CUT, DO-NOT-TAKE, DECLINED, KILLED, MOOT, no LICENSE file → DROP.

Competitors already expose undo and approval flows. Claim a gap only after checking.

## Verdict

- Core thesis: an AI-agnostic editor door (typed verbs from one schema, journaled writes with rollback, spatial checks, progressive discovery, honest refusals). Build natively in M1–M2.
- engine2040 over Forge plus Flax: MIT ownership, deterministic headless simulation with replay, AI command service built in.

## AI-agnostic core

- schema-tools-docs: IN — N3 command service, N8 MCP
- journaled-write-path: IN — N3; one patch with many ops, rollback on mismatch
- closed-enums-typed-errors: IN — N3, N8; caller vs engine fault codes
- receipts-refusals: IN — N3, N8; refusals name a recovery step
- progressive-tool-discovery: IN — N8; tool-list token budget as a CI gate
- health-summary-orientation: IN — N8 (`health.summary` / doctor)
- mcp-loopback-token: IN — N8; token door for any external agent
- provider-adapters: IN — M2; mock, local, remote; not implemented yet
- propose-then-dispose: IN — M2; minimal conflict set on failure
- model-off-tick: IN — law (HASH-LAW, VISION); recorded and replayed
- openai-compatible-http-abstraction: DROP — as the abstraction; fine as one adapter

## Spatial report

- spatial-deterministic-checks: IN — M2 gate on every AI edit; engine-spatial has 22
- scene-relations: IN — after the scene crate (N2), in M2 (spatial-100x P1)
- models-labels-only: IN — law (Forge decision 0129)
- spatial-model-context: IN — SP4 brief plus MCP2 captures (operator 2026-09-23)
- incremental-spatial-db: IN — SP3 rebuilds changed subtrees; sleeping statics LATER
- intent-placement-solver: IN — SP5, MCP3 (ConstraintLLM, SpatialGrammar)
- ledger-learner: LATER — tunes thresholds per name family; needs real usage
- spatial-sidecar-process: DROP — Forge rejected it (0129); in-engine path instead

## Mini AI and ONNX

- onnx-local-provider: LATER — M2+ adapter behind a feature (tract or ort); never in core or tick
- tiny-trained-classifiers: LATER — after M2; train on real request logs
- embedding-search: LATER — lexical search first (BM25 + rank fusion)
- small-local-llm-llama-cpp: IN — the M2 "local" adapter
- whisper-speech-to-text: LATER — after M3
- float-nets-in-tick: DROP — float results differ across CPUs
- integer-e2nn-nets: IN — operator 2026-09-23; integer arithmetic is exact (AI-NATIVE pillar 8)
- runtime-tts-music-motion: DROP — runtime; offline bake only, after license review

## Gameplay AI for the arena

- seeded-bt-utility-goap: IN — M3; engine-ai already has BT, utility and director code
- gladiator-brain-morale-director: IN — M3; port your own logic (Forge kits/ai, MEAT)
- live-announcer: IN — M3 presentation showcase; scripted fallback (EDITOR_AI_CONTROL §4)
- model-bt-baked-offline: IN — owner 2026-09-24; AB3 writes encounters through validated tables
- rl-training-harness: LATER — post-alpha, for bots and QA
- generative-agent-memory: IN — owner 2026-09-24; LW1–LW4, async, bounded, recorded
- per-npc-lora: LATER — after LW4 shows memory alone falls short

## Visuals

- forward-clustered-pbr-ibl: IN — V1 (N5), V2–V3 (M3); Filament math, cascades, tonemap, TAA
- bruneton-sky-sh-probes-lightmaps: IN — V2 (HUNT100, ASPECTS-10)
- gpu-particles-decals-fog: IN — V3 (HUNT100, ROMAN)
- khronos-samples-cc0-textures: IN — V1; sample assets and viewer as correctness references
- software-renderer-ci-smoke: IN — smoke only, never a quality or performance pass
- meshlets-visibility-hiz: IN — V5 RT and Ultra tiers (VG1–VG3); Base keeps CT2 LODs
- ocean-fft-400k-vegetation-cdlod: LATER — the arena does not need them
- rented-gpu-matrix: LATER — M3 performance evidence
- runtime-nerf-cross-vendor-hashes: DROP — runtime NeRF; cross-vendor pixel hashes
- gaussian-splat-backdrops: LATER — VX7, presentation only, until WD3 (LIGHT, operator 2026-09-24)

## Engine core and game scope

- rapier3d-enhanced-determinism: IN — N1 decision (DECISIONS 0164)
- second-physics-backend: DROP — Jolt shim or custom XPBD; not for alpha
- egui-docking-gizmo: IN — N6, one theme (STEROIDS, AIWAVE_100)
- replay-tapes-first-divergence: IN — done in M0 (BEATMAP P2)
- hash-ci-windows-linux: IN — Windows and Linux hash CI (BEATMAP P5)
- wasm-hash-parity: LATER — WASM hash parity
- q16-16-fixed-math: LATER — only if networking needs cross-platform lockstep (BEATMAP P4)
- rollback-netcode-lan-server-auth: IN — M3 multiplayer core; M4 hardens (more fighters, hostile faults)
- multiplayer-transport-rollback: IN — M3 LAN core (2+ fighters, AI-filled slots); M4 dedicated server under faults
- relay-wan: IN — ON1–ON4: QUIC, self-hosted relay, NAT traversal, WAN soak (operator 2026-09-24)
- full-anticheat: DROP — kernel and client anticheat; server-authority checks are ON5
- server-authority-checks: IN — ON5
- server-browser: IN — LIGHT as ON6 (operator 2026-09-24)
- arena-slice (now docs/plan/render-look.md, arena wiring): IN — M3 (sword and shield, 2+ fighters over LAN, hit-react, crowd); see ROADMAP.md
- chariot-vehicle-path: IN — M3 LIGHT; proves input tape and replay are vehicle-agnostic
- full-racing-game: LATER — post-v1 expansion; timing OPEN
- repair-craft-blacksmith: IN — M3 LIGHT; the kept crafting half
- full-crafting-tree: LATER — post-alpha; timing OPEN
- campaign-coop: LATER — OPEN, operator call; not scheduled (ROMAN, NAVALAI)
- multiplayer-io-replay-contracts: IN — M1; rollback needs them, cheap now
- active-ragdoll-gore-ballistics: IN — M3 multiplayer core (MEAT, ROMAN); chariot path is M3 LIGHT
- runtime-fracture: LATER — M3 ships combat first; baked debris stays in the slice
- naval-warfare: DROP — CUT; no files, no deps, no waves
- survival-rhythm-br-td-moderation-crossplay: DROP — not for this product (GAPS4)

## Process

- hundreds-parallel-writers: DROP — review stays the bottleneck (EDGE)
- waves-of-20-workers: IN — operator 2026-09-23; one code packet per crate, one sprint lock, one gate per wave
- acceptance-retry-stop: IN — engine-orchestrate: acceptance number per task, retry once
- auto-commit-when-green: IN — inside /sprint (operator-requested); never force-push
- weights-never-in-git: IN — manifest and SHA for weights; porting rule per AGENTS rule 4
- port-own-forge-code: IN — where it fits; becomes MIT here; exclude Flax-EULA and third-party files
- royalty-model: LATER — needs users first

## 2026 additions (operator-requested research, 2026-09-23)

- wgsl-renderer-ports: IN — R1–R5 (Bevy 0.18 and 0.19 WGSL, same wgpu stack)
- temporal-upscaling-6gb: IN — RP1 engine-owned TAA/TAAU; R6 unverified port parked
- official-rust-mcp-sdk: IN — rmcp; MCP1–MCP3
- cross-platform-deterministic-physics: IN — PH1; confirms DECISIONS 0164
- client-server-rollback-crystal-orb: IN — NET1 reference design (pattern only)
- relation-encoding-scene-graph-context: IN — SP3/SP4 format
- ai-qa-agents-play-and-file-bugs: IN — QA1; bugs ship with replay tapes
- frame-grounded-self-repair: IN — MCP2, QA2 (judge progress from captures)
- split-radiance-cascades: LATER — too slow for the 60 fps budget on the reference GPU; baked GI first
- hardware-ray-queries: IN — RT0–RT8 on the V5 RT tier; Base keeps raster fallbacks (operator 2026-09-24)
- reflective-prompt-evolution-gepa: LATER — tune provider prompts after QA1 has traces
- counterfactual-forks: IN — FK1, EA1c (AI-NATIVE pillar 3); fork tools over MCP (MCP4) parked
- causal-replay: IN — CR1
- verifiers-content-compilers: IN — VF1, EA1b; brief-to-variant generator (LG1) parked
- mcp-agent-benchmark-reporting: IN — EV1
- engine-borrows-client-model-sampling: DROP — MCP sampling deprecated (SEP-2577); engine keeps its own adapters
- neural-world-model-runtime: DROP — not deterministic at runtime; offline concept previs LATER
- integer-only-neural-policies-in-tick: IN — NN1, NN2, BR2 (`.e2nn`)
- any-model-teaches-tiny-student: IN — DT1–DT3
- provenance-receipts-ai-content: IN — PR1; C2PA export LATER
- format-museum: IN — LT1; every released format still loads and replays
- wasm-plugins-skill-packs: LATER — WS1; no code mods, the MCP door covers skills (operator 2026-09-23)
- openusd-scene-exchange: LATER — after Finish A
- onnx-on-render-gpu-effects: LATER — presentation-only neural effects
- large-agent-workforces: IN — SC1, SC2; systems/README.md tiers
- evolutionary-implementation-tournaments: IN — SC3
- otel-genai-spans: IN — OB1, OB3
- mape-k-auto-heal: IN — HL1, HL2; recorded, bounded, deterministic heals
- agent-invariants-fuzzing: IN — FZ1
- learned-motion-matching: IN — LMM1 as integer nets

## Hunt 2026-09-24 (operator-approved 2026-09-24)

- boolean-fracture-chunks: IN — DX2 behind DX1; baked, determinism x2, no runtime crack sim
- easy-mixamo-verification-rules: IN — CT1 follow-up inside existing cooker rows
- mixamo-merge-path-easy-mixamo: IN — CT3 follow-up; content/local only, Adobe terms on raw files
- vcad-cadcore-procedural-props: IN — CT4-adjacent; CLI or MCP use, never crate deps
- kotoba-skeleton-ccd-ik: LATER — reimplement reference only
- curvo-avian-backroll-renzora: LATER — renzora feeds PLN10
- meshopt-xatlas-mikktspace: IN — profile-gated upgrades inside existing R/CT rows
- lau-fixedpoint-donor: LATER — only if netcode needs exact lockstep; no row yet

## 2040 tier (operator 2026-09-24)

- truth-first-gpu-look-rows: IN — GX1–GX6 before any other render row
- many-shadowed-lights-stochastic: IN — RT3 (MegaLights pattern)
- dynamic-gi-no-bake: IN — RT1, RT4; known limits become acceptance tests
- probe-light-grid-gi: IN — RT8 for the Base tier
- virtual-geometry-bvh-compute-raster: IN — VG1–VG3; foliage-class assemblies LATER
- layered-materials: IN — RL6, extended by VX2/VX3
- networked-physics-rewind-resim: IN — ON7
- relevancy-priority-replication: IN — ON3 at 64 players + AI
- quic-datagrams-self-hosted-relay: IN — ON1, ON2
- neural-texture-compression: LATER — runtime waits for wgpu access; offline encode may land
- ml-upscaling-vendor-adapters: LATER — optional adapters after license rows; engine TAAU and RT6 ship by default
- editor-mcp-access: IN — already here (MCP5, EA1); keep it

## Rust repo sweep 2026-09-24

- cBournhonesque/lightyear: IN — patterns for ON3, MP5 (prediction, rooms, lag compensation)
- simgine/bevy_replicon: IN — patterns for ON3 (replication rules, per-client visibility)
- johanhelsing/matchbox: IN — patterns for ON2 (signaling, NAT traversal)
- quinn-rs/quinn: IN — dependency candidate for ON1
- ValveSoftware/GameNetworkingSockets: IN — reference for ON2; no C++ vendoring
- janhohenheim/rerecast: IN — patterns for NV3 tiled rebake
- vleue/polyanya + vleue/vleue_navigator: LATER — NV1 query is enough until legions
- idanarye/bevy-tnua + janhohenheim/bevy_ahoy: IN — patterns for PX8 (steps, slopes, platforms)
- voxell-tech/bevy_motion_matching: LATER — RES-STEAL: compare against LM1 first
- mbrea-c/bevy_animation_graph: LATER — RES-STEAL: compare against AM1 and ED first
- Dusty844/bevy_fabrik_solver: IN — patterns for AM9 fallback
- Kurble/bevy_mod_inverse_kinematics: DROP — no license file
- SlimeYummy/ozz-animation-rs: DROP — MPL-2.0 is banned; C++ ozz (MIT) stays the reference
- dimforge/rapier parallel feature: DROP — breaks enhanced determinism; PX6 splits islands itself
- avianphysics/avian: IN — patterns only; a second physics backend stays DROP
- dimforge/salva: LATER — VX5 blood and water first as presentation sims
- EmbarkStudios/kajiya: IN — patterns for RT4, RT6 (archived; no dependency)
- Patryk27/strolle: IN — patterns for RT3, RT4
- bevyengine/bevy (Solari, meshlets, atmosphere): IN — PORT() rule; RT, VG, VX rows (no LICENSE file at root in wave 29; Cargo.toml says MIT OR Apache-2.0)
- pannapudi/voidin: IN — patterns for VG1
- schell/renderling: LATER — UNVERIFIED; read LICENSE first (NOASSERTION)
- ArthurBrussee/brush: IN — patterns for VX7 (Gaussian splat training and render)
- evroon/bevy-volumetric-clouds: IN — patterns for VX2
- djeedai/bevy_hanabi: IN — patterns for RP2 and VX5
- zeux/meshoptimizer + gwihlidal/meshopt-rs: IN — VG2 (bindings or reimplementation)
- rkyv/rkyv: IN — PF7 zero-copy archives
- Lokathor/wide: IN — AM14 and PF5 portable SIMD
- nagisa/rust_tracy_client + rerun-io/rerun: IN — PF9 profiling and per-tick recordings
- DioxusLabs/dioxus (subsecond) + rksm/hot-lib-reloader-rs: IN — PF6, dev-only tool
- tesselode/kira: IN — patterns for SN3
- FyroxEngine/Fyrox: LATER — RES-STEAL compare for ED and AM rows; no LICENSE file at root (wave 29)
- renew-engine/renew: IN — patterns for GX6 and SL

## Architecture sweep 2026-09-24

The benchmarks that set the bar (GameCraft-Bench, CraftBench-UE, GameDevBench, AutoUE) are findings, not verdicts. They stay in the archive. They drive EV2, VK1 and the vision-in-loop rule.

- unity-insight-index: IN — API1; same idea as forge flax-nav, built from our own code (2609.27585)
- gamegen-verifier: IN — VK1; keypoint specs on FK1 forks (2605.07442)
- agent-interfaces-typed-tools: IN — API1, api-reviewer check 5 (2603.20300)
- swe-agent-aci: IN — API1, API2; small, searchable tools over raw shell (2405.15793)
- code-world-model: IN — PLN20; rules stay deterministic code, look is a separate layer; models do not transfer (2608.25927)
- 3d-layout-r1: IN — PLN20, AB1; scene edits as relations; models do not transfer (2603.22279)
- darwin-godel-mendel-sbco: IN — retro only; automatic edits to proof gates LATER
- task-tokens-add-maskedmimic: IN — PX5 and LM rows; RES-STEAL for Hard-stuff rows
- gamengine-hunyuan-gamecraft-2: LATER — not a core: no determinism, no authoring; watch only

## Video sweep 2026-09-25

- SHOW1-png-sequence-manifest: IN — SHOW1; external ffmpeg stitch
- video-encoder-crates: DROP — ffmpeg-next, gstreamer, openh264, rav1e as dependencies (GM3 clean machine)
- continuity-camera-grammar: IN — folded into SHOW2
- highlight-reel-ranking: IN — folded into SHOW3; vision advisory only
- flip-perceptual-metric: IN — named in PQ4; SSIM secondary; LPIPS and VMAF offline only
- kill-cam-replay-theater: IN — already MP4 (in progress)
- video-mcp-tools-godot-unreal-blender: IN — patterns for VP1, MCP6, VK1; no new rows
- MT1-animation-montage-timeline: LATER — the AM rows own animation sequencing
- forge-prompt-fixtures-previs-checkpoints: LATER — EV3/EV4, WD-PRE1/2, SV2, SV3, FK2; until a consumer needs them
- runtime-neural-video-nerf-tts: DROP — at runtime
- forge-packs-media-otio: DROP — forge's own tool; not an engine2040 row

## Deep dives 01-12, first pass 2026-09-25 (verdicts; rows landed from center)

- counter-rng-correct-rounding-merkle: IN — RC7 landed; RC8 boundary harness first; RC9 offline, full fold_hash authoritative
- wan-rollback-interest-relay-checks: IN — numbers added to ON1, ON3, ON4; GGPO, backroll, naia idea-only
- restir-ddgi-svgf-pathtraced-ref: IN — RT3, RT4, RT6, RT7, RT8, RT11
- vendor-denoisers: DROP — CUT
- maskedmimic-simbicon-ragdoll: IN — PX5 and PX4 notes
- avian-xpbd-substeps-in-tick: DROP — do not take in the tick
- PX9-self-collision: LATER — spike with its kill rule; the numbers decide
- float-cloth-fem-coupling: DROP — r-adaptive cloth, B-spline FEM, WebGPU cloth, DAT coupling: float, GPU, breaks determinism in the tick (phase-field coarsening lives in DX8; DAT idea folded into PX12)
- tiled-rebake-orca-platoons: IN — NV3 (rebake is a stub today), NV4, NV5; SUMO EPL-2.0 patterns only
- streaming-disconnected-replay-injection: IN — PV4, RV1
- insight-parallel-keypoints-fresh-rebuild: IN — API1, VK1, EV2 (rule gates, vision advisory), API3
- gamecraft-bench-llm-judge: DROP — rejected as a gate; its failure taxonomy stays
- generative-agents-pddl-bitemporal: IN — LW5, AB5, LW6; Fast Downward (GPL) never a dependency
- meshopt-acl-flip: IN — VG2 and CT2 port path, CT11, PQ4
- training-stack-gaps: IN — DT5 (dataset provenance), NN3 (QAT), DT4 (replay buffer), NN4 (ONNX Runtime option, Tier C)
- bevy-remote-fyrox-grouped-salsa: IN — MCP7, ED7, API3; no new dependencies

## GitHub plugin and MCP hunt 2026-09-25 (chat hunt, landed from center)

- sqlite-wal-bitemporal: IN — ED8 (its "ED7" collided)
- fixed-point-pid-servo: IN — PX11
- fyrox-command-coalescing: IN — already ED7; blend-weight and additive mask checked against AM2/AM3 first
- mcp-subscription-cleanup-batch-errors: IN — tightened MCP7
- bevy-brp-remote-shim: IN — tightened MCP6
- blender-godot-unreal-mcp-guards: IN — MCP8
- bevy-inspector-egui: IN — ED9 (wave 25)
- bevy-asset-handles-load-states: IN — AS4
- wgpu-timestamps-rend3-egui-dock-hdr: IN — tightened PF9; dock and HDR fallback wait for UIX rows
- salsa-memo: IN — already API3 (not ARC11)
- context-firewall-tool-collapse: IN — API4 experiment with a kill rule (claims unreproduced)
- provider-law-mock-local-remote: IN — holds; no row
- blender-mcp-cook: IN — CT4 done; any future use wraps bpy in propose, validate, journal, apply
- godot-mcp-playtest-tools: LATER — patterns for VP1; no LICENSE file, idea only
- read-only-db-doors-allowed-dirs: IN — capability-scoped doors go to deep dive 12 and RV1
- voxelcraft-rust: LATER — PARKED: no voxel consumer
- unrealgenaisupport: DROP — not on the allow-list; no consumer
- gdext: DROP — MPL-2.0
- chongdashu/unreal-mcp: DROP — badge-only license
- sled: DROP — no consumer
- duckdb: DROP — no consumer
- cranelift: DROP — no consumer
- ros2-sidecar: DROP — no consumer
- veldwake: DROP — no LICENSE file

## Chat research rounds 3-4, 2026-09-25 (landed from center)

- SN5-hrtf: IN — as proposed
- AM15-additive-layers: IN — as proposed
- DX9-graded-bonds: IN — as proposed
- ON14-datagram-discipline: IN — as proposed
- NV6-portal-funnel: IN — portal funnel only; ORCA is NV4, platoons NV5
- LW7-recall-gate: IN — folded into LW5 and LW6; LW7 is now dialogue scripts
- AB6-closed-schema-encounters: IN — folded into AB3's acceptance; AB6 is now the e-graph variant-dedup experiment
- PF10-gpu-input-to-photon-soak: IN — folded into PF9 and PQ3; no new row
- MCP9-declarative-tools: IN — tool sandbox per REPO28-1 (WASM, fuel and memory caps)
- MCP10-explain-guards: IN — EXPLAIN guards on tools
- SP11-intent-macros: IN — intent macros
- LW7-dialogue-compiler: IN — dialogue compiler (LW7 scripts)
- AB1-actor-critic-gates: IN — actor and critic gates
- CT12-bake-manifest: IN — bake manifest
- in-engine-vendor-ai-plugins: DROP — break determinism and the provider law; generation stays outside the tick, bakes validated
- runtime-neural-world-models-hunyuan-matrix: DROP — runtime; offline previs at most, LATER
- jasonsuhari-godot-mcp-playtest: IN — ideas are free; code port allowed with attribution

## Rounds 5-6 and the deep 3D pass, 2026-09-25 (landed from center)

- reflexion-aider-gepa-trials: IN — loop rules in all three commands (Getting better); not engine rows
- textgrad-metagpt-swe-trajectory: LATER — judges already see only the packet patch
- golden-ratio-widths-budgets: DROP — golden-section needs a smooth single-peak metric
- provider-matrix-orak: IN — PV5
- SP7-SP8-SP9-scout: IN — folded into SP6 as candidates (IDs collided)
- scene-graph-prompts-dry-run-cegis: IN — tightened SP7, SP10, MCP10
- engine-critic-not-llm-critic: IN — already AB1
- quatrope-3dgraphllm-encoders: DROP — float and GPU as code; the relations-beat-coordinates principle stays
- virtual-geometry-ray-queries-fog-splats: IN — existing rows VG1–VG6, RT2–RT11, VX1–VX11; gates tightened on RT7, RT11, VG3, VX9, PF9
- rt-proxies-residency-usd-lightmaps: IN — CT13, AS5, AS6 (Tier C); lightmaps already RL5
- wgpu-v30-inline-ray-queries: IN — enough for 1–2 shadow rays and 1 reflection ray; BLAS/TLAS rebuilt fully
- neural-irradiance-cache-fsr-dlss: LATER — never default

## Forge combat-motion sidecar into engine2040 (owner question, 2026-09-25)

- sidecar-in-core-or-tick: DROP — Python and PyTorch, about 37 s per clip on GPU, not deterministic, weights outside git
- combat-motion-content-source: IN — CT14 MotionSource provider adapter (mock, local sidecar over loopback, remote)
- generated-moves-in-combat: IN — HR6 clip to HR3 move windows, gated by VK1 keypoints and the PQ2 tournament
- teacher-for-learned-control: IN — PX5 under DT5 manifests; never non-commercial data for a shipped student
- combat-motion-licenses: LATER — clips non-commercial until retrained on clean data or cleared (verify)
- combat-motion-home: IN — stays in forge (`sidecars/combat-motion/`); engine2040 calls it as one adapter, never vendors it

## Evidence-led bets for the existing plan (owner 2026-09-25)

These are candidate experiments, not finish claims.

- proof-of-built-game: LATER — experiment on EV2, AB4, VK1, QA1; stop if a bad build passes
- paid-vs-free-task-routing: LATER — experiment; stop if quality drops or no repeated gain
- living-roman-characters: LATER — experiment after LW1/LW2 and WD2; stop if replay differs
- discovery-history-replay: LATER — experiment after real outcomes; stop if too few labeled outcomes

## Wave 23 verdicts (2026-09-25)

- wild-dataflow-ivm-on3: DROP — ON14 taken; duplicates ON15 and ON20; revisit only if their kill rules fire
- wild-richardson-pq3: DROP — proposed PQ6; duplicates PQ5 (STA/LTA picker); revisit with numbers if PQ5 spikes
- ga4-subsumption-spike: DROP — killed with numbers (2.00x vs 3x bar); DepthBrain stays
- quaternius-qal: DROP — license no longer CC0; CT7 uses OGA System G6 CC0 instead

## Wave 24 verdicts (2026-09-25)

- free-subsumption-stack-ga4: DROP — moot; GA4 already killed at 2.00x vs the 3x bar
- arxiv-maskedmimic-r-adaptive-phase-field: LATER — notes only, no rows (L/XL cost, determinism blockers)
- spacetimedb-subscription-predicates: IN — concept reimplemented std-only; BSL-1.1 not on the allow-list, no donor bytes

## Wave 26 verdicts (2026-09-25)

Serving GE8, ON10, MCP10, SP15.

- REPO26-1 bytestring-net/bevy_lunex: IN — retained layout engine for Bevy ECS (Apache-2.0)
- REPO26-2 TheRedDeveloper/ply-engine: IN — cross-platform UI framework (0BSD)
- REPO26-3 vladbat00/bevy_egui: IN — Bevy egui integration (MIT)
- REPO26-4 Linus-Mussmaecher/mooeye: IN — ggez UI library (MIT)
- REPO26-5 gold-silver-copper/egui_ratatui: IN — egui widget for bevy, macroquad, eframe (MIT and Apache-2.0)
- REPO26-6 SergioRibera/bevy_splash_screen: IN — splash screen plugin (Apache-2.0)
- IDEDARY/Stardawn: DROP — no LICENSE file
- Tarator-Engine/Tarat: DROP — GPL-3.0, not on the allow-list
- andelf/rust-2048: DROP — a game, not a UI crate
- Far-Beyond-Pulsar/Pulsar-Native: LATER — scene cite only; no LICENSE file at root (wave 29)
- liquidhelium/helium_framework: LATER — scene cite only (Bevy editor egui framework)
- renzora/engine: LATER — scene cite only (2D/3D engine with UI topic)
- IDEDARY/Bevypunk: LATER — scene cite only (neon-noir UI recreation with bevy_lunex)

## Wave 27 verdicts (2026-09-25)

Serving RL7, RP4, MCP7, ON4, LW3.

- REPO27-1 catppuccin/mdBook: IN — mdBook pastel theme for docs (MIT)
- REPO27-2 tommilligan/mdbook-admonish: IN — admonishments preprocessor (MIT)
- REPO27-3 hougesen/mdsf: IN — Markdown code block formatter (MIT)
- REPO27-4 steveklabnik/doxidize: IN — Rust documentation tooling (Apache-2.0)
- REPO27-5 zjp-CN/mdbook-theme: IN — mdBook config themes (MIT)
- rust-lang/mdBook: DROP — MPL-2.0, not on the allow-list; idea only
- rust-lang/book: DROP — NOASSERTION, no LICENSE file; idea only
- bevy-cheatbook/bevy-cheatbook: DROP — license null; idea only
- badboy/mdbook-mermaid: DROP — MPL-2.0
- microsoft/api-guidelines: DROP — NOASSERTION, no LICENSE file; idea only
- zalando/restful-api-guidelines: DROP — CC-BY-4.0, not on the allow-list
- nix-community/nixdoc: DROP — GPL-3.0
- RL7 fog-contact-shadows: DROP — no arXiv paper; fog.rs already has the shape; RL9 depth feed is the open work
- ON4 wan-rollback-arxiv: DROP — no game WAN paper; GGPO, backroll-rs and lightyear patterns cover it (bounded)
- LW3 npc-memory-consistency: IN — bounded, ideas only (2603.10062); 20 scripted visits must give consistent memories

## Wave 28 verdicts (2026-09-25)

Serving SL6, RP5, MCP9, ON11, WD10, LW6.

- REPO28-1 MichaelS1011/ephemora-cell: IN — capability WASM sandbox for MCP9 tool execution; fuel, memory, time caps (Apache-2.0)
- REPO28-2 Azure-Samples/mcp-auth-servers: IN — MCP auth patterns for ON11 (MIT)
- REPO28-3 localden/mcp-auth-servers: IN — fork of REPO28-2 (MIT)
- REPO28-4 peterpeterparker/kyushu: IN — self-hostable Wasm sandbox pattern (MIT)
- REPO28-5 praetorian-inc/wasmforge: IN — WASM-sandboxed executables pattern (Apache-2.0)
- REPO28-6 Geeksfino/openskills: IN — skill sandbox pattern for LW6 (Apache-2.0)
- SL6 quality-security: IN — bounded; wasm sandbox patterns for tool execution
- ON11 online-server-authority: IN — ideas only; MCP auth patterns for server authority
- RP5 render-look-security: DROP — no transfer
- WD10 world-security: DROP — no transfer
- LW6 living-world-sandbox: IN — ideas only; bounded AI skill sandbox
- golemfactory/v8-wasm: DROP — GPL-3.0
- golemfactory/sp-wasm: DROP — GPL-3.0
- poli0981/wardsweep: DROP — GPL-3.0
- brucedota/antiCheatGame: DROP — NOASSERTION
- Saurav-218/AntiCheatGame: DROP — NOASSERTION
- SoyJuan2816/The_Anti-Cheat_Game: DROP — NOASSERTION
- adityamastwal/Anti-Cheat-Game-Monitor: DROP — NOASSERTION
- bogdanzarubin794-cmd/amazing-anti-cheat-game-boost-soft: DROP — NOASSERTION
- samgeorg777/nahimic-game-crash-fix: DROP — NOASSERTION
- leoEL69/GTFO-EXTERNAL: DROP — NOASSERTION
- refzal/0xe0010161-Easy-Anti-Cheat-Game-File-Modifi: DROP — NOASSERTION
- dorev/WasmSandbox: DROP — NOASSERTION
- wajidraza/wasmsandbox-code-runtime: DROP — NOASSERTION
- knguyen-figma/mcp-auth-server-software-statement: DROP — NOASSERTION
- GlenFlock/mcp-auth-server: DROP — NOASSERTION
- emergingrobotics/mcp-authenticated-server: DROP — NOASSERTION
- inoue2002/mcp-auth-server: DROP — NOASSERTION
- AbhishreeGahlot/ephemora: DROP — NOASSERTION
- Lumeriino/Ephemora: DROP — NOASSERTION
- ternilabs/ephemora: DROP — NOASSERTION
- Ephemoral/Ephemoral: DROP — NOASSERTION
- evanshortiss/shipwars-cheat-detection-serverless: DROP — NOASSERTION
- verimand/mcp-authority-server: DROP — no URL
- 0xdevcraft/mcp_boilerplate: DROP — 0 stars; idea only
- SourabhDhalia/mcp-auth-server: DROP — 0 stars; idea only
- vikhyatprabhu/mcp-authorized-server: DROP — 0 stars; idea only
- chinmaygarde/wasmsandbox: DROP — 1 star; idea only

## Wave 29 verdicts (2026-09-25)

Serving RP6, LW9, SP10, SL18-20.

- SP10-LW9 Coding-Solo/godot-mcp: IN — Godot scene tree and spatial queries through MCP; scene cite (MIT)
- RP6 raysan5/raylib: IN — lightweight render reference for RP6 (zlib)
- SL18-20 TanTanDev/macroquad_tantan_toolbox: IN — dev-quality tool for SL18–20 checks (MIT)
- LW9 nullstare/ReiLua: IN — Lua scripting experiment for LW9; kill over 1 ms per tick (MIT)
- godotengine/godot: DROP — no LICENSE file at root; idea only
- Redot-Engine/redot-engine: DROP — no LICENSE file; idea only
- stride3d/stride: DROP — no LICENSE file; idea only
- o3de/o3de: DROP — NOASSERTION, no LICENSE file
- defold/defold: DROP — NOASSERTION, no LICENSE file
- not-fl3/macroquad: DROP — no LICENSE file (Cargo.toml says MIT OR Apache-2.0)
- RodZill4/material-maker: DROP — no LICENSE file
- peptidejteam/ecs-survivors: DROP — GPL-3.0, not on the allow-list
- heroiclabs/fishgame-macroquad: DROP — a game, not a consumer crate

## Wave 30 verdicts (2026-09-25)

Serving SP12, SP11, LW7, RL9, AM16, NV3, PF10, AH6.

- REPO30-1 Bentlybro/bevy_box3d: IN — deterministic Box3D backend for NV3 queries and PF10 benchmarks (MIT)
- REPO30-2 pavlov-net/bevy_meadow: IN — GPU patch grass for the RL9 vegetation pass (MIT)
- REPO30-3 DJ-Huang/PCG-AI: IN — graph PCG patterns for SP12 scene layout (Apache-2.0)
- REPO30-4 Adel-Ayoub/rabbet: IN — study ECS, PBR and scene serialization for RL9 and SP12 (Apache-2.0)
- REPO30-5 lsgmasa33/modoki-engine: IN — study visual editor UX for AM16 and SP12 (Apache-2.0)
- REPO30-6 tettethu/VibeGame: IN — multi-agent patterns for LW7 and SP11; never its AGPL-3.0 code (Apache-2.0)
- REPO30-7 GenshinmasterJinHang/Miku-Material-Converter: IN — material conversion pipeline for AM16 (MIT)
- REPO30-8 ConocoFieldsForever/atlas: IN — GPU-driven map rendering study for RL9 (MIT)
- DraftedDev/bevy_pages: DROP — no LICENSE file; idea only
- rvdende/bevy_iroh: DROP — no LICENSE file; idea only
- sayhisam1/bevy-aqua: DROP — no LICENSE file; idea only
- reearth/kenro: DROP — no LICENSE file; idea only
- mmannerm/animsmith: DROP — no LICENSE file; idea only
- ukaanwat/selfstarter: LATER — UNVERIFIED; repo returned 404, cannot check license
- ziv/bevytiles: LATER — UNVERIFIED; repo returned 404
- teamfireworks/gargantuan: DROP — MPL-2.0
- Krobenlakroc/tachyonfire: DROP — GPL-3.0
- GenieEngine/GenieEngine: DROP — GPL-3.0
- Orrin-Team/orrin: DROP — GPL-2.0
- Sheilem/maplewright: DROP — AGPL-3.0
- Eriskii/pixelgameengine: DROP — LGPL-3.0
- M5Devs/OpenROM: DROP — GPL-3.0
- 7etsuo/anvil: DROP — no LICENSE file
- realfakenerd/wah-krah-jol: DROP — a game, not a consumer crate
- samwhosung/benilla: DROP — a game client, not a consumer crate
- vladtrc/iw4L: DROP — a game runtime, not a consumer crate
- davidolverson/claudescape: DROP — a game, not a consumer crate

## Wave 31 verdicts (2026-09-26)

Serving DX10, NV7, GA2, AB7, ED8, RL10.

- REPO31-1 DX10 AlexanderNyr/AuraLite-Physics-Engine: IN — deterministic collision shapes and body queries for DX10 (Apache-2.0)
- REPO31-2 NV7 seldom_map_nav Seldom-SE/seldom_map_nav: IN — tilemap navmesh queries for NV7 path stamps (Apache-2.0)
- GA2 legion-ai-flocking: IN — bounded, ideas only (Toner & Tu; Sar & Ghosh; Takko); integer reimplementation; no donor repo
- ED8 crash-safe-journal-wal: IN — SQLite WAL patterns (SquirrelFS, Pellegatti & Vieira); owned by crates/editor/src/wal.rs
- NV7 nv7-versioned-paths: IN — reimplement 2602.04130v1 version-stamped paths in integer E2NAV1
- RL10 fog-sun-scatter-tint: LATER — no donor found; reimplement from HUNT100 and ROMAN patterns
- AB7 shared-brief-envelope: LATER — no donor found; engine-owned pattern (crates/endpoint/src/brief.rs not yet built)

## Wave 32 ARXIV verdicts (2026-09-26, RES-ARXIV Dive 11 game feel)

- GP6 feel-quantification: IN — REPRODUCE from Pichlmair & Johansen 2011.09201v1 into PQ1 bands and VK1 rubric; license UNVERIFIED, reimplement
- GA7 diverse-fighting-ai: LATER — UNVERIFIED license; offline `.e2nn` students from Halina & Guzdial 2211.02759v1
- GP7 feel-keypoints: IN — REPRODUCE; GameGen-Verifier extended to feel specs (VK1); remapped from colliding GP3

## flax-mcp E-groups (RES-STEAL wave 32, 2026-09-26)

Source is MIT (flax-mcp@a26f960). rvo2 has no LICENSE file in that tree; add one before porting RVO-lite crowd code.

- flax-E1-camera-feel-cores: IN — GP8; 36 pure-CLR camera cores as engine-gameplay camera modules
- flax-E2-combat-depth-cores: IN — GP9; 19 combat cores, a GAS-like layer in engine-combat
- flax-E3-eqs-cover-flank: IN — GA6; 20 EQS components in engine-ai, composes with GA1
- flax-E4-snapshot-interpolation: IN — MP10 fills the interpolation gap after MP9; Hermite, quaternion, snapshot buffer, extrapolation guard
- flax-E5-city-grammar-map-import: IN — WD13 extends the city grammar; WD1–WD12 exist; road graph, OSM import
- flax-E6-content-addressed-store: IN — fold into CT16; compose with CT15 provenance
- flax-E7-cluster-lod-meshopt-xatlas: IN — fold into CT16
- flax-E8-wfc-mdp: IN — fold into WD13
- flax-E9-vat-encoder: IN — fold into RP7
- flax-E10-voxel-destruction: DROP — DX1–DX4 cover destruction; no voxel consumer
- flax-E11-sdf-raymarch: DROP — no engine consumer

## Wave 33-REPO5 verdicts (2026-09-26)

- REPO5-1 dimforge/rapier: IN — KinematicCharacterController move_shape pattern for the core character controller
- REPO5-2 avianphysics/avian: IN — MoveAndSlide pattern for ECS character controllers
- REPO5-3 jrouwe/JoltPhysics: IN — reference only; C++, not portable

## Wave 36 verdicts (2026-09-26)

- RV2 nab-prompt-injection-defense: IN — prompt-injection defense for the MCP door
- MCP11 capability-scoped-permissions: IN — capability-scoped permissions for MCP tools
- EA2 wirken-audit-log: IN — audit log; remapped from colliding EA1c
- CT16 munal-os-wasm-sandbox: IN — WASM sandbox for tool execution
- MCP12 boxagnts-tool-sandbox: IN — tool sandbox pattern
- steal36-noassertion-rejects: DROP — six repos with NOASSERTION license; names in the archive prose
- LLM-OAuth-checker: DROP — provider law
- flaw-framework-2605.22333v1: LATER — MCP auth measurement; not yet a row
- nanite-webgpu-yuphin-lumen: LATER — take recorded; needs render design first (no board row)
- kajiya-restir-pt-retry: LATER — clone failed; retry
- RC12-1 tesselode/kira: IN — underlying audio engine reference
- RC12-2 SamiPerttu/fundsp: IN — DSP algorithm reference (ideas only)
- RC12-3 NiklasEi/bevy_kira_audio: IN — Bevy audio integration reference
- BillyDM/Firewheel: DROP — spatial audio too basic
- Ralith/oddio: DROP — stalled
- capdilla/audiotee_rust: DROP — rejected in the audio corner
- AdetsGithub/SonicLink: DROP — rejected in the audio corner

## Wave 41 verdicts (2026-09-26)

- NV4-NV8 wowemulation-dev/recast-rs: IN — NV3 rebake and NV8 soak (Apache-2.0)
- NV6 Ruddle/rustfield: IN — flow field pathfinding for the NV6 funnel (MIT)
- NV4-NV7 bnomei/condor: IN — multi-agent routing and navmesh pathfinding (MIT)
- NV1-NV3 julien-blanchon/saddle-ai-navmesh: IN — Bevy navmesh baking and tooling (MIT No Attribution)
- NV4 ralyassi/Robot-RVO2: IN — ORCA avoidance overrides; ideas only (Apache-2.0)
- ext-sakamoro/ALICE-Navigation: DROP — license not on the allow-list
- ohchase/divert: DROP — license UNVERIFIED (also wave 31)
- SyedAnees21/neuclidean: DROP — license UNVERIFIED
- Lulzx/skein: DROP — license UNVERIFIED
- Hakkology/Rust-ECS-Battlefield: DROP — license UNVERIFIED

## Wave 41 REPO13 verdicts (2026-09-26) — Game AI: BT, GOAP, utility, squads

- REPO13-1 zkat/big-brain: IN — utility AI for Bevy: scorers, actions, thinkers, pickers (Apache-2.0)
- REPO13-2 Sollimann/bonsai: IN — deterministic behavior trees; blackboard, serde (MIT)
- REPO13-3 victorb/dogoap: IN — GOAP with Bevy ECS; A* planning (MIT)
- crashkonijn/GOAP: DROP — Unity C#, not portable
- luxkun/ReGoap: DROP — Unity C#, not portable
- forester-bt/forester: LATER — too early (41 stars)
- tynril/rgoap: LATER — too early (33 stars)

## Wave 48 verdicts (2026-09-27, PLN48 frontier.md + readers P1/P2)

- EV15 ladder-file: IN — PLN48 frontier take
- SP19 hierarchical-brief: IN — PLN48 frontier take
- SP20 graph-qa-verbs: IN — PLN48 frontier take
- SP21 index3-free-space-field: IN — PLN48 frontier take
- AX10 tool-surface-benchmark: IN — reshaped from PROPOSED AX2
- PF13 reference-gpu-baseline: IN — PLN48 frontier take
- RT14 many-light-spike: IN — PLN48 frontier take
- VG12 meshlet-spike: IN — PLN48 frontier take
- UI3 hud-overlay: IN — orphan, PLN48 frontier take
- GP24 camera-wiring: IN — PLN48 frontier take; staged camera.rs is GP24 presentation work
- CT19 fighter-incoming-bind: IN — PLN48 frontier take
- BET-GP24: LATER — held feel kernel
- QA3 bonsai-bt-brains: IN — was QA2; PLN48 STEAL dive14 take
- PQ9 skillratings-scorer: IN — PLN48 STEAL dive14 take
- CR2 foldback-capture-schema: IN — PLN48 STEAL dive14 take
- EV16 bevy-ggrs-checksum-discipline: IN — was EV7; PLN48 STEAL dive14 take
- PQ10 arboriter-mcts-sweep: IN — PLN48 STEAL dive14 take
- BET-EV8: LATER — held bet; needs EV2, EV16, CR2
- arxiv47-cards-1-4: IN — experiment sketches on CT12, EV12, MCP14, EV13 (no rows)
- arxiv47-card-5-lifts: DROP — Debian, Vault, SLSA, Kettle lifts; disciplines only
- rhai-rune-bevy-mod-scripting: LATER — wait for the RC follow-up substrate (PLN48 REPO19)
- SP22 habitat-lab-affordance: IN — MIT, read live (PLN48 reader P1)
- SP23 holodeck-constraint-compile: IN — Apache-2.0, read live (PLN48 reader P1)
- SP24 fyrox-nav-derived-tags: IN — MIT, read live (PLN48 reader P1)
- SP25 bevy-query-door-hardening: IN — idea only (PLN48 reader P1)
- SP26 framing-rules: LATER — UNVERIFIED (PLN48 reader P1)
- BET-SP27: LATER — held bet
- AX11 rideway-efficiency-utility: LATER — idea only, UNVERIFIED (PLN48 reader P2)
- API6 protocol-pinned-collapse-guardrail: IN — PLN48 reader P2 take
- MCP17 turn-vs-tool-refusal-tags: IN — PLN48 reader P2 take
- AX12 chronos-branch-merge-receipts: IN — PLN48 reader P2 take
- BET-AX13: LATER — held bet
- HouraiTeahouse/backroll-rs: DROP — transport idea-only; eval home taken by EV16
- skyne98/soap: DROP — no gap beyond QA3
- board-card-game-mcts-toys: DROP — no real-time transfer; PQ10 covers the pattern
- visual-bracket-managers: DROP — no rating math; PQ9 covers the gap
- mlua-rs/mlua: DROP — tick substrate; interop reference only
- bytecodealliance/wasmtime-as-scripting: DROP — sandbox reference; never per tick
- khvzak/script-bench-rs: DROP — harness only; never code

## World compiler paste (owner 2026-10-07)

Source: the owner pasted a long chat plan and asked what is gold. engine2040 already follows its core law. The real gap is the level graph above one room. Paper and product claims in the paste are its own, UNVERIFIED here.

### HAVE IT (no new rows)

- generative-propose-engine-decides: IN — the thesis (AI-NATIVE; EDITOR_AI_CONTROL)
- provider-neutral-ai-fallback: IN — crates/provider; A6 met 75/75
- typed-actions-permission-budgets: IN — crates/editor/src/capability.rs; MCP door
- semantic-world-graph: IN — crates/spatial (SP1–SP4; SP19 open)
- verify-then-repair: IN — room scale: VF1, reachability, AB1 (at most 3 repairs), SP23, SP54
- asset-compiler-license-gate: IN — GN1 crates/assets/src/gen_intake.rs
- provenance-content-addressed: IN — crates/scene/src/provenance.rs; PR1
- same-spec-seed-version-same-world: IN — the law: HASH-LAW.md; golden hash
- one-event-log: IN — tapes (crates/proof/src/tape.rs); AB9 offline brief replay
- synthetic-players: IN — crates/playtest; AB3 fork balance
- cognitive-lod-many-agents: IN — legions: crates/ai/src/legion.rs think_period
- npc-memory-importance: IN — crates/ai/src/memory.rs
- safe-rule-language: IN — crates/rules (Rhai, integers only; D2, NS32)
- wfc-graph-grammars-city: IN — rowed: WD14, WD13
- world-model-not-runtime: DROP — runtime; previs LATER (2026 additions)
- game-works-with-ai-off: IN — the law: VISION "offline useful"
- no-model-in-frame-tick: IN — the law: VISION boundaries; integer `.e2nn` only

Two places ahead of the paste: counterfactual forks (pillar 3) and causal replay (pillar 4).

### GOLD (proposed rows, not boarded; prefix AB, planner boards on owner word)

Today `level.build` makes one 10 m room with at most 24 props. These rows grow the room into a level and measure it. Order: AB19, AB17, AB18, AB21, AB20; AB22 stands alone.

- AB19 brief-chain-bench: IN — measures what exists; reads the AB14 fix when it lands; Tier A, size S; needs AB4, AB9
- AB17 level-graph-locks-keys: IN — checker over (room, keys held) states; upstream of WD14 and BET-AB10; Tier B, size S; needs VF1, AB7
- AB18 brief-to-multi-room-level: IN — Tier B, size M; needs AB17, AB1, SP23
- AB21 change-level-keep-what-matters: IN — typed delta with pins; Tier B, size M; needs AB18
- AB20 experience-targets-seed-search: IN — MAP-Elites archive over at most 256 seeds; Tier B, size M; needs AB18, AB19
- AB22 asset-briefs-ask-for-function: IN — affordance asks checked after cook; Tier B, size S; needs GN2, SP22

Two rules kept from the paste (also in AI-NATIVE pillar 5): never ask a model what an algorithm can prove; ask for function, not looks.

### LATER (cards only, no rows)

- world-paging-abstract-dormant: LATER — cells already stream; a seamless world is CUT; wakes when off-screen state is needed
- generation-planner: LATER — one generator per output today; wakes at three generators for one output
- compute-classes-on-device: LATER — no shipped local model (LAI row unparks)
- quests-as-predicates: LATER — living world parked; wakes with urban-war mission objectives
- generated-levels-by-digest-online: LATER — no generated level is played online yet (wakes with MP rows)
- world-model-imagines-compiler-rebuilds: LATER — research only; needs GPU priors
- vision-model-scores-taste: LATER — wakes with the LAI vision-judge row
- faction-city-agents-above-squads: LATER — FU27 commander layer is the first step

### DROP

- rename-forgeai-restart-from-five-crates: DROP — throws away 36 packages and about 1,000 board tasks
- bevy-ecs-as-core: DROP — owner D9 (2026-10-07): no Bevy dependency; the `crates/ecs` spine is landing
- ai-makes-assets-inside-shipped-game: DROP — owner D3: generation is an editor job; the shipped game never generates
- llm-per-npc-or-frame: DROP — integer brains in the tick; live models off-tick and recorded
- neural-world-model-as-runtime-2026-10-07: DROP — same verdict as 2026-09-23
- infinite-seamless-world: DROP — FOCUS CUT; maps stream in cells
- steal-ideas-from-any-source: DROP — the 2026-10-07 allow-list rule; superseded by AGENTS rule 4 (2026-10-10), which allows donor ports

## Reference-only rows

The research references behind the paste (MAP-Elites, Dormans, WFC citations) are not ideas. They stay in `docs/archive/2026-10-10/RESEARCH-VERDICT-prose.md`.

Papers for the hard rows (reimplement, never copy): SIMBICON (Yin et al. 2007) and later balance work for PX4; AMP (Peng et al. 2021) and DeepMimic (Peng et al. 2018) for PX5; Paragon animation (GDC 2016) for LM2; Aristidou and Lasenby FABRIK (2011) for AM9; URO-style update-rate LOD for AM13.
