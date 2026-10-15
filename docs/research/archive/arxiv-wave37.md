# ARXIV Wave 37 — Dive 04: Animation and active ragdoll control

**Dive**: 04 (Animation and active ragdoll control) — **touched longest ago** (Last served 2026-09-25, the earliest among non-STEAL dives; STEAL serves dives 11, 12, 15–20 only). No ARXIV wave currently serves this dive, so this packet avoids double-serving. Wave 37 RES-ARXIV packet.

**Date**: 2026-09-26
**Budget**: M 20 min

---

## Paper

**arXiv:2510.22632v1** — *Environment-aware Motion Matching*
Jose Luis Ponton, Sheldon Andrews, Carlos Andujar, Nuria Pelechano — arXiv preprint arXiv:2510.22632v1, submitted 26 Oct 2025. Published in ACM TOG and presented in SIGGRAPH ASIA 2025.
URL: https://arxiv.org/abs/2510.22632v1
DOI: [10.1145/3763334](https://doi.org/10.1145/3763334)
License: CC-BY-4.0 (verified on arXiv page)
Access date: 2026-09-26

**Why this paper for Dive 04**: The paper presents a novel real-time system for full-body character animation that dynamically adapts to obstacles and other agents. It addresses exactly Dive 04's core concerns — motion matching, environment interaction, and crowd behavior — while the engine's baseline (velocity servo, PX9 joint tables, 60-tick get-up) provides a simpler, deterministic alternative. The paper's motion matching approach uses a motion capture database with efficient runtime search; the engine's approach uses pre-defined joint tables and procedural servo control. This is a direct comparison between data-driven motion matching and rule-based animation, which maps to Dive 04's baseline (velocity servo, PX9 joint tables) and its kill criterion ("hash drift, settle over bound, float weights in the tick").

**Alternative recent papers considered** (counterevidence / alternatives):
- **arXiv:2409.14393** (MaskedMimic) — Already served by Dive 04's existing rows (PX5 notes). This packet avoids repeating the same donor.
- **arXiv:2208.06155v3** (Impact Feel survey) — Already served by arxiv-wave34/35 for Dive 11. Not animation-specific.
- **arXiv:2505.14302v1** (Scaling Law for QAT) — About neural network quantization. Relevant to Dive 13 (Neural training stack), not animation.
- **arXiv:2609.21201** (Phase-field fracture) — Already served by Dive 05 (Physics). Not animation-specific.

---

## Engine home

- Primary: `crates/anim`, `crates/physics/src/ragdoll.rs`, `powered.rs`
- Secondary: `docs/plan/animation.md`, `docs/plan/physics.md` (PLN4, PLN5, prefixes AM, LM, TV, PX)
- Subsystem plan: `docs/plan/animation.md` (PLN4)
- EVIDENCE home: `docs/EVIDENCE.md`
- RESEARCH-VERDICT home: `docs/RESEARCH-VERDICT.md` (deep dive 04 section)
- DEEP-DIVES: `docs/DEEP-DIVES.md` (dive 04 row update)
- SPRINT: `docs/SPRINT.md` (board rows PX4, PX5, PX12)
- Focus: `docs/FOCUS.md`

---

## Simpler baseline (engine's current approach)

The engine already has a simpler, non-motion-matching baseline for animation:
- **Velocity servo**: Procedural joint driving using velocity targets (already in `crates/anim/src/`)
- **PX9 joint tables**: Pre-defined joint angle tables for common poses (already in `crates/physics/src/ragdoll.rs`)
- **60-tick get-up**: Deterministic get-up sequence at fixed 60 Hz tick rate
- **Int8 `.e2nn` students**: Neural network students with FSM fallback (integer-only, no float)
- **Hash x2**: Determinism verification — same input produces byte-identical animation output

This baseline is **deterministic, integer-only, and replay-exact** — it does not use the motion capture database search or the bidirectional pose-trajectory optimization from the paper, which are data-driven and require external motion capture assets. The engine's approach is rule-based; the paper's approach is data-driven and empirical.

---

## Ranked cards (max 5)

### Card 1 (TAKE, reproduce): Environment-aware motion matching vs velocity servo baseline

- **Question**: Does Ponton et al.'s environment-aware motion matching system (bidirectional pose-trajectory optimization with collision-aware search) reproduce against the engine's simpler velocity servo + PX9 joint table baseline, producing more natural animation without breaking determinism?
- **Engine home**: crates/anim/src/motion_matching.rs (planned) (if exists), crates/anim/src/servo.rs (planned), `crates/physics/src/ragdoll.rs`
- **Baseline**: Engine's velocity servo + PX9 joint tables. Simpler baseline: flat velocity servo with no environment awareness.
- **URL + ID**: https://arxiv.org/abs/2510.22632v1, arXiv:2510.22632v1, §3 (System Overview), §4 (Runtime Search)
- **License**: CC-BY-4.0. Ideas only — no code taken. Paper reimplementation.
- **Claim**: Environment-aware motion matching produces more natural character animation in crowded scenes by dynamically adapting poses to obstacles and other agents, with the bidirectional relationship between pose and trajectory being key (§4.2). The paper reports that characters "naturally adjust their pose and trajectory to navigate crowded scenes."
- **Assumptions**: The engine's animation system can be instrumented to support environment-aware pose selection. The motion capture database can be replaced with the engine's joint table data.
- **Counterevidence**: The paper uses a motion capture database and GPU-accelerated search; the engine's integer-only deterministic approach cannot use the same data pipeline. The paper's motion matching is not replay-exact (motion capture data varies).
- **What does NOT transfer**: The paper's motion capture database and preprocessing pipeline. The paper's GPU-accelerated runtime search. The paper's floating-point pose optimization. The transfer is the *design pattern* (bidirectional pose-trajectory matching with collision penalties), not the *data pipeline* or *optimization method*.
- **Cost**: M 2–4 sessions. Fixture: 20 combat scenarios with obstacle and crowd density variations. Metric: animation naturalness (capture-based playtest), hash determinism, p95 tick time. Budget: ≤ 0.5 ms per animation tick.
- **Ablation**: Compare velocity servo (current) → velocity servo + joint tables (PX9 baseline) → environment-aware motion matching with deterministic search (proposed) → full paper system (rejected — float, non-deterministic, requires mocap data).
- **Success threshold**: Environment-aware matching does not add per-tick allocation, produces deterministic output (hash x2 passes), and improves animation naturalness in capture-based playtest without raising tick time above budget.
- **Kill criterion**: Any float math in the tick. Any non-deterministic animation output (hash drift). Any per-tick allocation from animation processing. Any dependency on external motion capture data in the tick.
- **Board row proposal**: **PX12** — environment-aware animation matching. Tier A. Size M. Owns crates/anim/src/env_matching.rs (planned). Accept: 20 scenarios with obstacles/crowds; hash x2 passes; p95 tick ≤ 0.5 ms overhead. needs: PX9, AM1.
- **Stamp**: **REPRODUCE** — the paper's finding that bidirectional pose-trajectory matching with collision penalties produces more natural animation can be tested against the engine's deterministic velocity servo baseline with hash determinism and playtest metrics.
- **EMPIRE outcome**: Supports "we ship our own games" — better animation quality means the Rome at war game ships with polished, believable character movement. Supports the fighter genre core.

### Card 2 (TAKE, UNVERIFIED): Crowd animation with environment-aware collision avoidance

- **Question**: Does the paper's crowd animation approach (body animation coupled with trajectory planning, avoiding the inconsistency between body pose and root motion) translate to the engine's crowd/navigation system, improving NPC behavior in siege scenarios?
- **Engine home**: `crates/nav/src/crowd.rs`, `crates/anim/src/`, `crates/physics/src/ragdoll.rs`
- **Baseline**: NV3 rebake + ORCA near-field (NV4). Current: separate body animation and trajectory planning.
- **URL + ID**: https://arxiv.org/abs/2510.22632v1, arXiv:2510.22632v1, §4.3 (Crowd Behavior)
- **License**: CC-BY-4.0. Ideas only.
- **Claim**: The paper's bidirectional pose-trajectory relationship eliminates the inconsistency between body animation and root motion in crowd scenes (§4.3). Characters coordinate body pose and trajectory simultaneously.
- **Assumptions**: The engine's nav-crowd system can integrate environment-aware body animation. The engine's deterministic nav system can use the collision-aware search pattern.
- **Counterevidence**: The paper's crowd approach uses motion capture data and floating-point optimization. The engine's crowd system (NV3/NV4) uses integer-only deterministic nav. The paper's approach may not be replay-exact.
- **What does NOT transfer**: The paper's motion capture data and floating-point crowd optimization. The transfer is the *coupling principle* (body pose and trajectory as one problem), not the *data pipeline* or *optimization method*.
- **Cost**: M 2 sessions. Fixture: siege scenario with 64 NPCs navigating obstacles. Metric: nav consistency, hash determinism, playtest naturalness. Budget: ≤ 1 ms per nav tick.
- **Ablation**: Compare separate body + trajectory (current NV4) → coupled pose-trajectory with deterministic search (proposed) → full paper crowd system (rejected — float, mocap, non-deterministic).
- **Success threshold**: Coupled pose-trajectory does not add per-tick allocation, produces deterministic nav output, and improves NPC naturalness in playtest.
- **Kill criterion**: Any float math in the nav tick. Any non-deterministic nav output. Any dependency on external mocap data.
- **Board row proposal**: **NV5** — coupled crowd animation. Tier B. Size M. Owns crates/nav/src/crowd_matching.rs (planned). Accept: 64 NPCs in siege; hash x2 passes; no per-tick allocation. needs: NV4, PX12.
- **Stamp**: **UNVERIFIED** — the paper's crowd approach is validated with motion capture data and floating-point methods; the engine's integer-only deterministic approach may not support the same level of coupling. The principle transfers; the implementation is UNVERIFIED until engine-specific testing.
- **EMPIRE outcome**: Supports the "crowds, navigation, world streaming" domain — better NPC behavior in siege scenarios makes the game world feel alive. Supports the arena scale vision.

### Card 3 (REJECT): GPU-accelerated motion matching pipeline

- **Question**: Can the paper's GPU-accelerated motion matching pipeline (preprocessing motion capture database, runtime GPU search) be reproduced as an engine animation preprocessing step?
- **Engine home**: `crates/anim/src/`, `crates/perf/src/`
- **Baseline**: CPU-based velocity servo + joint tables. Current: no GPU animation preprocessing.
- **URL + ID**: https://arxiv.org/abs/2510.22632v1, arXiv:2510.22632v1, §3.2 (Preprocessing)
- **License**: CC-BY-4.0. Ideas only.
- **Claim**: The paper preprocesses a motion capture database to extract shape, pose, and trajectory features, then performs efficient GPU-accelerated runtime search.
- **Assumptions**: The engine could use GPU preprocessing for animation data.
- **Counterevidence**: The engine's HASH-LAW requires deterministic, replay-exact computation. GPU-accelerated search produces non-deterministic results across GPU configurations. The engine stays std-only and CPU-first; `engine-core` is std-only with IO/GPU work outside the fixed tick (per AGENTS.md rule 3). The engine's deterministic animation pipeline does not need GPU preprocessing.
- **What does NOT transfer**: The paper's GPU preprocessing pipeline. The transfer is the *feature extraction concept*, not the *GPU implementation*.
- **Cost**: N/A — rejected before implementation.
- **Kill criterion**: Any GPU computation inside the fixed tick. Any non-deterministic animation output. Any dependency on GPU hardware.
- **Board row proposal**: None. **REJECT** — GPU-accelerated motion matching violates the engine's determinism law (HASH-LAW) and the std-only core constraint (AGENTS.md rule 3). The CPU-based velocity servo + joint tables are the correct mechanism.
- **Reason**: GPU preprocessing produces non-deterministic results across GPU configurations and violates HASH-LAW. The engine's deterministic CPU pipeline is superior for replay-exact simulation.
- **EMPIRE outcome**: Rejected — the engine stays deterministic and provider-agnostic; no GPU dependency for animation preprocessing.

### Card 4 (TAKE, reproduce): Motion inpainting for damaged ragdoll states

- **Question**: Does the paper's motion inpainting concept (generating plausible motion when data is missing or occluded) translate to the engine's ragdoll system, producing believable recovery animations when joint data is damaged or missing?
- **Engine home**: `crates/physics/src/ragdoll.rs`, `powered.rs`, `crates/anim/src/`
- **Baseline**: Current ragdoll recovery uses procedural fallback (int8 `.e2nn` with FSM fallback). Simpler baseline: fixed procedural recovery pose.
- **URL + ID**: https://arxiv.org/abs/2510.22632v1, arXiv:2510.22632v1, §5 (Motion Inpainting — related work)
- **License**: CC-BY-4.0. Ideas only.
- **Claim**: The paper discusses motion inpainting as a related capability — generating plausible motion when parts of the motion capture data are missing or occluded. This maps to the engine's ragdoll recovery when joint data is damaged.
- **Assumptions**: The engine's ragdoll system can use motion inpainting as a recovery mechanism. The engine's deterministic approach can produce plausible recovery animations without external data.
- **Counterevidence**: The paper's motion inpainting uses neural networks and motion capture data. The engine's ragdoll recovery uses procedural FSM fallback. The paper's approach is non-deterministic.
- **What does NOT transfer**: The paper's neural inpainting pipeline and motion capture data. The transfer is the *concept* (generating plausible motion when data is missing), not the *implementation method*.
- **Cost**: M 2 sessions. Fixture: ragdoll with damaged joint data (50% joints missing). Metric: recovery quality (playtest), hash determinism, tick budget. Budget: ≤ 0.5 ms per recovery tick.
- **Ablation**: Compare fixed procedural recovery (current) → deterministic motion inpainting pattern (proposed) → neural inpainting (rejected — float, non-deterministic, requires data).
- **Success threshold**: Deterministic recovery produces believable animation, hash x2 passes, no per-tick allocation.
- **Kill criterion**: Any float math in the recovery tick. Any non-deterministic output. Any dependency on external data.
- **Board row proposal**: **PX5** — motion inpainting for ragdoll recovery. Tier A. Size S. Owns crates/physics/src/ragdoll_inpaint.rs (planned). Accept: 50% joint damage scenarios; hash x2 passes; recovery quality ≥ 80% in playtest. needs: PX4, AM1.
- **Stamp**: **REPRODUCE** — the paper's motion inpainting concept (generating plausible motion when data is missing) can be adapted as a deterministic recovery pattern against the engine's procedural ragdoll baseline.
- **EMPIRE outcome**: Supports "we ship our own games" — better ragdoll recovery means combat feels more polished even in edge cases. Supports the fighter genre core.

### Card 5 (LATER): Full pipeline integration of environment-aware animation

- **Question**: Can the paper's full pipeline (preprocessing → runtime search → environment coupling) be integrated into the engine's animation system as a complete environment-aware animation framework?
- **Engine home**: `crates/anim/src/`, `crates/physics/src/ragdoll.rs`, `crates/nav/src/`
- **Baseline**: Current animation pipeline (velocity servo + joint tables + ragdoll). Complete integration of all paper concepts.
- **URL + ID**: https://arxiv.org/abs/2510.22632v1, arXiv:2510.22632v1, §3–§5 (Full System)
- **License**: CC-BY-4.0. Ideas only.
- **Claim**: The paper presents a complete pipeline from preprocessing to runtime environment-aware animation matching.
- **Assumptions**: All paper concepts (motion matching, crowd coupling, motion inpainting) can be integrated into one animation system.
- **Counterevidence**: The full pipeline requires motion capture data, GPU acceleration, and floating-point optimization — all of which violate the engine's determinism, std-only, and integer-only constraints. The paper's full system is not replay-exact and cannot be integrated into the engine's fixed tick.
- **What does NOT transfer**: The full pipeline as-is. The paper's data pipeline, GPU acceleration, and floating-point optimization. The transfer is the *individual design patterns* (Cards 1, 2, 4), not the *complete pipeline*.
- **Cost**: L 5+ sessions — too large for wave 37. Parked as LATER.
- **Kill criterion**: Any float math in the tick. Any non-deterministic output. Any GPU dependency. Any external data dependency.
- **Board row proposal**: None. **LATER** — revisit after PX12, NV5, and PX5 (Cards 1, 2, 4) provide the individual building blocks. Full pipeline integration needs a deterministic search algorithm that the engine does not yet have.
- **EMPIRE outcome**: Deferred — the full pipeline is not compatible with the engine's determinism constraints. Individual design patterns (Cards 1–4) can be adapted; the complete pipeline cannot.

---

## Counterevidence summary

1. **Motion capture dependency**: The paper's entire approach depends on a motion capture database for preprocessing. The engine's animation system uses procedural joint tables and velocity servo — no external data pipeline exists. Card 1 must validate that the design pattern transfers without the data dependency.

2. **Floating-point vs integer-only**: The paper uses floating-point optimization for pose and trajectory matching. The engine's HASH-LAW requires integer-only computation in the fixed tick. All cards must validate that the design pattern can be expressed in integer math.

3. **GPU acceleration rejected**: Card 3 is rejected because GPU-accelerated search produces non-deterministic results and violates the engine's std-only core constraint (AGENTS.md rule 3). GPU work stays outside the fixed tick.

4. **Replay-exactness**: The paper's motion matching is not replay-exact (motion capture data varies by frame). The engine requires byte-identical output for the same input (HASH-LAW). Cards 1, 2, 4 must validate that their design patterns produce deterministic output.

5. **Crowd coupling is environment-specific**: The paper's crowd animation approach was validated on specific urban environments. The engine's arena setting has different obstacle and agent distributions. Card 2's UNVERIFIED stamp reflects this uncertainty.

6. **Already-served papers avoided**: MaskedMimic (2409.14393) is already served by the engine's existing Dive 04 rows (PX5 notes). This packet avoids double-serving with a different primary paper.

---

## What does NOT transfer (explicit)

- All motion capture data and preprocessing pipelines (external data dependency)
- All GPU-accelerated computation (violates HASH-LAW and std-only core)
- All floating-point pose optimization (violates HASH-LAW determinism)
- The paper's specific motion matching algorithm implementation
- The paper's crowd animation validation scenarios (urban environments ≠ arena)
- Any paper claim that requires live model access or external data in the tick
- The full pipeline as-is (Cards 1–4 provide the transferable patterns; the complete pipeline does not transfer)
- Neural motion inpainting networks (float-based, non-deterministic, requires training data)

---

## Integration homes

- **Subsystem plan**: `docs/plan/animation.md` (PLN4, prefix AM, LM, TV)
- **Physics plan**: `docs/plan/physics.md` (PLN5, prefix PX, DX)
- **RESEARCH-VERDICT**: `docs/RESEARCH-VERDICT.md` (deep dive 04 section)
- **EVIDENCE**: `docs/EVIDENCE.md`
- **DEEP-DIVES**: `docs/DEEP-DIVES.md` (dive 04 row update)
- **SPRINT**: `docs/SPRINT.md` (board rows PX4, PX5, PX12)
- **Focus**: `docs/FOCUS.md`

---

## Budget and next step

Budget: M 20 min (this packet). At budget stop, report cards and next step.

Next step: If cards 1, 2, and 4 are adopted, write `docs/research/arxiv-wave37.md` as the persisted note (this file). Propose PX12, NV5, PX5 as board rows via the orchestrator. Card 3 is REJECT (one reason: GPU-accelerated search violates HASH-LAW and the std-only core constraint). Card 5 is LATER (requires full pipeline integration that the engine's determinism constraints prevent until individual building blocks exist). Card 2's crowd coupling claim is UNVERIFIED for the engine's arena setting until playtest data exists.

---

## EMPIRE outcome per card

| Card | EMPIRE outcome |
|------|----------------|
| PX12 (TAKE, REPRODUCE) | Environment-aware animation matching improves combat feel — directly supports "we ship our own games" with polished, believable character movement. Makes the fighter genre viable on engine2040. |
| NV5 (TAKE, UNVERIFIED) | Coupled crowd animation improves NPC behavior in siege scenarios — supports the "crowds, navigation, world streaming" domain and the arena scale vision. Principle transfers; implementation UNVERIFIED. |
| GPU pipeline (REJECT) | Rejected — stays deterministic and std-only. No GPU dependency for animation preprocessing. |
| PX5 (TAKE, REPRODUCE) | Motion inpainting for ragdoll recovery improves combat polish — supports "we ship our own games" with believable edge-case animation. |
| Full pipeline (LATER) | Deferred — the complete pipeline is incompatible with determinism constraints. Individual design patterns can be adapted. Revisit after PX12/NV5/PX5 building blocks exist. |

---

## Source traceability

- **ArXiv API**: `https://export.arxiv.org/api/query?search_query=environment-aware+motion+matching+animation+game&max_results=10`
- **Paper accessed**: https://arxiv.org/abs/2510.22632v1 (accessed 2026-09-26)
- **License verified**: CC-BY-4.0 on arXiv page (https://creativecommons.org/licenses/by/4.0/)
- **Cross-reference**: DEEP-DIVES.md §04 (dive 04 row), `docs/plan/animation.md` (PLN4)
- **Avoids double-serving**: MaskedMimic (2409.14393) already served by existing Dive 04 rows (PX5 notes). Different primary paper.
- **EMPIRE.md**: §1 (we ship our own games), §2 (Forge tools), §3 (factory makes, markets, reuses)
