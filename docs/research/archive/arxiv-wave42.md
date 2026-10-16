# ARXIV Wave 42 — Dive 08: The agent loop that edits the game

**Dive**: 08 (The agent loop that edits the game — API, VK, EV, VP, MCP) — **touched longest ago** among non-STEAL, non-ARXIV dives (Last served 2026-09-25; STEAL serves dives 01, 02, 05, 06, 07, 11, 12, 15–20; ARXIV waves 37–41 serve dives 04, 03, 02, 12, 11, 09 respectively). Dives 08, 09, 10, 13, 14 remain unserved by any ARXIV wave; Dive 08 is selected as the oldest non-STEAL, non-ARXIV dive with the most arXiv-relevant research content (agent editing, code-as-harness, game generation). Wave 42 RES-ARXIV packet.

**Date**: 2026-09-26
**Budget**: M 20 min

---

## Paper

**arXiv:2605.18747v1** — *Code as Agent Harness*
Xuying Ning, Katherine Tieu, Dongqi Fu, Tianxin Wei, Zihao Li, Yuanchen Bei, Jiaru Zou, Mengting Ai, Zhining Liu, Ting-Wei Li, Lingjie Chen, Yanjun Zhao, Ke Yang, Bingxuan Li, Cheng Qian, Gaotang Li, Xiao Lin, Zhichen Zeng, Ruida Wang, Rui Pan, Chenyuan Yang, Dylan Zhang, Liri Fang, Zikun Cui — arXiv preprint arXiv:2605.18747v1, submitted 18 May 2026. Category cs.CL.
URL: https://arxiv.org/abs/2605.18747v1
License: Not verified on arXiv page (no explicit license found). Access date: 2026-09-26

**Why this paper for Dive 08**: The paper frames the shift where code is no longer only a target output but increasingly serves as the **operational substrate for agent reasoning, action, environment modeling, and execution-based verification**. This directly addresses Dive 08's core concern — the agent loop that edits the game — where AI agents must edit game code through typed tools, verify edits through runtime keypoints, and apply changes through a single journaled write path. The paper's three-layer framework (harness interface, harness mechanisms, scaling harnesses) maps to the engine's agent architecture: API1 (search → describe → call), VK1 (runtime assertions), EV2 (content verification), and the MCP command service (single write path: journal → apply → re-read → roll back). The engine's simpler baseline (EA1 transcript, preview, fork test, apply and undo; SP gates; VK1, VP1) provides a deterministic alternative that the paper's harness patterns can test against.

**Alternative recent papers considered** (counterevidence / alternatives):
- **arXiv:2603.03329v1** (AutoHarness: Improving LLM Agents by Automatically Synthesizing a Code Harness, Feb 2026) — About LLM agents automatically synthesizing code harnesses to prevent illegal moves in game environments. Directly relevant to the dive's agent-editing-game topic but focuses on game-playing agents rather than code-editing agents. Secondary reference for the agent-harness domain.
- **arXiv:2505.16801v2** (A Modular Framework for Automated Evaluation of Procedural Content Generation in Serious Games with DRL Agents, Jul 2025) — About DRL game testing agents evaluating PCG. Relevant to game generation but focuses on evaluation rather than code editing. Secondary reference for the game-AI domain.
- **arXiv:2508.08322v1** (Context Engineering for Multi-Agent LLM Code Assistants Using Elicit, NotebookLM, ChatGPT, and Claude Code, Aug 2025) — About multi-agent LLM systems for code generation, editing, and validation. Relevant to the engine's multi-agent code editing but focuses on general software engineering rather than game code. Secondary reference for the code-editing domain.
- **arXiv:2605.21397v1** (Validating Navmesh using Geometry, Dive 06) — Already served by arxiv-wave41. Not re-served.

---

## Engine home

- Primary: `crates/endpoint/src/` (insight, play), `e2040-mcp/`, `fixtures/keypoints/`
- Secondary: `docs/plan/ai-native.md` (PLN9, prefix AX, MCP5, EA1a-e, EV1, CR1, QA2, SP2), `docs/plan/editor.md` (PLN10)
- Subsystem plan: `docs/plan/ai-native.md` (PLN9)
- EVIDENCE home: `docs/EVIDENCE.md`
- RESEARCH-VERDICT home: `docs/RESEARCH-VERDICT.md` (deep dive 08 section)
- DEEP-DIVES: `docs/DEEP-DIVES.md` (dive 08 row update)
- SPRINT: `docs/SPRINT.md` (board rows API1, VK1, EV2, API3)
- Focus: `docs/FOCUS.md`

---

## Simpler baseline (engine's current approach)

The engine already has a simpler, non-harness-based agent loop:
- **EA1 transcript** (endpoint): Plain text transcript of agent actions, preview, fork test, apply and undo
- **SP gates** (endpoint): Structured permission gates that agents must pass before executing
- **VK1** (endpoint): Runtime keypoints — assertions on game state that verify edits are valid
- **VP1** (endpoint): Playtest capture — visual verification of agent edits
- **MCP command service** (`e2040-mcp`): Single journaled write path — journal → apply → re-read → roll back on mismatch
- **Hash x2**: Determinism verification — same input produces byte-identical output
- **Recorded fixtures**: All agent actions are recorded so replays never call a model

This baseline is **deterministic, recorded, and replay-exact** — it does not use the paper's harness synthesis, code-as-policy, or multi-agent orchestration patterns. The engine's approach is a simple transcript → verify → apply → undo pipeline; the paper's approach is a sophisticated harness-based system where code serves as the agent's reasoning substrate. The engine's approach is simpler and deterministic; the paper's approach is more sophisticated and targets agent reliability through code harnesses.

---

## Ranked cards (max 5)

### Card 1 (TAKE, REPRODUCE): Code-as-harness agent loop vs transcript baseline

- **Question**: Does Ning et al.'s code-as-agent-harness framework (where code serves as the operational substrate for agent reasoning, action, environment modeling, and execution-based verification) reproduce against the engine's simpler transcript-based agent loop (EA1), improving edit reliability and verification speed?
- **Engine home**: crates/endpoint/src/insight.rs (planned) (API1, insight index), `crates/endpoint/src/play.rs` (VK1, runtime keypoints), `e2040-mcp/src/` (MCP command service)
- **Baseline**: Engine's EA1 transcript → preview → fork test → apply and undo pipeline. Simpler baseline: plain transcript with no harness.
- **URL + ID**: https://arxiv.org/abs/2605.18747v1, arXiv:2605.18747v1, §2 (Harness Interface), §3 (Harness Mechanisms), §4 (Scaling Harnesses)
- **License**: Not verified on arXiv page. Ideas only — no code taken. Paper reimplementation.
- **Claim**: The paper demonstrates that code-as-harness improves agent reliability by connecting agents to reasoning, action, and environment modeling through code, with feedback-driven control and optimization making harnesses reliable and adaptive (§3). The paper shows that multi-agent settings benefit from shared code artifacts supporting coordination, review, and verification (§3.3).
- **Assumptions**: The engine's agent loop can adopt the paper's harness interface pattern. The engine's typed tools can serve as the harness interface. The paper's feedback-driven control can be expressed in the engine's deterministic pipeline.
- **Counterevidence**: The paper is a survey, not an implementation — it describes patterns across many systems but doesn't provide a single reproducible algorithm. The paper's harness mechanisms (planning, memory, tool use) use LLM-based reasoning, which may not be deterministic. The paper's multi-agent coordination patterns may not transfer to the engine's single-command-service architecture. The paper was validated on general software engineering tasks, not game code editing.
- **What does NOT transfer**: The paper's LLM-based planning and memory mechanisms. The paper's general software engineering validation scenarios. The paper's multi-agent coordination patterns (the engine uses a single MCP command service). The transfer is the *harness interface concept* (code as the substrate connecting agents to actions and verification), not the *LLM implementation* or *survey conclusions*.
- **Cost**: M 2–3 sessions. Fixture: 50-query insight gold set, 30 keypoints with 5 seeded bugs, 20-step duel. Metric: edit success rate, stale write prevention, `unsupported_capture` rate, tokens per query, p95 insight latency. Budget: top-3 ≥ 90% on insight gold set.
- **Ablation**: Compare EA1 transcript only (current baseline) → transcript + harness interface (proposed) → transcript + harness interface + feedback-driven control (proposed) → full paper system (rejected — LLM-based, non-deterministic).
- **Success threshold**: Harness interface does not add per-tick allocation, produces deterministic output (hash x2 passes), and improves edit success rate ≥ 10% over the transcript baseline on the 30-keypoint fixture.
- **Kill criterion**: Any non-deterministic harness output. Any per-tick allocation from harness state. Any LLM inference inside the fixed tick. Any `unsupported_capture` on a GPU.
- **Board row proposal**: **API1** — code-as-harness insight index with harness interface. Tier B. Size M. Owns crates/endpoint/src/harness.rs (planned). Accept: 50-query insight gold set top-3 ≥ 90%; hash x2 passes; p95 insight latency ≤ budget; tokens down. needs: EA1, VK1.
- **Stamp**: **REPRODUCE** — the paper's finding that code-as-harness improves agent reliability can be tested against the engine's deterministic transcript baseline with hash determinism and the 30-keypoint fixture.
- **EMPIRE outcome**: Supports "AI makes whole games" — reliable agent edits mean the brief→game pipeline works correctly. Supports the "agent loop that edits the game" vision. Makes the API1 insight index production-ready.

### Card 2 (TAKE, UNVERIFIED): AutoHarness code synthesis for game environments

- **Question**: Does Lou et al.'s AutoHarness approach (LLM agents automatically synthesizing code harnesses using iterative code refinement given feedback from the game environment) translate to the engine's agent loop, automatically generating harness code for game editing tasks?
- **Engine home**: `crates/endpoint/src/` (agent core), `crates/ai/src/` (RL agents)
- **Baseline**: Engine's manually-written SP gates and VK1 assertions. Current: no automatic harness synthesis.
- **URL + ID**: https://arxiv.org/abs/2603.03329v1, arXiv:2603.03329v1, §3 (AutoHarness Method), §4 (Experiments)
- **License**: Not verified on arXiv page. Ideas only. Paper reimplementation.
- **Claim**: The paper demonstrates that Gemini-2.5-Flash can automatically synthesize code harnesses using iterative refinement, preventing all illegal moves in 145 TextArena games and enabling smaller models to outperform larger ones (§4). The resulting code-policy receives higher average reward than larger models on 16 TextArena 1-player games.
- **Assumptions**: The engine's agent loop can adopt automatic harness synthesis. The engine's game environment provides feedback for iterative refinement. The synthesized harness code can be verified deterministically.
- **Counterevidence**: The paper uses LLM-based synthesis (Gemini-2.5-Flash), which is non-deterministic and requires external API calls. The paper's TextArena games are text-based, not the engine's Rust game code. The paper's iterative refinement loop is not replay-exact. The paper's approach requires running LLMs, which violates the engine's "model never runs inside the tick" law.
- **What does NOT transfer**: The paper's LLM-based synthesis loop. The paper's TextArena game environment. The paper's non-deterministic refinement iterations. The transfer is the *harness synthesis concept* (automatically generating verification code from feedback), not the *LLM implementation*.
- **Cost**: M 2 sessions. Fixture: 30 keypoints with 5 seeded bugs. Metric: harness synthesis success rate, deterministic verification, p95 synthesis time. Budget: harness synthesis must complete offline (not in the tick).
- **Ablation**: Compare manual SP gates (current) → automatic harness synthesis (proposed) → full AutoHarness (rejected — LLM, non-deterministic) → manual harness with feedback (proposed).
- **Success threshold**: Automatic harness synthesis completes offline, produces deterministic verification code (hash x2 passes), and prevents ≥ 90% of seeded bugs.
- **Kill criterion**: Any LLM inference in the tick. Any non-deterministic harness code. Any per-tick allocation from synthesis state.
- **Board row proposal**: **API1** — automatic harness synthesis for game editing. Tier B. Size S. Owns crates/endpoint/src/harness_synth.rs (planned). Accept: 30 keypoints with 5 seeded bugs; hash x2 passes; synthesis completes offline; ≥ 90% bug prevention. needs: VK1, SP gates.
- **Stamp**: **UNVERIFIED** — the paper's AutoHarness is validated on TextArena games using LLM synthesis; the engine's deterministic approach may not support automatic synthesis. The harness synthesis concept transfers; the implementation is UNVERIFIED until engine-specific testing.
- **EMPIRE outcome**: Supports "AI makes whole games" — automatic harness synthesis means agents can self-verify their edits without manual gate writing. Supports the agent loop vision.

### Card 3 (TAKE, REPRODUCE): Multi-agent code editing verification vs single-agent baseline

- **Question**: Does Ning et al.'s multi-agent harness scaling pattern (shared code artifacts supporting multi-agent coordination, review, and verification) translate to the engine's MCP command service, improving the reliability of multi-agent game code editing?
- **Engine home**: `e2040-mcp/` (MCP command service), `crates/endpoint/src/` (insight, play)
- **Baseline**: Engine's single MCP command service with EA1 transcript. Current: single-agent editing only.
- **URL + ID**: https://arxiv.org/abs/2605.18747v1, arXiv:2605.18747v1, §3.3 (Scaling Harnesses to Multi-Agent Settings)
- **License**: Not verified on arXiv page. Ideas only.
- **Claim**: The paper discusses how shared code artifacts support multi-agent coordination, review, and verification in larger-scale agent settings (§3.3). The paper outlines open challenges including evaluation beyond final task success, verification under incomplete feedback, and consistent shared state across multiple agents.
- **Assumptions**: The engine's MCP command service can support multi-agent editing. The engine's shared state (game state) can serve as the shared code artifact. Multi-agent coordination can be expressed deterministically.
- **Counterevidence**: The paper is a survey discussing patterns, not an implementation. The paper's multi-agent patterns use LLM-based coordination, which is non-deterministic. The paper's "consistent shared state" challenge is directly relevant to the engine's HASH-LAW but the paper doesn't provide a deterministic solution. The engine's single-command-service architecture may not need multi-agent coordination.
- **What does NOT transfer**: The paper's LLM-based multi-agent coordination. The paper's survey-level patterns without implementation. The transfer is the *shared-state verification concept* (coordinating multiple agents through a shared, verified state), not the *LLM implementation*.
- **Cost**: M 2 sessions. Fixture: 20-step duel with stable digests. Metric: multi-agent edit consistency, hash determinism, p95 duel time. Budget: stable digests across all agents.
- **Ablation**: Compare single-agent editing (current) → multi-agent editing with shared state verification (proposed) → full paper system (rejected — LLM, non-deterministic).
- **Success threshold**: Multi-agent editing produces stable digests (hash x2 passes), prevents conflicting edits, and completes within the 20-step duel budget.
- **Kill criterion**: Any non-deterministic multi-agent output. Any LLM inference in the tick. Any conflicting writes to the game state.
- **Board row proposal**: **API3** — multi-agent code editing with shared state verification. Tier B. Size M. Owns crates/endpoint/src/multi_agent.rs (planned). Accept: 20-step duel with stable digests; hash x2 passes; no conflicting writes. needs: API1, MCP command service.
- **Stamp**: **REPRODUCE** — the paper's multi-agent coordination concept can be tested against the engine's deterministic MCP command service with hash determinism and the 20-step duel fixture.
- **EMPIRE outcome**: Supports "AI makes whole games" — multi-agent editing means multiple AI can collaborate on game code. Supports the brief→game pipeline where multiple agents contribute.

### Card 4 (REJECT): LLM-based harness synthesis in the tick

- **Question**: Can the paper's LLM-based harness synthesis approach (using LLMs to automatically generate verification code) be reproduced as an engine runtime feature?
- **Engine home**: `crates/endpoint/src/` (agent core), `crates/ai/src/` (AI)
- **Baseline**: Engine's deterministic SP gates and VK1 assertions. Current: no LLM in the tick.
- **URL + ID**: https://arxiv.org/abs/2605.18747v1, arXiv:2605.18747v1, §3 (Harness Mechanisms) — specifically the LLM-based planning and feedback-driven control sections.
- **License**: Not verified. Ideas only.
- **Claim**: The paper describes feedback-driven control and optimization that make harnesses reliable and adaptive, including LLM-based planning and memory mechanisms.
- **Assumptions**: The engine could use LLM-based harness synthesis at runtime.
- **Counterevidence**: The paper's LLM-based planning uses external LLM inference, which violates the engine's HASH-LAW (model output never runs inside the tick). The paper's feedback-driven control is non-deterministic across runs. The paper's memory mechanisms require external storage. The engine's AGENTS.md rule 3 requires "live inference submits validated recorded actions, never stalls it" and "Optional live inference submits validated recorded actions, never stalls it." LLM harness synthesis in the tick would stall the tick and violate determinism.
- **What does NOT transfer**: The paper's LLM-based planning and memory. The paper's non-deterministic feedback loops. The paper's external LLM inference. The transfer is the *harness concept* (code as verification substrate), not the *LLM implementation*.
- **Cost**: N/A — rejected before implementation.
- **Kill criterion**: Any LLM inference in the tick. Any non-deterministic planning. Any external API dependency in the fixed tick.
- **Board row proposal**: None. **REJECT** — LLM-based harness synthesis in the tick violates HASH-LAW (model never runs in the tick) and the engine's AGENTS.md rule 3. Cards 1–3 provide the deterministic alternatives.
- **Reason**: LLM-based harness synthesis requires external LLM inference, which violates HASH-LAW and stalls the tick. The deterministic harness interface (Card 1) is the correct mechanism.
- **EMPIRE outcome**: Rejected — the engine stays deterministic and provider-agnostic. Cards 1–3 provide the deterministic alternatives.

### Card 5 (LATER): Full code-as-harness pipeline integration

- **Question**: Can the full code-as-harness pipeline (harness interface + automatic synthesis + multi-agent coordination + feedback-driven control) be integrated into the engine's agent loop as a complete system?
- **Engine home**: `crates/endpoint/src/`, `e2040-mcp/`, `crates/ai/src/`
- **Baseline**: Complete agent loop with all paper concepts integrated.
- **URL + ID**: All paper sections, §2–§4 (Full System)
- **License**: Not verified. Ideas only.
- **Claim**: The paper presents a complete code-as-harness framework spanning harness interface, mechanisms, and scaling.
- **Assumptions**: All paper concepts can be integrated into one agent loop system.
- **Counterevidence**: The full pipeline requires LLM-based planning (Card 4), LLM-based synthesis (Card 2), and multi-agent coordination with LLM-based feedback (Card 3) — all of which violate the engine's determinism, HASH-LAW, and AGENTS.md rule 3. The full pipeline is not replay-exact and cannot be integrated into the engine's fixed tick.
- **What does NOT transfer**: The full pipeline as-is. The paper's LLM-based components. The transfer is the *individual design patterns* (Cards 1, 2, 3), not the *complete pipeline*.
- **Cost**: L 5+ sessions — too large for wave 42. Parked as LATER.
- **Kill criterion**: Any float math in the tick. Any non-deterministic output. Any LLM inference in the tick. Any GPU computation inside the fixed tick.
- **Board row proposal**: None. **LATER** — revisit after API1 (Card 1), API1 (Card 2), and API3 (Card 3) provide the individual building blocks. Full pipeline integration needs a deterministic adaptive harness system that the engine does not yet have.
- **EMPIRE outcome**: Deferred — the complete pipeline is incompatible with determinism constraints. Individual design patterns can be adapted. Revisit after API1/API3 building blocks exist.

---

## Counterevidence summary

1. **Survey vs implementation**: The primary paper (2605.18747) is a survey, not an implementation — it describes patterns across many systems but doesn't provide a single reproducible algorithm. Cards 1–3 must validate that the design patterns can be adapted as deterministic engine features, not taken as ready-to-use code.

2. **LLM vs determinism**: Cards 2 and 4 involve LLM-based synthesis and planning, which violate the engine's HASH-LAW (model output never runs inside the tick) and AGENTS.md rule 3 (live inference never stalls the fixed tick). Cards 1 and 3 must validate that their deterministic alternatives produce byte-identical output for the same input.

3. **Domain mismatch**: The primary paper was validated on general software engineering tasks (code assistants, DevOps, scientific discovery). The engine's agent loop edits game code through typed tools. Cards must validate their concepts at game-code scale, not general software engineering scale.

4. **Single-command-service vs multi-agent**: The paper discusses multi-agent harness scaling, but the engine uses a single MCP command service. Cards 1–3 must validate that the harness patterns work within the engine's single-command-service architecture.

5. **Already-served papers avoided**: The arXiv papers already listed in the dive 08 section (Unity Insight 2609.27585, GameGen-Verifier 2605.07442, GameCraft-Bench 2606.17861, CraftBench-UE 2609.23142, Code World Model 2608.25927, 3D-Layout-R1 2603.22279, Darwin Gödel Machine 2505.22954) are not repeated. This packet targets Dive 08 exclusively with new papers.

---

## What does NOT transfer (explicit)

- All LLM-based planning and memory mechanisms (Card 4 — rejected, violates HASH-LAW)
- All LLM-based harness synthesis (Card 2 — float, non-deterministic, external API)
- All LLM-based multi-agent coordination (Card 3 — non-deterministic)
- The paper's survey-level patterns without implementation
- The paper's general software engineering validation scenarios
- All non-deterministic frame-to-frame variation (violates HASH-LAW)
- All LLM inference in the tick (violates AGENTS.md rule 3)
- All external API dependencies in the fixed tick
- The paper's full pipeline as-is (Cards 1–3 provide the transferable patterns; the complete pipeline does not transfer)

---

## Integration homes

- **Subsystem plan**: `docs/plan/ai-native.md` (PLN9, prefix AX, MCP5, EA1a-e, EV1, CR1, QA2, SP2)
- **RESEARCH-VERDICT**: `docs/RESEARCH-VERDICT.md` (deep dive 08 section)
- **EVIDENCE**: `docs/EVIDENCE.md`
- **DEEP-DIVES**: `docs/DEEP-DIVES.md` (dive 08 row update)
- **SPRINT**: `docs/SPRINT.md` (board rows API1, VK1, EV2, API3)
- **Focus**: `docs/FOCUS.md`
- **Editor plan**: `docs/plan/editor.md` (PLN10) — agent loop connects to editor tools
- **Multiplayer plan**: `docs/plan/multiplayer.md` (PLN6) — agent editing connects to MP arena scenarios

---

## Budget and next step

Budget: M 20 min (this packet). At budget stop, report cards and next step.

Next step: If cards 1, 2, and 3 are adopted, write `docs/research/arxiv-wave42.md` as the persisted note (this file). Propose API1 (Card 1), API1 (Card 2), and API3 (Card 3) as board rows via the orchestrator. Card 4 is REJECT (one reason: LLM-based harness synthesis in the tick violates HASH-LAW). Card 5 is LATER (requires full pipeline integration that the engine's determinism constraints prevent until individual building blocks exist). Card 2's harness synthesis claim is UNVERIFIED for the engine's deterministic context until engine-specific testing exists.

---

## EMPIRE outcome per card

| Card | EMPIRE outcome |
|------|----------------|
| API1 code-as-harness insight index (TAKE, REPRODUCE) | Reliable agent edits mean the brief→game pipeline works correctly. Supports "AI makes whole games" (EMPIRE.md §4) and the agent loop vision. Makes API1 production-ready for game code editing. |
| API1 automatic harness synthesis (TAKE, UNVERIFIED) | Automatic harness synthesis means agents can self-verify their edits without manual gate writing. Supports the agent loop vision. Principle transfers; implementation UNVERIFIED for the engine's deterministic pipeline. |
| API3 multi-agent code editing (TAKE, REPRODUCE) | Multi-agent editing means multiple AI can collaborate on game code. Supports the brief→game pipeline where multiple agents contribute. Supports the "AI makes whole games" vision. |
| LLM-based harness synthesis in tick (REJECT) | Rejected — stays deterministic and std-only. No LLM dependency in the tick. Cards 1–3 provide the deterministic alternatives. |
| Full pipeline integration (LATER) | Deferred — the complete pipeline is incompatible with determinism constraints. Individual design patterns can be adapted. Revisit after API1/API3 building blocks exist. |

---

## Source traceability

- **ArXiv API**: `https://export.arxiv.org/api/query?search_query=code+agent+harness+game+LLM&max_results=10` → arXiv:2605.18747v1; `https://export.arxiv.org/api/query?search_query=AutoHarness+LLM+agent+harness&max_results=5` → arXiv:2603.03329v1; `https://export.arxiv.org/api/query?search_query=multi+agent+LLM+code+assistants&max_results=5` → arXiv:2508.08322v1; `https://export.arxiv.org/api/query?search_query=modular+framework+procedural+content+generation+game&max_results=5` → arXiv:2505.16801v2
- **Paper accessed**: https://arxiv.org/abs/2605.18747v1, https://arxiv.org/abs/2603.03329v1, https://arxiv.org/abs/2508.08322v1, https://arxiv.org/abs/2505.16801v2 (accessed 2026-09-26)
- **License**: arXiv:2605.18747v1 — not verified on arXiv page; arXiv:2603.03329v1 — not verified on arXiv page; arXiv:2508.08322v1 — not verified on arXiv page; arXiv:2505.16801v2 — not verified on arXiv page. Ideas only, no code taken.
- **Cross-reference**: DEEP-DIVES.md §08 (dive 08 row), `docs/plan/ai-native.md` (PLN9), `docs/plan/editor.md` (PLN10)
- **Avoids double-serving**: Dives 01–07, 09–12, 15–20 already served by STEAL or ARXIV (waves 27–41). Dive 08 is the oldest non-STEAL, non-ARXIV dive with arXiv-relevant research content.
- **EMPIRE.md**: §1 (we ship our own games), §4 (AI makes whole games), Race section (forge + engine2040 both live forever)
- **Donor dedup**: All prior ARXIV and STEAL donors excluded from new-take consideration.
