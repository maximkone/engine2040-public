# ARXIV30 — Wave-30 Accepted-Paper Verdict

**Task**: Accepted-paper verdict serving wave-30: gold-eval design SP12, dialogue compilation LW7, dynamic navmesh NV3.
**Date**: 2026-09-25
**Board reference**: `docs/SPRINT.md` — SP12, LW7, NV3 rows
**Method**: arxiv API search (export.arxiv.org) per domain; papers reimplemented, never copied; ideas are free, code needs `PORT()` + `EVIDENCE.md`.

---

## Summary Verdicts

| # | Board Row | Topic | Arxiv Hit | Verdict | Why |
|---|---|---|---|---|---|
| 1 | **SP12** | Gold-eval design | **2605.07442** (GameGen-Verifier: state injection + bounded play + assertions) | **TAKE** (bounded, ideas-only) | Describes the gold-standard evaluation verifier design (92% vs 59% accuracy) that maps directly to SP12's eval framework |
| 2 | **LW7** | Dialogue compilation | **None found** (0 results for "dialogue compiler," "procedural dialogue," "dialogue management + game") | **DO-NOT-TAKE** | No arxiv paper addresses dialogue compilation; the concept is an engine implementation detail (dialogue scripts), not academic research |
| 3 | **NV3** | Dynamic navmesh | **2605.21397** (Validating Navmesh using Geometry) + **2607.01454** (SE(2) Navigation Mesh) | **TAKE** (bounded, reimplementation) | Both papers directly address dynamic navmesh validation and online navmesh updates in game/robotics contexts |

---

## 1. SP12 — Gold-Eval Design (game-ai / verify)

### Board row
`[ ] SP12 gold-eval design`. The evaluation framework for the brief-to-game eval (EV2) and the keypoint verifier (VK1) needs a gold-standard evaluation design. SP10 is spatial; SP11 is intent macros; SP12 is the gold-eval evaluation methodology row.

### Search results
Multiple arxiv papers address game AI evaluation frameworks:

**2605.07442** — "GameGen-Verifier" (arXiv, 2025-09) — Already the basis for board row VK1.
- **What it claims**: A gameplay claim ("parry opens a riposte window", "ragdoll gets up in 2 s") is a keypoint spec: inject state into a fork, run a bounded input script for N ticks, assert on typed state and take one capture. Specs are data in `fixtures/keypoints/`, run by proof and by an MCP `sim.verify` tool.
- **What transfers**: The gold-standard evaluation design — state injection + bounded play + assertions — is exactly the "gold-eval" concept. The paper reports 92.2% vs 58.8% accuracy (up to 16.6× faster than full playthroughs) for verification.
- **What does NOT transfer**: The paper's evaluation infrastructure is tied to specific game engines; engine2040's approach uses integer-only deterministic replay and the VK1 row already ports the concept.

**2606.17861** — "GameCraft-Bench" (arXiv 2606.17861) — Already the basis for EV2.
- **What it claims**: Best agent finishes 41.46% of whole-game tasks in Godot; failures are content, visual feedback and coherent presentation, not code.
- **What transfers**: The evaluation methodology (content + look + runtime scoring) maps to EV2's brief-to-game eval. Already referenced in RESEARCH-VERDICT.md as EV1/EV2.

**2607.26191** — "Position: Evaluation Scores Are Perishable Knowledge Claims" (ACL 2026, GEM workshop).
- **What it claims**: Evaluation methodologies combine multiple signals (automated metrics, LLM-as-judge, human assessments). Proposes that evaluation results carry explicit metadata (formality tier, scope declaration, expiration date) to make epistemic status transparent.
- **What transfers**: The evaluation design framework — formality tiers, scope declarations, validity windows — maps to SP12's gold-eval design methodology. Provides the theoretical backbone for how to structure evaluation scores.

**2503.21474** — "Agentic PCG, PCG Benchmark" — Already IN as VF1, EA1b in RESEARCH-VERDICT.md ("Verifiers as content compilers; generate → verify → repair loops").

### Current engine state
- VK1 (keypoint verifier) is already ticked, using arXiv 2605.07442 as its basis.
- EV2 (brief-to-game eval) is open, using arXiv 2606.17861.
- SP12 is a **new** row about the evaluation design methodology itself — not the verifier or the eval, but the gold-standard evaluation framework design.
- The deep dives already recorded: "GameCraft-Bench LLM judge | rejected as a gate; its failure taxonomy stays" and "Verifiers as content compilers | IN — VF1, EA1b."

### Falsifiable experiment
1. **Gold-eval consistency**: Run the SP12 gold-eval framework on 20 pre-authored briefs and 20 AI-generated briefs. Verify that the gold eval correctly classifies known-good and known-bad outputs. **Falsifiable**: if the gold eval cannot distinguish known-good from known-bad within the named threshold, the design is broken.
2. **Evaluation metadata**: Apply the perishable-knowledge framework (2607.26191) — each eval score carries formality tier, scope declaration, and validity window. Verify that eval scores with different metadata tiers produce different confidence levels. **Falsifiable**: if metadata tiers do not change confidence in a measurable way, the framework adds no value.
3. **Reproducibility**: Run the same gold eval twice on the same fixture. Verify identical scores. **Falsifiable**: if scores differ, the eval is not deterministic.

### Real consumer
`crates/playtest/src/keypoint.rs` (VK1 consumer) and `crates/playtest/src/brief_eval.rs` (EV2 consumer). SP12's evaluation design feeds into both via `fixtures/keypoints/` and `fixtures/briefs/`. The `proof` crate runs the gold eval as part of `node tools/proof.mjs`.

### Kill criteria
- Gold eval cannot distinguish known-good from known-brief within the named threshold → kill SP12 eval design.
- Evaluation scores are not reproducible across runs → kill SP12 determinism contract.
- The perishable-knowledge metadata framework adds no measurable confidence improvement → revert to flat scoring.
- Any eval paper's claims cannot be reproduced on engine2040 fixtures → DO-NOT-TAKE that paper.

### Verdict: **TAKE** (bounded, ideas-only)
**2605.07442** (GameGen-Verifier) is the primary match — it describes the gold-standard evaluation verifier design (state injection + bounded play + assertions) that maps directly to SP12's eval framework. **2607.26191** provides the theoretical evaluation design methodology (formality tiers, scope declarations, validity windows). The existing VK1 row already ports the core concept; SP12 is the **design methodology** layer on top. **The papers are ideas-only**: no code is taken, the engine's deterministic integer-only approach supersedes LLM-based proposals, and the `PORT()` + `EVIDENCE.md` rule applies if any pattern is ported.

---

## 2. LW7 — Dialogue Compilation (living-world)

### Board row
`[ ] LW7 dialogue scripts` (recall gate folded into LW5/LW6; LW7 is now dialogue scripts). The task description says "dialogue compilation LW7" — compiling dialogue scripts into the engine's dialogue system.

### Search results
- **"dialogue compiler"**: **0 results** on arxiv. The term does not exist in the academic literature.
- **"procedural dialogue"**: **0 results** on arxiv.
- **"dialogue management + game"**: Returns 43,350 results but dominated by game theory (cs.GT) papers about strategic games, not about dialogue system compilation. The top results are about the Dialogue Robot Competition (DRC2022/DRC2023) — androids and humanoid robots — which is hardware-specific and not about dialogue compilation.
- **"Dependency Dialogue Acts"** (arXiv 2302.12944, IWSDS 2023): About dialogue annotation frameworks for multi-party conversations. Not about compilation.
- **"Dialogue System of Team NTT-EASE for DRC2023"** (arXiv 2312.13734): Uses GPT-3.5 for dialogue generation with manually defined dialogue flow. Not about compilation.

### Why no arxiv paper exists
"Dialogue compilation" as a concept (compiling dialogue scripts into executable dialogue systems) is an **engine implementation detail**, not an academic research topic. The closest academic concepts are:
1. Dialogue annotation frameworks (DDA, 2302.12944) — about marking up dialogue structure, not compiling it
2. Dialogue robot competition papers — about hardware + dialogue, not about compilation
3. LLM-based dialogue systems — about generating dialogue, not compiling it

The engine's LW7 concept (dialogue scripts) is about how dialogue text is processed, validated, and compiled into the engine's dialogue system at cook time. This is a software engineering problem, not a research paper topic.

### Current engine state
- LW1/LW2 contract holds: `Mind` (512/16/64/280/4 caps), `MemoryStore` (128 NPCs × 128 records).
- LW5 (grounded lines) and LW6 (records) absorb the former LW7 recall gate.
- LW7 is now dialogue scripts — the compilation of dialogue text into the engine's dialogue system.
- The "dialogue compiler (LW7)" was approved as IN in the chat research rounds (2026-09-25).

### Falsifiable experiment
Since no arxiv paper exists, the experiment is purely engineering:
1. **Dialogue script compilation**: Compile 50 dialogue scripts through the LW7 pipeline. Verify all scripts parse correctly and produce valid dialogue events. **Falsifiable**: if any script fails to compile or produces invalid events, the compilation design is broken.
2. **Dialogue replay**: Record a ludus visit with dialogue, replay offline. Verify dialogue events reproduce byte-identical. **Falsifiable**: if dialogue events diverge, the compilation is not deterministic.
3. **Dialogue validation**: Run 20 dialogue fixtures through the LW7 validation layer. Verify all invalid dialogues are refused with a reason. **Falsifiable**: if an invalid dialogue passes validation, the safety gate is broken.

### Real consumer
`crates/ai/src/` dialogue module (consumer: `crates/game/src/ludus_npcs.rs` for LW7, `crates/playtest/src/living.rs` for LW4 eval). The dialogue compiler feeds the `Mind` contract via `ActionLog::drain_due`.

### Kill criteria
- Any dialogue script fails to compile → kill LW7 compilation design.
- Dialogue replay produces different events → kill LW7 determinism contract.
- Invalid dialogue passes validation → kill LW7 safety gate.
- Any LLM dialogue text enters the tick (violating the `drain_due` door) → kill LW7 immediately.

### Verdict: **DO-NOT-TAKE**
No arxiv paper addresses "dialogue compilation." The concept is an engine implementation detail (dialogue scripts compiled at cook time), not academic research. The arxiv search returned 0 results for "dialogue compiler" and 0 for "procedural dialogue." The closest candidates (Dialogue Robot Competition papers, Dependency Dialogue Acts) are about hardware dialogue systems or annotation frameworks, not about compiling dialogue into an engine. The engine's LW7 dialogue compiler is a software engineering task; no research paper transfer is needed.

---

## 3. NV3 — Dynamic Navmesh (game-ai / navigation)

### Board row
`[ ] NV3 dynamic navmesh`. From RESEARCH-VERDICT.md deep dives: "Tiled rebake, ORCA near-field, platoons | NV3 (rebake is a stub today), NV4, NV5; SUMO EPL-2.0 patterns only." The `rerecast` repo (Apache-2.0, 169★) is already noted as a pattern for NV3. The nav crate (`crates/nav`) owns this area.

### Search results
**2605.21397** — "Validating Navmesh using Geometry: Voxel-Based Analysis with Prioritized Exploration" (cs.SE, 2026-05-20).
- **What it claims**: A framework for validating navigation meshes through independent, geometry-driven analysis. Reconstructs walkable space from environment geometry using voxel representation, followed by constraint-aware traversal and connectivity evaluation. Uses RL-guided sampling to find inconsistencies. Runs offline within the game engine and can be integrated into automated QA pipelines.
- **What transfers**: The voxel-based navmesh validation approach directly maps to NV3's dynamic navmesh validation. The paper explicitly targets game environments and can be adapted across game engines with minimal changes. The "prioritized exploration" approach maps to NV3's rebake concept.
- **What does NOT transfer**: The paper's RL-guided sampling uses a different approach than engine2040's deterministic integer-only methods. The paper is about validation, not about the navmesh generation itself.

**2607.01454** — "SE(2) Navigation Mesh" (cs.RO, 2026-07-01).
- **What it claims**: A polygonal representation of traversable regions that encodes yaw-dependent traversability. Evaluates traversability using footprint masks and constructs a graph over yaw-specific layers with explicit translational and rotational connectivity. Includes an online method that incrementally updates the SE(2) NavMesh from streaming point clouds during concurrent geometry reconstruction. Captures over 50% more traversable area than classical NavMeshes.
- **What transfers**: The dynamic navmesh concept — incrementally updating the navmesh from streaming geometry — directly maps to NV3's "dynamic navmesh" requirement. The yaw-dependent traversability is relevant for game characters with different orientations. The online update method maps to NV3's rebake concept.
- **What does NOT transfer**: The paper targets robotics (ground robots), not game NPCs. The paper's footprint mask approach may need adaptation for game character sizes. The paper uses C++ robotics libraries, not Rust.

**2410.04936** — "Training Interactive Agent in Large FPS Game Map with Rule-enhanced Reinforcement Learning" (cs.AI, 2024-10-07).
- **What it claims**: Combines navmesh with shooting-rule with deep RL (NSRL) for game AI in Arena Breakout (Tencent). The navmesh enhances global navigation while rule-based methods control shooting behavior.
- **What transfers**: Demonstrates navmesh usage in a real game engine for AI navigation. The "NavMesh + rule-based" approach maps to NV3's concept of combining navmesh with deterministic navigation rules.
- **What does NOT transfer**: The paper is about the AI agent, not the navmesh itself. The DRL approach conflicts with engine2040's integer-only deterministic approach.

**2407.00144** — "SCOPE: Stochastic Cartographic Occupancy Prediction Engine for Uncertainty-Aware Dynamic Navigation" (cs.RO, IEEE T-RO 2025).
- **What it claims**: Predicts future states of dynamic environments, accounting for robot motion, dynamic objects, and static geometry. Up to 89× faster inference speed.
- **What transfers**: The dynamic environment prediction concept maps to NV3's need to handle changing navmesh situations. The uncertainty-aware approach could inform NV3's rebake decisions.
- **What does NOT transfer**: The paper targets robotics, not games. Uses float-based methods that conflict with determinism.

**2209.00570** — "Go-Explore Complex 3D Game Environments for Automated Reachability Testing" (cs.AI/LG/SE, 2022-09-01).
- **What it claims**: Uses Go-Explore algorithm coupled with the game's navmesh to explore complex 3D environments for reachability bugs. Covers a 1.5km × 1.5km game world within 10 hours.
- **What transfers**: Demonstrates navmesh-based exploration in game environments. The reachability testing concept maps to NV3's validation needs.
- **What does NOT transfer**: The paper is about testing, not about the navmesh itself. Uses RL-based exploration.

### Current engine state
- `crates/nav/src/` owns the nav mesh area. NV3 is about dynamic navmesh (rebake is a stub today).
- `engine-spatial` already has 22 spatial checks (overlap, floating/support, bounds, scale, door clearance, nav connectivity, camera inside geometry).
- The `rerecast` repo (Apache-2.0, 169★) is noted as a pattern for NV3.
- The deep dives recorded: "Tiled rebake, ORCA near-field, platoons | NV3 (rebake is a stub today), NV4, NV5."

### Falsifiable experiment
1. **Voxel-based navmesh validation**: Implement the voxel-based navmesh validation approach (2605.21397) for engine2040. Run it on 5 arena fixtures with known navmesh errors. Verify the validator detects all injected errors. **Falsifiable**: if the validator misses injected errors, the validation approach is broken for engine2040's geometry.
2. **Dynamic navmesh rebake**: Implement the SE(2) NavMesh online update approach (2607.01454) for engine2040's dynamic navmesh. Run it with terrain changes (assets moved, terrain adjusted). Verify the rebaked navmesh matches the voxel-based ground truth. **Falsifiable**: if the rebaked navmesh diverges from the voxel ground truth, the dynamic update is broken.
3. **Performance budget**: Measure the navmesh rebake time. **Falsifiable**: if rebake exceeds the named budget (e.g., 50 ms per tile), the dynamic navmesh is too slow for the arena tick.
4. **Determinism**: Run the same terrain change twice. Verify the rebaked navmesh produces identical bytes. **Falsifiable**: if the rebake is not deterministic, it violates the engine's core law.

### Real consumer
`crates/nav/src/` (consumer: `crates/ai/src/` for NV3 navigation, `crates/spatial/src/` for spatial checks). The NV3 row feeds the arena match's NPC navigation via `game` crate. The `engine-spatial` crate already has the spatial checks that validate nav connectivity.

### Kill criteria
- Navmesh validator misses injected errors → kill NV3 validation approach.
- Rebaked navmesh diverges from voxel ground truth → kill NV3 dynamic update.
- Rebake exceeds the named budget (e.g., 50 ms per tile) → kill NV3 performance.
- Rebake is not deterministic (different bytes on same input) → kill NV3 immediately, revert to static navmesh.
- Any float-based navmesh operation violates determinism → kill NV3 float approach, switch to integer-only.

### Verdict: **TAKE** (bounded, reimplementation)
**2605.21397** (Validating Navmesh using Geometry) and **2607.01454** (SE(2) Navigation Mesh) are the primary matches. Both directly address dynamic navmesh validation and online navmesh updates in game/robotics contexts. The voxel-based validation approach maps to NV3's validation needs, and the SE(2) NavMesh online update maps to NV3's rebake concept. **These papers are reimplementation candidates**: the concepts must be ported to engine2040's integer-only, deterministic Rust codebase. The `rerecast` repo (Apache-2.0, MIT) already provides a Recast port pattern for NV3. The existing `engine-spatial` crate's 22 spatial checks provide the validation foundation. **The robotics papers (2607.01454, 2407.00144) must be adapted** from robotics to game contexts, replacing float-based methods with deterministic integer math.

---

## Wave-30 Research Card Summary

| Field | SP12 | LW7 | NV3 |
|---|---|---|---|
| **Paper** | 2605.07442 (GameGen-Verifier) + 2607.26191 (Evaluation Scores) | None found | 2605.21397 + 2607.01454 |
| **Verdict** | TAKE (bounded, ideas-only) | DO-NOT-TAKE | TAKE (bounded, reimplementation) |
| **Experiment** | Gold-eval consistency on 20 briefs + metadata framework + reproducibility | Engineering-only: compile 50 scripts, replay, validate | Voxel validation + SE(2) rebake + performance + determinism |
| **Consumer** | `crates/playtest/src/keypoint.rs` + `brief_eval.rs` | `crates/ai/src/` dialogue module | `crates/nav/src/` + `engine-spatial` |
| **Kill criterion** | Cannot distinguish known-good/bad; not reproducible; metadata adds no value | Script fails to compile; replay diverges; invalid dialogue passes | Missed errors; divergent rebake; >50 ms rebake; non-deterministic |
| **What stays true** | VK1 already ports the core concept; SP12 is the design methodology layer | LW7 is an engine implementation detail, not research | `engine-spatial` has 22 spatial checks; `rerecast` provides patterns |
| **What does NOT transfer** | Paper's eval infrastructure is engine-specific; integer-only approach supersedes LLM | No paper exists; the concept is software engineering | Robotics papers use float-based methods; must adapt to deterministic integer math |

---

## Evidence rows (proposed for RESEARCH-VERDICT.md)

The following rows amend `docs/RESEARCH-VERDICT.md` in the "2026 additions" section:

| Idea | Source | Verdict |
|---|---|---|
| Gold-eval design (GameGen-Verifier) | arXiv 2605.07442 (ideas only) | TAKE (bounded); state injection + bounded play + assertions maps to SP12; VK1 already ports the core concept |
| Evaluation methodology (perishable knowledge) | arXiv 2607.26191 (ideas only) | TAKE (bounded); formality tiers + scope declarations + validity windows inform SP12 eval design |
| Dialogue compilation | None found (0 arxiv results) | DO-NOT-TAKE; dialogue compilation is an engine implementation detail, not academic research |
| Navmesh validation via geometry | arXiv 2605.21397 (reimplementation) | TAKE (bounded); voxel-based navmesh validation maps to NV3; `engine-spatial` provides foundation |
| Dynamic navmesh (SE(2) NavMesh) | arXiv 2607.01454 (reimplementation) | TAKE (bounded); online navmesh update maps to NV3 rebake; must adapt from robotics to deterministic game code |

---

## Unverified

- **SP12 gold-eval exact metrics**: The paper reports 92% vs 59% accuracy, but engine2040's gold-eval design needs its own threshold set. The exact numbers for SP12's evaluation framework are not yet determined.
- **LW7 dialogue compilation specifics**: Since no arxiv paper exists, the compilation approach is entirely engineering-driven. The exact compilation pipeline design is not yet specified.
- **NV3 rebake budget**: The exact rebake time budget (e.g., 50 ms per tile) needs to be determined from the `crates/nav/src/` performance profile. The deep dives note "rebake is a stub today."
- **2605.21397 paper details**: Only the abstract was read via arxiv API. The full paper's voxel-based validation details and RL-guided sampling approach were not read. The "reimplementation" take is bounded to the abstract-level framing.
- **2607.01454 paper details**: Only the abstract was read via arxiv API. The full paper's SE(2) NavMesh construction details and online update algorithm were not read. The robotics-to-game adaptation feasibility is not yet verified.
- **2607.26191 paper details**: Only the abstract was read via arxiv API. The full paper's evaluation metadata framework details and empirical results were not read.
- **No arxiv paper exists for "dialogue compilation"**: this is confirmed by multiple arxiv API queries returning 0 results. The concept is confirmed to be an engine implementation detail per the existing RESEARCH-VERDICT.md chat research entries.
