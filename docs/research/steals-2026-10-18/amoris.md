STEAL-AMORIS scout, 2026-10-18. Wave 2026-10-18/1, block 1: github.com/topics/game-engine (l=rust, o=desc, s=updated), position 41. Area: sectioned world hash, replay divergence record, validated/budgeted agent actions, process-level determinism check.
Read only, ideas only; no code copied, nothing vendored. Every donor file below was fetched live on 2026-10-18.

== 1. WHAT IT IS ==
Amoris (qiulinfan, 5 stars, updated 2026-10-17) is an "agent-native" 3D engine: a Rust host with TypeScript gameplay, a wgpu renderer running natively (Metal/Vulkan/D3D12) and on WebGPU in the browser, and a desktop **and** web editor whose edits, CLI, HTTP and MCP calls all enter one undoable history. It advertises Play-in-a-fork with pause/step, TypeScript debugging, snapshot/replay with deterministic-run checks, and CPU+GPU Perfetto traces. It has separated the engine into `amoris-sim`, `amoris-persist` (a versioned sectioned snapshot + replay format), `amoris-interface` (a declared action/perception contract for agents) and `amoris-check` (the determinism/limits test battery). Our interest is exactly that last pair: the closest donor we have seen to engine2040's hash law, replay law and MCP door.

== 2. LICENCE ==
MIT (SPDX `MIT` via GitHub API 2026-10-18; LICENSE read live 2026-10-18: https://raw.githubusercontent.com/qiulinfan/amoris/main/LICENSE — "MIT License, Copyright (c) 2026 Qiulin Fan"). Ideas only; nothing vendored.

== 3. STEALS (ranked) ==
1. A **sectioned** world hash: a digest per named section, then one hash over the ordered digests | qiulinfan/amoris@main:crates/amoris-persist/src/hash.rs:1-35 | SIM (verification only) | engine2040 golden `d57b5c8ae3096614` and the determinism-debug "bisect to the first bad tick" | M
   Idea: each section is hashed as XXH3-128 over its **kind, name, version and bytes** (:1-3), and the world hash is then XXH3-128 over the format, the section count and the section digests in canonical order (:1-3,26-35); the two are seeded separately (`SECTION_SEED` "P3DSEC"+1, `WORLD_SEED` "P3DWOR"+1, :12-15), and the snapshot format number is *inside* the world hash so a layout change cannot silently match (:16-17). `WorldHash` is the raw 16 little-endian bytes printed as 32 hex digits (:26-35).
   Maps: engine2040 has one opaque 16-hex golden; a section digest list turns "the hash moved" into "the combat section moved at tick 431", which is the first half of determinism-debug. The section-version-inside-the-digest rule is the part that keeps a format change honest.
   Kill: if splitting the hash changes the pinned golden value or the section order is not canonical/total -> keep the single golden as the gate and add sections beside it, never instead of it.

2. A divergence record rich enough to explain the failure, with field dumps bounded | qiulinfan/amoris@main:crates/amoris-persist/src/replay/divergence.rs:13-45 | SIM (verification only) | engine2040 determinism-debug MATCH/DIVERGE/SKIP report fields | S-M
   Idea: `Divergence` (:31-45) carries the tick reference, a `kind` that distinguishes more than "different" — `Hash`, `Write { index, refused_by }` (one side refused a write the other applied), `EndedEarly { side }`, `Stopped { error }` (:13-29) — then `expected`/`actual` hashes, the differing `sections` (empty when unknown), the differing `fields` with a `fields_truncated` flag, the writes applied at the boundary `tick - 1`, and an optional `reconverged_at` telling you the run came back on its own (:37-44).
   Maps: engine2040's report is a verdict; this makes the diverging *tick*, the *side*, the *section*, a bounded field diff and the boundary *writes* one serialisable object — the actionable half of the MATCH/DIVERGE pack.
   Kill: if the diff stores unbounded fields (must respect a `DEFAULT_LIMIT` as here) or the record allocates every tick -> keep it to the first divergence only.

3. Agent actions validated **whole** before anything is applied, and answers cut to a token budget | qiulinfan/amoris@main:crates/amoris-interface/src/action/validate.rs:1-50; action/budget.rs:1-45 | PRESENTATION/TOOLING (the door) | engine2040 `endpoint` MCP verbs / ai-door `ReplayVerb` (verbs recorded, idempotent) | M
   Idea: validation runs in phases — shape, then names, then semantics — stops at the first phase that fails and reports **every** problem that phase found (:1-5); what comes out is a *plan* where every reference is resolved and every parameter is canonical and accepted, so nothing half-applied can exist (:4-5,22-42). Separately, `budget::fit` (:20-45) keeps the longest prefix of an answer list whose compact JSON plus an `omitted` count plus 16 reserved bytes fits `budget_tokens * 4`, reports the `tokens` estimate, and refuses a budget that cannot even hold an empty list with `perception.budget_too_small` (:1-5,44-50).
   Maps: engine2040's door verbs must be recorded, validated and idempotent; "validate the whole call, then apply a resolved plan" is the atomicity rule, and a per-answer token budget with an explicit `omitted` count is how a tool answer stays bounded and honest about what it dropped.
   Kill: any verb applied before its whole call validates, or a truncated answer that does not say how many items it dropped -> refuse the call instead.

4. Determinism checked in-process (two worlds in lockstep) and across processes (a child re-run) | qiulinfan/amoris@main:crates/amoris-check/src/determinism.rs:42-55 | SIM (verification only) | engine2040 "reproduce twice", golden run, cross-process check | S
   Idea: the same-process variant builds two games of the same seed and steps them alternately through `lockstep`, feeding each the recorded inputs, which catches state shared through process globals (:42-55); the cross-process variant runs the same check again as a **child process**, which additionally catches hash-map seeds, addresses and thread identities (:1-5). The cross-build/cross-target variants are left to the build tool (:5).
   Maps: engine2040's hash law already wants identical output from identical input; the two-tier split is cheap and names *what kind* of determinism you proved (in-process vs across-process), which is what a determinism-debug report should say.
   Kill: if the cross-process re-run doubles the check time and adds no named finding -> keep in-process in the loop and make the child run an opt-in gate.

== 4. NOT WORTH STEALING ==
- The TypeScript script host, Monaco editor and web editor (`amoris-web`, `amoris-debug` CDP): a whole scripting/editor stack engine2040's egui shell and fixed tick do not want.
- The MCP/HTTP surface itself: engine2040 already has its own door; only the validation/atomicity/budget rules above transfer.
- The PCE binary section format and its migrations (`crates/amoris-persist/src/pce/*`, `migrate.rs`): a full versioned codec; engine2040's scene codec already writes sorted, diff-stable JSON (`scene/src/codec.rs:10-12`), so take the section *hash* idea only.

== 5. FILES READ ==
README.md, LICENSE, crates/amoris-persist/src/hash.rs, replay/divergence.rs, replay/verify.rs, crates/amoris-check/src/determinism.rs, crates/amoris-interface/src/action/catalog.rs, action/budget.rs, action/validate.rs (all @main, 2026-10-18).
