# DEEP-DIVES — research rotation for depth (owner 2026-09-25)

The RES-ARXIV and RES-STEAL readers in `/sprint` take the dive served longest
ago (blank first), hunt arXiv (2023-2026 first) and GitHub for it, and return
at most three cards plus one moonshot. Update the dive's row every time. A
dive whose rows all landed gets fresh hunt terms from the newest findings. This
replaces pasting research prompts by hand: the loop covers every area itself.

**Card shape:** question, engine home, baseline; URL + ID/revision + access
date; code, data and weights licenses separately (or UNVERIFIED); claim,
assumptions, counterevidence, what does NOT transfer; cost, fixture, metric,
budget; ablation, threshold, kill; board row proposal. Papers are not code
licenses; ideas are reimplemented; Unreal, Unity and Flax source is never read.
Every AI-facing card also says how it stays provider-agnostic.

| # | Dive | Last served | Cards | Rows made |
|---|---|---|---|---|
| 01 | Deterministic spine (ARC, RC) | 2026-09-26 | 3+2 → +5 W37 | RC10 (`madsim-rs/madsim`, REPRODUCE), RC11 (`rust-lang/rustc-hash`, REPRODUCE), RC12 (`copper-project/copper-rs`, REPRODUCE), RC13 (`turbofish-org/orga`, REPRODUCE), RC14 (`eldruin/wyhash-rs`, REPRODUCE). See docs/research/archive/steal-wave37.md |
| 02 | Netcode and rollback at scale (MP, ON) | 2026-09-26 | 3+moonshot → +5 W39 | MP8 (`benny-n/netcode`, REPRODUCE), ON1 (`lowquark/uflow`, REPRODUCE), ON4 (arXiv:2604.15044v1 CoGrid & MUG, REPRODUCE); REJECT `alexdesander/hexgate` (MPL-2.0), `iQua/days` (AGPL-3.0), `canndrew/netsim` (UNVERIFIED), `bacongobbler/bevy_simple_networking` (Other), `gschup/bevy_ggrs` (Other), `3-Hz/pfengine` (UNVERIFIED), `MattKulka/revenant` (UNVERIFIED), `genxium/DelayNoMore` (UNVERIFIED). See docs/research/archive/steal-wave39.md |
| 03 | The 2040 look (RT, VG, VX, GX) | 2026-09-26 | 3+2 → +5 W40 | **arxiv-wave40** (arXiv:2605.11536v1 ToF ReSTIR): RT4 (TAKE, REPRODUCE), RT3 (TAKE, UNVERIFIED), RT8 (TAKE, REPRODUCE); REJECT ToF path-length rendering (HASH-LAW, no game consumer). See docs/research/archive/arxiv-wave40.md |
| 04 | Animation and active ragdoll control (AM, LM, TV, PX) | 2026-09-26 | 3+moonshot | PX4, PX5 notes; PX12 DAT coupling proposed; **arxiv-wave37** (arXiv:2510.22632v1 Environment-aware Motion Matching): PX12 (TAKE, REPRODUCE), NV5 (TAKE, UNVERIFIED), PX5 (TAKE, REPRODUCE); REJECT GPU pipeline (HASH-LAW). See docs/research/archive/arxiv-wave37.md |
| 05 | Physics and destruction at scale (PX, DX) | 2026-09-26 | 3+moonshot → +5 W38 | DX8; PX12 (TAKE: `AlexanderNyr/AuraLite-Physics-Engine`, REPRODUCE), PX12 (TAKE: `sudokatie/vortex`, REPRODUCE), PX12 (TAKE: `agroudiev/dynamics`, REPRODUCE), PX12 (TAKE: `jim-ec/constraint_solver`, REPRODUCE), PX12 (REJECT: `Doritozzzz/phys-rs`, UNVERIFIED license). See docs/research/archive/steal-wave38.md |
| 06 | Crowds, navigation, world streaming (GA, NV, WD) | 2026-09-26 | 3+2 → +5 W41 | NV4 (TAKE: `wowemulation-dev/recast-rs`, REPRODUCE), NV6 (TAKE: `Ruddle/rustfield`, REPRODUCE), NV4/NV7 (TAKE: `bnomei/condor`, REPRODUCE), NV1/NV3 (TAKE: `julien-blanchon/saddle-ai-navmesh`, REPRODUCE), NV4 (TAKE: `ralyassi/Robot-RVO2`, REPRODUCE). See docs/research/archive/steal-wave41.md |
| 07 | Provider-agnostic AI and LLM adapters (AX, PV, AH) | 2026-09-26 | 3+2 → +5 W40 | AX1 (`llmkit-ai/llmkit`, REPRODUCE), MCP8 (`hibuka-labs/phi-agent`, REPRODUCE), AX3 (`clark-labs-inc/clark-agent`, REPRODUCE); REJECT `mmstudio-games/armillae` (AGPL-3.0), `vincents-ai/llm` (UNVERIFIED). See docs/research/archive/steal-wave40.md |
| 08 | The agent loop that edits the game (API, VK, EV, VP, MCP) | 2026-09-25 | 3 | API1, VK1, EV2, API3 |
| 09 | Living world and brief to game (LW, AB, GP) | 2026-09-25 | 3 | LW5, LW6, AB5 |
| 10 | Content, audio, editor, proof (AS, CT, SN, ED, PF, AH, SL, PQ) | 2026-09-26 | 3+2 → +5 W43 | CT11 (TAKE, REPRODUCE), CT11 (TAKE, UNVERIFIED), PQ4 (TAKE, REPRODUCE); REJECT Neural 3DGS compression (HASH-LAW). See docs/research/archive/arxiv-wave43.md |
| 11 | Game feel, camera and combat design (GP, GA, SHOW, PQ2) | 2026-09-26 | 3+2 | GP11 (TAKE: Pichlmair game-feel survey), GP12 (TAKE: FootsiesGym), AB5 (TAKE: Gajardo behavior algorithms); REJECT Halina-Guzdal (deep RL, tick-incompatible), Parr (Smash Bros specialist, no framework). See target/research/steal-wave35.md |
| 12 | Trust and security for AI edits and MCP (EA, MCP, AX, ON) | 2026-09-26 | 3+2 → +5 W36 | MCP11, RV2, CT16 proposed; MCP auth LLM (REJECT), Automated detection (LATER); zero-trust architecture (UNVERIFIED). See docs/research/archive/steal-wave36.md |
| 13 | Neural training stack (NN, DT, PX5) | 2026-09-25 | 3 | DT5, NN3, DT4, NN4 |
| 14 | Rust ecosystem sweep: engines, editors, protocols (MCP, ED, API) | 2026-09-25 | 3 | MCP7, ED7, API3 |
| 15 | Spatial intelligence: the AI knows and controls the scene (SP, MCP, AB) | 2026-09-25 | 2+2 → +5 W26 | TAKE: 3D-SLIM (masking spatial reasoning), Lexicon3D (structured probing); REJECT: ArgusSceneGraph, SplatGraph, O3DE, Holodeck/ConceptGraphs/HOV-SG/EmbodiedRAG. **W26 STEAL26**: TAKE georust/rstar (SP15 incremental index, MIT OR Apache-2.0), cooper-tretter/bq-guard (MCP10 EXPLAIN gate, MIT), dagryazev/bqcost (MCP10 regression gate, MIT/Apache-2.0); REJECT emssik/agent-search (Commons Clause), Calbabreaker/crile (LGPL-2.1), cell-local-if/strata-search (UNVERIFIED). See target/research/steal-wave26.md |
| 16 | STEAL27 wave-27 rotation: GE9, RL7, RP4, MCP7, ON4, LW3 | 2026-09-25 | 2+4 | **TAKE**: OpenGameAgent (LW3 NPC memory, MIT), ai-character-engine (LW3 NPC memory, MIT). **REJECT**: GE9 (internal proof ratchet), RL7 (internal fog/contact), RP4 (internal SMAA/scale), MCP7 (internal discover/watch), ON4 (internal WAN soak/rollback). See target/research/steal-wave27.md |
| 17 | STEAL28 wave-28 rotation: SL6, RP5, MCP9, ON11, WD10, LW6 | 2026-09-25 | 0+6 | **DO-NOT-TAKE (all 6)**: SL6 (zero-alloc tick — `CountingAlloc` already exists in `crates/perf/src/alloc.rs`), RP5 (scale seam — `aa_scale.rs` already has SMAA + scale controller), MCP9 (declarative tools — `#[e2_tool]` already integrated), ON11 (long soak — `wan_soak.rs` already has complete WAN fault soak framework), WD10 (ballistics — fixed-point deterministic projectile sim is engine-internal), LW6 (provenance — engine-specific AI infrastructure). No external donor meets the TAKE threshold (consumer crate + falsifiable experiment). See target/research/steal-wave28.md |
| 18 | STEAL29 wave-29 rotation: RP6, LW9, SP10, SL18-20 | 2026-09-25 | 0+4 | **DO-NOT-TAKE (all 4)**: RP6 (quality selection — `tier.rs` already has complete quality tier system), LW9 (memory save — `memory.rs` already has bounded memory store with deterministic serialization), SP10 (spatial queries/sampler — `index3.rs` already has R*-tree + Grid3 with comprehensive query methods; `georust/rstar` already adopted for SP15), SL18-20 (buffer reuse — `StagingBelt`, `CpuCache`, `PipelineCache`, `CountingAlloc` already cover all buffer reuse patterns). No external donor meets the TAKE threshold (consumer crate + falsifiable experiment). See target/research/steal-wave29.md |
| 19 | STEAL30 wave-30 rotation: SP12, SP11, LW7, RL9, AM16, NV3, PF10, AH6 | 2026-09-25 | 0+8 | **DO-NOT-TAKE (all 8)**: SP12 (gold evals — no `spatial_eval.rs`, harness is ours), SP11 (intent macros — O3DE reference only, reimplemented over SP5/SP8), LW7 (dialogue — `dialogue.rs` missing, Godot Dialogue Manager UNVERIFIED), RL9 (depth feeds — `present.rs` has no depth readback, donor: none), AM16 (determinism audits — internal quality audit), NV3 (nav rebake — `rebake.rs` already implemented), PF10 (perf matrices — extends `workloads.rs`/`baselines.rs`), AH6 (chaos — `chaos.rs` already implemented, extends `FAULTS` array). No external donor meets the TAKE threshold (consumer crate + falsifiable experiment). See target/research/steal-wave30.md |
| 20 | STEAL31 wave-31 rotation: RL10 (sun-scatter), DX10 (break-nav contracts), GA2 (legion AI), AB7 (envelopes), NV7 (path versions), ED8 (WAL journals) | 2026-09-26 | 0+6 | **DO-NOT-TAKE (all 6)**: RL10 (sun-scatter — no `sun_scatter.rs`, GitHub search 0 repos, render-look plan "Donor: none"), DX10 (break-nav contract — no `break_nav.rs`, GitHub search 0 repos, internal contract test), GA2 (legion AI — no `legion.rs`, `depth.rs` reserves the name, GitHub search 0 repos, FyroxEngine/Fyrox UNVERIFIED), AB7 (envelopes — no `brief.rs` in endpoint crate, GitHub search 0 repos, internal type consolidation), NV7 (path versions — `crates/nav/src/tiles.rs` already exists with 1336 lines, plan drift confirmed), ED8 (WAL journals — `crates/editor/src/durable.rs` already exists with 373 lines, complete WAL journal with atomic writes/CRC32/recovery, E3 journal 52/52). No external donor meets the TAKE threshold (consumer crate + falsifiable experiment). See target/research/steal-wave31.md |

## 01 Deterministic spine
Home `crates/core`, `crates/ecs`, `docs/plan/runtime.md` (spine), `docs/HASH-LAW.md`. Baseline: fixed 60 Hz, xorshift + counter RNG (RC7), full rehash, sorted IDs.
arXiv: counter-based RNG, correctly rounded math (CORE-MATH, RLIBM), Merkle tick digests, deterministic scheduling, archetype ECS layouts. GitHub: bevy_ecs schedule, flecs, hecs, hashbrown order traps.
Fixture: the ARC3 parity harness. Kill: any per-tick allocation, any sim `HashMap`, any hash drift between Windows and Linux.
**Wave-37 STEAL37**: TAKE `madsim-rs/madsim` (RC10, deterministic simulation testing harness, Apache-2.0), `rust-lang/rustc-hash` (RC11, deterministic hash for cross-platform entity IDs, Apache-2.0), `copper-project/copper-rs` (RC12, deterministic tick recording and replay, Apache-2.0), `turbofish-org/orga` (RC13, deterministic ECS schedule verification, Apache-2.0), `eldruin/wyhash-rs` (RC14, counter-based PRNG quality upgrade, Apache-2.0). REJECT `wangyi-fudan/wyhash` (not a Rust crate), `nautechsystems/nautilus_trader` (LGPL-3.0), `nerdsane/redis-rust` (UNVERIFIED license), `MeloniZippoProjects/LockstepLibrary` (UNVERIFIED license), `osukhoroslov/anysystem` (too generic). See docs/research/archive/steal-wave37.md |

## 02 Netcode and rollback at scale
Home `crates/net`, `crates/arena`, `docs/plan/multiplayer.md`, `docs/archive/2026-10-08/plan/online.md`. Baseline: UDP LAN, prediction + resim, snapshot deltas, 8-bot soak.
arXiv: rollback theory, input prediction, interest management, priority replication, QUIC latency, cheat detection, WAN fault models. GitHub: GGPO, backroll, quinn, naia, lightyear, renet, matchbox.
Fixture: loopback drop/rejoin with jitter, loss and reorder; 64-bot siege for 10 minutes, exact replay. Kill: desync that does not converge, p95 input over 50 ms, bandwidth over budget.

## 03 The 2040 look
Home `crates/render`, `docs/archive/2026-10-08/plan/render-2040.md`, `docs/VISUAL-TARGET.md`. Baseline: forward PBR, CPU references, GX rows making the GPU path real.
arXiv: ReSTIR DI/GI, reservoir resampling, neural and classic denoisers, meshlets and virtual geometry, volumetrics, skin and hair, probe GI. GitHub: Falcor, Daxa, wgpu ray query samples, kajiya, strolle, Bevy Solari, FLIP (BSD-3).
Fixture: material chart, night with 250 torches, day-night sweep; ms on the reference GPU. Kill: pass over its budget, perceptual regression against RT7.
**Wave-40 ARXIV40**: TAKE `arXiv:2605.11536v1` (ToF ReSTIR: Time-of-Flight Rendering with Spatio-temporal Reservoir Resampling, CC-BY-4.0, EGSR 2026). RT4 (TAKE, REPRODUCE), RT3 (TAKE, UNVERIFIED), RT8 (TAKE, REPRODUCE). REJECT ToF path-length rendering (HASH-LAW, no game consumer). See docs/research/archive/arxiv-wave40.md.

## 04 Animation and active ragdoll control
Home `crates/anim`, `crates/physics/src/ragdoll.rs`, `powered.rs`, `docs/plan/animation.md`, `docs/plan/body-and-break.md` (physics base). Baseline: velocity servo, PX9 joint tables, 60-tick get-up.
arXiv: MaskedMimic 2409.14393, AMP, DeepMimic, SIMBICON, motion matching, motion inpainting, foot IK, inertialization. GitHub: JoltPhysics PoweredRagdoll, avian XPBD joints, bevy_animation_graph, bevy_motion_matching.
Fixture: knock, drive, stand over 600 ticks, hash x2. Kill: hash drift, settle over bound, float weights in the tick (students are int8 `.e2nn` with an FSM fallback).

## 05 Physics and destruction at scale
Home `crates/physics`, `crates/destruction`, `docs/plan/body-and-break.md` (physics base). Baseline: CPU verlet cloth, bonded chunks, BSP bake, caps 16 per event and 1,024 total.
arXiv: XPBD substeps, CCD, phase-field fracture 2609.21201, variational cloth 2608.17833, B-spline FEM 2506.18867, coupling (DAT 2604.15513). GitHub: salva, Jolt, Voronoi shatter, brusher BSP. Wave-38 STEAL38: TAKE `AlexanderNyr/AuraLite-Physics-Engine` (PX12, CCD/GJK-EPA/SAT collision + XPBD cloth + PBF fluids, Apache-2.0), `sudokatie/vortex` (PX12, rigid body dynamics + collision detection, MIT), `agroudiev/dynamics` (PX12, rigid body dynamics algorithms, CC-BY-4.0 ideas-only), `jim-ec/constraint_solver` (PX12, position-based rigid body dynamics, MIT); REJECT `Doritozzzz/phys-rs` (UNVERIFIED license + f64 float physics violates HASH-LAW). See docs/research/archive/steal-wave38.md.
Fixture: 8 fighters for 300 ticks with destruction on; a 10x stiffness plus 2x wind blow-up drill; 8-way insertion permutation. Kill: NaN, tunneling, runtime fracture in the tick without DX5's budget.

## 06 Crowds, navigation, world streaming
Home `crates/nav`, `crates/world`, `docs/plan/gameplay.md` (brains), `docs/plan/parked.md` (world). Baseline: Recast-pattern bake, separation/arrive LOD, no rebake on break (NV3), no 500-soldier LOD (PX7).
arXiv: ORCA/RVO, flow fields, navmesh streaming, world cell streaming, weather and day-night, army AI. GitHub: recastnavigation, polyanya, flowfield crates, Godot streaming, big_space. Cross-domain welcome: robotics, GIS, traffic.
Fixture: district plus siege field, break then rebake, flat cost with density. Kill: rebake stalls the tick, stuck fighters, memory drift.

## 07 Provider-agnostic AI and LLM adapters
Home `crates/provider`, `crates/endpoint`, `docs/plan/ai-native.md`, `docs/EDITOR_AI_CONTROL.md`. Baseline: mock, local and remote pass the PV1 suite; recorded fixtures; scripted fallback.
arXiv: capability negotiation, tool-use failure taxonomies, timeouts, cancellation and backpressure, offline replay, cost and latency evals, prompt-injection containment. GitHub: MCP Rust SDK, ollama-rs, async-openai, Anthropic and Gemini SDKs.
Fixture: 20 authoring + 5 play tasks across mock, local, remote and an external MCP client; 10k ticks advance with the model slow or offline. Kill: a live call stalls the tick, only one real adapter, network needed for replay.
**Wave-40 STEAL40**: TAKE `llmkit-ai/llmkit` (AX1, provider-agnostic LLM toolkit with prompt management/versioning/testing, MIT), `hibuka-labs/phi-agent` (MCP8, MCP support + extensible tools + streaming + multi-agent orchestration, MIT), `clark-labs-inc/clark-agent` (AX3, typed hookable provider-agnostic agent loop, Apache-2.0); REJECT `mmstudio-games/armillae` (AGPL-3.0), `vincents-ai/llm` (UNVERIFIED). See docs/research/archive/steal-wave40.md |

## 08 The agent loop that edits the game
Home `crates/endpoint` (insight, play), `e2040-mcp`, `fixtures/keypoints/`. Baseline: EA1 transcript, preview, fork test, apply and undo; SP gates; VK1, VP1.
arXiv: Unity Insight 2609.27585, GameGen-Verifier 2605.07442, GameCraft-Bench 2606.17861, CraftBench-UE 2609.23142, Code World Model 2608.25927, 3D-Layout-R1 2603.22279, Darwin Gödel Machine 2505.22954. GitHub: forge's FlaxApiFts weights, mcp-for-blender, other engine MCP servers.
Fixture: 50-query insight gold set top-3 ≥ 90%, 30 keypoints with 5 seeded bugs, 20-step duel with stable digests. Kill: tokens not down, stale writes applied, `unsupported_capture` on a GPU.

## 09 Living world and brief to game
Home `crates/ai`, `docs/plan/living-world.md`, `gameplay.md`, `docs/FOCUS.md`. Baseline: utility brains, phases, feints, stamina, squad director; AB1-AB4.
arXiv: memory streams (generative agents), goal planners, grounded dialogue, difficulty directors, procedural quests and encounters, playable-slice generation, vision rubrics. GitHub: CICERO mind contracts (license first), quest graphs, roguelike encounter generators.
Fixture: 20 briefs through AB1-AB4, scored by VK1, a vision judge and a replayed demo. Kill: an unplayable slice passes, memory unbounded, no undo or provenance.

## 10 Content, audio, editor, proof
Home `crates/assets`, `content/`, `crates/audio`, `crates/editor`, `crates/perf`, `crates/proof`, `crates/playtest`. Baseline: CT1-CT5, PF1 budgets and zero-alloc gate, AH heals, PQ bars.
arXiv: quadric simplification, animation compression, FLIP and SSIM, input-latency measurement, soak and leak detection, differential fuzzing. GitHub: glTF sample models, Poly Haven, 100STYLE, Quaternius, meshoptimizer, puffin, cargo-semver-checks.
Fixture: cook x2, triangle budgets, 200-match balance, p95 input ≤ 50 ms, one-hour soak, FLIP catches a seeded exposure bug. Kill: UNVERIFIED license, missing provenance, per-tick allocation, golden moved without a receipt.

## 11 Game feel, camera and combat design
Home `crates/combat`, `crates/game`, `crates/anim`, `docs/FOCUS.md`, `docs/plan/gameplay.md`. Baseline: hit-stop, reactions, camera trauma, move tables (HR3), PQ2 tournament.
arXiv and talks: melee combat design, hit confirmation and input buffering, cancel windows, camera framing for third-person melee, readability of silhouettes, difficulty and flow. GitHub: open-source fighting and melee games (license first), game-feel toolkits, camera rigs.
Fixture: PQ1 latency, PQ2 balance, a playtester session with captures. Kill: feel measured by numbers only with no playtest, or a change that raises input latency.

## 12 Trust and security for AI edits and MCP
Home `crates/endpoint` (sandbox, tools), `crates/provider`, `crates/net` (server authority), `docs/EDITOR_AI_CONTROL.md`. Baseline: typed tools, refusals, undo, recorded provider fixtures.
arXiv: prompt injection through assets and briefs, tool-permission models, sandboxing untrusted content and mods, MCP authentication, cheat and exploit detection. GitHub: MCP security guides and scanners, wasm sandboxes, server-authoritative anti-cheat patterns.
Fixture: a poisoned brief, a malicious asset name and an out-of-scope tool call are each refused and logged; a modified client cannot move the authoritative state. Kill: any write outside the command service, any secret in a log.

## 13 Neural training stack
Home `crates/nn`, `crates/distill`, `crates/ai`, PX5. Baseline: `.e2nn` dense/embed/GRU, int8 whitelist, full-batch SGD, tournament gate.
arXiv: quantization-aware training, offline RL and replay, small-model distillation, cross-toolchain determinism. GitHub: tract, ort, candle, burn (licenses first; the default build stays dependency-light).
Fixture: DT3 tournament and a two-toolchain bit check of the int8 student. Kill: a float in the tick, a dataset without provenance.

## 14 Rust ecosystem sweep
Home repo-wide. Baseline: the Rust repo sweep in `docs/RESEARCH-VERDICT.md`.
GitHub: what Bevy, Fyrox, salsa, wgpu, rapier and the MCP Rust SDK shipped since the last sweep; protocols and editor patterns worth copying as specs. Fixture: each take names the row it tightens. Kill: a new dependency without a DEPENDENCIES row, or a take that duplicates an existing row.

## 15 Spatial intelligence
Home `crates/spatial`, `crates/endpoint` (scene tools), SP6-SP13. Baseline: XZ index, 5 relations, bounded brief, intent solver, staged gate, semantic diff.
arXiv: 3D scene graphs (ConceptGraphs 2309.16650, HOV-SG), LLM layout (LayoutGPT, Holodeck 2312.09067, 3D-Layout-R1 2603.22279), spatial reasoning benchmarks for language and vision models, embodied retrieval (EmbodiedRAG 2410.23968). GitHub: forge `packs/scene` (describe, diagnose, find, camera, pixel math; same owner), O3DE AI companion, Holodeck (license first).
Fixture: SP12 spatial gold eval. Kill: a relation that fails its gold cases, a query over budget, coordinates emitted by a model instead of relations.

**Wave-26 SP15 incremental index** (`engine-spatial`): SP15 implements salsa red-green incremental updates — each mutating edit bumps `revision` and rebuilds only dirty ids' subtrees inside `apply`. `crates/spatial/src/index3.rs` already contains the SP15 comment noting "Single-edit rebuild p95 < 2 ms at 10k objects." Donor: `georust/rstar` (MIT OR Apache-2.0) provides the R*-tree underlying structure. Reimplementation is std-only, no new dependency. The transferable pattern is the R*-tree data structure; the incremental update mechanism is engine-internal.

**Wave-26 MCP10 EXPLAIN gate** (`engine-endpoint`): `crates/endpoint/src/explain.rs` already implements the cost model (`ExplainCaps` with max_objects, max_dirty_subtrees, max_bytes, max_nav_tiles) and `check_batch` with typed `ExplainRefusal`. The wave-26 steal adds the dry-run-cost-cap-refuse gating pattern from `cooper-tretter/bq-guard` (MIT) and `dagryazev/bqcost` (MIT/Apache-2.0). Every refusal carries the violated invariant plus one recovery (CEGIS idea).

## 10 Content, audio, editor, proof

Home `crates/assets/`, `crates/perf`, `crates/editor`, `docs/plan/content.md`, `docs/archive/2026-10-08/plan/render-2040.md`. Baseline: CT1-CT5, PF1 budgets and zero-alloc gate, AH heals, PQ bars.
arXiv: quadric simplification, animation compression, FLIP and SSIM, input-latency measurement, soak and leak detection, differential fuzzing. GitHub: meshoptimizer, xatlas, FLIP/SSIM, cargo-semver-checks.
Fixtures: cook x2, triangle budgets, 200-match balance, p95 input ≤ 50 ms, one-hour soak, FLIP catches a seeded exposure bug. Kill: UNVERIFIED license, missing provenance, per-tick allocation, golden moved without a receipt.
**Wave-43 ARXIV43**: TAKE arXiv:2609.30245v1 (COSA-GS: Practical Compression of 3D Gaussian Splatting — CT11, content compression with bit-exact cross-platform consistency, CC-BY-4.0 ideas-only), CT11 (TAKE, REPRODUCE), CT11 (TAKE, UNVERIFIED), PQ4 (TAKE, REPRODUCE); REJECT Neural 3DGS compression in the cook pipeline (HASH-LAW, float, non-deterministic). See docs/research/archive/arxiv-wave43.md.
