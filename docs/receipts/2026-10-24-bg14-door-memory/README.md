# BG14: the door reaches the lesson store and the task suite (2026-10-24)

Board row: `docs/SPRINT-35-book-gaps-4.md` BG14 (book gap, EG49 stopped at the
tool surface). `tools/memory/memory.mjs` and `tools/tasks/run.mjs` already held
the data and the judges, but an agent at the MCP door could see neither: no
`e2040://memory` resource, no `memory.record`, no `tasks.list`, and
`run.mjs --solve` did not exist.

## What landed

| surface | what it answers | cost on the tool list |
|---|---|---|
| `e2040://memory` (+ `?kind=`, `?path=`, `?hash=`) | every lesson row the build left, in UTC-stamp order, each with its derived id | 0 chars (a resource, not a verb) |
| `memory.record` | append one lesson row; the 16-hex tape hash is required | 1 verb |
| `tasks.list` (the new `tasks` pack) | the ten build-task jobs, their titles and whether a referee sits beside them | 1 verb |
| `run.mjs --solve --arm scripted\|mutation` | a real pass@1 over the suite, judged end to end | node only |

`crates/endpoint/src/memory_store.rs` is the new lane: one row
(`kind, hash, path, utc, text, evidence?, id`), the same rules
`memory.mjs` applies, and the same id — the head of SHA-256 over the
`\0`-joined canonical fields — so one row has one name in both readers. The door
adds no second store: the resource and the verb read and append
`tools/memory/memory.jsonl` itself. The digest reuses the owner the crate
already calls (`engine_game::camera_feed::showcase_sha256` through
`crate::play::hex_bytes`), so no new dependency edge appeared: the row's draft
carried an optional `sha2` and it is gone.

`tasks.list` copies the two Node rules on purpose (the folder shape
`/^\d\d-[a-z0-9-]+$/` and the first-line title) and prints a repo-relative
suite name, so the door and the referee count the same ten folders and no build
path reaches the wire. It lists and runs nothing: `node tools/tasks/run.mjs
--selftest` stays the only judge.

`--solve` builds each task's start tree, lays that arm's patch and judges the
solved tree, so a pass@1 is a referee result and not a log read. The arms are
calibration oracles: `scripted` solves all ten (expected 1.000) and `mutation`
solves none (expected 0.000), and the run fails the suite if an arm misses its
expectation. `tools/tasks/README.md` documents the leg.

## Proof

- Node, on this PC: `node tools/tasks/run.mjs --selftest` -> `PASS tasks
  selftest: 10 referees, 45 logged attempts, 3 models x 10 tasks, both solve
  arms at their pass@1`, printing `solve scripted: pass@1 1.000 (10/10 tasks
  solved)` and `solve mutation: pass@1 0.000 (0/10 tasks solved)`.
- The row's P2P tapes: `cargo test -p engine-playtest --lib eval` -> 84 passed,
  0 failed, 1 ignored (run 40085189098), so the EV1 eval lane and its scorecard
  are unchanged by this row.
- `cargo test -p engine-endpoint --features mcp --lib` -> 977 passed, 1 failed,
  1 ignored (run 40083760879). Every BG14 name is in the passing set: the
  `memory_store` lane (row id equals the shipped store's id, the four hash
  refusals, stamp order, filters, append idempotence, the new byte-bound
  refusal), the `packs::memory` record verb (a row without a 16-hex hash
  refuses with `record_bad_hash` and the store keeps its bytes), the
  `packs::tasks` pack (ten jobs, folder order, a written suite, a half-read
  suite refusing by name) and the `resources` lesson view (listed beside the
  four static families, its query shape advertised, rows in stamp order, the
  same bytes twice). The one failure is
  `scene_tools::place_facing_tests::place_facing_refuses_to_mix_with_classic_wants`
  (`apply_in_flight` where it expects `bad_want`) — no BG14 file, and it is the
  same test on a tree another seat has dirty in `crates/endpoint/src/packs/play.rs`.
- `cargo test -p engine-endpoint --features mcp --test it` -> 57 passed, 2
  failed, 2 ignored (run 40084297608), and the row's own P2P is the passing
  one: `tool_count_pin::frozen_registry_pins_26_and_router_is_pinned` PASS at
  the new 145 (143 measured at HEAD plus `memory.record` plus `tasks.list`).
- Refusal classification: `refusal_every_code_the_crate_produces_is_classified`
  and `refusal_each_code_has_exactly_one_kind_and_the_tables_stay_sorted` were
  red on the first tree (run 40083070516, 975 passed / 2 failed) because
  `store_refusal` passed its code through a variable, so the scanner never saw
  the literal. Each arm now names its code at the constructor, and
  `mcp_refusal.rs` carries ten new rows: five Input (`record_bad_hash`,
  `record_bad_kind`, `record_bad_path`, `record_bad_stamp`,
  `record_empty_text`), three State (`record_store_line`, `tasks_bad_task`,
  `tasks_no_suite`), one Internal (`record_store_io`), one Permission
  (`memory_not_granted`). Both pass on the run above.
- Lints: `node tools/rust-quality.mjs --crate engine-endpoint` -> RESULT PASS,
  score 100 (A), 0 ratchet regressions; `rustfmt --edition 2021 --check` clean
  on every touched file.
- The golden: no sim code moved in this row (endpoint door plus `tools/tasks`),
  so `GOLDEN_600` `d57b5c8ae3096614` is outside its scope and was not re-run.

## Reds on the shared tree this row did not cause

- `collapse_fixtures::guardrail_holds_the_wide_surface_with_no_answer_loss`
  refuses: loading every shipped pack advertises 84,534 tool chars against the
  72,952 cap (`TOOL_LIST_BUDGET_CHARS` 75,000 minus 2,048 headroom). `shipped()`
  loads in `order`, and the refusal names `fx` — the third pack, after `scene`
  and `break_` — so `memory` (order 13) and `tasks` (order 26) were never
  loaded and BG14's ~1,150 description chars are not inside that number at all.
  The wide surface was already over the guardrail before this row. Closing it is
  a budget decision — raise the cap, trim shipped descriptions, or split a pack
  — not a BG14 leg.
- `mcp_stdio::scripted_client_lists_previews_applies_and_undoes_over_stdio`
  expects 23 advertised names and gets 26: `prefab.place`, `prefab.save`,
  `scene.save` and `scene.load` joined the default set before this row (DOOR2).
  The stale expectation is its owner's pin.
- `cargo clippy -p engine-endpoint --features mcp --all-targets -- -D warnings`
  could not reach the endpoint crate: `crates/spatial/src/lod.rs:348` fails
  `clippy::question_mark` on the shared tree (run 40084783064). Not a BG14 file.

## What this receipt does not claim

The door surface is unit- and integration-tested inside one process; **no
external MCP client was driven against `e2040://memory` or `memory.record` over
stdio in this row**, and nothing here was visually verified — there is no pixel
in it. `tasks.list` lists the suite; the referee legs it names were measured by
`run.mjs`, not by the door. The row's `--solve` arms are the two canned
oracles, so `pass@1 1.000` says the referee judges a solved tree, not that a
model can solve the suite.
