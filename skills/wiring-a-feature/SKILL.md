---
name: wiring-a-feature
description: "Use when finished code exists but no binary calls it (a pub type nobody uses, a row that says wire or ends in w), or before you write something new that may already exist: find it, add the one call site, prove the call happens."
---

# Wiring a feature that already exists

Owner question 2026-10-17: much of the plan is code that is written, tested and never called. Wiring is cheaper than writing. The long analysis is `docs/plan/wiring-triage-2026-10-17.md` (features F1 to F10 with the missing call named, section 4 has paste-ready rows).

## 1. Is it really unwired

1. Grep the type or fn name over `crates` (Grep tool, or `rg -n <Name> crates`). Hits only in its own file and its tests mean unwired. A hit in a comment or a `#[cfg(test)]` block is not a caller.
2. `node tools/wiring-scan.mjs` lists `pub` items with no production user (crate, file, line, kind, name). Report goes to `target/reports/wiring-scan.json`. Treat it as a hint only:
   - it counts a name in a comment as used (misses `BreakLook` before RZ4w);
   - it cuts a file at its first `#[cfg(test)]`, so big files are half-read;
   - 37 percent of its items are `pub const`s.
3. `node tools/wiring-scan.mjs --check` fails when a crate's island count rises over `tools/wiring-scan.baseline.json`. It was already red on 2026-10-17 (5554 vs 5317), so a FAIL is yours only if your crate's number rose.
3b. Before you commit new `pub` code run `node tools/wiring-scan.mjs crates --diff`: every new `pub fn`, `struct` or `enum` needs a production caller in another file. If the consumer is a later board row, pass `--unlocks=<ROW>` (the row must be open). A FAIL on items you did not add is another session's.
4. Check the board before you start: `node tools/board-slice.mjs --find <Name>`. A ticked row or an existing wire row (`RZ4w`, `EG14w` style) means read it first.
5. Deleted on purpose? Then it is not unwired, it is cut: skill `recover-deleted-code`. Section 5 of the triage lists what must stay deleted.

## 2. Find the live roots

Only code reachable from a binary counts. The roots: `arena` (window), `arena_capture`, `staged_fight`, `e2040-mcp` (door), `e2040-editor`, `net-bout`, `net-relay`, `bundle_cook`, `e2bundle`, `distill_duel`.
The window path, where most wiring lands (`crates/game/src/bin/arena.rs` and `crates/game/src/bin/arena_frame/mod.rs`):

| when | function | use it for |
|---|---|---|
| once at start | `ArenaApp::init_presenter` | upload meshes, build look objects |
| each accepted sim tick | `run_sim_ticks` in `arena_frame/mod.rs` | feed the object the tick's `TickOutcome` |
| each presented frame | `ArenaApp::push_fighters`, `ArenaApp::present_frame` | append draws, stage overlays |
| exit | `ArenaApp::finish_exit` | print the receipt line |

`TickOutcome` (in `crates/game/src/lib.rs`) is what a tick hands out. If the data you need is dropped inside `App::simulate_tick`, add one field to `TickOutcome` and fill it there.

## 3. Add the call site (smallest change that works)

- One call per hook above. If the object needs data the owner crate keeps private, add one additive `pub fn` accessor there. No state change.
- Presentation code never feeds the sim hash. If you must touch the sim, stop: that is a hash row (skill `determinism-debug`).
- Give the picture an off switch (`--no-break-look` is the model) and make a failure drop the picture and print one `notice` line, not panic.
- Several rows write `crates/game/src/fighter_look.rs`. Run `git status --short` first and serialize with any other writer (skill `commit-and-board`).
- A new module still needs its `mod` line in `lib.rs` and, for tests, its module in the integration-test main file (tests, module `it`).

Worked example: RZ4w. `BreakLook` existed since `57d7a472`. `crates/game/src/break_window.rs` is the hook: `BreakScene::observe_outcome` once per accepted tick, `BreakScene::push` once per frame. Commit `30d3e1a8`.

## 4. Prove it is wired

A wire is proven by three things, not by a green unit test of the old code:

1. The grep from step 1 now hits a binary (`crates/game/src/bin/arena.rs`), not just the module.
2. A test that goes through the production entry: build the `TickOutcome`, call the hook the way the window calls it, check the draw list or the state. Name it so the filter finds it (`break_wire_*`). Run it with `node tools/gh-check.mjs cargo test -p engine-game --features app --lib break_wire` (skill `github-check`; `app` runs on the PC).
3. The hash is equal with the picture on and off, and the golden is unchanged: `golden unchanged` or `d57b5c8ae3096614 x2`.

Then run the window once and read the receipt line (skill `arena-capture-and-view-line`): `arena break: breaks=.. debris=..` must be non-zero after a break. A feature that only shows a zero is still not wired.

## 5. Traps

- A test that builds the object by hand proves the object, not the wire.
- A `pub fn` that only a `#[cfg(test)]` block calls is still an island.
- Do not wire the dead list (triage section 5): the old solver, lagcomp, and similar. They stay deleted.
- Tick the row only with the grep and the receipt line as evidence (skill `tick-a-row`).
