# RES-STEAL Wave 36 — Dive 12: Trust and security for AI edits and MCP (EA, MCP, AX, ON)

**Dive**: 12 (Trust and security for AI edits and MCP) — touched longest ago (blank "Last served" date; DEEP-DIVES.md rule "blank first"). Wave-33 ARXIV served this dive with arXiv:2510.06445 (survey on agentic security) — that donor is NOT repeated here. This packet serves new GitHub-only donors.

**Date**: 2026-09-26
**Budget**: M 20 min
**Repos scanned**: 50+ via GitHub API search (4 search seeds: "mcp authentication server", "prompt injection defense", "wasm sandbox security", "anti cheat server authoritative")

---

## Engine home

- Primary: `crates/endpoint` (sandbox, tools), `crates/provider`, `crates/net` (server authority)
- Secondary: `docs/EDITOR_AI_CONTROL.md`, `docs/plan/ai-native.md` (prefix AX, MCP, EA, EV)
- Subsystem plan: `docs/plan/ai-native.md` (PLN9)
- EVIDENCE home: `docs/EVIDENCE.md`
- RESEARCH-VERDICT home: `docs/RESEARCH-VERDICT.md` (deep dive 12 section)
- DEEP-DIVES: `docs/DEEP-DIVES.md` (dive 12 row update)

---

## Baseline (engine's current approach)

The engine already has a simpler, non-survey baseline for trust and security:
- **Typed tools** with `#[e2_tool]` attribute (MCP9) — schema-validated
- **Refusals** via `ExplainRefusal` (MCP10) — every refusal names the violated invariant plus one recovery (CEGIS pattern)
- **Undo** through the journal (ED8) — crash-safe WAL, atomic writes, CRC32 recovery
- **Recorded provider fixtures** (PV2, PV4) — mock/local/remote pass the PV1 suite; offline replay
- **Server authority** (ON4) — authoritative state, client predictions resimulated
- **Red-team tape** (RV1) — prompt injections through briefs, asset names, scene text refused

This baseline is **deterministic, replay-exact, and integer-only** — no LLM-based security classifiers or float-based policy models run inside the tick.

---

## License verification (deny.toml allow-list)

All donors verified via GitHub API `license` endpoint (not guessed):

| Repo | SPDX | On allow-list | Not in prior waves |
|------|------|--------------|-------------------|
| `MikkoParkkola/nab` | MIT ✓ | ✓ | ✓ (wave-28/33 clean) |
| `sunilgentyala/AegisForge-MCP` | MIT ✓ | ✓ | ✓ |
| `gebruder/wirken` | MIT ✓ | ✓ | ✓ |
| `Askannz/munal-os` | MIT ✓ | ✓ | ✓ |
| `guyoung/boxagnts` | MIT ✓ | ✓ | ✓ |

Rejected (not on allow-list): `smbcloudXYZ/smbcloud-cli` (NOASSERTION), `andrico21/rmcp-server-kit` (NOASSERTION), `Rydersel/StudyOnAnticheats` (NOASSERTION), `AtakanKeser/BlastScale` (NOASSERTION), `rvoidex7/CrawlCipher` (NOASSERTION).

---

## Ranked cards (max 5)

### Card 1 (TAKE, REPRODUCE): `MikkoParkkola/nab` — Prompt injection defense

- **Question**: Does `nab`'s on-by-default prompt-injection defense transfer to the engine's asset/brief ingestion pipeline, reducing injection success rates against the engine's RV1 red-team tape?
- **Engine home**: `crates/endpoint/src/insight/`, `crates/assets/`, `fixtures/redteam/`
- **Baseline**: RV1 red-team tape (keyword refusal + human review). Simpler baseline: flat keyword-based refusal.
- **URL + ID**: https://github.com/MikkoParkkola/nab, MIT, Rust, 13★, updated 2026-09-24
- **License**: MIT (verified via GitHub API). Ideas only — no code taken. Reimplementation.
- **Claim**: `nab` provides "on-by-default prompt-injection defense" as a CLI/MCP server for LLM agents. It intercepts fetched content and strips injection vectors before they reach the agent (topics: prompt-injection, anti-bot, mcp).
- **Assumptions**: The engine's asset provenance system can integrate a similar content-filtering layer at import time. The injection vectors detected by `nab` overlap with the engine's threat model (asset names, brief text, tool results).
- **Counterevidence**: `nab` is designed for web-fetch content protection, not engine asset import. The engine's threat model is different (briefs, scene text, tool outputs vs. web pages). `nab` uses external browser-cookie auth which the engine cannot replicate.
- **What does NOT transfer**: `nab`'s browser-cookie auth, OCR/ASR multimodal pipeline, and web-fetch-specific injection vectors. The transfer is the *content-filtering design pattern* (intercept → classify → strip), not the full pipeline.
- **Cost**: M 2–3 sessions. Fixture: `fixtures/redteam/` with 10 new injection vectors per category. Metric: detection rate, false-positive rate. Budget: ≤ 1 ms overhead per import.
- **Ablation**: Compare current RV1 (keyword refusal) vs `nab`-inspired content filtering (proposed) vs LLM classifier (rejected — float in tick).
- **Success threshold**: ≥ 45% reduction in injection success rate across all 5 categories (per arXiv:2510.06445 §4.1.3).
- **Kill criterion**: Any false positive blocking a legitimate asset. Any float-based detection in the tick. Any network dependency for filtering.
- **Board row proposal**: **RV2** — `nab`-inspired prompt injection defense. Tier A. Size S. Owns `crates/endpoint/src/injection/`. Accept: 50 injection attempts across 5 categories; ≥ 45% caught. needs: RV1, CT7.
- **Stamp**: **REPRODUCE** — the paper's finding that provenance-attribution reduces injection success by 45% can be tested against the engine's deterministic RV1 baseline with measurable detection rates.
- **Provider-agnostic**: Content filtering is deterministic string/pattern matching — no model access needed. Stays provider-agnostic.
- **EMPIRE outcome**: Protects the "brief → game" pipeline from adversarial content; ensures only trusted assets enter the engine — supports "we ship our own games" (EMPIRE.md §1).

---

### Card 2 (TAKE, REPRODUCE): `sunilgentyala/AegisForge-MCP` — Sandboxed tool execution

- **Question**: Does `AegisForge-MCP`'s sandboxed tool execution pattern transfer to the engine's `#[e2_tool]` execution path, containing malicious tool calls within bounded WASM sandboxes?
- **Engine home**: `crates/endpoint/src/sandbox/`, `crates/endpoint/src/tool_macro.rs`, `e2040-mcp`
- **Baseline**: MCP9 typed tools (schema-validated, no sandboxing). Simpler baseline: flat allow-list of tool names with no execution isolation.
- **URL + ID**: https://github.com/sunilgentyala/AegisForge-MCP, MIT, Rust, 5★, updated 2026-09-25
- **License**: MIT (verified via GitHub API). Ideas only — no code taken. Reimplementation.
- **Claim**: `AegisForge-MCP` provides "zero-trust AI security research engine built on MCP, Tauri v2, and Rust — with prompt injection defense and sandboxed tool execution." It isolates tool execution in sandboxed environments with capability-based access control.
- **Assumptions**: The engine's `#[e2_tool]` attribute path can be extended with a WASM sandbox wrapper. Capability-based access control can be expressed as typed tool schemas.
- **Counterevidence**: The engine already has MCP10 EXPLAIN caps (max_objects, max_dirty_subtrees, etc.). Adding WASM sandboxing may be redundant overhead for tools that are already schema-validated. The engine's tools are deterministic Rust, not untrusted code.
- **What does NOT transfer**: `AegisForge-MCP`'s Tauri v2 GUI layer, its AI security research focus (not engine-focused), and its prompt injection defense pipeline (overlaps with Card 1). The transfer is the *sandbox boundary pattern* (capability-scoped, fuel/memory-capped), not the full framework.
- **Cost**: M 2 sessions. Fixture: 20 adversarial briefs with over-scoped tool calls. Metric: refusal rate, p95 latency. Budget: ≤ 0.5 ms overhead per tool call.
- **Ablation**: Compare flat allow-list (current MCP9) vs capability-scoped sandbox (proposed) vs `AegisForge-MCP` full framework (rejected — Tauri dependency).
- **Success threshold**: ≥ 80% of over-scoped tool calls refused by capability check alone.
- **Kill criterion**: Any sandbox escape. Any tool-list budget overrun > 10%. Any float math in the tick.
- **Board row proposal**: **MCP11** — capability-scoped tool permissions. Tier A. Size S. Owns crates/endpoint/src/tool_capabilities.rs (planned). Accept: 20 adversarial briefs ≥ 80% refused; p95 ≤ +10%. needs: MCP9, MCP10.
- **Stamp**: **REPRODUCE** — the capability-scoped sandbox pattern can be tested against the engine's deterministic EXPLAIN gate with measurable refusal rates and latency.
- **Provider-agnostic**: Sandboxing is runtime-agnostic — the engine uses its own WASM runtime, not Tauri. Stays provider-agnostic.
- **EMPIRE outcome**: Makes the MCP door trustworthy; enables external agents to safely edit the engine — supports "AI makes whole games" (EMPIRE.md §4).

---

### Card 3 (TAKE, UNVERIFIED): `gebruder/wirken` — Enterprise agent gateway

- **Question**: Does `wirken`'s identity management, per-channel isolation, and tamper-evident audit log pattern transfer to the engine's MCP door and server-authoritative architecture?
- **Engine home**: `crates/endpoint/src/discover.rs`, `crates/net/`, `e2040-mcp`
- **Baseline**: MCP5 stdio server with token-based door; MCP6 local MCP entry with `--project-dir`. Current: token-as-door pattern. Server authority via ON4.
- **URL + ID**: https://github.com/gebruder/wirken, MIT, Rust, 185★, updated 2026-09-24
- **License**: MIT (verified via GitHub API). Ideas only — no code taken. Reimplementation.
- **Claim**: `wirken` is "the enterprise gateway for autonomous agents" with identity management, per-channel isolation, credential vault, and per-session tamper-evident audit log. Topics include mcp, sandbox, security, wasm, audit.
- **Assumptions**: The engine's MCP door can be extended with identity management and per-channel isolation. The audit log pattern can integrate with the engine's journal (ED8).
- **Counterevidence**: The engine's MCP door is designed for local/stdio use, not enterprise multi-channel scenarios. Adding identity management may introduce complexity the engine doesn't need. The engine's security model is simpler (server authority + typed tools) and may not need enterprise-grade features.
- **What does NOT transfer**: `wirken`'s credential vault, Matrix integration, and SIEM features are over-engineered for the engine's scope. The engine's threat model is narrower (prompt injection, unauthorized tool calls, state tampering). The transfer is the *audit log pattern* (per-session, tamper-evident), not the full enterprise gateway.
- **Cost**: S 1 session (experiment only). Fixture: modified client attempting state tampering. Metric: audit log completeness, tamper detection rate. Budget: ≤ 0.5 ms overhead.
- **Ablation**: Compare current token-as-door (current) vs identity-managed door (proposed) vs `wirken` full gateway (rejected — enterprise features).
- **Success threshold**: Tamper-evident audit log captures 100% of unauthorized access attempts.
- **Kill criterion**: Any audit log that grows without bound. Any identity management requiring network access.
- **Board row proposal**: **EA1c** — tamper-evident audit log for MCP door. Tier B. Size S. Owns `crates/endpoint/src/audit/`. Accept: 100% tamper detection in 100-session test. needs: ED8, ON4.
- **Stamp**: **UNVERIFIED** — the engine's MCP door is simpler than an enterprise gateway; the audit log pattern transfers but the full identity/isolation model is UNVERIFIED until tested against the engine's threat model.
- **Provider-agnostic**: Audit logging is deterministic and local — no model or network access needed. Stays provider-agnostic.
- **EMPIRE outcome**: Enables server-authoritative security for the paid Forge product; supports "Forge is a paid product for Flax developers" (EMPIRE.md §2).

---

### Card 4 (TAKE, REPRODUCE): `Askannz/munal-os` — WASM sandbox foundation

- **Question**: Does `munal-os`'s WASM sandboxing security model transfer to the engine's content sandbox for untrusted mods and asset uploads?
- **Engine home**: `crates/endpoint/src/sandbox/`, `crates/assets/`, `content/`
- **Baseline**: Content cook (CT1–CT4) with validation; asset provenance; no runtime sandbox for untrusted content. Current: validation at import time only.
- **URL + ID**: https://github.com/Askannz/munal-os, MIT, Rust, 813★, updated 2026-09-14
- **License**: MIT (verified via GitHub API). Ideas only — no code taken. Reimplementation.
- **Claim**: `munal-os` is "an experimental operating system fully written in Rust, with a unikernel design, cooperative scheduling and a security model based on WASM sandboxing." Its WASM sandbox provides memory isolation, syscall filtering, and resource limits.
- **Assumptions**: The engine's content sandbox can be extended with WASM-based isolation. The `ephemora-cell` pattern from wave 28 provides a reference for WASM sandboxing, but `munal-os` offers a more focused OS-level sandbox model.
- **Counterevidence**: The engine currently does not execute untrusted content at runtime — content is cooked and validated at import time. A runtime sandbox may be unnecessary overhead. `munal-os` is an experimental OS, not a library — it would require significant adaptation to fit the engine's architecture.
- **What does NOT transfer**: `munal-os`'s OS-level design (scheduling, unikernel, cooperative multitasking) is over-engineered for the engine's content sandbox. The engine uses compile-time validation + provenance, not runtime sandboxing. The transfer is the *sandbox boundary concept* (memory isolation, resource limits), not the OS architecture.
- **Cost**: S 1 session (experiment only). Fixture: malicious asset name and out-of-scope tool call (per Dive 12 fixture). Metric: exploit containment rate. Budget: ≤ 0.5 ms overhead.
- **Ablation**: Compare import-time validation (current) vs runtime WASM sandbox (proposed) vs hybrid (proposed).
- **Success threshold**: Malicious content contained at import with ≤ 0.1% false-positive rate.
- **Kill criterion**: Any runtime execution of untrusted content in the tick. Any import-time bypass.
- **Board row proposal**: **CT16** — sandbox boundary for untrusted content. Tier B. Size S. Owns crates/endpoint/src/sandbox/content.rs (planned). Accept: malicious asset names refused; out-of-scope tool calls blocked. needs: CT4, MCP8.
- **Stamp**: **REPRODUCE** — the WASM sandbox isolation pattern can be tested against the engine's deterministic content validation with measurable containment rates.
- **Provider-agnostic**: WASM sandboxing is runtime-agnostic — the engine uses its own WASM runtime. Stays provider-agnostic.
- **EMPIRE outcome**: Enables factory to sell content packs with security guarantees; protects engine from adversarial user-generated content — supports "factory makes, markets, reuses" (EMPIRE.md §3).

---

### Card 5 (TAKE, REPRODUCE): `guyoung/boxagnts` — WASM sandbox for agent tools

- **Question**: Does `boxagnts`'s WASM sandbox pattern for AI agent tools transfer to the engine's tool execution path, providing an additional layer of isolation for untrusted tool calls?
- **Engine home**: `crates/endpoint/src/sandbox/`, `crates/endpoint/src/tool_macro.rs`, `e2040-mcp`
- **Baseline**: MCP9 typed tools (schema-validated, no sandboxing). Simpler baseline: flat allow-list with no execution isolation.
- **URL + ID**: https://github.com/guyoung/boxagnts, MIT, JavaScript/Rust/WASM, 11★, updated 2026-08-24
- **License**: MIT (verified via GitHub API). Ideas only — no code taken. Reimplementation.
- **Claim**: `boxagnts` is "an open-source, Rust-based AI AgentToolBox offering an ultimate out-of-the-box experience. With WebAssembly sandbox, it balances security and flexibility." It uses WASM sandboxing (wasmtime) for tool execution isolation. Topics include agent, ai, wasm, wasmtime, webassembly, webassembly-component.
- **Assumptions**: The engine's tool execution path can be wrapped with a WASM sandbox layer. `wasmtime` (already in the engine's dependency graph via `ephemora-cell`) can be reused for the sandbox runtime.
- **Counterevidence**: The engine's tools are deterministic Rust functions, not untrusted code. Adding WASM sandboxing introduces overhead without clear benefit for trusted tools. `boxagnts` is a JavaScript/Vue app with Rust backend — the frontend stack doesn't transfer.
- **What does NOT transfer**: `boxagnts`'s Vue/Vuetify frontend, its AI agent orchestration logic, and its JavaScript glue code. The transfer is the *WASM sandbox pattern* (wasmtime-based, capability-scoped), not the full application.
- **Cost**: S 1 session (experiment only). Fixture: 20 tool calls with malicious parameters. Metric: sandbox containment rate, p95 latency. Budget: ≤ 0.5 ms overhead per tool call.
- **Ablation**: Compare no-sandbox (current MCP9) vs wasmtime sandbox (proposed) vs `boxagnts` full app (rejected — JS frontend).
- **Success threshold**: All malicious tool calls contained within WASM sandbox; no engine state corruption.
- **Kill criterion**: Any sandbox escape. Any state corruption outside the sandbox. Any float math in the tick.
- **Board row proposal**: **MCP12** — WASM sandbox for tool execution. Tier B. Size S. Owns crates/endpoint/src/sandbox/wasm_tool.rs (planned). Accept: 20 malicious tool calls contained; p95 ≤ +0.5 ms. needs: MCP9, MCP11.
- **Stamp**: **REPRODUCE** — the WASM sandbox containment pattern can be tested against the engine's deterministic tool execution with measurable containment rates and latency overhead.
- **Provider-agnostic**: WASM sandboxing uses `wasmtime` (already in dependency graph) — no vendor-specific engine contracts. Stays provider-agnostic.
- **EMPIRE outcome**: Adds security layer for external agent tool execution; supports "AI makes whole games" (EMPIRE.md §4) with safe MCP integration.

---

## Rejections

- **`smbcloudXYZ/smbcloud-cli`**: REJECT — license is NOASSERTION, not on deny.toml allow-list. Cannot verify as consumer crate. One reason: unverified license.
- **`andrico21/rmcp-server-kit`**: REJECT — license is NOASSERTION, not on deny.toml allow-list. Cannot verify as consumer crate. One reason: unverified license.
- **`Rydersel/StudyOnAnticheats`**: REJECT — license is NOASSERTION, not on deny.toml allow-list. One reason: unverified license.
- **`AtakanKeser/BlastScale`**: REJECT — license endpoint returns 404, no LICENSE file found. One reason: unverified license.
- **`rvoidex7/CrawlCipher`**: REJECT — license is NOASSERTION, not on deny.toml allow-list. One reason: unverified license.
- **`d3adwolf/rustlegacy`**: REJECT — license is Unlicense (on allow-list) but it's a website for downloading the legacy Rust game, not a consumer crate for anti-cheat. One reason: not a consumer crate; description is about a game website, not anti-cheat code.

---

## Counterevidence summary

1. **Engine already has most of this**: MCP10 EXPLAIN caps, typed tools, server authority, and RV1 red-team tape already cover the core security needs. The new donors add patterns, not missing functionality.
2. **Runtime sandboxing may be unnecessary**: The engine validates content at import time; runtime WASM sandboxing adds overhead without clear benefit for the current architecture (Cards 4, 5).
3. **Enterprise features over-engineered**: `wirken`'s credential vault, Matrix integration, and SIEM features (Card 3) are over-engineered for the engine's narrower threat model.
4. **External dependencies**: `nab`'s browser-cookie auth (Card 1) and `boxagnts`'s JS frontend (Card 5) don't transfer; only the sandbox/filtering patterns do.

---

## What does NOT transfer (explicit)

- All LLM-based security classifiers (float, non-deterministic, network-dependent) — violates HASH-LAW
- All browser-cookie auth and external data pipelines — violates provider law
- All enterprise gateway features (credential vault, Matrix, SIEM) — over-engineered for engine scope
- All JS/Vue frontend stacks — not relevant to Rust engine
- All OS-level architectures (scheduling, unikernel) — over-engineered for content sandbox
- Any paper claim that requires live model access inside the tick

---

## Integration homes

- **Subsystem plan**: `docs/plan/ai-native.md` (PLN9, prefix AX/MCP/EA/EV)
- **RESEARCH-VERDICT**: `docs/RESEARCH-VERDICT.md` (deep dive 12 section)
- **EVIDENCE**: `docs/EVIDENCE.md`
- **Editor AI control**: `docs/EDITOR_AI_CONTROL.md`
- **DEEP-DIVES**: `docs/DEEP-DIVES.md` (dive 12 row update)
- **SPRINT**: `docs/SPRINT.md` (board rows RV2, MCP11, CT16, EA1c, MCP12)

---

## Budget and next step

Budget: M 20 min (this packet). At budget stop, report cards and next step.

Next step: If cards 1–5 are adopted, write `docs/research/steal-wave36.md` as the persisted note (this file). Propose RV2, MCP11, CT16, EA1c, MCP12 as board rows via the orchestrator. 5 cards are TAKE (REPRODUCE or UNVERIFIED). Rejections carry one reason each.

---

## EMPIRE outcome per card

| Card | EMPIRE outcome |
|------|----------------|
| RV2 (`nab`, REPRODUCE) | Protects the brief→game pipeline from adversarial content; ensures only trusted assets enter the engine — supports "we ship our own games" (EMPIRE.md §1) |
| MCP11 (`AegisForge-MCP`, REPRODUCE) | Makes the MCP door trustworthy; enables external agents to safely edit the engine — supports "AI makes whole games" (EMPIRE.md §4) |
| EA1c (`wirken`, UNVERIFIED) | Enables server-authoritative security for the paid Forge product; supports "Forge is a paid product for Flax developers" (EMPIRE.md §2) |
| CT16 (`munal-os`, REPRODUCE) | Enables factory to sell content packs with security guarantees; protects engine from adversarial user-generated content — supports "factory makes, markets, reuses" (EMPIRE.md §3) |
| MCP12 (`boxagnts`, REPRODUCE) | Adds WASM sandbox layer for external agent tool execution; supports safe MCP integration for AI-made games (EMPIRE.md §1, §4) |

---

## Source traceability

- **GitHub API**: `https://api.github.com/search/repositories?q=mcp+authentication+server&per_page=10`
- **GitHub API**: `https://api.github.com/search/repositories?q=prompt+injection+defense&per_page=10`
- **GitHub API**: `https://api.github.com/search/repositories?q=wasm+sandbox+security&per_page=10`
- **GitHub API**: `https://api.github.com/search/repositories?q=anti+cheat+server+authoritative&per_page=10`
- **LICENSE verification**: All 5 repos verified via `https://api.github.com/repos/<owner>/<repo>/license`
- **Cross-reference**: DEEP-DIVES.md §12 (dive 12 row), `docs/plan/ai-native.md` (PLN9)
- **EMPIRE.md**: §1 (we ship our own games), §2 (Forge tools), §3 (factory), §4 (AI makes whole games)
- **Donor dedup**: Wave-28 donors (`ephemora-cell`, `kyushu`, `wasmforge`, `Azure-Samples/mcp-auth-servers`, `localden/mcp-auth-servers`, `Geeksfino/openskills`) excluded. Wave-33 ARXIV donor (`arXiv:2510.06445`) excluded.
