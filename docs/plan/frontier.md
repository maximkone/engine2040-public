# Frontier (research to a spatial, AI-native, fast engine)

Pre-merge text: `git show a0e13976:docs/plan/frontier.md`. Merged in below: `docs/archive/2026-10-10/plans/futures.md` (the topic table of GAP and COLD rows, the Tier A GAP list, FU1-FU8 as a table; its target-design, hard-problem and donor sections are dropped) and `docs/archive/2026-10-10/plans/NORTH-STAR-2026.md` (the gap line per pillar, the NS1-NS24 table, the quality gates; the steal lists and the S47 history are dropped). Both files move to `docs/archive/2026-10-10/plans/`; their pre-merge text is in git.

Goal: every research idea earns a spike (one test, one baseline, a kill rule) before it becomes a row.

Acceptance:
1. Each spike has one test, one baseline and a dated kill rule.
2. `node tools/steal-next.mjs` still picks the coldest topic from the section 1 table (its rows keep the Capability, Status and Evidence columns).
3. Each NS row names an F2P test, and golden `d57b5c8ae3096614` stays unchanged.

Order: the seven GAP topics first (section 2), then the COLD topics coldest first, then the table in section 7 (FU1-FU8, NS rows in ID order).

Owner crates: `tools/` (steal-next, research readers) and one crate per spike.

Open board IDs: FU1 FU2 FU3 FU4 FU5 FU6 FU7 FU8 NS1 NS3 NS4 NS6 NS7 NS8 NS9 NS10 NS11 NS12 NS13 NS14 NS15 NS16 NS17 NS18 NS19 NS20 NS21 NS22 NS23 NS24 NS32c NS32d NS32e

## 1. Where it stands

Research volume is high, research yield is near zero. Numbers on 2026-09-27:

| Capability | Status |
|---|---|
| Research output | 27 per-wave files, verdict + sweep ~680 lines each |
| Ideas reproduced | none in wave 46: no spike run, no integrated gains |
| Reader cost | researcher 167M tokens a day, 75 runs, 14.7% own calls failing |
| Experimentalist | grade A (2 of 2 ADOPT with numbers), 2 runs in 24 h |
| Spatial core | 16 files in `crates/spatial/src` (afford, compose present; SP22/SP26 landed); A6 spatial proof met (EV3, wave 44) |
| Spatial open | stacked placement (SP16), repair hints (SP17 wiring-only), hierarchical brief (SP19), question verbs (SP20) — SP9/SP13/SP15/SP18/SP29 ticked since the 2026-09-27 count |
| Spatial scorer | gold eval 100 tasks with typed errors and integer-cm oracles, 2618 lines |
| AI-agnostic core | A6 met: 20 authoring + 5 play tasks x mock/local/remote + external MCP client, 75/75 green |
| Look | A4 partial: forward PBR live; GPU skinning unit-tested, unused by game per render-look.md:60; GX3/GX5 open on the reference GPU (RTX 3050 laptop, O5) |
| Speed | PF1/PF2/PF12 ticked; PF4/PF5/PF9 open; no arena frame-time baseline on the reference GPU |
| HUD overlay orphan | `DuelOverlay` (names, HP pips, tick, flash, x2-identical bytes) committed, no board row |
| Camera orphan | director 4230 lines ticked (GP3); staged/damping work present beside it, distinct from combat-feel moonshot |
| Fighter incoming orphan (CLOSED 2026-10-02: CT19 landed `d89113a`, `content/fighters/incoming/` exists on disk; was true at base `e76c0f2` only) | `content/fighters/` has atlas/bundle/glb/manifest/previews/sidecars/tools plus `incoming/` since the CT19 cook |
| PLAYTEST48 pin | CLEAN white-screen gate (pinned release runtime, 5 play-steps t10-t180, 2 smokes, staged 600-tick 27 hits 3 knockdowns, 0 errors, twin cube + circles + noise, none white); sim alive, pixels placeholder |
| PLAYTEST48 captures | 8 PNGs opened + 2 smoke logs + staged receipt; B1-B7 numbered with repro; confusion screen + feel scores (readability 2, feel 2, fairness 3, replay 2) |
| Frame checks home | `frame_check.rs` owns black/missing/clipping only (QA2 ticked; no white_min/frozen gate); QA4 white detail + QA14 frozen detail boarded open (needs QA2 ticked = ready); no twin/play-step frozen fixtures |
| Coherence home | `coherence.rs` owns hit-synced impact audio (PQ8 ticked, 10 scenarios + 0-alloc); logs carry no per-hit loudness/clipping/delta |
| Afford home | `afford.rs` exists (SP22 landed); SP29 deepens it after SP22, BET-SP27 bakes behind it |
| Cycle check | missing (PLN50 rows it as SP28) |
| Compose collision | SP26 landed owns `compose.rs`; SP30 deepens it after, never beside |
| Timeline collision | PF9 owns open `rerun_sink.rs`; PF16 deepens it after, never beside |
| Profiler present | puffin scopes behind `profile`, off-by-default switch; no conditional ScopeId, no tick-frame marks |
| SP32 probe-sync | rowed open (SPIKE49 ADOPT 87.8% vs 71.1%, ~0.001 ms/object; needs SP12) |
| Cluster z-span | RT19 spike ran 2026-09-29 ADOPT-grade (tightened 0.5-20 + early-z reject cuts 35.4% at brute identical, digest x2) but the §7 RT19 row was NEVER boarded (0 hits in SPRINT.md history 2026-10-02; the 2026-10-01 merge note "new row RT19 only" did not land); re-proposed via fixdrift packet 2026-10-02 (needs GX2, done) |
| Conflict audit legs | boarded open (SP38 owns the home; SP55 baseline tape + SP56 proof leg, 60/60 + 20/20 + digest `05b20def` x2, needs SP38) |
| Frame-aware verb | scoped detail, no second row (SP20 owns the home; SP58 scopes SP20; SP60 baseline tape boarded open, needs SP7+SP10, no `frame_verb` in code yet) |

What is wrong is the loop, not the people: four readers every wave produce cards; nothing runs them; the verdict file grows. The fix is a ladder with a spike in the middle, and a map of where the frontier is. PLAYTEST48 adds the second half: play first, then row what breaks — sim solid, look FAIL stands, R1-R8 below are pixels before juice.

### Topic table (docs/archive/2026-10-10/plans/futures.md section 1: GAP and COLD rows)

Report: `center/research/FRONTIER-COVERAGE.md` (snapshot 2026-09-29T23:10Z,
99 topics: 7 GAP, 24 COLD, 62 OK, 6 HOT; the writer is retired, so the
snapshot no longer refreshes). The live pick is the Tier A GAP list in §2
plus `node tools/steal-next.mjs` (fewest unspiked cards first);
open-prompts GAP now has code hits. Counts are
lexical mentions, not implementation proof.

| Capability | Status | Evidence (file or commit) |
|---|---|---|
| Audio middleware class (DSP graphs, mixers, buses, adaptive music, voice limiting) | GAP: 0 plan files, 0 research | `crates/audio/src/dsp.rs:1` |
| AI-native editor (scene diff, semantic undo, co-editing, prompt to scene) | GAP: 0 plan files, 0 research | `crates/editor/src/commands.rs:1` |
| Open prompts and system prompts (prompt libraries, tool descriptions, evals) | GAP: 0 plan files, 0 research | `crates/endpoint/src/discover.rs:1` |
| Learned physics and graph-network simulators | GAP at write; 2026-09-30: integer tick shape ACCEPTED as BX13 (`body-and-break.md`, boarded needs BX12), learned float authority KILLED same run; FU4 stays offline advisor | `crates/physics/src/lib.rs:1` (fixed-dt semi-implicit Euler bodies, no learned step anywhere; `learned|gns|graph_network|message_pass` 0 hits 2026-09-30) + `crates/destruction/src/bonded.rs:549` (`refresh_support` binary flood only, no loads — BX13 unlanded) |
| Differentiable rendering | GAP: 0 plan files, 0 research | `crates/render/src/lib.rs:1` |
| AI material and texture generation | GAP: 0 plan files, 0 research | `crates/assets/src/texture.rs:1` |
| Engine comparison tables (features, benchmarks, coverage vs Unreal, Unity, Godot, Bevy, O3DE, Flax, Stride) | GAP: 0 plan files, 1 research | `crates/playtest/src/spatial_eval.rs:1` |
| Facial animation and lip sync from audio, blendshape retargeting | COLD: 2 plan files, 0 research | `crates/anim/src/lipsync.rs:1` |
| Physical sound synthesis (modal, waveguide, procedural SFX) | COLD: 0 plan files, 0 research | `crates/audio/src/bank.rs:1` |
| Neural audio (neural TTS, neural codecs, generative SFX) | COLD: 0 plan files, 0 research | `crates/audio/src/voice.rs:1` |
| Neural rendering (neural denoisers, neural materials, radiance cache) | COLD: 0 plan files, 0 research | `crates/render/src/gi.rs:1` |
| NeRF and Gaussian splats as a content type | COLD: 0 plan files, 0 research | `crates/render/src/present.rs:1` |
| Neural texture compression (learned codecs, NTC) | COLD, OPEN at runtime: 0 plan files, 0 research | `crates/assets/src/texture.rs:15` |
| ML upscaling (learned super-resolution, ray reconstruction) | COLD: 0 plan files, 0 research | `crates/render/src/aa_scale.rs:26` |
| Virtual geometry (meshlets, visibility buffer, indirect draws) | COLD: 0 plan files, 1 research (VG12 spike open) | `crates/render/src/draw.rs:1` |
| Virtual texturing (sparse VT, streaming tiles, transcoding) | COLD: 0 plan files, 0 research | `crates/assets/src/stream.rs:1` |
| Procedural vegetation (wind, LOD, GPU instancing) | COLD: 0 plan files, 0 research | `crates/world/src/sky.rs:1` |
| GPU particles at scale (10k+ emitters, indirect dispatch) | COLD: 1 plan file (RP2 pool), 0 frontier research | `crates/render/src/particles_gpu.rs:1` |
| MPM and material-point sand/snow (authoritative stays CUT) | COLD: 0 plan files, 0 research | `crates/physics/src/lib.rs:1` |
| Water surface (waves, buoyancy probe, no tick coupling) | COLD: 0 plan files, 0 research | `crates/world/src/terrain.rs:1` |
| Physics-based character control (torque/force, not kinematic) | COLD: 0 plan files, 1 research (kcc open) | `crates/physics/src/kcc.rs:1` |
| GPU physics and collision (broadphase offload, tick stays CPU) | COLD: 2 plan files, 1 research (FRONTIER-COVERAGE 2026-10-01) | `crates/physics/src/broadphase.rs:1` |
| Vegetation wind sway (gust over blades, presentation only) | COLD: 4 plan files, 1 research | `crates/world/src/sky.rs:12` |
| Hierarchical commanders (squad directives, LLM advisors, no tick branch) | COLD: 1 plan file, 1 research (legion open) | `crates/ai/src/legion.rs:1` |
| 3D-scan import (photogrammetry cleanup, LOD, no tick path) | COLD: 0 plan files, 0 research | `crates/assets/src/mesh_cook.rs:1` |
| Wheeled vehicles (suspension + tires, detach/snapshot stay UC19) | COLD: 1 plan file (UC19 open), 1 research | `crates/physics/src/vehicle.rs:1` |
| Soft-tissue deformation (verlet + tissue, authoritative CUT) | COLD: 1 plan file, 1 research (cloth PX2) | `crates/physics/src/cloth.rs:1` |
| Cinematic sequencing (cuts, holds, no tick coupling) | COLD: 0 plan files, 0 research | `crates/game/src/director.rs:1` |

## 2. Finish line

- **Tier A (next 10 waves):** research yield, not volume. At least one experimentalist spike per wave on the top ACCEPTED card; 5 spikes with numbers against a baseline; 3 of them turned into board rows; 1 landed and moving a finish bar (A4, P1, C1-C3 or an SP row). Per-wave research files stop. PLAYTEST48 slice: R1 + R2 land first pixels (twin + staged show the fight), R5 + R6 lock readability and the white-screen gate, all moving A4/C1/P2. Lean pipeline (w2-08, 2026-10-03): rows/cards >= 1.0 — every ACCEPTED card opens its spike row by T+2 with T+7 expiry, no orphan CARDs; no verdict by T+7 marks the line `EXPIRED <date>`, counted as loss and re-proposable once only; a pillar/part pile >= 6 unspiked gates new cards (spike first, NOOP).
- **Tier B (30 waves):** each of the five pillars below has one landed, measured frontier feature, and `docs/research/LADDER.md` shows the path card -> spike -> row -> commit -> bar for each. Feel slice: GP25 + CB4 + GP26 land framing and hit layers with A-B numbers, SN10 lands hit-sync numbers.
- **Tier C:** the engine is the reference "spatial engine for AI": a public benchmark (the 20 A6 tasks plus a spatial suite) that any MCP client can run, with engine2040 results next to other engines' published ones.

### Futures (docs/archive/2026-10-10/plans/futures.md section 2, Tier A GAP list)

- **Tier A (this plan lands):** all 7 GAP topics plus the coldest COLD topics (FU1-FU31) each own one cpu-light spike test in their consumer crate with one baseline number posted (fixture, candidate, metric, number). No GAP topic stays at zero plan files; LADDER carries 31 SPIKE lines or dated rejections with recheck triggers. Bar: no GAP topic left in §1 without a spike or a dated rejection, and every FU row proves bytes x2 (or docs x2 for FU7) with the golden `d57b5c8ae3096614` unchanged.
- **Tier B (v1 hardening):** winners graduate to one boarded follow-up each behind their consumer parent (FU1 SN9/SN12, FU2 ED7, FU3 AX10, FU8 AM/SN5). Bar: 3 FU spikes to ROW, 1 LANDED moving A4/P1/C3/SP; losers become REJECTED LADDER lines, never silent drops.
- **Tier C (2040 tier):** offline-only stays offline: gradients (FU5), learned/MPM/torque/GPU/commander/vehicle/soft advisors (FU4/FU19/FU21/FU22/FU27/FU29/FU30, never in tick), AI-material + NTC/transcode/scan encode (FU6/FU13/FU23/FU28, runtime decoders stay OPEN), neural audio bake (FU10, TTS refused), neural/NeRF bake + presentation (FU11/FU12), upscale adapter behind TAAU (FU14, never default), wind/buoyancy/cinematic presentation (FU25/FU26/FU31, never in tick); FU7 is the Tier C benchmark input (fixed tasks, O5 adapter line, version/source/date); FU24 tooling graduates behind HR5. O5 carries adapter + driver + power-mode lines.
- **What a 2040-level version would be:** every frontier topic either
  holds a landed row with a consumer and a number, or holds a dated
   rejection with a recheck trigger; readers pick the coldest Tier A topic
   with room (fewest unspiked cards per `node tools/steal-next.mjs`).
   Presentation never moves the hash.

## 3. Target design

**The five pillars** (every research packet names one; the rotation cursor is
`docs/research/LADDER.md` "Pillar cursor"):
1. **Spatial understanding for AI** — the scene legible to any model as facts,
   not pixels: typed scene graph with relations, affordances, free-space
   queries, deterministic checks on every model proposal; models give intent,
   engine code owns geometry. Frontier: 3D scene graphs for agents, spatial
   VQA grounding, language-to-layout with a solver, affordance fields,
   composition scores, spatial memory across edits.
2. **AI-native and AI-agnostic** — one command door (MCP + typed API) for every
   model, local or remote, same results; counterfactual forks; causal replay;
   verifiers as content compilers; distilled integer policies (`.e2nn`).
   Frontier: tool-list size vs task success, small local models on the 20
   tasks, learned verifiers, world-model-assisted play-tests.
3. **Fast** — data-oriented ECS, job graph, GPU-driven submission, hot reload
   within 1 s (A10 met), zero hitches while streaming. Frontier: archetype vs
   sparse storage for AI queries, GPU culling and indirect draws, compile/link
   time for the edit loop, spatial index update cost.
4. **Good looks, measured** — GPU-only lighting (RT tier), virtual geometry,
   denoising, temporal upscaling; every claim on the reference GPU with
   captures. Frontier: ReSTIR DI/GI vs DDGI vs radiance cascades on a laptop
   RTX 3050, meshlet virtual geometry in wgpu, neural denoisers vs SVGF-class
   filters, Gaussian splats as content.
5. **AI builds the game** — brief -> level -> encounter -> playable slice
   through the command door, with fork play-tests. Frontier:
   constraint-passing layout generation (Holodeck/LayoutVLM style), playtest
   agents that find seeded bugs, NPC minds that replay without providers.

### North star pillars: the gap in one paragraph each

- **Pillar 1 — Physics: ragdolls, articulated bodies, destruction, cloth.** no generic articulated chain (winch plus rope plus load move as one); fracture always cuts cold (Voronoi pre-compute has no cook cache); ragdoll-vs-ragdoll clinches interpenetrate; no cross-engine oracle proves our joints and contacts against Jolt/Rapier.
- **Pillar 2 — Animation: skeletal, blend, IK, motion matching, physics-driven.** blend graphs are code, not agent-authorable data; no full-body solve for carry and climb; motion matching has no style or terrain dials; nets run offline only, no live tracker drives the powered-ragdoll servos.
- **Pillar 3 — Rendering look (D8/D9: own renderer only, viewer dropped).** the window still draws cubes (arena-slice §1); no skin/eye/hair tier; BreakEvents have no GPU particle feed; no measured probe-GI decision.
- **Pillar 4 — AI-native: agent API and MCP, LLM NPC brains, deterministic replay.** provider parity is measured only for spatial tasks, not editor-plus-game verbs; no LLM NPC brain with provider-off deterministic replay; the refusal catalog is a fixed 20 briefs; no brief-to-game bundle that fails a seeded bad build.
- **Pillar 5 — In-game asset generation (D8/D9: S47 pipeline body dropped).** intake covers rough prop meshes only — no texture/material, sound, motion, or second mesh source; no content-addressed cache with output hashes.
- **Pillar 6 — Rome-at-war crowds.** kit and LOD are rowed but no reacting behavior model drives them; no arena ingress/egress flow proof; formations proven at squad scale only, not legion scale; crowd determinism at scale unproven.

## 4. Hard problems

| Problem | Technique | Why | Reference |
|---|---|---|---|
| Models misplace objects | relation-based placement solved by engine code, then spatial checks; model proposes relations, never coordinates | keeps geometry exact and replayable | LayoutGPT (arXiv 2305.15393), Holodeck (2312.09067), LayoutVLM (2412.02193) |
| Scene too big for a context window | hierarchical scene graph brief: rooms -> groups -> objects, with a token budget and "expand" verbs | bounded brief is already the A6 design | ConceptGraphs (2309.16650), HOV-SG (2403.17846), SayPlan (2307.06135) |
| Spatial questions ("what is left of the gate?") answered wrongly | answer from the graph with engine code; the model only asks | no hallucinated geometry | SpatialVLM (2401.12168), SpatialRGPT (2406.01584) |

## 7. Board tasks

Ticked rows are dropped; rows never boarded are not listed. Table: FU1-FU8 (futures) and the NS rows (north star), with the `needs` column.

| ID | Title | Tier | needs |
|---|---|---|---|
| FU1 | audio-middleware spike | B | SN6 |
| FU2 | AI-native editor spike | A | EV3 |
| FU3 | open-prompt eval spike | A | EV3 |
| FU4 | learned-physics rollout spike | C | PX1 |
| FU5 | differentiable-rendering spike | C | RA1 |
| FU6 | AI-material offline-encode spike | C | CT1 |
| FU7 | engine-comparison rubric pin | A | EV3 |
| FU8 | facial lip-sync map spike | B | CT7 |
| NS1 | articulated chains as one constraint graph | - | PX1 |
| NS3 | ragdoll-vs-ragdoll clinch contacts | - | PX1, AR2 |
| NS4 | joint and contact conformance vs Jolt/Rapier oracles | - | PX1 |
| NS6 | full-body IK solve for carry and climb | - | TV1, PX4 |
| NS7 | motion-matching style and terrain dials | - | LOC2, LMM1 |
| NS8 | live motion tracker driving PX1 servos | - | PX1, LMM1 |
| NS9 | look-rubric auto-score over captures | - | RA14 |
| NS10 | skin, eye and hair tier | - | VS5b |
| NS11 | BreakEvent to GPU particle feed | - | RA4 |
| NS12 | probe-GI decision spike | - | RT14 |
| NS13 | editor-plus-game verb conformance | - | EV1, MCP5 |
| NS14 | LLM NPC brain with provider-off replay | - | EA1e, LW5 |
| NS15 | refusal catalog growth loop | - | MCP8 |
| NS16 | brief-to-game bundle that fails a seeded bad build | - | CT4, EV1 |
| NS17 | texture and material intake | - | CT40, CT28 |
| NS18 | sound intake for event-to-sound tables | - | CT40 |
| NS19 | motion intake as BVH through the CT4 loop | - | CT40, LOC2 |
| NS20 | second mesh source plus content-addressed cache | - | CT40, CT28, AS1 |
| NS21 | reacting-crowd behavior model as data | - | CT64, AU2 |
| NS22 | vomitoria ingress/egress flow proof | - | NV2, CT25 |
| NS23 | formation locomotion LOD at legion scale | - | GA2, AM18 |
| NS24 | crowd determinism harness | - | PX16 |
| NS32c | quiet-box p99 for the arena adapter | - | NS32a |
| NS32d | second-PC replay of the scripted match | - | NS32a |
| NS32e | scripted slot on a walled scene through the door | - | NS32b |

## 8. Quality gates

- `docs/research/LADDER.md` exists and every research return adds a line (retro checks it, the drift check reads the file's age).
- A spike line carries: fixture, baseline, candidate, metric, numbers, hash check when the tick is touched.
- No row from research without a spike, except a pure documentation fix.
- Research tokens per landed row are reported every retro; the goal is under a quarter of today's (167M tokens a day for near zero rows).
- Every frontier row keeps the golden `d57b5c8ae3096614` x2 unless its acceptance names the migration receipt (ARC7 pattern).
- Reference-GPU numbers carry the adapter + driver + power-mode line, or the spike does not count toward A4/C1.
- Every playtest row pins its runtime (build bytes + scene + seed + adapter) or refuses before it starts; captures carry digests + manifests.
- QA4 fails proof on a seeded white/black fault; UI4 fails when log counts differ from painted counts; GP26 fails GP21's tick until the blind call is recorded.
- PLN50 determinism: SP28/SP29/SP30 prove bytes x2 with integer-only math; PF14/PF15/PF16 prove the engine hash x2 with the profiler gated off. Kill bars are gates, never waivers: any FP (SP28), a refusal without its derivation (SP29), disabled overhead above 2 ns/op (PF14), or replay drift (BET-SP31) kills the row with numbers. One owner per new file: `afford.rs` (SP22 then SP29), `compose.rs` (SP26 then SP30), `rerun_sink.rs` (PF9 then PF16); second implementations are refused in review.
- RT19 kill bars are gates, never waivers: pair cut <15% or any brute mismatch or any digest drift kills the row with numbers; tick untouched, golden `d57b5c8ae3096614` held x2. QA14 kill bars are gates, never waivers: any miss, any FP, or any digest drift kills the row; presentation/playtest only, tick untouched.
- Steal-spike 2026-10-03 kill bars are gates, never waivers: any model call at play or any nondeterminism kills CA1; a single missed seeded fault kills SQ1; any nondeterminism or any float in orders kills SB1; any golden drift or any float in tick kills the GW1 spike seat; any bank/dsp change or any live-synth path kills FU9. New prefixes CA/SQ/SB board here as spike-only IDs (orchestrator adds the VISION Plans-map + README prefix-table lines on boarding); no second graph, no second engine, no model in the fixed tick, ever.
- Research-merge gates: no second row for scoped details (BX13/R6 ride sibling plans; NV13/OPT17/GX11 ride theirs once spiked); HELD stays held without numbers; no row without a spike (all 2026-10-01b candidates `not run`); no second row while the home row stays open (MCP11 open, WD14 open owns absent `gen/wfc.rs`); sibling kill bars (BX13 loads 8/8 + digest x2 + 1ms, R6 unique>=8 + flick<=2 + digest x2) ride sibling plans, never this one. 01c collisions: QA3-explorer has no QA3 (BT brains open); GW has no GW row here (docs/archive/2026-10-10/plans/axiom.md PROPOSED owns GW1-GW7); SP50 open corrects the LADDER:555 inline REJECT; MCP11 ticked takes hardening details with no second row; model-in-tick plus copyleft/proprietary/UNVERIFIED bytes stays never (axiom non-commercial verified do-not-take 2026-10-01).

### North star quality gates (docs/archive/2026-10-10/plans/NORTH-STAR-2026.md)

- Golden `d57b5c8ae3096614` unchanged by every NS row (P2P in each Accept).
- Every NS Accept names an F2P test (fails today) with a number, never a vibe.
- No sim math outside `+ - * /`, `sqrt`, integer or fixed; presentation out
  of checkpoint types by grep (body-and-break §8 rides along).
- Community-license generators stay offline; weights never tracked
  (workspace-check LFS guard watches).
- Donor-covered rows prove through our doors (VS9 captures, proof corpus),
  never through another engine's internals (FOCUS D9).

