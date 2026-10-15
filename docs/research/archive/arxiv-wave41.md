# ARXIV Wave 41 — Dive 06: Crowds, navigation, world streaming

**Dive**: 06 (Crowds, navigation, world streaming — GA, NV, WD) — **touched longest ago** among non-STEAL, non-ARXIV dives (Last served 2026-09-25; STEAL serves dives 01, 05, 11, 12, 15–20; ARXIV waves 27–40 serve dives 16, 09, 20, 12, 11, 11, 12, 04, 05, 02, 03 respectively). Dives 06, 08, 10, 13 are the remaining unserved dives; Dive 06 is selected as the first non-STEAL, non-ARXIV dive in table order. Wave 41 RES-ARXIV packet.

**Date**: 2026-09-26
**Budget**: M 20 min

---

## Paper

**arXiv:2605.21397v1** — *Validating Navmesh using Geometry: Voxel-Based Analysis with Prioritized Exploration*
Ramesh Raghavan, Ojas Sharma, Sebastien Larrue, Alan Isaac Kunder, Aakash Sai, Rishi Mathur — arXiv preprint arXiv:2605.21397v1, submitted 20 May 2026. Category cs.SE.
URL: https://arxiv.org/abs/2605.21397v1
License: Not verified on arXiv page (no explicit license found). Access date: 2026-09-26

**Why this paper for Dive 06**: The paper presents a framework for validating navigation meshes through independent, geometry-driven analysis — reconstructing walkable space from environment geometry using voxel-based representation, constraint-aware traversal, and connectivity evaluation. This directly addresses Dive 06's core concerns — navmesh baking (Recast-pattern bake), navigation correctness, and world cell streaming — while the engine's baseline (Recast-pattern bake, separation/arrive LOD, no rebake on break) provides a simpler, deterministic alternative. The paper's voxel-based geometry validation can test against the engine's current navmesh bake correctness. The paper runs offline within the game engine and can be integrated into automated QA pipelines, which aligns with the engine's deterministic approach.

**Alternative recent papers considered** (counterevidence / alternatives):
- **arXiv:2407.16771v2** (Topology-Guided ORCA, Jul 2024) — About ORCA-based multi-agent motion planning in constrained environments. Relevant to crowd navigation but focuses on ORCA improvement rather than navmesh validation. Secondary reference for the crowd steering domain.
- **arXiv:2512.08309v4** (InfiniteDiffusion: Open-World Terrain Generation, Dec 2025) — About procedural terrain generation using diffusion models. Relevant to world/terrain generation but focuses on visual fidelity rather than navigation correctness. Secondary reference for the world streaming domain.
- **arXiv:2604.03350v1** (Multi-Stage Workflow for Agent-Based Models, Apr 2026) — About exploring stochastic agent-based models. Relevant to crowd simulation but too abstract for the engine's deterministic nav system. Secondary reference only.
- **arXiv:2203.08975v2** (Survey of MADRL with Communication, 2022) — About multi-agent deep RL. Not directly about navmesh or navigation correctness. Not a good match for Dive 06's deterministic approach.

---

## Engine home

- Primary: `crates/nav/src/` (`tiles.rs`, `rebake.rs`, `mesh.rs`, `links.rs`, `lib.rs`, `flow.rs`, `crowd.rs`, `bake.rs`, `astar.rs`), `crates/world/src/` (`terrain.rs`, `cells.rs`, `blockout.rs`)
- Secondary: `docs/plan/game-ai.md` (PLN8, prefix GA, NV), `docs/plan/world.md` (PLN17, prefix WD)
- Subsystem plan: `docs/plan/game-ai.md` (PLN8)
- EVIDENCE home: `docs/EVIDENCE.md`
- RESEARCH-VERDICT home: `docs/RESEARCH-VERDICT.md` (deep dive 06 section)
- DEEP-DIVES: `docs/DEEP-DIVES.md` (dive 06 row update)
- SPRINT: `docs/SPRINT.md` (board rows NV3, NV4, NV5)
- Focus: `docs/FOCUS.md`

---

## Simpler baseline (engine's current approach)

The engine already has a simpler, non-voxel-based navmesh validation baseline:
- **Recast-pattern bake** (NV3): `bake.rs` with Recast-style navmesh generation from world geometry; `rebake.rs` for break-then-rebake
- **Separation/arrive LOD** (NV4): `crowd.rs` with separation/arrive steering; no rebake on break (NV3), no 500-soldier LOD (PX7)
- **A* pathfinding** (`astar.rs`): Grid-based A* on navmesh tiles
- **Flow fields** (`flow.rs`): Flow field pathfinding for crowd movement
- **Hash x2**: Determinism verification — same input produces byte-identical nav output
- **Integer/Fixed math**: The engine uses Q16.16 fixed-point arithmetic throughout the deterministic core

This baseline is **deterministic, integer-only, and replay-exact** — it does not use the paper's voxel-based geometry validation or reinforcement-learning-guided sampling. The engine's approach is a bake-then-use system; the paper's approach is a geometry-first validation system that checks bake correctness independently.

---

## Ranked cards (max 5)

### Card 1 (TAKE, REPRODUCE): Voxel-based navmesh geometry validation vs Recast bake baseline

- **Question**: Does the paper's voxel-based navmesh geometry validation framework (reconstructing walkable space from environment geometry via voxel representation with constraint-aware traversal and prioritized exploration) reproduce against the engine's simpler Recast-pattern bake baseline (NV3), correctly identifying navmesh inconsistencies that the bake process misses?
- **Engine home**: `crates/nav/src/bake.rs` (NV3, Recast bake), `crates/nav/src/tiles.rs` (navmesh tiles), `crates/nav/src/rebake.rs` (break-then-rebake)
- **Baseline**: Engine's Recast-pattern bake + A* pathfinding (NV3). Simpler baseline: flat bake with no independent validation.
- **URL + ID**: https://arxiv.org/abs/2605.21397v1, arXiv:2605.21397v1, §3 (Voxel Representation), §4 (Constraint-Aware Traversal), §5 (Prioritized Exploration)
- **License**: Not verified on arXiv page. Ideas only — no code taken. Paper reimplementation.
- **Claim**: The paper demonstrates that voxel-based geometry validation consistently detects navmesh inconsistencies (unwalkable regions marked walkable, walkable regions marked unwalkable) with lower exploration effort than RL-based approaches, and can run offline within the game engine (§5).
- **Assumptions**: The engine's navmesh bake can be instrumented to accept voxel-based validation output. The engine's Q16.16 fixed-point arithmetic can express voxel grid traversal. The validation can be run as an offline QA step without affecting the deterministic tick.
- **Counterevidence**: The paper uses reinforcement learning to guide voxel sampling (prioritized exploration), which introduces non-deterministic element selection. The paper's voxel grid uses floating-point geometry reconstruction, potentially violating HASH-LAW. The paper was validated on large-scale open-world environments, not the engine's arena-scale navmeshes. The paper's RL-guided exploration may not be replay-exact.
- **What does NOT transfer**: The paper's RL-guided prioritized exploration sampling strategy. The paper's floating-point voxel reconstruction. The paper's large-scale open-world validation scenarios. The transfer is the *voxel-based geometry validation concept* (reconstructing walkable space from geometry), not the *RL sampling method* or *floating-point pipeline*.
- **Cost**: M 2–3 sessions. Fixture: district and siege maps with break-then-rebake scenarios. Metric: navmesh inconsistency detection rate, bake correctness (hash x2), p95 bake time, memory usage. Budget: bake ≤ 50 ms per rebake event.
- **Ablation**: Compare Recast bake only (current NV3) → Recast bake + simple geometric validation (proposed) → Recast bake + voxel-based validation with deterministic traversal (proposed) → full paper system (rejected — RL-guided, float, non-deterministic).
- **Success threshold**: Voxel-based validation does not add per-tick allocation, produces deterministic validation output (hash x2 passes), and detects ≥ 90% of navmesh inconsistencies in the bake fixture.
- **Kill criterion**: Any float math in the validation tick. Any non-deterministic validation output. Any per-tick allocation from voxel grid. Any RL-guided sampling inside the fixed tick.
- **Board row proposal**: **NV5** — voxel-based navmesh geometry validation. Tier B. Size M. Owns crates/nav/src/navmesh_validate.rs (planned). Accept: district and siege maps with break-then-rebake; hash x2 passes; ≥ 90% inconsistency detection; p95 bake ≤ 50 ms. needs: NV3, NV4.
- **Stamp**: **REPRODUCE** — the paper's finding that voxel-based geometry validation detects navmesh inconsistencies can be tested against the engine's Recast bake baseline with hash determinism and break-then-rebake fixtures.
- **EMPIRE outcome**: Supports "we ship our own games" — validated navmeshes mean the arena game ships with correct crowd and fighter navigation. Supports the "living world" vision (LW/AB) by ensuring navmesh correctness at scale.

### Card 2 (TAKE, UNVERIFIED): Topology-guided ORCA for constrained-environment crowd navigation

- **Question**: Does the paper's topology-guided ORCA approach (constructing a graph to represent traversable region topology, using path-planned waypoints to guide ORCA around static obstacles) translate to the engine's crowd steering system (NV4), improving NPC movement in constrained arena environments?
- **Engine home**: `crates/nav/src/crowd.rs` (NV4, separation/arrive steering), `crates/nav/src/links.rs` (traversal links), `crates/nav/src/flow.rs` (flow fields)
- **Baseline**: Engine's separation/arrive LOD steering (NV4). Current: no topology-aware ORCA guidance.
- **URL + ID**: https://arxiv.org/abs/2407.16771v2, arXiv:2407.16771v2, §2 (Topology-Guided ORCA), §3 (Graph Construction), §4 (Experiments)
- **License**: CC-BY-NC-ND 4.0 (check arXiv page). Ideas only. Paper reimplementation.
- **Claim**: Topology-Guided ORCA outperforms standard ORCA in generating smooth, natural motions of multiple agents in constrained environments with static obstacles (§4). The graph-based topology representation guides agents around obstacles more effectively.
- **Assumptions**: The engine's crowd steering can incorporate topology-guided waypoint planning. The engine's deterministic pipeline can express the topology graph traversal. The ORCA avoidance algorithm can be adapted to integer-only arithmetic.
- **Counterevidence**: The paper's ORCA implementation uses floating-point vector math. The paper's topology graph construction may not be deterministic across platforms. The paper was validated on robotics/social navigation scenarios, not game arena environments. The paper's ORCA may produce non-deterministic frame-to-frame variation.
- **What does NOT transfer**: The paper's floating-point ORCA vector math. The paper's robotics-focused validation scenarios. The paper's non-deterministic graph construction. The transfer is the *topology-guided waypoint concept* (using a graph to guide avoidance), not the *ORCA implementation* or *floating-point math*.
- **Cost**: M 2 sessions. Fixture: siege scenario with static obstacles and 64 NPCs navigating. Metric: navigation smoothness, hash determinism, p95 steer tick time, obstacle avoidance success rate. Budget: ≤ 1 ms per steer tick.
- **Ablation**: Compare separation/arrive steering (current NV4) → topology-guided waypoints with deterministic search (proposed) → full topology-guided ORCA (rejected — float, non-deterministic) → standard ORCA (rejected — float).
- **Success threshold**: Topology-guided waypoints do not add per-tick allocation, produce deterministic nav output (hash x2 passes), and improve obstacle avoidance success in playtest.
- **Kill criterion**: Any float math in the steer tick. Any non-deterministic nav output. Any per-tick allocation from graph traversal.
- **Board row proposal**: **NV5** — topology-guided crowd waypoints. Tier B. Size S. Owns crates/nav/src/topo_guide.rs (planned). Accept: siege with static obstacles; hash x2 passes; obstacle avoidance ≥ 80% in playtest; p95 ≤ 1 ms. needs: NV4, NV3.
- **Stamp**: **UNVERIFIED** — the paper's topology-guided ORCA is validated on robotics/social navigation scenarios with floating-point methods; the engine's integer-only deterministic approach may not support the same level of smoothness. The topology-guided waypoint concept transfers; the implementation is UNVERIFIED until engine-specific testing.
- **EMPIRE outcome**: Supports the "crowds, navigation, world streaming" domain — topology-guided waypoints mean NPCs navigate obstacle-rich arena environments more naturally. Supports the arena scale vision.

### Card 3 (TAKE, REPRODUCE): InfiniteDiffusion terrain generation for world cell streaming

- **Question**: Does the paper's InfiniteDiffusion framework (training-free algorithm reformulating diffusion sampling for lazy, unbounded generation with seamless infinite extent and seed-consistency) translate to the engine's world terrain system (`crates/world/src/terrain.rs`, `cells.rs`), enabling deterministic infinite terrain generation against the simpler Perlin-noise baseline?
- **Engine home**: `crates/world/src/terrain.rs`, `crates/world/src/cells.rs`, `crates/world/src/blockout.rs`, `crates/nav/src/tiles.rs`
- **Baseline**: Engine's procedural noise functions (Perlin noise) for terrain generation. Current: finite terrain chunks with manual stitching.
- **URL + ID**: https://arxiv.org/abs/2512.08309v4, arXiv:2512.08309v4, §2 (InfiniteDiffusion Framework), §3 (Terrain Diffusion), §4 (Experiments)
- **License**: Not verified on arXiv page. Ideas only. Paper reimplementation.
- **Claim**: InfiniteDiffusion enables learned procedural terrain generation with seamless infinite extent, seed-consistency, and constant-time random access, outpacing orbital velocity by 9× on a consumer GPU (§3). The framework couples planetary context with local detail.
- **Assumptions**: The engine's terrain system can adopt the diffusion-based generation pattern. The engine's deterministic pipeline can express the Laplacian encoding and diffusion sampling deterministically. The seed-consistency property maps to the engine's hash determinism requirement.
- **Counterevidence**: The paper uses neural diffusion models, which are floating-point and non-deterministic. The paper's GPU acceleration (9× faster than orbital velocity) requires GPU hardware, violating the engine's CPU-first determinism constraint. The paper's diffusion sampling is not replay-exact. The paper's Laplacian encoding uses floating-point computation.
- **What does NOT transfer**: The paper's neural diffusion model and GPU pipeline. The paper's floating-point Laplacian encoding. The paper's diffusion sampling algorithm. The transfer is the *seed-consistency concept* (deterministic infinite terrain generation with constant-time random access), not the *neural implementation* or *GPU pipeline*.
- **Cost**: M 2 sessions. Fixture: district and siege maps with terrain streaming. Metric: terrain generation correctness, hash determinism, p95 generation time, memory usage. Budget: terrain bake ≤ 100 ms per chunk.
- **Ablation**: Compare Perlin noise terrain (current) → deterministic noise with seed-consistency (proposed) → diffusion-based terrain with deterministic sampling (proposed) → full InfiniteDiffusion (rejected — float, GPU, non-deterministic).
- **Success threshold**: Seed-consistent terrain does not add per-tick allocation, produces deterministic output (hash x2 passes), and enables seamless infinite extent with constant-time random access.
- **Kill criterion**: Any float math in the terrain tick. Any non-deterministic terrain output. Any GPU computation inside the fixed tick. Any dependency on neural network inference.
- **Board row proposal**: **WD3** — seed-consistent infinite terrain generation. Tier B. Size M. Owns crates/world/src/terrain_seed.rs (planned). Accept: seamless infinite terrain with constant-time random access; hash x2 passes; p95 ≤ 100 ms per chunk. needs: WD1, WD2.
- **Stamp**: **REPRODUCE** — the paper's seed-consistency and constant-time random access concept can be adapted as a deterministic terrain generation pattern against the engine's Perlin-noise baseline.
- **EMPIRE outcome**: Supports "we ship our own games" and the living world vision — infinite, seed-consistent terrain means the arena game can stream seamless worlds without bake steps. Supports the "day-to-night clip with no bake step" finish line for terrain.

### Card 4 (REJECT): RL-guided voxel exploration for navmesh validation

- **Question**: Can the paper's reinforcement-learning-guided voxel exploration strategy (using RL to guide sampling toward regions more likely to exhibit navmesh inconsistencies) be reproduced as an engine navigation QA feature?
- **Engine home**: `crates/nav/src/` (nav core), `crates/ai/src/` (AI RL)
- **Baseline**: Engine's deterministic A* pathfinding and Recast bake. Current: no RL-guided exploration in the tick.
- **URL + ID**: https://arxiv.org/abs/2605.21397v1, arXiv:2605.21397v1, §4.2 (RL-Guided Sampling)
- **License**: Not verified. Ideas only.
- **Claim**: The paper uses RL to guide voxel sampling toward regions more likely to exhibit inconsistencies, lowering exploration effort while maintaining similar defect detection coverage (§4).
- **Assumptions**: The engine could use RL-guided exploration for navmesh QA.
- **Counterevidence**: The paper's RL-guided exploration uses floating-point neural network inference, violating HASH-LAW. The RL-guided sampling is non-deterministic across platforms and runs, producing different exploration sequences each time. The engine's deterministic core requires integer-only, replay-exact computation. RL-guided exploration would need to run outside the fixed tick, making it a separate QA pipeline rather than an engine feature. The paper's RL model requires training data and retraining for different environments.
- **What does NOT transfer**: The paper's RL-guided sampling strategy. The paper's neural network inference. The paper's non-deterministic exploration order. The transfer is the *concept* of prioritized validation, not the *RL implementation*.
- **Cost**: N/A — rejected before implementation.
- **Kill criterion**: Any float math in the tick. Any non-deterministic exploration output. Any RL inference in the fixed tick. Any dependency on trained models.
- **Board row proposal**: None. **REJECT** — RL-guided voxel exploration violates the engine's determinism law (HASH-LAW) and the std-only core constraint (AGENTS.md rule 3). The deterministic voxel-based validation (Card 1) is the correct mechanism.
- **Reason**: RL-guided exploration produces non-deterministic sampling sequences and requires floating-point neural network inference, violating HASH-LAW. Card 1 provides the deterministic alternative.
- **EMPIRE outcome**: Rejected — the engine stays deterministic and provider-agnostic. Card 1 (deterministic voxel validation) is the correct path.

### Card 5 (LATER): Full navmesh validation + crowd navigation integration

- **Question**: Can the full pipeline (voxel-based navmesh validation + topology-guided ORCA crowd navigation + seed-consistent terrain generation) be integrated into the engine's navigation and world systems as a complete living-world navigation framework?
- **Engine home**: `crates/nav/src/`, `crates/world/src/`, `crates/ai/src/`
- **Baseline**: Complete navigation and world system with all paper concepts integrated.
- **URL + ID**: All paper sections, §3–§5 (Full System)
- **License**: All papers' licenses. Ideas only.
- **Claim**: The papers present complete pipelines for navmesh validation, crowd navigation, and terrain generation.
- **Assumptions**: All paper concepts can be integrated into one navigation/world system.
- **Counterevidence**: The full pipeline requires floating-point RL-guided exploration (Card 4), floating-point ORCA vector math (Card 2), and neural diffusion models (Card 3) — all of which violate the engine's determinism, integer-only, and std-only constraints. The full pipeline is not replay-exact and cannot be integrated into the engine's fixed tick.
- **What does NOT transfer**: The full pipeline as-is. The papers' RL-guided exploration, floating-point ORCA, and neural diffusion implementations. The transfer is the *individual design patterns* (Cards 1, 2, 3), not the *complete pipeline*.
- **Cost**: L 5+ sessions — too large for wave 41. Parked as LATER.
- **Kill criterion**: Any float math in the tick. Any non-deterministic output. Any GPU dependency. Any neural network dependency.
- **Board row proposal**: None. **LATER** — revisit after NV5 (Card 1), NV5 (Card 2), and WD3 (Card 3) provide the individual building blocks. Full pipeline integration needs a deterministic adaptive nav system that the engine does not yet have.
- **EMPIRE outcome**: Deferred — the complete pipeline is incompatible with determinism constraints. Individual design patterns (Cards 1–3) can be adapted; the complete pipeline cannot.

---

## Counterevidence summary

1. **Floating-point vs integer-only**: All three primary papers use floating-point computation (voxel reconstruction, ORCA vector math, diffusion sampling). The engine's HASH-LAW requires integer-only Q16.16 fixed-point computation in the deterministic core. Cards 1, 2, 3 must validate that their design patterns can be expressed in integer math or isolated in the presentation/QA layer.

2. **Non-determinism**: The voxel paper uses RL-guided sampling; the ORCA paper uses floating-point vector math; the InfiniteDiffusion paper uses neural diffusion models. All three introduce non-deterministic elements that violate the engine's replay-exactness requirement (HASH-LAW). Cards must validate that their deterministic alternatives produce byte-identical output for the same input.

3. **GPU dependency**: InfiniteDiffusion explicitly targets GPU acceleration (9× faster than orbital velocity). The engine's AGENTS.md rule 3 requires IO/GPU/provider work outside the fixed tick and keeps `engine-core` std-only. Card 3 must validate that terrain generation does not require GPU hardware.

4. **Domain mismatch**: The voxel paper was validated on large-scale open-world environments; the ORCA paper on robotics/social navigation; the InfiniteDiffusion paper on planetary-scale terrain. The engine's arena setting has different scale, obstacle density, and terrain complexity. Cards 1, 2, 3 must validate their concepts at arena scale.

5. **QA pipeline vs tick integration**: The voxel validation paper explicitly runs offline as a QA pipeline. This aligns with the engine's architecture (validation outside the tick) but means Card 1 is a QA tool, not a runtime feature. The engine's nav system remains deterministic at runtime.

6. **Already-served papers avoided**: The arxiv-wave37 (Dive 04, 2510.22632), arxiv-wave38 (Dive 05, 2608.17833), arxiv-wave39 (Dive 02, 2604.15044), arxiv-wave40 (Dive 03, 2605.11536) papers are not repeated. This packet targets Dive 06 exclusively with new papers.

---

## What does NOT transfer (explicit)

- All RL-guided voxel exploration sampling (Card 4 — rejected, float, non-deterministic)
- All floating-point ORCA vector math (Card 2 — float, non-deterministic)
- All neural diffusion models and GPU pipelines (Card 3 — float, GPU, non-deterministic)
- The papers' large-scale open-world validation scenarios (domain mismatch)
- The papers' robotics-focused navigation experiments (domain mismatch)
- All non-deterministic frame-to-frame variation (violates HASH-LAW)
- All GPU computation inside the fixed tick (violates AGENTS.md rule 3)
- All neural network inference in the tick (violates std-only core)
- The papers' full pipelines as-is (Cards 1–3 provide the transferable patterns; the complete pipeline does not transfer)
- All paper claims that require external data, non-deterministic computation, or specialized hardware

---

## Integration homes

- **Subsystem plan**: `docs/plan/game-ai.md` (PLN8, prefix GA, NV), `docs/plan/world.md` (PLN17, prefix WD)
- **RESEARCH-VERDICT**: `docs/RESEARCH-VERDICT.md` (deep dive 06 section)
- **EVIDENCE**: `docs/EVIDENCE.md`
- **DEEP-DIVES**: `docs/DEEP-DIVES.md` (dive 06 row update)
- **SPRINT**: `docs/SPRINT.md` (board rows NV3, NV4, NV5)
- **Focus**: `docs/FOCUS.md`
- **Multiplayer plan**: `docs/plan/multiplayer.md` (PLN6) — nav/crowd connects to MP arena scenarios
- **World plan**: `docs/plan/world.md` (PLN17) — terrain streaming connects to WD rows

---

## Budget and next step

Budget: M 20 min (this packet). At budget stop, report cards and next step.

Next step: If cards 1, 2, and 3 are adopted, write `docs/research/arxiv-wave41.md` as the persisted note (this file). Propose NV5 (Card 1), NV5 (Card 2), and WD3 (Card 3) as board rows via the orchestrator. Card 1 is TAKE with REPRODUCE stamp. Card 2 is TAKE with UNVERIFIED stamp (the ORCA approach is float-based; the topology-guided waypoint concept transfers but engine-specific testing is needed). Card 3 is TAKE with REPRODUCE stamp (the seed-consistency concept can be adapted deterministically). Card 4 is REJECT (one reason: RL-guided exploration violates HASH-LAW with non-deterministic sampling). Card 5 is LATER (requires full pipeline integration that the engine's determinism constraints prevent until individual building blocks exist).

---

## EMPIRE outcome per card

| Card | EMPIRE outcome |
|------|----------------|
| NV5 voxel-based navmesh validation (TAKE, REPRODUCE) | Validated navmeshes mean the arena game ships with correct crowd and fighter navigation. Supports "we ship our own games" (EMPIRE.md §1) and the living world vision (LW/AB). Navmesh correctness at scale enables the siege scenarios. |
| NV5 topology-guided crowd waypoints (TAKE, UNVERIFIED) | Topology-guided waypoints mean NPCs navigate obstacle-rich arena environments more naturally. Supports the "crowds, navigation, world streaming" domain and the arena scale vision. Principle transfers; implementation UNVERIFIED for the engine's deterministic pipeline. |
| WD3 seed-consistent infinite terrain (TAKE, REPRODUCE) | Infinite, seed-consistent terrain means the arena game can stream seamless worlds without bake steps. Supports "we ship our own games" and the living world vision. Supports the "day-to-night clip with no bake step" finish line for terrain. |
| RL-guided voxel exploration (REJECT) | Rejected — stays deterministic and std-only. No RL dependency in the tick. Card 1 provides the deterministic alternative. |
| Full pipeline integration (LATER) | Deferred — the complete pipeline is incompatible with determinism constraints. Individual design patterns (Cards 1–3) can be adapted. Revisit after NV5/WD3 building blocks exist. |

---

## Source traceability

- **ArXiv API**: `https://export.arxiv.org/api/query?search_query=navmesh+validation+geometry+voxel&max_results=5` → arXiv:2605.21397v1; `https://export.arxiv.org/api/query?search_query=ORCA+multi-agent+crowd+constrained+obstacle&max_results=10` → arXiv:2407.16771v2; `https://export.arxiv.org/api/query?search_query=InfiniteDiffusion+terrain+procedural+world&max_results=5` → arXiv:2512.08309v4
- **Paper accessed**: https://arxiv.org/abs/2605.21397v1, https://arxiv.org/abs/2407.16771v2, https://arxiv.org/abs/2512.08309v4 (accessed 2026-09-26)
- **License**: arXiv:2605.21397v1 — not verified on arXiv page; arXiv:2407.16771v2 — CC-BY-NC-ND 4.0 (check page); arXiv:2512.08309v4 — not verified on arXiv page. Ideas only, no code taken.
- **Cross-reference**: DEEP-DIVES.md §06 (dive 06 row), `docs/plan/game-ai.md` (PLN8), `docs/plan/world.md` (PLN17)
- **Avoids double-serving**: Dives 01, 02, 03, 04, 05, 07, 09, 11, 12, 15–20 already served by STEAL or ARXIV (waves 27–40). Dive 06 is the first non-STEAL, non-ARXIV served dive in table order.
- **EMPIRE.md**: §1 (we ship our own games), §4 (AI makes whole games), Race section (forge + engine2040 both live forever)
- **Donor dedup**: All prior ARXIV and STEAL donors excluded from new-take consideration.
