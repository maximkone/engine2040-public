# SPRINT — board and receipts

> Read slices only: `node tools/board-slice.mjs --ready` for the list, `node tools/board-slice.mjs <ID>` for one row — one line does both (paste once, replace ID): `node tools/board-slice.mjs --ready; node tools/board-slice.mjs <ID>`. Never read this file whole and never read the shards whole; the slice is byte-identical. Tools read the index plus every `docs/SPRINT-*.md` shard as one board.

The live work list for `/sprint` and `node tools/sprint-loop.mjs`. The game is
`FOCUS.md`; the only order is its "## Now" (owner 2026-10-10: four Core
proofs). `FINISH-RICH.md` holds the gates; `ROADMAP.md` and `PLAN-REVIEW.md`
are background. Tick a task only after its acceptance passed, with a receipt
line under the receipt log.

## How the board runs

- A task is ready when it is open, every `needs:` task is ticked, its text
  holds neither `BLOCKED:` nor `LATER:`, and it is not an operator task.
  Every NEW row carries `F2P:` (fails before, passes after) and `P2P:`
  (stays passing); without both it is not ready.
- A wave takes rows from `node tools/board-slice.mjs --focus` first: the READY
  rows of `FOCUS.md` "## Now", in its order. Width = the number of focus rows
  ready, never padded; the cap is knob `width`. Only when `--focus`
  is empty may a wave take other ready rows, at most 2 per wave, smallest
  first; never invent tasks. One tree: `C:\engine2040` on master; no lanes, no
  worktrees (owner 2026-10-10). One coder per crate (knob `coders` is the
  count), each on OWNED paths (its `paths:`, or a claim line in
  `target/queue/claims.txt`; a shared contract may land in an earlier wave
  through the integrator). Builders edit only owned paths and never commit; the
  judge reads `git diff -- <owned paths>`; the keeper lands a PASS
  (`tools/land.mjs`). A FAIL is fixed in place; only the lead abandons work
  (`/sprint` §4). `PLN` rows go to `planner`, one
  `docs/plan/` file each; `RES-ARXIV`, `RES-STEAL`,
  `RES-DOCS` and `RES-JAN` rows go to `researcher`. The `docs/plan/` wave-packing
  notes are packing guidance; this file wins for the loop. Order: red-gate
  fixes first, then `--focus`; there is no other order list.
- The orchestrator owns root `Cargo.toml`, `Cargo.lock`, `tools/`, CI and docs;
  it writes shared type-only contracts and adds each dependency (license read,
  `deny.toml`, `DEPENDENCIES.md` row, `cargo deny check`) before the wave that
  needs them.
- The orchestrator runs `node tools/proof.mjs` once per wave. Each wave
  receipt carries the proof result plus golden x2 plus an explicit Unverified
  list, so debt never hides. One
  sprint per working tree: `node tools/sprint-lock.mjs`.
- `node tools/sprint-loop.mjs` runs waves autonomously, one fresh OpenCode
  session each, until a replan wave adds no ready work, two waves make no
  board progress, or
  `target/sprint.halt` exists.
- Second identical failure ends it: `BLOCKED:` plus `BLOCKERS.md` row; never resend whole.

## Areas

| Area | Owns | Area | Owns |
|---|---|---|---|
| assets | `crates/assets` | net | `crates/net` |
| content | `content/` | mcp | `crates/endpoint` |
| render | `crates/render` | provider | `crates/provider` |
| editor | `crates/editor` | spatial | `crates/spatial` |
| arena | `crates/arena` | playtest | `crates/playtest` |
| physics | `crates/physics` | nn, distill | `crates/nn`, `crates/distill` |
| combat | `crates/combat` | observe | `crates/observe` |
| brain | `crates/ai` | perf | `crates/perf` |
| nav | `crates/nav` | proof | `crates/proof`, `tests/`, `fixtures/` |
| anim | `crates/anim` | audio, ui, input | `crates/audio`, `crates/ui`, `crates/input` |
| destruction | `crates/destruction` | game | `crates/game` |
| app | `crates/app` (composition/lifecycle) | core | `crates/core` (tick/seeds/hashes) |
| ecs | `crates/ecs` (lifetime/storage/queries) | save | `crates/save` (versioned schemas) |
| scene | `crates/scene` (IDs/hierarchy/codec) | world | `crates/world` (terrain/sky/blockout) |
| plan | `docs/plan/` (planner only) | research | `RES-ARXIV`, `RES-STEAL`, `RES-DOCS`, `RES-JAN` (researcher only) |
| urban | `games/urban` (second game, UW/UC game rows) | geo | `crates/geo` (cook-time mesh tools; not a default member) |

## Board

Rows live in one shard per section (byte-identical; each shard header is the canonical title). The live list is `node tools/board-slice.mjs --ready`.

- [`docs/SPRINT-01-rome-first.md`](docs/SPRINT-01-rome-first.md) — Rome first
- [`docs/SPRINT-02-done.md`](docs/SPRINT-02-done.md) — Done
- [`docs/SPRINT-03-finish-line.md`](docs/SPRINT-03-finish-line.md) — Finish line
- [`docs/SPRINT-04-finish-quality.md`](docs/SPRINT-04-finish-quality.md) — Finish quality
- [`docs/SPRINT-05-code-quality.md`](docs/SPRINT-05-code-quality.md) — Code quality
- [`docs/SPRINT-06-plans.md`](docs/SPRINT-06-plans.md) — Plans
- [`docs/SPRINT-07-accepted-plan-rows.md`](docs/SPRINT-07-accepted-plan-rows.md) — Accepted plan rows
- [`docs/SPRINT-08-hard-stuff.md`](docs/SPRINT-08-hard-stuff.md) — Hard stuff
- [`docs/SPRINT-09-empire.md`](docs/SPRINT-09-empire.md) — Empire
- [`docs/SPRINT-10-empire-inbox-drain.md`](docs/SPRINT-10-empire-inbox-drain.md) — Empire inbox drain
- [`docs/SPRINT-11-north-star.md`](docs/SPRINT-11-north-star.md) — North star 2026 (S44)
- [`docs/SPRINT-12-tier-2040.md`](docs/SPRINT-12-tier-2040.md) — 2040 tier
- [`docs/SPRINT-13-engine-edge.md`](docs/SPRINT-13-engine-edge.md) — Engine edge
- [`docs/SPRINT-14-agent-workspace.md`](docs/SPRINT-14-agent-workspace.md) — Agent workspace
- [`docs/SPRINT-15-platform-live-ai.md`](docs/SPRINT-15-platform-live-ai.md) — Platform and live AI
- [`docs/SPRINT-16-urban-war.md`](docs/SPRINT-16-urban-war.md) — Urban war
- [`docs/SPRINT-17-body-and-break.md`](docs/SPRINT-17-body-and-break.md) — Body and break and hard systems
- [`docs/SPRINT-18-gaps.md`](docs/SPRINT-18-gaps.md) — Look gaps
- [`docs/SPRINT-18-operator.md`](docs/SPRINT-18-operator.md) — Operator
- [`docs/SPRINT-18-steals.md`](docs/SPRINT-18-steals.md) — Steals and potato tier
- [`docs/SPRINT-20-game.md`](docs/SPRINT-20-game.md) — Game
- [`docs/SPRINT-21-look-feel.md`](docs/SPRINT-21-look-feel.md) — Look and feel steals

### Ready (live, never a snapshot)

The ready list is live — snapshots rot. List it with `node tools/board-slice.mjs --ready`, slice one row with the one-line wrapper above. No timestamped snapshot table lives here (removed 2026-10-08; earlier snapshots live in git history).

## Parked (not on the board; add a row to revive)

R6 FSR port (RP1 replaces it), WS1 WASM mods, LG1 text brief to arena
variant, MCP4 fork tools over MCP, own-video motion capture, OpenUSD, C2PA
export; from the operator board: TT1 time travel, PA1 performance autopilot,
RV1 red-team agent, SC2 merge queue, SC3 tournament harness, PD1 performance
doctor, BAL1 balance assistant; its EA1 panel folds into the editor plan.
World/terrain/sky/open-world is CUT by design: no W rows; sky lives as RL4
physical sky — see `docs/plan/render-look.md` and `FOCUS.md`.

## Receipt log

Receipts live in [`docs/SPRINT-19-receipts.md`](docs/SPRINT-19-receipts.md) (newest first). Earlier receipt text lives in git history (`git log -p -- docs/SPRINT.md`). Tick a task only after its acceptance passed, with a receipt line in that file.
