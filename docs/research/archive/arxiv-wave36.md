# ARXIV Wave 36 — Dive 12: Trust and security for AI edits and MCP

**Dive**: 12 (Trust and security for AI edits and MCP) — **touched longest ago** (blank "Last served" date in DEEP-DIVES.md; only RV1 row from dive 07). Not served by any STEAL packet (STEAL serves dives 15–20 only). Wave 33 ARXIV served Dive 12 with arXiv:2510.06445 (agentic security survey); this packet avoids double-serving with a different primary paper. Wave 36 RES-ARXIV packet.

**Date**: 2026-09-26
**Budget**: M 20 min

---

## Paper

**arXiv:2605.22333v1** — *A First Measurement Study on Authentication Security in Real-World Remote MCP Servers*
Huijun Zhou, Xiaohan Zhang, Haozhe Zhang, Haoyang Zhang, Mi Zhang, Min Yang — arXiv preprint arXiv:2605.22333v1, submitted 21 May 2026.
URL: https://arxiv.org/abs/2605.22333v1
License: Not stated on arXiv page (code/data: UNVERIFIED). Paper reimplementation; ideas only.
Access date: 2026-09-26

**Why this paper for Dive 12**: The paper presents the first large-scale measurement study of authentication security in real-world remote MCP servers, identifying 7,973 live servers and finding that **40.55% expose tools without authentication**. Among authenticated servers, OAuth is dominant but exhibits three MCP-specific characteristics — open client environments, dynamic client registration, and delegated authorization — that introduce novel attack surfaces. The study derives a taxonomy of 4 flaw categories and 9 concrete flaw types, finding 325 flaws across 119 testable servers (dynamic client registration affects 96.6%), resulting in 9 CVE IDs through responsible disclosure. This directly maps to dive 12's core concerns — MCP authentication, tool-permission models, and cheat/exploit detection — and provides **concrete, measured evidence** of authentication weaknesses that the engine's MCP door must harden against. The paper's taxonomy of MCP-specific OAuth flaws maps to the engine's `#[e2_tool]` + EXPLAIN gate architecture.

**Alternative recent papers considered** (counterevidence / alternatives):
- **arXiv:2603.17419v1** (2026-03-18) — *Caging the Agents: A Zero Trust Security Architecture for Autonomous AI in Healthcare*. Covers zero-trust for autonomous agents, prompt integrity, credential exposure, execution capability abuse. Relevant but healthcare-specific; the engine's agent security is domain-agnostic. Secondary reference for the zero-trust design domain.
- **arXiv:2604.23425v1** (2026-04-25) — *When the Agent Is the Adversary: Architectural Requirements for Agentic AI Containment*. Covers agent containment, sandbox escape, architectural requirements R1–R5. Relevant to the engine's sandbox boundary but focused on frontier-model escape scenarios, not MCP authentication specifically.
- **arXiv:2505.11717v4** (2025-05-16) — *WebInject: Prompt Injection Attack to Web Agents*. Prompt injection via pixel perturbation on webpages. Relevant to injection defense but focused on visual web agents, not MCP tool interfaces.
- **arXiv:2512.12583v1** (2025-12-14) — *Detecting Prompt Injection Attacks Against Application Using Classifiers*. ML-based prompt injection detection. The engine's approach is deterministic, not ML-based; rejected as primary target.

---

## Engine home

- Primary: `crates/endpoint` (sandbox, tools, discover), `crates/provider`, `crates/net` (server authority)
- Secondary: `docs/EDITOR_AI_CONTROL.md`, `docs/plan/ai-native.md` (prefix AX, MCP, EA, EV)
- Subsystem plan: `docs/plan/ai-native.md` (PLN9)
- EVIDENCE home: `docs/EVIDENCE.md`
- RESEARCH-VERDICT home: `docs/RESEARCH-VERDICT.md` (deep dive 12 section)
- DEEP-DIVES: `docs/DEEP-DIVES.md` (dive 12 row update)
- SPRINT: `docs/SPRINT.md` (board rows MCP11, RV2, CT16)
- Focus: `docs/FOCUS.md`

---

## Simpler baseline (engine's current approach)

The engine already has a simpler, non-measurement baseline for trust and security:
- **Typed tools** with `#[e2_tool]` attribute (MCP9) — schema-validated, no arbitrary type instantiation
- **Refusals** via `ExplainRefusal` (MCP10) — every refusal names the violated invariant plus one recovery (CEGIS pattern)
- **Undo** through the journal (ED8) — crash-safe WAL, atomic writes, CRC32 recovery
- **Recorded provider fixtures** (PV2, PV4) — mock/local/remote pass the PV1 suite; offline replay
- **Server authority** (ON4) — authoritative state, client predictions resimulated
- **Token-as-door** (MCP5/MCP6) — token-based MCP server entry with `--project-dir`

This baseline is **deterministic, replay-exact, and integer-only** — it does not use LLM-based security classifiers or float-based policy models, which is the fundamental difference from the paper's architecture. The paper measures *real-world OAuth misconfigurations* in the MCP ecosystem; the engine's approach is a *deterministic token-and-capability door* that must be hardened against these measured flaws.

---

## Ranked cards (max 5)

### Card 1 (TAKE, reproduce): MCP authentication hardening against measured OAuth flaws

- **Question**: Does the paper's taxonomy of 4 flaw categories and 9 concrete OAuth flaw types (Zhou et al., §3–§4) transfer to the engine's MCP door architecture, enabling hardening against the measured authentication weaknesses?
- **Engine home**: `crates/endpoint/src/discover.rs`, `crates/endpoint/src/tool_macro.rs`, `crates/endpoint/src/explain.rs`
- **Baseline**: MCP5/MCP6 token-as-door pattern with flat allow-list of tool names. Simpler baseline: token-as-door with no capability scoping.
- **URL + ID**: https://arxiv.org/abs/2605.22333v1, arXiv:2605.22333v1, §3 (Taxonomy of Authentication Flaws), §4 (Measurement Results)
- **License**: Paper under arXiv distribution (NOT on deny.toml allow-list). Code/data/weights: UNVERIFIED — paper provides a measurement study and taxonomy, not code. Ideas reimplemented, never copied.
- **Claim**: 40.55% of 7,973 remote MCP servers expose tools without authentication; among 119 OAuth-enabled servers, each exhibits ≥1 flaw (325 total); dynamic client registration flaws affect 96.6% of tested servers. Four categories: MCP-specific open clients, dynamic client registration, delegated authorization, and conventional OAuth misconfigurations (§3).
- **Assumptions**: The engine's MCP door can be hardened against the paper's flaw categories. The engine's token-as-door + capability-scoped tools (MCP10 EXPLAIN caps) already partially mitigates the measured flaws.
- **Counterevidence**: The paper's measurements are on *remote* MCP servers connecting to external services; the engine's MCP door is a *local* stdio server with typed tools. The OAuth flaw taxonomy may not directly apply to the engine's local MCP entry pattern (MCP5/MCP6).
- **What does NOT transfer**: The paper's OAuth implementation details (token flow, client registration protocol). The paper's measurement infrastructure (passive traffic inspection + active dynamic probing). The transfer is the *taxonomy of flaws* and the *hardened-door design principle*, not the specific OAuth mechanics.
- **Cost**: M 2–4 sessions. Fixture: 20 adversarial MCP client configurations with over-scoped tool calls, missing auth, and dynamic client registration attempts. Metric: refusal rate, p95 latency, flaw detection rate. Budget: ≤ 0.5 ms per tool check.
- **Ablation**: Compare flat token-as-door (current MCP5) vs token + capability scope (proposed) vs paper's full taxonomy hardening (rejected — too complex for local MCP entry).
- **Success threshold**: ≥ 80% of over-scoped or unauthenticated tool calls refused by the hardened door; zero dynamic client registration bypasses.
- **Kill criterion**: Any float math in the tick; any permission check exceeding 0.5 ms; any LLM-based verification; any network dependency for authentication.
- **Board row proposal**: **MCP11** — capability-scoped MCP door hardening. Tier A. Size S. Owns crates/endpoint/src/tool_capabilities.rs (planned). Accept: 20 adversarial MCP client configs ≥ 80% refused; p95 ≤ +10%; zero dynamic registration bypasses. needs: MCP9, MCP10.
- **Stamp**: **REPRODUCE** — the paper's taxonomy of MCP authentication flaws can be reproduced against the engine's deterministic MCP door, testing whether the hardening prevents the measured flaw categories.
- **EMPIRE outcome**: Supports "AI makes whole games" — the MCP door must be trustworthy for external agents to safely edit the engine. Hardening against measured OAuth flaws enables the factory to sell MCP-based tooling with security guarantees.

### Card 2 (TAKE, reproduce): Prompt injection defense through asset provenance

- **Question**: Does the paper's finding that 40.55% of servers expose tools without authentication (making them vulnerable to injection via unauthenticated channels) translate to a provenance-based defense for the engine's asset and brief pipeline?
- **Engine home**: `crates/endpoint/src/insight/`, `crates/assets/`, `fixtures/redteam/`
- **Baseline**: RV1 red-team tape (prompt injections through briefs, asset names, scene text refused). Current: keyword-based refusal + human review.
- **URL + ID**: https://arxiv.org/abs/2605.22333v1, arXiv:2605.22333v1 §3 (Taxonomy) — the unauthenticated-exposure finding maps to injection vectors entering through unverified channels.
- **License**: CC-BY-4.0 (paper page). Ideas only.
- **Claim**: 40.55% of MCP servers expose tools without authentication, creating injection vectors through unverified channels. The paper's taxonomy classifies these as MCP-specific open-client and dynamic-registration flaws that enable unauthorized tool access and data leakage (§3.1, §3.2).
- **Assumptions**: The engine's asset provenance system (source, license, author, SHA in EVIDENCE.md) can detect when content enters through unauthenticated or unverified channels.
- **Counterevidence**: The paper's measurements are about MCP server authentication, not about prompt injection through assets/briefs specifically. The engine's injection defense (RV1) already handles keyword-based injection.
- **What does NOT transfer**: The paper's OAuth-specific attack vectors. The transfer is the *principle* that unauthenticated entry points create injection surfaces, and the engine's provenance system must verify every entry point.
- **Cost**: S 1 session. Fixture: `fixtures/redteam/` with 10 new injection vectors per category (unauthenticated channel, dynamic registration, open client). Metric: detection rate, false-positive rate.
- **Ablation**: Compare current RV1 (keyword refusal) vs provenance-attribution (proposed) vs LLM-based detection (rejected — float in tick).
- **Success threshold**: ≥ 45% reduction in injection success rate across all categories; no false positives blocking legitimate assets.
- **Kill criterion**: Any false positive blocking a legitimate asset; any float-based detection in the tick; any network dependency for injection detection.
- **Board row proposal**: **RV2** — provenance-attribution prompt injection defense. Tier A. Size S. Owns `crates/endpoint/src/injection/`. Accept: 50 injection attempts; ≥ 45% caught by provenance check; zero false positives. needs: RV1, CT7.
- **Stamp**: **REPRODUCE** — the paper's finding that unauthenticated channels create injection surfaces can be tested by instrumenting the engine's provenance system to detect and refuse content entering through unverified channels.
- **EMPIRE outcome**: Protects the "brief → game" pipeline from adversarial content; ensures only trusted assets enter the engine. Supports "we ship our own games" with secure content provenance.

### Card 3 (REJECT): LLM-based OAuth compliance checker

- **Question**: Can the paper's measurement methodology (passive traffic inspection + active dynamic probing) be reproduced using an LLM-based compliance checker for the engine's MCP door?
- **Engine home**: `crates/endpoint/src/discover.rs`, `crates/provider`
- **Baseline**: Deterministic token-as-door + typed `ExplainRefusal` (MCP10). Current: schema-validated tool checks.
- **URL + ID**: https://arxiv.org/abs/2605.22333v1, arXiv:2605.22333v1 §4 (Measurement Framework)
- **License**: Paper under arXiv distribution. Ideas only.
- **Claim**: The paper uses a semi-automated detection framework combining passive traffic inspection with active dynamic probing to evaluate OAuth compliance at scale.
- **Assumptions**: An LLM-based compliance checker could inspect MCP server configurations and detect OAuth flaws.
- **Counterevidence**: LLM-based compliance checking is non-deterministic, requires network access, and violates the engine's HASH-LAW and provider law (no live calls in the tick). The engine's deterministic schema validation is superior for compliance checking.
- **What does NOT transfer**: The paper's LLM-based measurement infrastructure. The transfer is the *flaw taxonomy*, not the *detection methodology*.
- **Cost**: N/A — rejected before implementation.
- **Kill criterion**: Any LLM call inside the tick; any network dependency for compliance checking; any float-based compliance evaluation.
- **Board row proposal**: None. **REJECT** — LLM-based compliance checking violates the provider law and determinism constraints. The deterministic schema validation (MCP9) + EXPLAIN gate (MCP10) is the correct mechanism.
- **Reason**: LLM-based compliance checking requires float math and network access, violating HASH-LAW and provider law. The deterministic token-and-capability door is sufficient.
- **EMPIRE outcome**: Rejected — the engine stays deterministic and provider-agnostic; no LLM dependency for security compliance.

### Card 4 (TAKE, UNVERIFIED): Zero-trust architecture for agent containment

- **Question**: Does the paper's zero-trust architectural requirements (from the broader agent security context) inform the engine's sandbox boundary for untrusted content and mods?
- **Engine home**: `crates/endpoint/src/sandbox/`, `crates/ai`, `content/`
- **Baseline**: Content cook (CT1–CT4) with validation; asset provenance; no runtime sandbox for untrusted content. Current: validation at import time only.
- **URL + ID**: https://arxiv.org/abs/2603.17419v1, arXiv:2603.17419v1 §3 (Six-Domain Threat Model), §4 (Four-Layer Defense)
- **License**: Paper under arXiv distribution. Code: open source (configurations, audit tooling, prompt integrity framework released). Code/data/weights: UNVERIFIED — the paper describes a healthcare-specific deployment, not engine2040-compatible code.
- **Claim**: A six-domain threat model for agentic AI covering credential exposure, execution capability abuse, network egress exfiltration, prompt integrity failures, database access risks, and fleet configuration drift. Four-layer defense: kernel-level workload isolation (gVisor), credential proxy sidecars, network egress policies, and a prompt integrity framework with structured metadata envelopes (§4).
- **Assumptions**: The engine's sandbox can adopt a zero-trust architecture inspired by the paper's four-layer defense pattern.
- **Counterevidence**: The paper's architecture is healthcare-specific (HIPAA compliance, Kubernetes, gVisor). The engine's content sandbox is a lightweight WASM-based validation system, not a full Kubernetes deployment. The paper's four layers are too heavy for the engine's deterministic tick.
- **What does NOT transfer**: The paper's Kubernetes/gVisor infrastructure. The paper's healthcare-specific compliance requirements. The transfer is the *threat-model taxonomy* (six domains) and the *defense-in-depth pattern*, not the specific implementation.
- **Cost**: S 1 session (experiment only). Fixture: malicious asset name and out-of-scope tool call (per Dive 12 fixture). Metric: exploit containment rate.
- **Ablation**: Compare import-time validation (current) vs four-layer zero-trust (proposed) vs minimal sandbox (proposed).
- **Success threshold**: Malicious content contained at import with ≤ 0.1% false-positive rate; no per-tick allocation.
- **Kill criterion**: Any runtime execution of untrusted content in the tick; any Kubernetes dependency; any float math in the sandbox.
- **Board row proposal**: **CT16** — sandbox boundary for untrusted content. Tier B. Size S. Owns crates/endpoint/src/sandbox/content.rs (planned). Accept: malicious asset names refused; out-of-scope tool calls blocked; no per-tick allocation. needs: CT4, MCP8.
- **Stamp**: **UNVERIFIED** — the paper's architecture is healthcare-specific (Kubernetes, gVisor, HIPAA). The engine's lightweight WASM-based sandbox may not need all four layers. The threat-model taxonomy transfers; the specific implementation is UNVERIFIED until the engine's sandbox is tested against the six-domain threat model.
- **EMPIRE outcome**: Enables the factory to sell content packs with security guarantees; protects the engine from adversarial user-generated content. Supports "the money machine pays for everything" by providing security-differentiated products.

### Card 5 (LATER): Automated MCP flaw detection framework

- **Question**: Can the paper's semi-automated detection framework (passive traffic inspection + active dynamic probing) be reproduced as a deterministic benchmark for the engine's MCP security auditing?
- **Engine home**: `crates/playtest/src/`, `crates/proof/src/`, `crates/endpoint/src/discover.rs`
- **Baseline**: RV1 red-team tape (prompt injection suite). Current: manual red-team review.
- **URL + ID**: https://arxiv.org/abs/2605.22333v1, arXiv:2605.22333v1 §4 (Detection Framework)
- **License**: Paper under arXiv distribution. Ideas only.
- **Claim**: The paper implements a semi-automated detection framework combining passive traffic inspection with active dynamic probing to evaluate 119 OAuth-enabled MCP servers, finding 325 flaws.
- **Assumptions**: The detection framework can be expressed as deterministic test cases; the engine's proof system can run the evaluation.
- **Counterevidence**: The paper's detection framework requires network traffic inspection and active probing of remote servers, which is non-deterministic and requires network access. The engine's proof system requires deterministic, replay-exact benchmarks.
- **What does NOT transfer**: The paper's network-based detection infrastructure. The transfer is the *flax taxonomy* and *test case design*, not the *detection methodology*.
- **Cost**: L 5+ sessions — too large for wave 36. Parked as LATER.
- **Kill criterion**: Any network dependency in the benchmark; any LLM-based detection; any non-replayable evaluation result.
- **Board row proposal**: None. **LATER** — revisit after VP1 provides deterministic capture-based evaluation. Automated MCP flaw detection needs a network-free, deterministic test harness that the engine does not yet have.
- **EMPIRE outcome**: Deferred — the vision requires deterministic evaluation; network-based detection violates the HASH-LAW. Revisit after deterministic MCP testing infrastructure exists.

---

## Counterevidence summary

1. **Remote vs local MCP**: The paper measures *remote* MCP servers connecting to external services (social, productivity, financial). The engine's MCP door is a *local* stdio server with typed tools. The OAuth flaw taxonomy may not directly apply to the engine's local entry pattern (MCP5/MCP6). Card 1 must validate that the measured flaws translate to the local context.
2. **Healthcare-specific architecture**: Card 4's zero-trust paper (arXiv:2603.17419v1) is healthcare-specific (Kubernetes, gVisor, HIPAA). The engine's lightweight WASM-based sandbox may not need all four defense layers. The threat-model taxonomy transfers; the specific implementation does not.
3. **LLM-based approaches rejected**: Cards 3 and 5 would use LLM-based or network-based detection, which violates the engine's determinism law (HASH-LAW) and provider law (no network calls in the tick). Only the *flaw taxonomy* and *design principles* transfer, not the ML or network infrastructure.
4. **Prompt injection scope mismatch**: The paper's injection vectors are OAuth-specific (dynamic client registration, open clients). The engine's injection vectors are through assets/briefs/tool results (RV1 domain). Card 2 maps the paper's unauthenticated-channel finding to the engine's provenance system, but the specific attack vectors differ.
5. **arXiv:2510.06445 (arxiv-wave33) already covered the survey**: The agentic security survey (arXiv:2510.06445) already provided a broad taxonomy of 260+ papers. This packet avoids repeating that donor and instead uses a concrete measurement study (arXiv:2605.22333v1) with specific, falsifiable flaw categories.

---

## What does NOT transfer (explicit)

- All LLM-based security classifiers, compliance checkers, and evaluation judges (float, non-deterministic, network-dependent)
- All Kubernetes/gVisor-based infrastructure (engine uses WASM-based sandbox + import-time validation)
- The paper's OAuth implementation details (token flow, client registration protocol, dynamic registration mechanics)
- The paper's network-based measurement infrastructure (passive traffic inspection, active dynamic probing)
- Healthcare-specific compliance requirements (HIPAA)
- Any paper claim that requires live model access or external data in the tick
- The paper's remote-server measurement scope (the engine's MCP door is local)
- arXiv:2510.06445 (already served by arxiv-wave33)

---

## Integration homes

- **Subsystem plan**: `docs/plan/ai-native.md` (PLN9, prefix AX/MCP/EA/EV)
- **RESEARCH-VERDICT**: `docs/RESEARCH-VERDICT.md` (deep dive 12 section)
- **EVIDENCE**: `docs/EVIDENCE.md`
- **Editor AI control**: `docs/EDITOR_AI_CONTROL.md`
- **DEEP-DIVES**: `docs/DEEP-DIVES.md` (dive 12 row update)
- **SPRINT**: `docs/SPRINT.md` (board rows MCP11, RV2, CT16)
- **Focus**: `docs/FOCUS.md`

---

## Budget and next step

Budget: M 20 min (this packet). At budget stop, report cards and next step.

Next step: If cards 1 and 2 are adopted, write `docs/research/arxiv-wave36.md` as the persisted note (this file). Propose MCP11, RV2, CT16 as board rows via the orchestrator. Card 3 is REJECT (one reason: LLM-based compliance checking violates provider law and determinism). Card 5 is LATER (requires deterministic MCP testing infrastructure that doesn't exist yet). Card 4's zero-trust architecture is UNVERIFIED for the engine's local context (healthcare-specific, Kubernetes/gVisor).

---

## EMPIRE outcome per card

| Card | EMPIRE outcome |
|------|----------------|
| MCP11 (TAKE, REPRODUCE) | Hardened MCP door against measured OAuth flaws — supports "AI makes whole games" by making external agent edits trustworthy. Enables factory to sell MCP tooling with security guarantees. |
| RV2 (TAKE, REPRODUCE) | Provenance-attribution prompt injection defense — protects the brief→game pipeline from adversarial content. Supports "we ship our own games" with secure asset provenance. |
| LLM compliance (REJECT) | Rejected — stays deterministic and provider-agnostic. No LLM dependency for security compliance. |
| CT16 (TAKE, UNVERIFIED) | Zero-trust sandbox boundary — enables factory to sell content packs with security guarantees. Healthcare-specific architecture needs engine-specific validation. |
| Automated detection (LATER) | Deferred — requires deterministic MCP testing infrastructure. Revisit after VP1 provides deterministic capture-based evaluation. |

---

## Source traceability

- **ArXiv API**: `https://export.arxiv.org/api/query?search_query=MCP+authentication+security+measurement+2026&max_results=5`
- **Paper accessed**: https://arxiv.org/abs/2605.22333v1 (accessed 2026-09-26)
- **License verified**: Paper under arXiv distribution (NOT on deny.toml allow-list). Code/data: UNVERIFIED.
- **Cross-reference**: DEEP-DIVES.md §12 (dive 12 row), `docs/plan/ai-native.md` (PLN9)
- **Avoids double-serving**: arXiv:2510.06445 (arxiv-wave33, same dive) — different primary paper
- **EMPIRE.md**: §1 (we ship our own games), §4 (AI makes whole games), §3 (factory makes, markets, reuses)
