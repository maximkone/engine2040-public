# SPRINT — board and receipts

> Read slices only: `node tools/board-slice.mjs --ready` for the list, `node tools/board-slice.mjs <ID>` for one row — one line does both (paste once, replace ID): `node tools/board-slice.mjs --ready; node tools/board-slice.mjs <ID>`. Never read this file whole and never read the shards whole; the slice is byte-identical. Tools read the index plus every `docs/SPRINT-*.md` shard as one board.

The game is `FOCUS.md`; the only order is its "## Now". A row's spec is
`node tools/plan-atlas.mjs --row <ID>`. Gates: `FINISH-RICH.md`; background:
`ROADMAP.md` and `PLAN-REVIEW.md`.

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
| assets | `crates/assets`, `content/` | net | `crates/net` |
| render | `crates/render` | mcp | `crates/endpoint` |
| editor | `crates/editor` | provider | `crates/provider` |
| arena | `crates/arena` | spatial | `crates/spatial` |
| physics | `crates/physics` | playtest | `crates/playtest` |
| combat | `crates/combat` | nn, distill | `crates/nn`, `crates/distill` |
| brain, nav | `crates/ai`, `crates/nav` | observe, perf | `crates/observe`, `crates/perf` |
| anim | `crates/anim` | proof | `crates/proof`, `tests/`, `fixtures/` |
| destruction | `crates/destruction` | audio, ui, input | `crates/audio`, `crates/ui`, `crates/input` |
| game, app | `crates/game`, `crates/app` | core, ecs | `crates/core`, `crates/ecs` |
| save, scene, world | `crates/save`, `crates/scene`, `crates/world` | plan, research | `docs/plan/`, the `RES-*` rows |
| urban, geo | `games/urban`, `crates/geo` | | |

## Board

Rows live in one shard per section (byte-identical; each shard header is the
canonical title). The live list is `node tools/board-slice.mjs --ready`.
Shards marked `*` had their open rows parked in `plan/parked-2026-10-22.md`.
A shard whose rows are all ticked moves to `archive/SPRINT-done-*.md`; the
tools still read it for `needs:`.

- [`SPRINT-01-rome-first.md`](SPRINT-01-rome-first.md) — Rome first
- [`SPRINT-02-done.md`](SPRINT-02-done.md) — Done
- [`SPRINT-03-finish-line.md`](SPRINT-03-finish-line.md) — Finish line
- [`SPRINT-04-finish-quality.md`](SPRINT-04-finish-quality.md) — Finish quality
- [`SPRINT-05-code-quality.md`](SPRINT-05-code-quality.md) — Code quality
- [`SPRINT-08-hard-stuff.md`](SPRINT-08-hard-stuff.md) — Hard stuff
- [`SPRINT-12-tier-2040.md`](SPRINT-12-tier-2040.md) — 2040 tier *
- [`SPRINT-13-engine-edge.md`](SPRINT-13-engine-edge.md) — Engine edge *
- [`SPRINT-15-platform-live-ai.md`](SPRINT-15-platform-live-ai.md) — Platform and live AI *
- [`SPRINT-16-urban-war.md`](SPRINT-16-urban-war.md) — Urban war *
- [`SPRINT-17-body-and-break.md`](SPRINT-17-body-and-break.md) — Body and break
- [`SPRINT-18-gaps.md`](SPRINT-18-gaps.md) — Look gaps
- [`SPRINT-18-steals.md`](SPRINT-18-steals.md) — Steals and potato tier
- [`SPRINT-20-game.md`](SPRINT-20-game.md) — Game
- [`SPRINT-21-look-feel.md`](SPRINT-21-look-feel.md) — Look and feel steals
- [`SPRINT-22-web-realism.md`](SPRINT-22-web-realism.md) — Web and realism
- [`SPRINT-23-books.md`](SPRINT-23-books.md) — Books
- [`SPRINT-24-editor.md`](SPRINT-24-editor.md) — Editor
- [`SPRINT-25-ai-native.md`](SPRINT-25-ai-native.md) — AI native
- [`SPRINT-26-motion.md`](SPRINT-26-motion.md) — Motion teacher rows
- [`SPRINT-27-final-picture.md`](SPRINT-27-final-picture.md) — Final picture
- [`SPRINT-28-rome.md`](SPRINT-28-rome.md) — Rome
- [`SPRINT-30-feel.md`](SPRINT-30-feel.md) — Feel
- [`SPRINT-31-research.md`](SPRINT-31-research.md) — Research 2026-10-22
- [`SPRINT-32-real-chars.md`](SPRINT-32-real-chars.md) — Realistic characters
- [`SPRINT-33-speed-demo.md`](SPRINT-33-speed-demo.md) — Speed demo
- [`SPRINT-34-hero-city.md`](SPRINT-34-hero-city.md) — Hero city
- [`SPRINT-35-book-gaps-4.md`](SPRINT-35-book-gaps-4.md) — Book gaps 4
- [`SPRINT-36-gap-audits.md`](SPRINT-36-gap-audits.md) - Gap audits: workspace, tools, LLM
- [`SPRINT-37-smarts.md`](SPRINT-37-smarts.md) - Smart steals (research 2026-10-23)

Done shards (`06 07 09 10 11 14 26-steals 28-powerups 29`) sit in
`archive/SPRINT-done-*.md`. They keep their text and their ticks.

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

## Receipts

Receipts live in [`SPRINT-19-receipts.md`](SPRINT-19-receipts.md) (newest first). Earlier receipt text lives in git history (`git log -p -- docs/SPRINT.md`). Tick a task only after its acceptance passed, with a receipt line in that file.
