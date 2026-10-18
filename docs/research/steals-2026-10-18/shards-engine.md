STEAL-SCOUT block 9 card, read 2026-10-18. Donor: larelgit/shards-engine (Rust, deterministic engine "coding agents can play, test and debug from the terminal", Bevy based).
Donor sha 5ba4efc6babf0ec339a0e43d966e84c3a7c8c276 (main, 2026-10-18). Read: file bodies fetched live today (tools.rs, invariants.rs, observe.rs, determinism.rs, tool_contracts.rs, tree, Cargo.toml). No build, no run. No bytes vendored: ideas only.
Gates: golden d57b5c8ae3096614 / 2aca9338670aa0cb are P2P on every row below; nothing here touches a tick or a hash.
Related: docs/research/steals-2026-10-18/05-ai-native.md (other AI-native engines), 00-combined.md (how a steal lands).

## What it is

A young Bevy game engine built the other way round: the *agent door* is the primary interface, and the game is written to serve it. It splits cleanly into `crates/ai_core` (typed tools, invariants, perception, replays, determinism) and the game crates. It is the closest thing to engine2040's `crates/endpoint` in this block: same shape of problem, different choices.

## Licence

`Cargo.toml` workspace line (read live): `license = "MIT OR Apache-2.0"`. Tree carries `LICENSE-APACHE` and `LICENSE-MIT`. GitHub's licence API reports SPDX `Apache-2.0` (it takes the first file). Treat as dual: `Apache-2.0 OR MIT`.

## What we have (checked, not guessed)

- Door verbs have capability cards and a read-only flag: `crates/endpoint/src/packs/mod.rs:202-210` (`VerbSpec { name, pack, card, readonly, .. }`).
- Every mutation already returns proof: `crates/endpoint/src/receipts.rs:15-27` (`Receipt { ok, effect, hash, undo_to, code, retryable, recovery[2] }`).
- A witnessed commit gate: `crates/endpoint/src/witness.rs:1-22` (observe digest, bind intent, revalidate, retain evidence).
- Streaming audit seal: `crates/endpoint/src/audit/verify.rs:15-25`.
- The `rules` pack is *scripted arena controllers*, not an invariant checker: `crates/endpoint/src/packs/rules.rs:1-8`.

## Steals (ranked by value per hour)

1. **A per-call "what changed" list, built from one digest pair** | shards-engine@5ba4efc6babf:crates/ai_core/src/tools.rs:439 (`let before = digest(world)`), :451 (`let after = digest(world)`), :453 (`clip_changes(diff(&before, &after, origin))`), :296 `MAX_CHANGES = 60`, :330 `MAX_JOURNALED_ARGS = 200`, :343 `call_tool` (the single entry point for BRP, dev keys, tests and replays), :276 `CallOutcome` (result, tick, paused, changes, journal, halted) | Apache-2.0 OR MIT | our file: `crates/endpoint/src/receipts.rs` |
   Our `Receipt` carries a hash and an undo revision but nothing an agent can read: the agent cannot see *which fields moved*. Shards takes a cheap world digest before and after the call and returns a clipped, ordered change list plus the journal lines the call caused. Map: add a bounded `changes: Vec<String>` to `Receipt`, filled from a document digest pair the door already computes for the witness gate. Moves: a mutating verb answers "3 actors moved, 1 clip changed" instead of a bare hash. Kill: if the change list pushes a tool reply over the door's per-call byte budget, keep the hash and return the list only on request.

2. **Tool kind decides journal and replay, in three classes not two** | shards-engine@5ba4efc6babf:crates/ai_core/src/tools.rs:56-64 (`ToolKind::{Query, Action, Control}`; Control = "changes *when*, not what": journaled with a diff like an action but never recorded in replays), :419-427 (query path returns early, no journal), :429-446 (action wraps `replay::begin_action`/`end_action`), :401 (a client runs no rules) | Apache-2.0 OR MIT | our file: `crates/endpoint/src/packs/mod.rs`, `crates/endpoint/src/packs/replay.rs` |
   Our verbs carry `readonly: bool`. The third class is the useful one: `play.step`, pause and possession must be journaled for the audit but must *not* land in the tape, or a replay re-runs a step that time already ran. Kill: if adding the class needs a tape format or WIRE bump, ship the doc table and the journal flag only.

3. **A schema-driven tool contract test** | shards-engine@5ba4efc6babf:crates/sandbox/tests/sandbox/tool_contracts.rs:1-21 (every registered tool is called with 30 argument sets made from its own schema: 20 that satisfy it, 10 that break exactly one field; a call must not panic, an error must say something and must name how to fix the call; at the end the session replay re-simulates from tick 0 and must match) | Apache-2.0 OR MIT | our file: `crates/endpoint/tests/it/` (`tool_count_pin.rs`, `tool_budget_exhaustion.rs`, `jsonrpc_conformance_vectors.rs`, `redteam.rs`) |
   We pin the tool count, the schema budget and the refusal shape. We do not drive every verb from its own advertisement. This test would catch a verb whose schema lies about its own arguments, which is exactly the failure a weak model hits. Kill: if the schema generator is too big to port, start with one hand-written valid and one hand-written broken call per pack.

4. **Invariants as a first-class rule set, with the violation as the definition** | shards-engine@5ba4efc6babf:crates/ai_core/src/invariants.rs:1-31 (rules run after every tick; engine rules named `<module>.<rule>`; game rules are `fn(&World) -> Option<String>`; a violation journals an error, pauses the game and saves `replays/invariant-<name>-tick<N>.json`; `--verify` lists every violation; `assert_reset` re-arms), :56 `type Invariant`, :138 `add_engine_rule`, :451 `take_unreported`, :465 `AssertOp`, :472 `apply` | Apache-2.0 OR MIT | our file: new harness beside `crates/proof` (owner call; owner is the door today, `crates/endpoint/src/packs/rules.rs:1-8`) |
   We have no always-on rule checker. We have goldens and per-test asserts. The steal is the *shape*: a rule is named by its violation, it fires once, it pauses and leaves a replay plus an explanation. That gives the duel loop a "why did this state happen" answer without a debugger. Kill: if a checker cannot run from a copy taken after the tick, keep it test-only and never in the tick path.

Not worth it: her typed `schemars`-driven schema emission (`tools.rs:475-493`) — we already advertise per-verb schemas and pay a char budget for them; the useful part is the test in rank 3, not a second schema printer. Their perception text (`observe.rs:1-26`) is close to our scene query + spatial crate; the "camera plays no part" rule is already our law.
