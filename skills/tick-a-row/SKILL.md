---
name: tick-a-row
description: "Use when you finish work on a board row and must close it or log partial progress: turn [ ] into [x] with Landed <sha7> and the evidence, or add a Progress note and leave it open; also when a ticked row's proof looks wrong."
---

# Tick a row (or leave honest progress)

The board is `docs/SPRINT.md` plus shards `docs/SPRINT-NN-*.md`. Row grammar, fields and `needs:` rules live in skill `commit-and-board`; this skill is the closing step only.

## 1. Read the row, never the file

- `node tools/board-slice.mjs <ID>` prints the row verbatim, its needs, its plan excerpt and owned paths.
- `node tools/board-slice.mjs --find <text>` finds rows (done ones too). `--ready`, `--focus`, `--next` list work; `--ready --open` shows every open row and what it waits on.
- Find the shard that holds it: Grep `- \[ \] <ID> ` over `docs` (Grep tool with `path: docs`, glob `SPRINT-*.md`). Re-read those lines right before you edit: other sessions tick rows in the same shard.

## 2. May I tick

Tick only when the row's own `Accept:` passed, with the command the row names (or a stronger one). Evidence is part of the tick, not a favour:

- The exact command, its result and a count: `node tools/gh-check.mjs cargo test -p engine-game --features app --lib break_wire` then `5 passed`.
- A run id when a runner did it (`gh run 39059423063`), or "on this PC" for GPU lines.
- A cargo test that ran 0 tests is not a pass. A filter that matches no `fn`, `mod` or file name is not a pass. Check the filter names a real test.
- The golden: `golden d57b5c8ae3096614 unchanged` when the row says so.
- A visual row needs a capture you opened (skill `test-and-proof`); a perf row a measured number and where.
- Any part of the Accept you did not do: do not tick. Write progress (step 4).

`node tools/ticked-rows-audit.mjs` is the static check afterwards: it flags a ticked row whose `-p <crate>` is gone, whose `--test` target moved, whose test filter matches nothing, or whose `crates/...` path left git. Fix the row text it names. It never runs cargo.

## 3. Close it

1. Commit the code first. The sha comes from your own commit: `git log -1 --format=%h -- <one of your paths>` (not `HEAD`: an auto-backup commit moves `HEAD` every few minutes).
2. Edit the FIRST physical line of the row (the line that starts `- [ ] ID`). Change `[ ]` to `[x]` and append at the end of that line: ` Landed <sha7>. <evidence>`. Example from the tree:
   `- [x] GP35 game: kit walls stop fighters — all 11 actors in Landed abf3bace. cargo test ... 8 passed ...`
   The continuation lines (indented) stay as they are. The row stays one bullet.
3. Use the Edit tool with the whole first line as `old_string`. Do not use `sed`:
   - evidence has `/` (paths) and ends the `s/a/b/` early, giving `unterminated s command`;
   - `&` in the replacement pastes the match, `[`, `]`, `.`, `*` in the pattern are regex;
   - backticks, `$` and `"` are eaten by pwsh or bash first.
   If you must script it, write a temp `.mjs` that reads the file, uses `String.replace` with a plain string, and writes it back (same newline style: the board is LF).
4. Run `node tools/workspace-check.mjs` (ids unique, every `needs:` resolves, a ticked row cannot need an open one) and `node tools/board-slice.mjs <ID>` (the row prints, under 8 KB).
5. Commit the board file as its own commit: message `board: <ID> landed` (batch: `board: tick audit batch 3 - GA13 GP40 ...`), body lists the proofs. If another session has hunks in that shard, use Case B in skill `commit-and-board` (temp index), because `git commit -- <path>` ships their lines.

## 4. Partial: keep it open, say what is left

A row with more landings stays `[ ]`. Append to its first line: ` Progress <date>: <what exists, with test names and run ids>. Left: <what the Accept still needs>.` Example: GA14 and GA15 in `docs/SPRINT-20-game.md`. A later landing turns it into `Landed`.
For a multi-landing row the older form is also valid: `Landed so far (<date>): <sha7> (<what>). Remaining: ...`.

## 5. Blocked, wrong or stale

- Second identical failure: add `BLOCKED:` to the row and one row to `docs/BLOCKERS.md` (never read it whole). Never tick green.
- A ticked row whose code is gone or whose proof was a skipped test: re-open it (`[x]` back to `[ ]`, add `Progress` saying why) rather than leaving a false green.
- Do not reuse an ID that exists in `docs/archive/SPRINT-done-*.md`: the tools count it as ticked.

## 6. Traps

- Never write the word `needs:` in the evidence: the parser takes the FIRST `needs:` in the row text.
- Evidence with a backticked path that does not exist is flagged by the audit. Quote real paths only.
- Do not tick on a red or link-failed tree: file the BLOCKERS row (skill `github-check`).
