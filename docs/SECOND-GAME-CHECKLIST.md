# Second-game checklist (GP5)

> SUPERSEDED 2026-10-09: historical GP5 note, kept for record only.
> Current game flow lives in `docs/FOCUS.md`. Fixture proof receipts live in
> `docs/archive/2026-10-03/GE1-FIXTURES.md`. Do not follow the steps below as current.

Fixture rules crate boots, plays headless and replays with zero engine edits.

- Needs: RC1 + GM1 (both ticked).
- Tier B, Size S. Gates GE8 (fixture camera + mode tables load without edits).

## What landed

- `fixtures/games/second-game/rules.json` — fixture rule set v2 (tag mode,
  seed 77, 120 ticks, 4 runners, goal disc, follow-camera table, 3 moves).
- `tests/second_game.rs` — fixture harness over public engine APIs only
  (`engine-core`, `engine-ecs`, `engine-physics`): boot, headless play,
  record, replay with first-divergence naming, rules-as-data tables.
- This checklist doc.

## How to verify

- `cargo test --locked --test second_game` — 8 fixture tests green.
- `cargo test --locked --test proof_cli` — proof CLI untouched and green.
- `node tools/proof.mjs --skip-build` — golden `d57b5c8ae3096614` x2,
  tape replay, corruption names its tick, unsupported backend refused.
- `git diff -- crates/` — empty (no engine edits).
- `node tools/workspace-check.mjs` — audit green (edges, modules, ledger).

## Acceptance

- Fixture boots headless, plays 120 ticks, replays exactly x2.
- Different goal disc gives different receipts (rules as data, not forks).
- Camera tables share every checkpoint but frame differently (data, not code).
- Neighboring seeds never alias (SplitMix64 expansion, fixture v2).
- Engine diff empty; golden unchanged; proof untouched.

## Budget

- Headless tag at 4 runners: well under 1 ms per full 120-tick match in the
  debug test profile; per-tick work is O(runners + moves-for-tick).

## For GE8

- Camera params and mode rule set already load as data tables; the director
  and mode framework can consume this fixture without engine edits.
