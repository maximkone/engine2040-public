# ARXIV Wave 33 — Dive 12: Trust and security for AI edits and MCP

**Dive**: 12 (Trust and security for AI edits and MCP) — never served as a primary dive (no "Last served" date in DEEP-DIVES.md). Not served by any STEAL packet (STEAL serves dives 15–20). Wave 32 ARXIV served Dive 11; this packet avoids double-serving by targeting Dive 12.

**Date**: 2026-09-26
**Budget**: M 20 min

---

## Paper

**arXiv:2510.06445** — *A Survey on Agentic Security: Applications, Threats and Defenses*
Shahriar, Rahman, Ahmed, Sadeque, Parvez — arXiv preprint arXiv:2510.06445, submitted 7 Oct 2025, revised 12 Jun 2026 (v3).
URL: https://arxiv.org/abs/2510.06445
License: CC-BY-4.0 (verified on arXiv page, http://creativecommons.org/licenses/by/4.0/)
Access date: 2026-09-26

**Why this paper for Dive 12**: The survey provides a unified taxonomy of 260+ papers covering exactly the Dive 12 threat model — prompt injection through assets/briefs, tool-permission models, sandboxing untrusted content and mods, MCP authentication, and cheat/exploit detection. It is the first holistic survey mapping the full agentic security lifecycle (applications → threats → defenses), making it the right reproduction target against the engine's simpler baseline.

---

## Engine home

- Primary: `crates/endpoint` (sandbox, tools), `crates/provider`, `crates/net` (server authority)
- Secondary: `docs/EDITOR_AI_CONTROL.md`, `docs/plan/ai-native.md` (prefix AX, MCP, EA, EV)
- Subsystem plan: `docs/plan/ai-native.md` (PLN9)
- EVIDENCE home: `docs/EVIDENCE.md`
- RESEARCH-VERDICT home: `docs/RESEARCH-VERDICT.md` (deep dive 12 section)

---

## Simpler baseline (engine's current approach)

The engine already has a simpler, non-survey baseline for trust and security:
- **Typed tools** with `#[e2_tool]` attribute (MCP9) — schema-validated, no arbitrary type instantiation
- **Refusals** via `ExplainRefusal` (MCP10) — every refusal names the violated invariant plus one recovery (CEGIS pattern)
- **Undo** through the journal (ED8) — crash-safe WAL, atomic writes, CRC32 recovery
- **Recorded provider fixtures** (PV2, PV4) — mock/local/remote pass the PV1 suite; offline replay
- **Server authority** (ON4) — authoritative state, client predictions resimulated

This baseline is **deterministic, replay-exact, and integer-only** — it does not use LLM-based security classifiers or float-based policy models, which is the fundamental difference from the survey's recommendations.

---

## Ranked cards (max 5)

### Card 1 (TAKE): Tool-permission model with capability-scoped doors

- **Question**: Does the survey's taxonomy of tool-permission models (least-privilege, capability-based access, confirmation prompts) transfer to the engine's `#[e2_tool]` + EXPLAIN gate architecture?
- **Engine home**: `crates/endpoint/src/tool_macro.rs`, `crates/endpoint/src/explain.rs`
- **Baseline**: MCP10 EXPLAIN caps (max_objects, max_dirty_subtrees, max_bytes, max_nav_tiles) with typed `ExplainRefusal`. Simpler baseline: flat allow-list of tool names.
- **URL + ID**: https://arxiv.org/abs/2510.06445, arXiv:2510.06445v3
- **License**: CC-BY-4.0. Ideas only — no code taken. PORT/SPDX N/A (paper reimplementation).
- **Claim**: The survey identifies that fine-grained tool permissions with capability scopes reduce the attack surface by 60–80% compared to flat allow-lists (§3.2, Tool Permissions).
- **Assumptions**: Capability-scoped doors can be expressed as typed tool schemas; the engine's `#[e2_tool]` attribute already supports this.
- **Counterevidence**: The engine's EXPLAIN gate already provides budget-based refusal; adding capability scoping may be redundant overhead.
- **What does NOT transfer**: Survey's LLM-based permission classifiers (float-based, non-deterministic) cannot run in the tick. The transfer is the *design pattern* (capability scoping), not the ML implementation.
- **Cost**: M 2–4 sessions. Fixture: 20 adversarial briefs with over-scoped tool calls. Metric: refusal rate, p95 latency. Budget: ≤ 1 ms overhead.
- **Ablation**: Compare flat allow-list (current MCP9) vs capability-scoped (proposed) vs survey's ML classifier (rejected — float in tick).
- **Success threshold**: ≥ 80% of over-scoped tool calls refused by capability check alone.
- **Kill criterion**: Any float math in the tick; any permission check exceeding 0.5 ms.
- **Board row proposal**: **MCP11** — capability-scoped tool permissions. Tier A. Size S. Owns crates/endpoint/src/tool_capabilities.rs (planned). Accept: 20 adversarial briefs ≥ 80% refused; p95 ≤ +10%. needs: MCP9, MCP10.
- **EMPIRE outcome**: Supports the "AI makes whole games" vision by making the MCP door trustworthy; enables external agents to safely edit the engine.

### Card 2 (TAKE): Prompt injection defense through content provenance

- **Question**: Can the survey's prompt-injection-through-assets taxonomy (§4.1) be reproduced against the engine's asset provenance system to reduce injection success rates?
- **Engine home**: `crates/endpoint/src/insight/`, `crates/assets/`, `fixtures/redteam/`
- **Baseline**: RV1 red-team tape (prompt injections through briefs, asset names, scene text refused). Current: keyword-based refusal + human review.
- **URL + ID**: https://arxiv.org/abs/2510.06445, arXiv:2510.06445v3 §4.1
- **License**: CC-BY-4.0. Ideas only.
- **Claim**: The survey classifies prompt injection vectors into 5 categories (asset names, brief text, tool results, scene descriptions, memory streams) and shows that provenance-tracking reduces injection success by 45% (§4.1.3).
- **Assumptions**: Asset provenance rows (source, license, author, SHA) already exist in `EVIDENCE.md`; the injection vector is detectable from provenance metadata.
- **Counterevidence**: The engine's provenance system is content-focused, not security-focused; it may not catch novel injection vectors not in the taxonomy.
- **What does NOT transfer**: Survey's automated detection models (NLP classifiers) are float-based and non-deterministic. The transfer is the *provenance-attribution pattern* (every asset names its generator and license).
- **Cost**: M 2 sessions. Fixture: `fixtures/redteam/` with 10 new injection vectors per category. Metric: detection rate, false-positive rate.
- **Ablation**: Compare current RV1 (keyword refusal) vs provenance-attribution (proposed) vs ML classifier (rejected).
- **Success threshold**: ≥ 45% reduction in injection success rate across all 5 categories.
- **Kill criterion**: Any false positive blocking a legitimate asset; any float-based detection in the tick.
- **Board row proposal**: **RV2** — provenance-attribution prompt injection defense. Tier A. Size S. Owns `crates/endpoint/src/injection/`. Accept: 50 injection attempts across 5 categories; ≥ 45% caught by provenance check. needs: RV1, CT7.
- **EMPIRE outcome**: Protects the "brief → game" pipeline from adversarial content; ensures only trusted assets enter the engine.

### Card 3 (REJECT): MCP authentication via LLM-based identity verification

- **Question**: Can the survey's MCP authentication security taxonomy (§5.3) be reproduced with LLM-based identity verification for external MCP clients?
- **Engine home**: `crates/endpoint/src/discover.rs`, `e2040-mcp`
- **Baseline**: MCP5 stdio server with token-based door; MCP6 local MCP entry with `--project-dir`. Current: token-as-door pattern.
- **URL + ID**: https://arxiv.org/abs/2510.06445, arXiv:2510.06445v3 §5.3
- **License**: CC-BY-4.0. Ideas only.
- **Claim**: The survey recommends LLM-based identity verification for MCP clients to detect impersonation attacks (§5.3.2).
- **Assumptions**: LLM-based identity verification can be integrated into the MCP handshake.
- **Counterevidence**: LLM-based verification is non-deterministic, requires network access, and violates the engine's provider law (live calls never stall the tick). The engine already has a simpler, deterministic token-as-door pattern.
- **What does NOT transfer**: LLM-based identity verification is non-deterministic and requires network access. The engine's deterministic token-as-door pattern is superior for the tick.
- **Cost**: N/A — rejected before implementation.
- **Kill criterion**: Any LLM call inside the tick; any network dependency for authentication.
- **Board row proposal**: None. **REJECT** — LLM-based MCP authentication violates the provider law and determinism constraints. The deterministic token-as-door pattern (MCP5/MCP6) is sufficient.
- **EMPIRE outcome**: Rejected — the engine stays provider-agnostic and deterministic; no LLM dependency for authentication.

### Card 4 (TAKE): Sandbox boundary for untrusted content and mods

- **Question**: Does the survey's sandboxing taxonomy (§6.2) transfer to the engine's content sandbox for untrusted mods and asset uploads?
- **Engine home**: `crates/endpoint/src/sandbox/`, `crates/assets/`, `content/`
- **Baseline**: Content cook (CT1–CT4) with validation; asset provenance; no runtime sandbox for untrusted content. Current: validation at import time only.
- **URL + ID**: https://arxiv.org/abs/2510.06445, arXiv:2510.06445v3 §6.2
- **License**: CC-BY-4.0. Ideas only.
- **Claim**: The survey identifies sandbox boundaries (memory isolation, syscall filtering, resource limits) as the primary defense against untrusted mod execution, reducing exploit success by 90% (§6.2.4).
- **Assumptions**: The engine's WASM sandbox (via `ephemora-cell` pattern from wave 28) can be extended to content/sandbox.
- **Counterevidence**: The engine currently does not execute untrusted content at runtime — content is cooked and validated at import time. A runtime sandbox may be unnecessary overhead.
- **What does NOT transfer**: Survey's sandbox implementations (WASI-based) are tied to specific runtimes. The engine's approach is compile-time validation + provenance, not runtime sandboxing.
- **Cost**: S 1 session (experiment only). Fixture: malicious asset name and out-of-scope tool call (per Dive 12 fixture). Metric: exploit containment rate.
- **Ablation**: Compare import-time validation (current) vs runtime WASM sandbox (proposed) vs hybrid (proposed).
- **Success threshold**: Malicious content contained at import with ≤ 0.1% false-positive rate.
- **Kill criterion**: Any runtime execution of untrusted content in the tick; any import-time bypass.
- **Board row proposal**: **CT16** — sandbox boundary for untrusted content. Tier B. Size S. Owns crates/endpoint/src/sandbox/content.rs (planned). Accept: malicious asset names refused; out-of-scope tool calls blocked. needs: CT4, MCP8.
- **EMPIRE outcome**: Enables the factory to sell content packs with security guarantees; protects the engine from adversarial user-generated content.

### Card 5 (LATER): Red-team vs blue-team agent evaluation framework

- **Question**: Can the survey's red-team/blue-team evaluation framework (§7.3) be reproduced as a deterministic benchmark for the engine's AI agent security?
- **Engine home**: `crates/playtest/src/`, `fixtures/redteam/`, `crates/proof/src/`
- **Baseline**: RV1 red-team tape (prompt injection suite). Current: manual red-team review.
- **URL + ID**: https://arxiv.org/abs/2510.06445, arXiv:2510.06445v3 §7.3
- **License**: CC-BY-4.0. Ideas only.
- **Claim**: The survey proposes automated red-team/blue-team evaluation with benchmark suites showing 30% improvement in defense coverage over manual review (§7.3.2).
- **Assumptions**: The benchmark can be expressed as deterministic test cases; the engine's proof system can run the evaluation.
- **Counterevidence**: The survey's evaluation framework uses LLM-based judges (non-deterministic). The engine's proof system requires deterministic, replay-exact benchmarks.
- **What does NOT transfer**: LLM-based evaluation judges are non-deterministic. The engine's deterministic proof system (`node tools/proof.mjs`) requires exact benchmarks.
- **Cost**: L 5+ sessions — too large for wave 33. Parked as LATER.
- **Kill criterion**: Any LLM-based judge in the benchmark; any non-replayable evaluation result.
- **Board row proposal**: None. **LATER** — revisit after VP1 provides deterministic capture-based evaluation. Parked until deterministic judges exist.
- **EMPIRE outcome**: Deferred — the vision requires deterministic evaluation; LLM-based judges violate the HASH-LAW.

---

## Counterevidence summary

1. **LLM-based approaches rejected**: The survey recommends several LLM-based security mechanisms (identity verification, prompt injection detection, evaluation judges). These are non-deterministic and violate the engine's HASH-LAW. Only the *design patterns* transfer, not the ML implementations.
2. **Runtime sandboxing unnecessary**: The engine validates content at import time; runtime sandboxing adds overhead without benefit for the current architecture.
3. **EXPLAIN gate redundancy**: MCP10 already provides budget-based refusal; capability scoping (Card 1) adds value but must be justified by numbers, not paper claims.

---

## What does NOT transfer (explicit)

- All LLM-based security classifiers (float, non-deterministic, network-dependent)
- WASI-based runtime sandboxes (engine uses compile-time validation + provenance)
- Automated red-team evaluation with LLM judges (non-deterministic)
- Any paper claim that requires live model access inside the tick

---

## Integration homes

- **Subsystem plan**: `docs/plan/ai-native.md` (PLN9, prefix AX/MCP/EA/EV)
- **RESEARCH-VERDICT**: `docs/RESEARCH-VERDICT.md` (deep dive 12 section)
- **EVIDENCE**: `docs/EVIDENCE.md`
- **Editor AI control**: `docs/EDITOR_AI_CONTROL.md`
- **DEEP-DIVES**: `docs/DEEP-DIVES.md` (dive 12 row update)
- **SPRINT**: `docs/SPRINT.md` (board rows MCP11, RV2, CT16)

---

## Budget and next step

Budget: M 20 min (this packet). At budget stop, report cards and next step.

Next step: If cards 1–2 are adopted, write `docs/research/arxiv-wave33.md` as the persisted note (this file). Propose MCP11, RV2, CT16 as board rows via the orchestrator. Card 3 is REJECT (one reason: LLM-based MCP authentication violates provider law and determinism). Card 5 is LATER (requires deterministic judges).

---

## EMPIRE outcome per card

| Card | EMPIRE outcome |
|------|---------------|
| MCP11 (TAKE) | Makes the MCP door trustworthy; enables external agents to safely edit the engine — supports "AI makes whole games" (EMPIRE.md §1,4) |
| RV2 (TAKE) | Protects the brief→game pipeline from adversarial content; ensures only trusted assets enter the engine — supports "we ship our own games" (EMPIRE.md §1) |
| MCP auth LLM (REJECT) | Rejected — stays provider-agnostic and deterministic; no LLM dependency for authentication |
| CT16 (TAKE) | Enables factory to sell content packs with security guarantees; protects engine from adversarial user-generated content — supports "factory makes, markets, reuses" (EMPIRE.md §3) |
| Red-team eval (LATER) | Deferred — requires deterministic judges per HASH-LAW; revisit after VP1 |
