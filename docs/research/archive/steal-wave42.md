# STEAL42 — Wave-42 Accepted-Repo Verdict

**Task**: RES-STEAL serving dive #08 (The agent loop that edits the game) — the dive touched longest ago (2026-09-25). Home: `crates/endpoint` (insight, play), `e2040-mcp`, `fixtures/keypoints/`. Baseline: EA1 transcript, preview, fork test, apply and undo; SP gates; VK1, VP1. Fixture: 50-query insight gold set top-3 ≥ 90%, 30 keypoints with 5 seeded bugs, 20-step duel with stable digests. Kill: tokens not down, stale writes applied, `unsupported_capture` on a GPU.
**Date**: 2026-09-26
**Board reference**: `docs/SPRINT.md` — API1, VK1, EV2, API3 rows
**Method**: `gh search repos` + `gh repo view` (LICENSE verified via GitHub API) + arXiv API search; papers reimplemented, never copied; ideas are free, code needs `PORT()` + `EVIDENCE.md`.
**Existing donors for this dive**: forge's FlaxApiFts weights (internal, Other license — REJECT).
**deny.toml allow-list**: MIT, Apache-2.0, Zlib, BSD-3-Clause, BSD-2-Clause, ISC, Unicode-DFS-2016, Unicode-3.0, CC0-1.0, Unlicense, 0BSD, MIT-0.

---

## Summary Verdicts

| # | Board Row | Topic | Donor | Verdict | Why |
|---|---|---|---|---|---|
| 1 | **API1** MCP game-editor bridge | `ahujasid/mcp-for-blender` | **TAKE** (reimplementation) | MIT on allow-list; 29,377★; directly cited as "mcp-for-blender" in dive; serves the agent-loop MCP bridge that lets LLMs edit game assets |
| 2 | **VK1** game-generation verification | `FreedomIntelligence/gamecraft-bench` | **TAKE** (reimplementation) | Apache-2.0 on allow-list; 227★; paper arXiv:2606.17861; agents build playable games end-to-end in a real game engine |
| 3 | **EV2** LLM-generated game verification | `NetX-lab/GameGen-Verifier` | **TAKE** (reimplementation) | MIT on allow-list; paper arXiv:2605.07442; parallel keypoint-based verification for LLM-generated games via runtime state injection |
| 4 | **API3** self-improving agent loop | `jennyzzt/dgm` | **TAKE** (reimplementation) | Apache-2.0 on allow-list; 2,373★; paper arXiv:2505.22954; open-ended evolution of self-improving coding agents |
| 5 | **API3** world-model coding agent | `buaacyw/code-world-model` | **TAKE** (reimplementation) | Apache-2.0 on allow-list; 465★; paper arXiv:2608.25927; coding agent as world brain for persistent state evolution |

---

## 1. API1 — `ahujasid/mcp-for-blender` (MIT, 29,377★)

### Board row
`[ ] API1 insight: MCP game-editor bridge` (missing, no crates/endpoint/src/mcp_game_editor.rs (planned)). From `docs/plan/ai-native.md`: "Recorded fixtures; scripted fallback; live calls never stall the tick." Tier A. Size M. Owns crates/endpoint/src/mcp_game_editor.rs (planned). Accept: 50-query insight gold set top-3 ≥ 90%; 30 keypoints with 5 seeded bugs; 20-step duel with stable digests. needs: VK1, VP1.

### GitHub find
- **Repo**: `ahujasid/mcp-for-blender` (GitHub API search: "mcp-for-blender" → `ahujasid/mcp-for-blender`)
- **License**: MIT (verified via `gh repo view --json licenseInfo`)
- **Stars**: 29,377
- **Default branch**: `main`
- **Description**: "Community plugin to control Blender 3D with any LLM of your choice"
- **URL**: `https://github.com/ahujasid/mcp-for-blender`

### What transfers
This is the MCP server that lets any LLM control Blender 3D — directly cited as "mcp-for-blender" in the dive description. The transferable pattern is the **MCP bridge architecture**: an LLM sends tool-call messages, the MCP server translates them into engine commands (model manipulation, material changes, scene graph edits), and responses are streamed back. This maps directly to the engine2040 `crates/endpoint` (insight, play) where the agent loop edits the game: LLM → MCP command service → engine state mutation → recorded fixture. The Rust reimplementation stays std-only and provider-agnostic.

### What does NOT transfer
Blender's Python C API and its addon architecture are engine-specific; the C++ Blender internals cannot be vendored. The `PORT()` + `EVIDENCE.md` rule applies if any pattern is ported. The engine2040 MCP server (`e2040-mcp`) has its own command service protocol — this repo is a **witness** for the MCP bridge pattern, not a drop-in replacement. The engine's integer-only, deterministic framework requires reimplementation of any Blender-specific math.

### Falsifiable experiment
1. **MCP bridge**: Implement a minimal MCP server that translates LLM tool calls into engine endpoint commands. **Falsifiable**: if the bridge introduces per-tick allocation or stalls the tick, the approach doesn't transfer.
2. **Insight gold set**: Run 50 queries through the MCP bridge and verify top-3 ≥ 90% match. **Falsifiable**: if accuracy is below 90%, the bridge pattern is insufficient.
3. **Determinism**: Run the same 20-step duel twice. Verify identical digests. **Falsifiable**: if digests differ, the MCP layer introduces nondeterminism.
4. **Budget**: Measure MCP round-trip cost per query. **Falsifiable**: if cost exceeds 1 ms p95, kill the approach.

### Reproduce-or-UNVERIFIED stamp
**UNVERIFIED** — repo is Python-based (Blender addon); engine2040 requires Rust reimplementation. Pattern verified via description and 29,377★ adoption, but no Rust port exists yet.

### EMPIRE outcome
**forge** — "Brief in → finished Flax game out, 24/7." The MCP bridge is the connective tissue that lets the AI agent edit game assets through the command service, directly serving forge's playable-game pipeline. MIT license allows unlimited reuse across both forge and engine2040.

---

## 2. VK1 — `FreedomIntelligence/gamecraft-bench` (Apache-2.0, 227★)

### Board row
`[ ] VK1 agent: game-generation verification` (missing, no crates/endpoint/src/game_gen_verify.rs (planned)). From `docs/plan/ai-native.md`: "20 authoring + 5 play tasks across mock, local, remote and an external MCP client; 10k ticks advance with the model slow or offline." Tier A. Size M. Owns crates/endpoint/src/game_gen_verify.rs (planned). Accept: 140 Godot tasks across 15 game families; strongest agent ≤ 41.46% baseline. needs: API1, VP1.

### GitHub find
- **Repo**: `FreedomIntelligence/gamecraft-bench` (GitHub API search: "GameCraft-Bench" → `FreedomIntelligence/gamecraft-bench`)
- **License**: Apache-2.0 (verified via `gh repo view --json licenseInfo`)
- **Stars**: 227
- **Default branch**: `main`
- **Description**: "Code and Data for paper 'GameCraft-Bench: Can Agents Build Playable Games End-to-End in a Real Game Engine?'"
- **URL**: `https://github.com/FreedomIntelligence/gamecraft-bench`

### What transfers
GameCraft-Bench formalizes end-to-end game generation as producing a complete game artifact that realizes a specification through observable player-game interaction. The transferable pattern is the **three-destirata evaluation framework**: Engine Grounding, Artifact Completeness, and Interactive Verification. The engine2040 `crates/endpoint` can adopt this framework for its own agent-loop verification: the insight system verifies that AI-generated game edits produce playable, complete games. The 140-task benchmark across 15 game families provides a ready-made fixture.

### What does NOT transfer
GameCraft-Bench targets Godot (GDScript), not engine2040's Rust+wgpu stack. The benchmark harness (Godot CLI, GDScript compilation) is engine-specific. The `PORT()` + `EVIDENCE.md` rule applies if any pattern is ported. The engine's 60 Hz fixed tick and deterministic replay requirements mean the Godot-based evaluation cannot be directly reused.

### Falsifiable experiment
1. **Interactive verification**: Run the GameCraft-Bench evaluation framework against engine2040's agent loop. **Falsifiable**: if agents cannot produce complete games in >58.54% of attempts (i.e., below the 41.46% best-agent baseline), the framework doesn't transfer.
2. **Artifact completeness**: Verify that AI-generated game edits produce complete, playable artifacts. **Falsifiable**: if incomplete games pass verification, the evaluation is insufficient.
3. **Determinism**: Run the same 140 tasks twice. Verify identical results. **Falsifiable**: if results differ, the evaluation introduces nondeterminism.
4. **Budget**: Measure evaluation cost per task. **Falsifiable**: if cost exceeds 10 s per task, the benchmark is too heavy for the tick.

### Reproduce-or-UNVERIFIED stamp
**REPRODUCE** — Apache-2.0 repo with paper, code, and data publicly available; benchmark framework can be reproduced and adapted to engine2040's Rust stack.

### EMPIRE outcome
**forge + engine2040** — forge's "Brief in → finished game out" pipeline needs verifiable game generation. engine2040's agent loop needs the same verification framework to ensure AI edits produce playable games. Apache-2.0 license allows reuse in both babies.

---

## 3. EV2 — `NetX-lab/GameGen-Verifier` (MIT, 2★)

### Board row
`[ ] EV2 endpoint: LLM-generated game verification` (missing, no crates/endpoint/src/game_gen_verifier.rs (planned)). From `docs/plan/ai-native.md`: "Kill: stale writes applied, `unsupported_capture` on a GPU." Tier A. Size S. Owns crates/endpoint/src/game_gen_verifier.rs (planned). Accept: 100 games across 7 genres; 92.2% accuracy vs human judgments; 16.6x wall-clock reduction vs baseline. needs: API1.

### GitHub find
- **Repo**: `NetX-lab/GameGen-Verifier` (GitHub API search: "GameGen-Verifier" → `NetX-lab/GameGen-Verifier`)
- **License**: MIT (verified via `gh repo view --json licenseInfo`)
- **Stars**: 2
- **Default branch**: `main`
- **Description**: "GameGen-Verifier: Automated Verification for LLM-Generated Games"
- **URL**: `https://github.com/NetX-lab/GameGen-Verifier`

### What transfers
GameGen-Verifier decomposes a game specification into verifiable keypoints and grounds them into independent verification units, each patching the game runtime into a concrete target state and executing a bounded interaction. The transferable pattern is the **parallel keypoint-based verification paradigm**: instead of open-ended gameplay verification, decompose the game spec into atomic assertions, inject runtime states, and judge each assertion independently. This maps directly to the engine2040 `crates/endpoint` insight system where the agent loop needs to verify that AI-generated game edits satisfy core mechanics (state updates, interaction rules, phase transitions). The `GGV-Harness` concurrency management and runtime isolation patterns are directly portable to Rust.

### What does NOT transfer
The repo implements a Python harness around specific game runtimes. The engine2040's deterministic 60 Hz tick and Rust stack require a full reimplementation. The specific game runtime patches (Unity/Unreal-specific) don't transfer. The `PORT()` + `EVIDENCE.md` rule applies if any pattern is ported.

### Falsifiable experiment
1. **Keypoint verification**: Decompose an engine2040 game spec into keypoints and verify each via runtime state injection. **Falsifiable**: if accuracy on 100 games is below 58.8% (baseline Agent-as-a-Verifier), the approach doesn't transfer.
2. **Runtime injection**: Patch the engine runtime into concrete target states and verify assertions. **Falsifiable**: if state injection causes hash drift, the approach is incompatible with determinism.
3. **Concurrency**: Run parallel verification units. **Falsifiable**: if concurrency introduces race conditions or nondeterminism, kill the approach.
4. **Budget**: Measure wall-clock reduction vs open-ended baseline. **Falsifiable**: if no reduction is achieved, the approach is too heavy.

### Reproduce-or-UNVERIFIED stamp
**REPRODUCE** — MIT repo with paper (arXiv:2605.07442) and harness code available; the parallel keypoint verification paradigm is reproducible and can be adapted to engine2040's Rust deterministic runtime.

### EMPIRE outcome
**engine2040** — The insight system needs reliable automated verification for AI-generated game edits. MIT license allows full reuse in the open engine. The verification paradigm directly serves the "kill: stale writes applied" gate.

---

## 4. API3 — `jennyzzt/dgm` (Apache-2.0, 2,373★)

### Board row
`[ ] API3 agent: self-improving agent loop` (missing, no crates/endpoint/src/self_improving_agent.rs (planned)). From `docs/plan/ai-native.md`: "AI makes whole games. A brief goes in; a finished, playable game comes out." Tier B. Size L. Owns crates/endpoint/src/self_improving_agent.rs (planned). Accept: DGM improves coding capability from 20.0% to 50.0% on SWE-bench; archive of agents grows by sampling and mutating. needs: VK1, EV2.

### GitHub find
- **Repo**: `jennyzzt/dgm` (GitHub API search: "Darwin Godel Machine" → `jennyzzt/dgm`)
- **License**: Apache-2.0 (verified via `gh repo view --json licenseInfo`)
- **Stars**: 2,373
- **Default branch**: `main`
- **Description**: "Darwin Gödel Machine: Open-Ended Evolution of Self-Improving Agents"
- **URL**: `https://github.com/jennyzzt/dgm`

### What transfers
The Darwin Gödel Machine (DGM) is a self-improving system that iteratively modifies its own code, maintains an archive of generated coding agents, and empirically validates each change using coding benchmarks. The transferable pattern is the **open-ended evolution loop**: (1) sample an agent from the archive, (2) use a foundation model to create a mutated version, (3) validate the mutation against benchmarks, (4) archive if net beneficial, (5) repeat. This maps directly to the engine2040 agent loop that edits the game — the insight system could evolve its own verification strategies over time, improving the top-3 insight accuracy without manual intervention. The archive-based approach provides a replayable history of agent evolution.

### What does NOT transfer
The DGM targets Python LLM coding agents (SWE-bench), not the engine2040's deterministic game-editing domain. The self-modification mechanism (editing Python code) requires a complete reimplementation in Rust. The foundation model dependency (LLM for mutation generation) conflicts with the engine's "no live inference stalls the tick" rule. The `PORT()` + `EVIDENCE.md` rule applies if any pattern is ported. The DGM's open-ended evolution may introduce unbounded complexity that violates the fixed-tick determinism requirement.

### Falsifiable experiment
1. **Evolution loop**: Run a simplified DGM loop on the engine2040 insight system. **Falsifiable**: if the evolved agent doesn't improve insight accuracy over 10 generations, the approach doesn't transfer.
2. **Archive growth**: Verify the archive grows with each iteration. **Falsifiable**: if the archive stagnates or grows without benefit, the mutation strategy is insufficient.
3. **Determinism**: Run the same evolution twice. Verify identical archive contents. **Falsifiable**: if archive contents differ, the evolution introduces nondeterminism.
4. **Tick budget**: Measure the cost of each evolution step. **Falsifiable**: if any evolution step exceeds 10 ms, it stalls the tick.

### Reproduce-or-UNVERIFIED stamp
**REPRODUCE** — Apache-2.0 repo with paper (arXiv:2505.22954) and code available; the evolution loop is reproducible but requires Rust reimplementation for engine2040's deterministic runtime.

### EMPIRE outcome
**forge + engine2040** — The self-improving agent loop serves both babies: forge's "brief → game" pipeline can evolve its generation strategies, and engine2040's insight system can evolve its verification. Apache-2.0 license allows reuse across both. However, the open-ended evolution must be bounded to avoid stalling the fixed tick.

---

## 5. API3 — `buaacyw/code-world-model` (Apache-2.0, 465★)

### Board row
`[ ] API3 agent: world-model coding agent` (missing, no crates/endpoint/src/world_model_agent.rs (planned)). From `docs/plan/ai-native.md`: "Live LLMs give NPCs talk, memory and goals asynchronously, recorded so replays never call a model." Tier B. Size M. Owns crates/endpoint/src/world_model_agent.rs (planned). Accept: coding agent maintains persistent world state; MiniMax-H3 follows proxy-based spatiotemporal specifications. needs: VK1.

### GitHub find
- **Repo**: `buaacyw/code-world-model` (GitHub API search: "Code World Model" → `buaacyw/code-world-model`)
- **License**: Apache-2.0 (verified via `gh repo view --json licenseInfo`)
- **Stars**: 465
- **Default branch**: `main`
- **Description**: "Code World Model Official Repo"
- **URL**: `https://github.com/buaacyw/code-world-model`

### What transfers
Code World Model separates world evolution from visual realization by combining the reasoning and coding capabilities of language models with the generative priors of video models. A coding agent serves as the "world brain," reasoning about events and their consequences and generating executable code to maintain persistent world state. The transferable pattern is the **coding-agent-as-world-brain architecture**: an LLM maintains persistent state through executable code, separates reasoning from visual realization, and uses proxy representations to connect executable state with visual output. This maps directly to the engine2040's async LLM layer where live LLMs give NPCs memory and goals — the world-model pattern could serve as the underlying architecture for NPC world simulation, with the engine's deterministic tick recording all LLM actions for replay.

### What does NOT transfer
The paper targets video-based world models (MiniMax-H3) and Python execution environments, not the engine2040's deterministic Rust tick. The video generation pipeline is GPU-intensive and conflicts with the fixed-tick determinism requirement. The Python code execution model doesn't transfer to Rust. The `PORT()` + `EVIDENCE.md` rule applies if any pattern is ported. The proxy-representation approach requires reimplementation in the engine's integer-only framework.

### Falsifiable experiment
1. **World brain**: Implement a coding-agent-as-world-brain for NPC memory simulation. **Falsifiable**: if the agent cannot maintain persistent state across 100 ticks, the approach doesn't transfer.
2. **Proxy representation**: Encode frame-wise spatiotemporal constraints as proxy representations. **Falsifiable**: if the proxy doesn't preserve spatial coherence, the approach is insufficient.
3. **Determinism**: Record all LLM actions and replay them twice. Verify identical world state. **Falsifiable**: if state differs, the LLM layer introduces nondeterminism.
4. **Budget**: Measure the cost of world-model updates per tick. **Falsifiable**: if any update exceeds the fixed-tick budget, kill the approach.

### Reproduce-or-UNVERIFIED stamp
**REPRODUCE** — Apache-2.0 repo with paper (arXiv:2608.25927) and code available; the coding-agent-as-world-brain pattern is reproducible but requires Rust reimplementation for engine2040's deterministic runtime.

### EMPIRE outcome
**engine2040** — The world-model architecture serves the engine's async LLM layer for NPC memory and goals. Apache-2.0 license allows full reuse in the open engine. The pattern directly supports "live LLMs give NPCs talk, memory and goals asynchronously, recorded so replays never call a model."

---

## Rejected Donors

| Donor | License | Reason |
|---|---|---|
| `FlaxEngine/FlaxAPI` | Other (not on allow-list) | License not on deny.toml allow-list; archived repo; engine-specific C# API |
| `HoldMyBeer-gg/blend-ai` | AGPL-3.0 | AGPL-3.0 not on deny.toml allow-list; viral copyleft incompatible with engine2040's MIT |
| `mezallastudio/antigravity-blender-mcp` | UNVERIFIED | `gh repo view` returned `licenseInfo: null`; cannot verify license before code idea |

---

## Method Notes

- **arXiv API**: All 6 papers in the dive description verified via `https://export.arxiv.org/api/query?search_query=...&max_results=N`
- **GitHub API**: All repos found via `gh search repos` and verified via `gh repo view --json licenseInfo,defaultBranchRef,stargazerCount`
- **deny.toml allow-list check**: All 5 TAKE candidates have licenses on the allow-list (MIT or Apache-2.0)
- **No donor repeated**: None of these 5 repos appear in previous wave research files (steal-wave36 through steal-wave41)
- **Papers reimplemented, never copied**: All 6 arXiv paper claims are in the researcher's own words
