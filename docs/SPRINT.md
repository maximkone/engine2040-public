# SPRINT — board and receipts

> Read slices only: `node tools/board-slice.mjs --ready` for the list, `node tools/board-slice.mjs <ID>` for one row — one line does both (paste once, replace ID): `node tools/board-slice.mjs --ready; node tools/board-slice.mjs <ID>`. Never read this file whole and never read the shards whole; the slice is byte-identical. Tools read the index plus every `docs/SPRINT-*.md` shard as one board.

The rows file of the plans (the name stays for the tools). The game is
`FOCUS.md`; the only order is its "## Now" (owner 2026-10-10: four Core
proofs). A row's spec is its plan in `docs/plan/` (`node tools/plan-atlas.mjs --row <ID>`).
`FINISH-RICH.md` holds the gates; `ROADMAP.md` and `PLAN-REVIEW.md` are background.

## How a row works

- A row is ready when it is open, every `needs:` row is ticked, its text
  holds neither `BLOCKED:` nor `LATER:`, and it is not an operator row.
  Every NEW row carries `F2P:` (fails before, passes after) and `P2P:`
  (stays passing); without both it is not ready.
- Take rows from `node tools/board-slice.mjs --focus` first: the ready rows of
  `FOCUS.md` "## Now", in its order. Only when `--focus` is empty take other
  ready rows, smallest first; never invent rows. Red-gate fixes come first.
- One tree: `C:\engine2040` on master; no lanes, no
  worktrees. Several AIs share it: edit only the files your row needs, commit
  by path, pull, push (`AGENTS.md` "Work from plans").
- `PLN` rows write one `docs/plan/` file each; `RES-ARXIV`, `RES-STEAL`,
  `RES-DOCS` and `RES-JAN` rows write research notes.
- Root `Cargo.toml`, `Cargo.lock`, `tools/` and CI change only for a row that
  needs it. A new dependency first gets its license read, a `deny.toml` and
  `DEPENDENCIES.md` row and `cargo deny check`.
- Tick a row only after its acceptance passed, with a receipt line in the
  receipt log: `- [x] ID ... Landed <sha7>.`
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
| plan | `docs/plan/` (plan files) | research | `RES-ARXIV`, `RES-STEAL`, `RES-DOCS`, `RES-JAN` (research notes) |
| urban | `games/urban` (second game, UW/UC game rows) | geo | `crates/geo` (cook-time mesh tools; not a default member) |

## Board

Rows live in one shard per section (byte-identical; each shard header is the canonical title). The live list is `node tools/board-slice.mjs --ready`.

- [`SPRINT-01-rome-first.md`](SPRINT-01-rome-first.md) — Rome first
- [`SPRINT-02-done.md`](SPRINT-02-done.md) — Done
- [`SPRINT-03-finish-line.md`](SPRINT-03-finish-line.md) — Finish line
- [`SPRINT-04-finish-quality.md`](SPRINT-04-finish-quality.md) — Finish quality
- [`SPRINT-05-code-quality.md`](SPRINT-05-code-quality.md) — Code quality
- [`SPRINT-06-plans.md`](SPRINT-06-plans.md) — Plans
- [`SPRINT-07-accepted-plan-rows.md`](SPRINT-07-accepted-plan-rows.md) — Accepted plan rows
- [`SPRINT-08-hard-stuff.md`](SPRINT-08-hard-stuff.md) — Hard stuff
- [`SPRINT-09-empire.md`](SPRINT-09-empire.md) — Empire
- [`SPRINT-10-empire-inbox-drain.md`](SPRINT-10-empire-inbox-drain.md) — Empire inbox drain
- [`SPRINT-11-north-star.md`](SPRINT-11-north-star.md) — North star 2026 (S44)
- [`SPRINT-12-tier-2040.md`](SPRINT-12-tier-2040.md) — 2040 tier
- [`SPRINT-13-engine-edge.md`](SPRINT-13-engine-edge.md) — Engine edge
- [`SPRINT-14-agent-workspace.md`](SPRINT-14-agent-workspace.md) — Agent workspace
- [`SPRINT-15-platform-live-ai.md`](SPRINT-15-platform-live-ai.md) — Platform and live AI
- [`SPRINT-16-urban-war.md`](SPRINT-16-urban-war.md) — Urban war
- [`SPRINT-17-body-and-break.md`](SPRINT-17-body-and-break.md) — Body and break and hard systems
- [`SPRINT-18-gaps.md`](SPRINT-18-gaps.md) — Look gaps (shard header `docs/SPRINT-18-gaps.md:1` is canonical per `docs/SPRINT.md:70`; O10 not in title 2026-10-16)
- [`SPRINT-18-steals.md`](SPRINT-18-steals.md) — Steals and potato tier
- [`SPRINT-20-game.md`](SPRINT-20-game.md) — Game
- [`SPRINT-21-look-feel.md`](SPRINT-21-look-feel.md) — Look and feel steals
- [`SPRINT-22-web-realism.md`](SPRINT-22-web-realism.md) — Web and realism
- [`SPRINT-23-books.md`](SPRINT-23-books.md) — Books
- [`SPRINT-24-editor.md`](SPRINT-24-editor.md) — Editor for indie developers
- [`SPRINT-25-ai-native.md`](SPRINT-25-ai-native.md) — AI native
- [`SPRINT-26-steals-wave3.md`](SPRINT-26-steals-wave3.md) — Steals wave 3, verified against the tree
- [`SPRINT-27-final-picture.md`](SPRINT-27-final-picture.md) — Final picture: auto-fix, compact code, touch and POV
- [`SPRINT-28-rome.md`](SPRINT-28-rome.md) — Rome: one legionary's war
- [`SPRINT-29-hunt.md`](SPRINT-29-hunt.md) — Hunt 2026-10-21: steals from outside the engine set
- [`SPRINT-30-feel.md`](SPRINT-30-feel.md) — Feel: weight, impact, particles and the game-building door (owner 2026-10-21)

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

Receipts live in [`SPRINT-19-receipts.md`](SPRINT-19-receipts.md) (newest first). Earlier receipt text lives in git history (`git log -p -- docs/SPRINT.md`). Tick a task only after its acceptance passed, with a receipt line in that file.
