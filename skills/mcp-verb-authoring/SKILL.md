---
name: mcp-verb-authoring
description: "Use when adding or changing an MCP verb in crates/endpoint (the e2040 door): name, capability card, refusal codes, dry run, audit, budget and tests."
---

# MCP verb authoring

Add one verb to a pack in crates/endpoint/src/packs, then prove it with the registry tests.
Danger: registration checks shape only. A write that skips the tx gate, or a handler that skips dispatch, still registers.

## Rules
1. A verb is one typed tool plus one `pack.add` line in its pack's `pack()`. Do: `crates/endpoint/src/packs/geo.rs::pack`, `crates/endpoint/src/tool_macro.rs::e2_tool`, `crates/endpoint/src/packs/mod.rs::PackDef::add`.
2. The name is `pack.verb`, equal to the tool's own name. It may not contain exec, shell, file, eval or terminal (substring match). Clients may show `e2040_api_search` for the door name `api.search`. Do: `crates/endpoint/src/packs/mod.rs::VerbSpec::of`.
3. A verb needs a read-only annotation, an input schema and a card other than Deny. Do: `crates/endpoint/src/packs/mod.rs::VerbSpec::of`, test `guard_refuses_verbs_without_annotation_or_capability`.
4. Read verbs use the Observe card. A Commit verb follows `geo_apply`: a dry run first, then tx preview and apply inside one dispatch. Do: `crates/endpoint/src/packs/geo.rs::geo_apply`, `crates/endpoint/src/tool_capabilities.rs::card_for_tool`.
5. Call `dispatch(TOOL_NAME, ...)` in the handler: it runs the scope policy and seals one audit record. `geo_preview` does not call it, so it seals no record. Do: `crates/endpoint/src/door_dispatch.rs::McpServer::dispatch`.
6. Errors are `McpToolError`, with a static code. The message names the legal values and one recovery word, such as `re-run` or `granted`. Do: `crates/endpoint/src/mcp.rs::McpToolError`, `crates/endpoint/src/tool_capabilities.rs::message_carries_recovery`.
7. A verb in a default-loaded pack counts toward the advertised list. That list must keep at least 2,048 chars of headroom under 43,514. Do: `crates/endpoint/src/mcp.rs::check_tool_budget`, test `headroom_below_2k_forces_raise_or_refusal`.

## Check
- `cargo test --locked -p engine-endpoint --features mcp --lib shipped_registry_is_clean_and_covers_every_pack`: `test result: ok`, 0 failed.
- `cargo test --locked -p engine-endpoint --features mcp --lib every_refusal_carries_a_recovery_for_weak_models`: same pass line.
- `cargo test --locked -p engine-endpoint --features mcp --lib seeded_bloat_of_500_chars_fails_the_gate`: same pass line.
- `cargo test --locked -p engine-endpoint --features mcp --test it mcp_stdio::`: a scripted client lists, previews, applies and undoes over stdio.

## Don't
- Don't use a Deny card or a forbidden word to get past the guard. Fix the verb.
- Don't keep a hand list of verbs in docs. `crates/endpoint/src/packs/mod.rs::PackRegistry::seed_index` indexes them, and `docs/book/ai-door.md` is stale (it says twenty tools and 32_768 chars).
- Don't raise `TOOL_LIST_BUDGET_CHARS` to pass the gate without the dated reason comment the gate message asks for.
