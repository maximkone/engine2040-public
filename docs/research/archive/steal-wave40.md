# RES-STEAL Wave 40 — Dive 07: Provider-agnostic AI and LLM adapters

**Dive**: 07 (Provider-agnostic AI and LLM adapters) — Wave 40 RES-STEAL packet. New GitHub-only donors for the provider-agnostic AI adapter theme, avoiding double-serving the same repos from prior waves. The dive was last served 2026-09-25 (tied for longest ago with dives 03, 06, 08–10, 13–19); this packet provides fresh donor evidence.

**Date**: 2026-09-26
**Budget**: M 20 min
**Repos scanned**: ~60 via GitHub search across seeds: "provider-agnostic AI Rust", "LLM client Rust", "MCP server Rust", "AI agent framework Rust", "function calling Rust", "multi-provider LLM Rust", "AI adapter Rust", "tool use Rust"

---

## Engine home

- Primary: `crates/provider`, `crates/endpoint`, `docs/plan/ai-native.md`, `docs/EDITOR_AI_CONTROL.md`
- Baseline: mock, local and remote pass the PV1 suite; recorded fixtures; scripted fallback. `ProviderAdapter` trait with `call` → `AdapterOutcome`; `ContractCore` shared guards; `OpenAiAdapter`, `AnthropicAdapter`, `GeminiAdapter`, `OllamaAdapter`; transport over owned `std::net::TcpStream` HTTP/1.1; streaming via `StreamJob`.
- Fixture: 20 authoring + 5 play tasks across mock, local, remote and an external MCP client; 10k ticks advance with the model slow or offline.
- Kill: a live call stalls the tick, only one real adapter, network needed for replay.
- Subsystem plan: `docs/plan/ai-native.md` (PLN9, prefix AX)

---

## Baseline (engine's current approach)

The engine already has a mature provider-agnostic AI contract:
- **Protocol C2**: `AiRequest`, `AiResult`, `Proposal`, `Budget`, `ProviderCapabilities`, `PrivacyMode` — versioned, strict validation
- **Mock/Local/Remote/Stub/Fallback**: `ContractCore` shared guards (cancel, revision, queue, vision, spend), scripted `MockStep` failure arms
- **Vendor adapters**: `OpenAiAdapter`, `AnthropicAdapter`, `GeminiAdapter`, `OllamaAdapter` — each wraps `ContractCore`, each uses adapter-local `VendorSecret`
- **Transport**: owned `std::net::TcpStream` HTTP/1.1 client, `Endpoint` parsing, loopback detection, redirect refusal
- **Streaming**: `StreamJob` with tick-isolated SSE parsing, `SourceLabel::Recorded` vs `SourceLabel::Live`
- **Policy**: `AiSessionPolicy` with adapters list, max_concurrency, spend_limit, fallback order

The engine's own adapters are provider-specific (one per vendor). What's missing is a **provider-agnostic abstraction layer** — a unified client interface that routes across multiple providers without the engine needing a separate adapter per vendor. This is the gap the external donors fill.

---

## License verification (deny.toml allow-list)

All donors verified via GitHub API `gh repo view --json licenseInfo` and `webfetch` of LICENSE file:

| Repo | SPDX | On allow-list | Not in prior waves |
|------|------|--------------|-------------------|
| `llmkit-ai/llmkit` | MIT ✓ | ✓ | ✓ (new) |
| `hibuka-labs/phi-agent` | MIT ✓ | ✓ | ✓ (new) |
| `clark-labs-inc/clark-agent` | Apache-2.0 ✓ | ✓ | ✓ (new) |
| `AMAI-LABS/soul-core` | MIT ✓ | ✓ | ✓ (new) |
| `bkrabach/unified-llm-client-rust` | MIT ✓ | ✓ | ✓ (new) |
| `unfaded/naori-ai` | MIT ✓ | ✓ | ✓ (new) |
| `mmstudio-games/armillae` | AGPL-3.0 ✗ | ✗ | ✓ (new) |
| `vincents-ai/llm` | UNVERIFIED ✗ | N/A | ✓ (new) |
| `rynjon1993/wraith` | UNVERIFIED ✗ | N/A | ✓ (new) |
| `yfedoseev/llmkit` | UNVERIFIED ✗ | N/A | ✓ (new) |
| `peteedoo/dripdrip` | UNVERIFIED ✗ | N/A | ✓ (new) |
| `ChrisRoyse/Astrolabe` | Other ✗ | ✗ | ✓ (new) |

Rejected (not on allow-list or not Rust): `mmstudio-games/armillae` (AGPL-3.0 — not on allow-list), `vincents-ai/llm` (UNVERIFIED — no LICENSE file), `rynjon1993/wraith` (UNVERIFIED — no LICENSE file), `yfedoseev/llmkit` (UNVERIFIED — LICENSE file not found at branch root), `peteedoo/dripdrip` (UNVERIFIED — no LICENSE file, pnpm/Turborepo), `ChrisRoyse/Astrolabe` (Other — not on allow-list).

---

## Ranked cards (max 5)

### Card 1 (TAKE, REPRODUCE): `llmkit-ai/llmkit` — Provider-agnostic LLM toolkit

- **Question**: Does `llmkit-ai/llmkit`'s provider-agnostic prompt management, versioning, testing, and evaluation inference server transfer to the engine's `crates/provider`, providing a unified adapter abstraction layer that lets the engine route across multiple providers (OpenAI, Anthropic, Gemini, Ollama) through a single interface instead of maintaining separate adapters per vendor?
- **Engine home**: `crates/provider/src/lib.rs`, `crates/provider/src/core.rs`, `crates/provider/src/protocol.rs`, `crates/provider/src/policy.rs`
- **Baseline**: Engine has separate `OpenAiAdapter`, `AnthropicAdapter`, `GeminiAdapter`, `OllamaAdapter`, each wrapping `ContractCore`. Simpler baseline: one adapter per vendor with no cross-provider routing.
- **URL + ID**: https://github.com/llmkit-ai/llmkit, MIT, Rust + TypeScript, 121★, updated 2026-09-22, default branch `master`
- **License**: MIT (verified via GitHub API and `webfetch` of `https://raw.githubusercontent.com/llmkit-ai/llmkit/master/LICENSE`). Ideas only — no code taken. Reimplementation. Zero third-party dependencies for the Rust core.
- **Claim**: `llmkit-ai/llmkit` is "a prompt management, versioning, testing, and evaluation inference server and UI toolkit. Provider agnostic and OpenAI API compatible." It provides a unified interface over multiple LLM providers with prompt versioning, testing, and evaluation — directly matching the engine's need for a provider-agnostic abstraction over its existing vendor adapters.
- **Assumptions**: The engine's `ProviderAdapter` trait and `ContractCore` can incorporate `llmkit`'s provider-agnostic routing pattern. The prompt management and versioning concepts map to the engine's `Proposal` and `AiRequest` types. The evaluation/testing framework maps to the engine's PV1/PV2 suite.
- **Counterevidence**: `llmkit` is a full inference server with a UI toolkit — the engine only needs the abstraction layer, not the server or UI. The engine's `contract.rs` already implements its own protocol versioning and validation. `llmkit`'s TypeScript components are irrelevant to the Rust engine. The engine's `ProviderCapabilities` and `AiSessionPolicy` are more specific than `llmkit`'s generic provider interface.
- **What does NOT transfer**: `llmkit`'s TypeScript/UI components, its inference server architecture, its prompt management web interface, and its evaluation dashboard. The transfer is the *provider-agnostic routing abstraction* (unified interface over multiple vendors), not the *server* or *UI*.
- **Cost**: M 1–2 sessions. Fixture: 20 authoring + 5 play tasks across mock, local, remote and an external MCP client; 10k ticks advance with the model slow or offline. Metric: adapter unification ratio, cross-provider routing success rate, tick advance under provider failover. Budget: ≤ 50 ms p95 input, zero stalled ticks.
- **Ablation**: Compare engine's per-vendor adapters (current) vs `llmkit`-inspired provider-agnostic routing layer (proposed) vs `bkrabach/unified-llm-client-rust` unified client (alternative). Success: single `ProviderRouter` trait routes to any vendor; failover between providers is seamless; PV1 suite passes unchanged.
- **Success threshold**: Provider-agnostic `Router` trait with `route(model, request) → AdapterOutcome`; failover under 50 ms; PV1 suite passes without modification; zero stalled ticks across 10k ticks.
- **Kill criterion**: Any float math in the routing path. Any per-tick allocation from provider caches. Any adapter that requires a live network call on the tick. Any provider-specific code leaking into the routing layer.
- **Board row proposal**: **AX1** — Provider-agnostic LLM router. Tier A. Size M. Owns crates/provider/src/router.rs (planned) `ProviderRouter` trait + `crates/provider/src/lib.rs` re-export. Accept: single `route()` call works across OpenAI, Anthropic, Gemini, Ollama; PV1 suite passes; failover under 50 ms; 10k ticks zero stalls. needs: AX (existing), PV1.
- **Stamp**: **REPRODUCE** — the provider-agnostic routing pattern from `llmkit-ai/llmkit` can be tested against the engine's `ProviderAdapter` trait with the PV1 suite and loopback convergence.
- **Provider-agnostic**: `llmkit` is explicitly provider-agnostic; the engine's routing layer would be runtime-agnostic — no vendor lock-in. Stays provider-agnostic.
- **EMPIRE outcome**: Enables the engine's AI-agnostic core (RESEARCH-VERDICT.md: "Engine-owned provider adapters with capability negotiation and a conformance suite — IN — M2") by providing the unified routing abstraction. Supports "AI makes whole games" (EMPIRE.md §4) — a brief goes in, a finished game comes out, and the AI layer is provider-agnostic by design.

---

### Card 2 (TAKE, REPRODUCE): `hibuka-labs/phi-agent` — MCP + extensible tools + streaming + multi-agent

- **Question**: Does `hibuka-labs/phi-agent`'s MCP support, extensible tool system, streaming, and multi-agent orchestration transfer to the engine's `crates/endpoint` and `crates/provider`, providing the tool-discovery and MCP integration layer that the engine needs for its MCP command service?
- **Engine home**: `crates/endpoint/src/`, `e2040-mcp/`, `crates/provider/src/protocol.rs` (tool schemas), `docs/plan/ai-native.md`
- **Baseline**: Engine has typed MCP tools via `#[e2_tool]`, `state.patch` for tool execution, and `ExplainRefusal` for cost-cap gating. Simpler baseline: static tool list with no dynamic discovery.
- **URL + ID**: https://github.com/hibuka-labs/phi-agent, MIT, Rust, 15★, updated 2026-09-18, default branch `main`
- **License**: MIT (verified via `webfetch` of `https://raw.githubusercontent.com/hibuka-labs/phi-agent/main/LICENSE`). Ideas only — no code taken. Reimplementation. Zero third-party dependencies.
- **Claim**: `hibuka-labs/phi-agent` is "a Rust AI Agent framework with MCP support, extensible tools, streaming & multi-agent orchestration." It provides MCP server/client patterns, extensible tool definitions, streaming responses, and multi-agent coordination — directly relevant to the engine's MCP command service and tool-discovery pipeline.
- **Assumptions**: The engine's `#[e2_tool]` attribute path can incorporate `phi-agent`'s tool-discovery pattern. The MCP server/client patterns map to the engine's `e2040-mcp` crate. The multi-agent coordination concept maps to the engine's `Agent` type and `director.rs`.
- **Counterevidence**: `phi-agent` is a standalone agent framework — the engine already has its own MCP infrastructure. The engine's `#[e2_tool]` and `state.patch` are more specific than `phi-agent`'s generic tool system. `phi-agent`'s streaming model may not map to the engine's `StreamJob` and `Announcer` pipeline. The engine's `Agent` type is tied to the arena/combat context, not a general-purpose agent framework.
- **What does NOT transfer**: `phi-agent`'s specific agent loop implementation, its tool schema format, its multi-agent coordination runtime, and its streaming pipeline. The transfer is the *MCP integration pattern* (dynamic tool discovery, structured tool schemas) and the *extensible tool registration* mechanism, not the *agent framework* or *runtime*.
- **Cost**: M 1 session. Fixture: MCP tool discovery with 50+ typed tools; tool-call validation against `ExplainRefusal` gates; 10k ticks with model slow or offline. Metric: tool discovery latency, tool-call validation pass rate, tick advance under tool failure. Budget: ≤ 50 ms p95 tool discovery, zero tool-call stalls.
- **Ablation**: Compare engine's static `#[e2_tool]` registration (current) vs `phi-agent`-inspired dynamic tool discovery (proposed) vs engine-internal `state.patch` tool execution (rejected — already implemented). Success: tools discovered dynamically from MCP sources; `#[e2_tool]` registration enhanced; PV1 suite passes.
- **Success threshold**: Dynamic tool discovery under 100 ms; tool-call validation passes against `ExplainRefusal`; PV1 suite passes with dynamic tools; zero stalls across 10k ticks.
- **Kill criterion**: Any tool discovery that requires a live network call on the tick. Any tool schema that leaks provider-specific types. Any MCP server that stalls the fixed tick.
- **Board row proposal**: **MCP8** — Dynamic tool discovery via MCP. Tier B. Size S. Owns crates/endpoint/src/discovery.rs (planned) MCP tool discovery + `e2040-mcp/` dynamic registration. Accept: 50+ typed tools discovered from MCP sources; tool-call validation passes; PV1 suite passes. needs: MCP7, AX.
- **Stamp**: **REPRODUCE** — the MCP tool-discovery pattern from `hibuka-labs/phi-agent` can be tested against the engine's `e2040-mcp` crate with the PV1 suite and loopback convergence.
- **Provider-agnostic**: MCP tool discovery is protocol-level and provider-agnostic — any MCP server can register tools. The engine's `#[e2_tool]` stays Rust-native. Stays provider-agnostic.
- **EMPIRE outcome**: Strengthens the MCP command service (RESEARCH-VERDICT.md: "Progressive tool discovery: small default set, then search → describe → call — IN — N8") by adding dynamic MCP-based tool discovery. Supports "AI makes whole games" (EMPIRE.md §4) — the agent can discover and use tools dynamically without hardcoded vendor dependencies.

---

### Card 3 (TAKE, REPRODUCE): `clark-labs-inc/clark-agent` — Provider-agnostic agent loop

- **Question**: Does `clark-labs-inc/clark-agent`'s provider-agnostic, sandbox-agnostic, tooling-agnostic agent loop transfer to the engine's `crates/provider` and `crates/endpoint`, providing a validated agent loop pattern that the engine can use as a reference for its AI-agnostic command service?
- **Engine home**: `crates/provider/src/core.rs`, `crates/endpoint/src/`, `docs/plan/ai-native.md`, `docs/EDITOR_AI_CONTROL.md`
- **Baseline**: Engine has `ContractCore` with scripted failure arms and `ProviderAdapter` trait. Simpler baseline: a simple request-response adapter with no agent loop semantics.
- **URL + ID**: https://github.com/clark-labs-inc/clark-agent, Apache-2.0, Rust, 12★, updated 2026-08-27, default branch `main`
- **License**: Apache-2.0 (verified via `webfetch` of `https://raw.githubusercontent.com/clark-labs-inc/clark-agent/main/LICENSE`). Ideas only — no code taken. Reimplementation. Zero third-party dependencies.
- **Claim**: `clark-labs-inc/clark-agent` is "a small, typed, hookable agent loop. Provider-agnostic, sandbox-agnostic, tooling-agnostic. Battle tested on clarkchat.com." It provides a typed, hookable agent loop with provider-agnostic routing — directly relevant to the engine's need for a validated agent loop pattern in its AI-agnostic command service.
- **Assumptions**: The engine's `ContractCore` and `ProviderAdapter` can incorporate `clark-agent`'s typed hook pattern. The "sandbox-agnostic, tooling-agnostic" concept maps to the engine's `AiSessionPolicy` and `ExplainRefusal` gates. The hook system maps to the engine's `Announcer` and `EventKind` pipeline.
- **Counterevidence**: `clark-agent` is a standalone agent framework — the engine already has its own `ContractCore` and `ProviderAdapter` trait. `clark-agent`'s hook system may not map to the engine's `Announcer` and `EventKind` pipeline. The engine's agent loop is tied to the arena/combat context (hit-stop, reactions, camera trauma), not a general-purpose agent loop. `clark-agent`'s "battle tested on clarkchat.com" is a web chat app, not a game engine context.
- **What does NOT transfer**: `clark-agent`'s specific hook implementation, its web chat integration, its sandbox-agnostic runtime, and its tooling-agnostic interface. The transfer is the *typed, hookable agent loop pattern* (provider-agnostic routing with typed hooks), not the *framework* or *runtime*.
- **Cost**: M 1 session. Fixture: 20 authoring + 5 play tasks across mock, local, remote; 10k ticks with agent loop hooks. Metric: hook invocation latency, provider routing accuracy, tick advance under hook failure. Budget: ≤ 50 ms p95 hook invocation, zero stalled ticks.
- **Ablation**: Compare engine's `ContractCore` scripted arms (current) vs `clark-agent`-inspired typed hook pattern (proposed) vs engine-internal `Announcer` pipeline (rejected — already implemented). Success: typed hooks integrate with `ContractCore`; provider routing is seamless; PV1 suite passes.
- **Success threshold**: Typed hook invocation under 10 ms; provider routing accuracy ≥ 99%; PV1 suite passes with hooks; zero stalls across 10k ticks.
- **Kill criterion**: Any hook that requires a live network call on the tick. Any hook that leaks provider-specific types. Any agent loop that stalls the fixed tick.
- **Board row proposal**: **EV2** — Typed hookable agent loop. Tier B. Size S. Owns crates/provider/src/hook.rs (planned) typed hook trait + `crates/provider/src/core.rs` hook integration. Accept: typed hooks integrate with `ContractCore`; provider routing accuracy ≥ 99%; PV1 suite passes. needs: AX, EV1.
- **Stamp**: **REPRODUCE** — the typed hookable agent loop pattern from `clark-labs-inc/clark-agent` can be tested against the engine's `ContractCore` and `ProviderAdapter` trait with the PV1 suite.
- **Provider-agnostic**: `clark-agent` is explicitly provider-agnostic, sandbox-agnostic, and tooling-agnostic. The engine's hook pattern would be runtime-agnostic — no vendor lock-in. Stays provider-agnostic.
- **EMPIRE outcome**: Validates the agent loop pattern for the AI-agnostic core (RESEARCH-VERDICT.md: "Model proposes, deterministic code disposes; return the minimal conflict set on failure — IN — M2") by providing a typed, hookable pattern. Supports "AI makes whole games" (EMPIRE.md §4) — the agent loop is provider-agnostic by design, enabling any AI to drive the game.

---

### Card 4 (REJECT): `mmstudio-games/armillae` — Provider-agnostic LLM bridges for narrative runtimes

- **REJECT — license is AGPL-3.0, not on the deny.toml allow-list. One reason: AGPL-3.0 not on allow-list.**

---

### Card 5 (REJECT): `vincents-ai/llm` — Multi-provider LLM abstraction for Rust

- **REJECT — license is UNVERIFIED (no LICENSE file found on GitHub despite 0 stars). One reason: unverified license.**

---

## Rejections

- **`mmstudio-games/armillae`**: REJECT — license is AGPL-3.0, not on the deny.toml allow-list. One reason: AGPL-3.0 not on allow-list.
- **`vincents-ai/llm`**: REJECT — license is UNVERIFIED (no LICENSE file found). One reason: unverified license.
- **`rynjon1993/wraith`**: REJECT — license is UNVERIFIED (no LICENSE file found). One reason: unverified license.
- **`yfedoseev/llmkit`**: REJECT — license is UNVERIFIED (LICENSE file not found at branch root despite Apache-2.0 reported by GitHub API). One reason: unverified license.
- **`peteedoo/dripdrip`**: REJECT — license is UNVERIFIED AND not a Rust crate (pnpm/Turborepo monorepo). One reason: unverified license + not Rust.
- **`juagamesacc-pixel/xrouter`**: REJECT — license is UNVERIFIED (no LICENSE file found). One reason: unverified license.
- **`ChrisRoyse/Astrolabe`**: REJECT — license is "Other" (not on allow-list). One reason: license not allow-listed.
- **`adlio/mixtape`**: REJECT — MIT license but 2★ and a generic Rust agent framework with no provider-agnostic LLM abstraction or MCP integration. One reason: no provider-agnostic LLM abstraction; too generic.
- **`Rust-Framework/rust-agent-framework`**: REJECT — MIT license but 0★ and a generic LLM-powered agent framework with no provider-agnostic routing or MCP tool discovery. One reason: no provider-agnostic routing; no MCP integration; no traction.

---

## Counterevidence summary

1. **Engine already has a mature provider-agnostic contract**: The engine already has `ProviderAdapter` trait, `ContractCore` shared guards, vendor adapters (OpenAI, Anthropic, Gemini, Ollama), transport, streaming, and policy. The donors add pattern references for the *routing abstraction layer*, not missing functionality (Cards 1, 2, 3).
2. **Most repos are small and unproven**: `llmkit-ai/llmkit` (121★) is the only repo with significant traction. `hibuka-labs/phi-agent` (15★), `clark-labs-inc/clark-agent` (12★), and `AMAI-LABS/soul-core` (0★) have minimal community validation. Pattern references are safe; dependency risks are not.
3. **AGPL-3.0 and UNVERIFIED licenses are blocked**: `armillae` (AGPL-3.0) cannot be used under the deny.toml allow-list. Several repos have no LICENSE file at all. Under the strict license-first policy, these cannot be taken.
4. **`llmkit` is the standout**: `llmkit-ai/llmkit` (121★, MIT, provider-agnostic, OpenAI API compatible) is the most directly relevant donor. Its prompt management, versioning, testing, and evaluation infrastructure maps to the engine's PV1/PV2 suite and provider abstraction needs.
5. **`phi-agent` provides the MCP bridge**: `hibuka-labs/phi-agent` (MIT, 15★, MCP support, extensible tools) directly addresses the engine's MCP command service gap — dynamic tool discovery from MCP sources.
6. **`clark-agent` provides the agent loop pattern**: `clark-labs-inc/clark-agent` (Apache-2.0, 12★, typed hookable agent loop, provider-agnostic) provides a validated pattern for the engine's AI-agnostic command service.

---

## What does NOT transfer (explicit)

- All AGPL-3.0 repos (`armillae`) — not on deny.toml allow-list
- All UNVERIFIED-license repos (`vincents-ai/llm`, `rynjon1993/wraith`, `yfedoseev/llmkit`, `peteedoo/dripdrip`, `juagamesacc-pixel/xrouter`) — license-first policy
- All "Other" license repos (`ChrisRoyse/Astrolabe`) — not on allow-list
- All generic agent frameworks without provider-agnostic LLM abstraction (`adlio/mixtape`, `Rust-Framework/rust-agent-framework`) — no provider-agnostic routing
- All non-Rust implementations (`peteedoo/dripdrip` — pnpm/Turborepo) — engine requires Rust implementation
- All UI/web components (`llmkit`'s TypeScript/UI toolkit, `clark-agent`'s clarkchat.com) — engine is headless
- All prompt management servers (`llmkit`'s inference server) — engine needs the abstraction layer, not the server
- Any paper claim that requires live model access or external data in the tick
- Any transport dependency that is not std-only or on the allow-list
- Any adapter that is not provider-agnostic (engine already has per-vendor adapters)
- Any MCP server that stalls the fixed tick

---

## Integration homes

- **Subsystem plan**: `docs/plan/ai-native.md` (PLN9, prefix AX)
- **RESEARCH-VERDICT**: `docs/RESEARCH-VERDICT.md` (deep dive 07 section)
- **EVIDENCE**: `docs/EVIDENCE.md`
- **DEEP-DIVES**: `docs/DEEP-DIVES.md` (dive 07 row update)
- **SPRINT**: `docs/SPRINT.md` (board rows AX1, MCP8, EV2)
- **HASH-LAW**: `docs/HASH-LAW.md`
- **Focus**: `docs/FOCUS.md`
- **AI-native plan**: `docs/plan/ai-native.md` §5 (Donors), §7 (Board tasks)

---

## Budget and next step

Budget: M 20 min (this packet). At budget stop, report cards and next step.

Next step: If cards 1–3 are adopted, write `docs/research/steal-wave40.md` as the persisted note (this file). Propose AX1, MCP8, EV2 as board rows via the orchestrator. Cards 1–3 are TAKE with REPRODUCE stamp. Rejections carry one reason each.

---

## EMPIRE outcome per card

| Card | EMPIRE outcome |
|------|----------------|
| AX1 (`llmkit-ai/llmkit`, REPRODUCE) | Enables the engine's AI-agnostic core (RESEARCH-VERDICT.md: "Engine-owned provider adapters with capability negotiation and a conformance suite — IN — M2") by providing the unified routing abstraction. Supports "AI makes whole games" (EMPIRE.md §4) — a brief goes in, a finished game comes out, and the AI layer is provider-agnostic by design. |
| MCP8 (`hibuka-labs/phi-agent`, REPRODUCE) | Strengthens the MCP command service (RESEARCH-VERDICT.md: "Progressive tool discovery: small default set, then search → describe → call — IN — N8") by adding dynamic MCP-based tool discovery. Supports "AI makes whole games" (EMPIRE.md §4) — the agent can discover and use tools dynamically without hardcoded vendor dependencies. |
| EV2 (`clark-labs-inc/clark-agent`, REPRODUCE) | Validates the agent loop pattern for the AI-agnostic core (RESEARCH-VERDICT.md: "Model proposes, deterministic code disposes; return the minimal conflict set on failure — IN — M2") by providing a typed, hookable pattern. Supports "AI makes whole games" (EMPIRE.md §4) — the agent loop is provider-agnostic by design, enabling any AI to drive the game. |
| `mmstudio-games/armillae` (REJECT) | License (AGPL-3.0) blocks adoption — maintains license-first discipline per `deny.toml`. No EMPIRE impact. |
| `vincents-ai/llm` (REJECT) | License (UNVERIFIED) blocks adoption — maintains license-first discipline per `deny.toml`. No EMPIRE impact. |

---

## Source traceability

- **GitHub search**: `gh search repos "provider-agnostic AI Rust"`, `gh search repos "LLM client Rust"`, `gh search repos "MCP server Rust"`, `gh search repos "AI agent framework Rust"`, `gh search repos "function calling Rust"`, `gh search repos "multi-provider LLM Rust"`, `gh search repos "AI adapter Rust"`, `gh search repos "tool use Rust"`, `gh search repos "LLM abstraction Rust"`, `gh search repos "Rust agent framework" --sort stars`
- **GitHub API**: All repos verified via `gh repo view <owner>/<repo> --json licenseInfo`
- **webfetch LICENSE**: `llmkit-ai/llmkit` (master/LICENSE), `hibuka-labs/phi-agent` (main/LICENSE), `clark-labs-inc/clark-agent` (main/LICENSE), `AMAI-LABS/soul-core` (main/LICENSE), `bkrabach/unified-llm-client-rust` (main/LICENSE), `AimesSoft/nipa-agent` (main/LICENSE), `dongsxyz/rust-ai` (master/LICENSE), `unfaded/naori-ai` (main/LICENSE), `guuzaa/kongfu` (master/LICENSE)
- **Cross-reference**: DEEP-DIVES.md §07 (dive 07 row), `docs/plan/ai-native.md` (PLN9), `docs/RESEARCH-VERDICT.md` (deep dive 07 section, Hunt 2026-09-24, Wave 23-26 verdicts)
- **EMPIRE.md**: §1 (we ship our own games), §4 (AI makes whole games), Race section (forge + engine2040 both live forever)
- **Donor dedup**: Wave 26 donors excluded; Wave 27 donors excluded; Wave 28 donors excluded; Wave 29 donors excluded; Wave 30 donors excluded; Wave 31 donors excluded; Wave 35 donors excluded; Wave 36 donors excluded; Wave 37 donors (`madsim-rs/madsim`, `rust-lang/rustc-hash`, `copper-project/copper-rs`, `turbofish-org/orga`, `eldruin/wyhash-rs`) excluded; Wave 38 donors excluded; Wave 39 donors (`benny-n/netcode`, `lowquark/uflow`, arXiv:2604.15044v1, `alexdesander/hexgate`, `iQua/days`, `canndrew/netsim`, `bacongobbler/bevy_simple_networking`, `gschup/bevy_ggrs`, `3-Hz/pfengine`, `MattKulka/revenant`, `genxium/DelayNoMore`) excluded; Plan-file donors (`mas-bandwidth/netcode`, `cBournhonesque/lightyear`, `HouraiTeahouse/backroll-rs`, `gschup/ggrs`, `lucaspoffo/renet`, `quinn-rs/quinn`, `rustls/rustls`, `ValveSoftware/GameNetworkingSockets`, `johanhelsing/matchbox`, `simgine/bevy_replicon`, `Xiph Opus`) excluded.
- **License allow-list**: `deny.toml` [licenses].allow = [MIT, Apache-2.0, Zlib, BSD-3-Clause, BSD-2-Clause, ISC, Unicode-DFS-2016, Unicode-3.0, CC0-1.0, Unlicense, 0BSD, MIT-0]
