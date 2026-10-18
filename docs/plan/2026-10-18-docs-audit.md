# Docs audit 2026-10-18 — index and plan pointer sweep

Scope: the four maps (`docs/README.md`, `docs/INDEX.md`, `docs/plan/README.md`,
`docs/FOCUS.md`) and the plan files `plan/README.md` indexes. Only pointer and
stale-reference errors were fixed; no plan text, no board row, no threshold was
touched. Every path below was checked with `Test-Path`.

## Fixed

1. `docs/plan/README.md` — added the missing index line for
   `2026-10-18-row-audit.md`. The file was committed (8f71b58d) without its line,
   so the plan index and the directory listed different files and
   `tools/plan-index.test.mjs` was red. Tag LATER, proof none: it is a read-only
   board audit, it owns no row.
2. `docs/plan/multiplayer.md:206` — `docs/systems/netcode.md` →
   `docs/archive/2026-10-03/systems-netcode.md`. The systems spec was retired
   2026-10-03 and archived under that name; the plan named a live file that no
   longer exists (AGENTS: fix every inbound link when a doc is retired).

## Checks (green)

- `node tools/book-links.mjs` — 29 pages, 151 internal links, 0 broken.
- `node tools/workspace-check.mjs` — all PASS (board 1361 tasks, maps 38
  crates / 1017 paths, modules 1047 files). The two report-only WARNs are
  unchanged (wiring islands, flaky quarantine).
- `node --test tools/plan-index.test.mjs docs/INDEX.test.mjs
  tools/index-check.test.mjs docs/FOCUS.test.mjs docs/VISION.test.mjs
  tools/plan-atlas.test.mjs tools/stale-refs.test.mjs tools/stale-targets.test.mjs`
  — 35 pass, 0 fail.
- The four maps and every plan path they name resolve; no markdown link in the
  four maps or in `docs/plan/*.md` is broken.

## Deliberately NOT touched

- The six pinned stale refs in `tools/stale-refs.test.mjs`
  (`docs/research/HARD-SYSTEMS-CARDS.md` ×4, `docs/research/DISCOVERED.md` ×2).
  The test header says the fix was denied and the lead applies the one-liners;
  its pin asserts `EXPECT-STALE 6`. Editing them would move a deliberate pin.
- `EMPIRE.md` / `EMPIRE-INBOX.md` refs: `docs/plan/README.md:83`,
  `docs/library/index.md:96`, `docs/plan/2026-10-18-handoff.md:16`,
  `docs/plan/gameplay.md:409`, `docs/design/STAGES.md:435`. The owner retired
  both files on 2026-10-17 (standalone 6bdf32bf moved them to
  `docs/archive/2026-10-17-standalone/`). The EMPIRE area is owner-owned and off
  the sweep; the truthful fix is a process decision (a decision goes to the owner
  in chat), not a path swap. `docs/book/ref-tools.md:386` and
  `docs/book/ref-process.md:151` name the inbox in export deny-lists, where a
  gone path is harmless.
- Planned/future paths, each a file a row will create:
  `docs/plan/gameplay.md:421` (`docs/receipts/2026-10-producer.md`, "new"),
  `docs/plan/seats.md:64` (`docs/receipts/YYYY-MM.md`, `docs/board-done.md`),
  `docs/plan/silicon-city.md:415` (`docs/silicon-fpga.md`, LATER plan).
- Historical and donor paths: the `git show <sha>:docs/plan/<old>.md` notes
  (animation, body-and-break, content, parked, quality, runtime, seats,
  urban-war, multiplayer:3, rome-spec) name archived plans at a pinned commit,
  not live files; `docs/plan/steals.md:35` is a donor repo path.
- Frozen `docs/archive/**` history (no authority) and the pinned leftovers
  `docs/README.md` still lists (`plan/seats.md`, `../team/`, `../opencode.jsonc`,
  `../arsenal.json`; loop wording in the dated `SPRINT-*.md` shards).
- `docs/plan/ATLAS.md` is a generated map (dated 2026-10-17) that now omits the
  newer plan files (`2026-10-18-*`, `spatial-*`). It is refreshed with
  `node tools/plan-atlas.mjs --write`; that regen also folds in concurrent board
  drift, so it is left to the next board change. It is a generated-file refresh,
  not a pointer error.
- `docs/SPRINT-18-gaps.md` was being edited by another session during this
  sweep; left untouched.
