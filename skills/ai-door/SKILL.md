---
name: ai-door
description: "Use when you add or change an MCP verb or pack (crates/endpoint), a provider or model adapter, an eval or win-rate number, or an offline search or int8 net. Verbs record actions and stay idempotent."
---

# AI door

The model runs outside the tick. Only recorded actions go in. Replay reads the record, never the model.

Related: `engine-code` (area rule for AI), `net-rollback`, `door-and-headless-runbook` (run lines for `e2040-mcp`), `test-and-proof`.

## 1. Door rules

1. Tick-stamp each model action and record it before it enters the tick. Replay reads the record.
2. Every door verb takes an idempotency key, can be undone, and states its cost before it runs.
3. Three guard layers: input, action, output. A model line passes the output rail before a player sees it.
4. No god-agent. Use fixed roles. Add a pattern only with a test and a metric.
5. Do not trust a win rate from a few games. 53 wins in 100 games proves nothing (p = 0.62). Play about 1000 games, swap sides, run a binomial test.
6. Search and training run offline. When a net binds the tick, it is a small int8 net distilled from that search. (`crates/nn/src/lib.rs` header today: offline and tooling consumers only, no arena or game tick binds a net yet.)
7. Quantize every float input to i8 the same way everywhere, before `infer`. Never put a float in hashed state. `engine-nn` is integer-only: `i8` weights, saturating `i32` accumulators, no SIMD, no wall clock. `load` and `infer` never panic.

Repo facts:

- `crates/provider` is used by `crates/endpoint` and `crates/playtest`. `crates/arena` has it only for the mock adapter in a stamped-reply test. The game window does not call it. A library with no caller is not done.
- `crates/provider/src/pins.rs`: `RequestPins` pins a request: `PINNED_SEED` (2040), `with_system`, `with_temperature_milli`, `with_tools`. Adapters (`openai.rs`, `anthropic.rs`, `gemini.rs`, `ollama.rs`) take the pins.
- `crates/provider/src/cassette.rs`: `CassetteKey` and `RequestImage` key a request. The cassette replays the stored answer, so a run replays without the model.

## 2. Evals

No claim lands without a frozen baseline, paired runs and a green regression suite.

1. **Frozen baseline pin.** Pin model id, prompt text, cassette version and seed 2040 before the first run. Rerun against the same pin, never a moving "latest". Flag a prompt edited mid-run.
2. **Paired runs.** Baseline and change on the same map, seeds, tick count and build. Equal N on both sides.
3. **Regression suite green.** Run the full prompt suite before the claim. A new AI behavior adds a case first and shows FAIL then PASS. Never delete a case to pass.
4. **No win rate without proof.** Report wins, N, sides swapped and binomial p. About 1000 games for a close call. Flag a rate with no N, no p, or one-sided maps.
5. **Offline replay.** Replay every eval from cassettes without the live model. The same inputs give the same outputs and verdict twice. A number that needs the live model to reproduce is not a result.
6. **Triage the delta.** List changed cases by name. Keep unrelated greens untouched. File one bug row per real regression with seed and repro. A mean with no per-case table is not a finding.

Paste both hashes from the paired seed 2040 run. Harness code: `crates/playtest/src/eval.rs` (authoring and play tasks across providers), `crates/playtest/src/tournament.rs`.

## 3. Providers

Take the adapter and test files in the row. Inspect the existing contracts first. Test every available implementation against: capabilities, structured actions, streaming, timeouts, cancellation, malformed output, unavailable service, resource limits. Add recorded offline replay.

- The conformance matrix is `crates/provider/tests/matrix.rs`: every backend (mock, local, OpenAI-compatible, Anthropic, Gemini, Ollama, external MCP client, offline) against nine columns (completion, bad output rejected, latency p50/p95, cost where unknown is never zero, streaming, cancel under 100 ms, timeout, offline replay twice, red-team tapes). The contract suite is `crates/provider/src/suite.rs`.
- No provider-specific fields in portable contracts.
- No network calls inside the fixed tick. Streams poll without dialing; cancel and timeout never touch the tick.
- Separate three kinds of evidence: fixture and contract tests, adapter integration, real provider execution.
- Two mocks do not prove two providers. Absent credentials or a missing second implementation are explicit gaps, never faked as live.
- A live call needs owner authorization in chat and a call and cost budget. Never read or print secrets. Never change pins.
- No secrets in the tree (the tree goes to a public mirror).
- Report: patch, conformance matrix, exact checks, failing contract, unknowns.

## 4. MCP verbs (crates/endpoint)

Add one verb to a pack in `crates/endpoint/src/packs`, then prove it with the registry tests. Danger: registration checks shape only. A write that skips the tx gate, or a handler that skips dispatch, still registers.

1. A verb is one typed tool plus one `pack.add` line in its pack's `pack()`. Models: `packs/geo.rs::pack`, `tool_macro.rs` (`e2_tool!`), `packs/mod.rs::PackDef::add`.
2. The name is `pack.verb`, equal to the tool's own name. It may not contain `exec`, `shell`, `file`, `eval` or `terminal` (substring match, `FORBIDDEN_NAME_WORDS`). Clients may show `e2040_api_search` for the door name `api.search`. See `packs/mod.rs::VerbSpec::of`.
3. A verb needs a read-only annotation, an input schema and a capability card other than Deny. Test: `guard_refuses_verbs_without_annotation_or_capability`.
4. Read verbs use the Observe card. A Commit verb follows `packs/geo.rs::geo_apply`: dry run first, then tx preview and apply inside one dispatch. Cards: `tool_capabilities.rs::card_for_tool`.
5. Call `dispatch(TOOL_NAME, ...)` in the handler (`door_dispatch.rs`). It runs the scope policy and seals one audit record. A handler that skips it seals nothing (`geo_preview` is read-only and does not call it).
6. Errors are `McpToolError` with a static code. The message names the legal values and one recovery word such as `re-run` or `granted` (`tool_capabilities.rs::message_carries_recovery`). A poisoned state lock fails closed as `lock_poisoned`.
7. A verb in a default-loaded pack counts toward the advertised tool list. The list must keep `TOOL_LIST_MIN_HEADROOM_CHARS` of headroom under `TOOL_LIST_BUDGET_CHARS` (both in `crates/endpoint/src/mcp.rs`; `check_tool_budget`). Read the constants, do not trust a number written elsewhere. Count pin: check the pin test `crates/endpoint/tests/it/tool_count_pin.rs` (`EXPECTED_ROUTER` per feature set) and update it when you add a verb.

Check (all four must pass):

- `cargo test --locked -p engine-endpoint --features mcp --lib shipped_registry_is_clean_and_covers_every_pack`
- `cargo test --locked -p engine-endpoint --features mcp --lib every_refusal_carries_a_recovery_for_weak_models`
- `cargo test --locked -p engine-endpoint --features mcp --lib seeded_bloat_of_500_chars_fails_the_gate`
- `cargo test --locked -p engine-endpoint --features mcp --test it mcp_stdio::` (a scripted client lists, previews, applies and undoes over stdio)

Also run `--test it tool_count_pin` and `tool_budget_exhaustion`. Send these through `node tools/gh-check.mjs`.

Don't:

- Use a Deny card or a forbidden word to get past the guard. Fix the verb.
- Keep a hand list of verbs in docs. `packs/mod.rs::PackRegistry::seed_index` indexes them. `docs/book/ai-door.md` is stale (it says twenty tools and a 32_768 budget; board row E71).
- Raise `TOOL_LIST_BUDGET_CHARS` to pass the gate without the dated reason comment the gate message asks for.

Sources: dushkin 11.1-11.3, 19.1, 20.1, 22.5, 23.2; gogame 6.2, 10.3.1-10.3.2, 14.2. Search: `node tools/books.mjs search "agent guardrails idempotency"`.
