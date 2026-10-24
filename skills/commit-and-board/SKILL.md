---
name: commit-and-board
description: "Use when you commit or push on the shared master tree, write a board row, close a row (Landed sha, evidence) or log partial progress, or add a doc that needs an index line: commit by path, temp index when others edit the same file."
---

# Commit and board

One branch `master`, one tree, many AIs at once. You may push `master` (AGENTS.md, standing permission). Steps: `reference.md`.

## Never

`git add -A`, `git commit -a`, rebase, amend, force-push, stash, `reset --hard`, `restore`, `git clean`. Never `git clean` under `target/`: it is one unit, `-X` on a subfolder wipes all of it.

## Before you edit

- `git pull --no-rebase --no-edit origin master`, then `git status --short`. Lines you did not make are someone else's, and staged lines (`D `, `M `) are theirs too: a bare `git commit` would ship them.
- Re-read a file right before you edit it. Never revert what you did not write.
- `Empire-AutoPush` is snapshot-only: it keeps the dirty tree on `refs/auto-backup/master` and never commits `master`, so HEAD moves only when a session commits. Cite your own sha.

## Commit your paths

- **Wholly yours:** `git add -N` new files, `git diff --numstat -- <paths>` (odd numbers mean foreign hunks), `git diff --check -- <paths>`, then `git commit -F <msgfile> -- <path1> <path2>`. `--` plus paths commits only those, whatever else is staged. One row's files go in one commit (the `mod x;` line and `x.rs`), or the mirror export and Pages break.
- **Foreign hunks in your file:** `git commit -- <path>` would ship their half-done lines. Use a temp index (`GIT_INDEX_FILE`, `read-tree HEAD`, `hash-object -w --path`, `update-index --cacheinfo`), then `git reset -q -- <path>` for your path only, and mirror the edit in the working copy. Steps: `reference.md`.
- **Push:** `git pull --no-rebase --no-edit origin master`, then `git push origin master`. Rejected: pull again, push again. Never force.

## Board rows

The board is `docs/SPRINT.md` (index) plus shards `docs/SPRINT-NN-*.md`; a row's spec is its plan (`node tools/plan-atlas.mjs --row <ID>`). Read rows by slice, never whole: `node tools/board-slice.mjs <ID>`, `--ready`, `--focus`, `--find <text>`, `--next`.

One bullet per row, `- [ ] ID` at line start plus the indented lines after it; IDs are unique over index, shards and `docs/archive/SPRINT-done-*.md`. Fields in order `Diff:` `Tier:` `Size:` `Owns` `Accept:` `F2P:` `P2P:` `needs:`.

Accept names a command and a number a second reader can run, or a printed line, or a file that must exist, with the golden (`golden unchanged`); every NEW row names its `F2P:` and `P2P:` test; `needs:` goes last and takes the first period; backticked paths count as owned. Table, rules: `reference.md`. Before you add one, `git grep` the function (a ticked row or existing function means NOOP) and name the measured number it moves. No number, no row.

## Close a row or log progress

Tick only when the row's own `Accept:` passed, with the command it names (or a stronger one): the exact command, its result and a count (`5 passed`), a run id or "on this PC". A cargo test with 0 tests is not a pass. Any part you did not do: do not tick, log progress instead.

Commit the code first (`git log -1 --format=%h -- <your path>`), then edit the FIRST line with the Edit tool, never `sed`: `[ ]` to `[x]`, append ` Landed <sha7>. <evidence>`. Then `node tools/workspace-check.mjs` and `node tools/board-slice.mjs <ID>`; `node tools/ticked-rows-audit.mjs` flags a stale tick. Commit the board file on its own: `board: <ID> landed`.

Partial keeps `[ ]` with ` Progress <date>: <what exists, test names, run ids>. Left: <what the Accept still needs>.` A second identical failure means `BLOCKED:` plus one line in `docs/BLOCKERS.md`; never tick on a red or link-failed tree.

## Pitfalls

A `git commit -- <path>` on a tracked file commits the whole file: check `--numstat` first. A `git pull` aborts when upstream touched a dirty file of yours, so commit first, then pull. The mirror is separate: `node tools/public-export.mjs --commit --push`.
