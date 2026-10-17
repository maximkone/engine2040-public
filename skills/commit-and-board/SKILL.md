---
name: commit-and-board
description: "Use when you commit or push on the shared master tree, tick or write a board row (docs/SPRINT-*.md, plan section 7), plan new rows, or add a doc that needs an index line: commit by path, temp GIT_INDEX_FILE, Landed sha, Accept, F2P, P2P, needs."
---

# Commit and board

One branch `master`, one tree `C:\engine2040`, several AIs editing at once. You may push `master` (AGENTS.md, standing permission).

## Never

`git add -A`, `git commit -a`, rebase, amend, force-push, stash, `reset --hard`, `restore`, `git clean`.
Never `git clean` under `target/`: it is one unit, `-X` on a subfolder wipes all of it.

## Before you edit

- `git pull --no-rebase --no-edit origin master`.
- `git status --short`. Lines you did not make are someone else's.
- Staged lines (`D `, `M `) in the real index are theirs too. A bare `git commit` would ship them.
- Re-read a file right before you edit it. Never revert what you did not write.
- An `auto: backup ...` commit lands every few minutes, so HEAD moves while you work. Rebuild any HEAD-based text just before you commit.

## Case A: the paths are wholly yours

1. New files: `git add -N <path> [<path> ...]` (intent to add). Without it `git commit -- <path>` says the path is unknown.
2. Check nobody else is in them: `git diff --numstat -- <paths>`. Numbers you did not expect are foreign hunks: use case B.
3. Write the message to a scratch file outside the repo. End it with the attribution line the session gives. Then:
   `git commit -F <msgfile> -- <path1> <path2>`
   `--` plus paths commits only those paths, whatever else is staged.
4. `git pull --no-rebase --no-edit origin master`, then `git push origin master`.

## Case B: someone else has hunks in your file

`git commit -- <path>` would ship their half-done lines. Commit only yours through a temp index.
`tools/gh-check.mjs` uses the same `GIT_INDEX_FILE` trick for its snapshot.

1. Make the file as HEAD plus only your edits. Use a small scratch `.mjs`: read `git show HEAD:<path>` with `execFileSync` as a Buffer, apply your change by script, write the scratch copy. Do not pipe `git show` through pwsh (it re-encodes bytes). Do not use a `-U0` patch by line number: it lands on the wrong line when HEAD moves.
2. Point git at a scratch index, then fill it from HEAD (pwsh):
   ```
   $env:GIT_INDEX_FILE = "<scratch>\idx"
   git read-tree HEAD
   ```
3. Store your copy as a blob. `--path` applies the CRLF and LFS filters:
   ```
   $blob = git hash-object -w --path=<path> <scratch copy>
   ```
4. Put it in the temp index (one line per file):
   ```
   git update-index --add --cacheinfo "100644,$blob,<path>"
   ```
5. Commit, then drop the variable:
   ```
   git commit -F <msgfile>
   Remove-Item Env:GIT_INDEX_FILE
   ```
6. Resync the real index for your paths only: `git reset -q -- <path>`. A bare `git reset -q` drops other sessions' staged files.
7. Make the same edit in the working copy too, or their next path commit drops your lines.

Bash form of steps 2 to 5: `export GIT_INDEX_FILE=<scratch>/idx`, `git read-tree HEAD`, `blob=$(git hash-object -w --path=<path> <copy>)`, `git update-index --add --cacheinfo 100644,$blob,<path>`, `git commit -F <msgfile>`, `unset GIT_INDEX_FILE`.

## Row grammar

The board is `docs/SPRINT.md` (index) plus shards `docs/SPRINT-NN-*.md`. A row's spec is its plan in `docs/plan/` (`node tools/plan-atlas.mjs --row <ID>`).
Read rows by slice, never whole: `node tools/board-slice.mjs <ID>`, `--ready`, `--focus`, `--find <text>`, `--next`.

One bullet per row:
```
- [ ] RZ8 physics: a material table for friction, bounce, density and sound — what and why in 1-3 sentences with real file names. Diff: regular. Tier A. Size M. Owns `crates/physics/src/material.rs`. Accept: `node tools/gh-check.mjs cargo test -p engine-physics --lib material` prints N passed; golden unchanged. F2P: what fails today. P2P: what must keep passing (golden `d57b5c8ae3096614`). needs: RZ1, WEB2.
```

What the tools parse (`tools/board-slice.mjs`):
- Head: `- [ ] ID` or `- [x] ID` at line start. ID is capitals, then a digit, then an optional lowercase letter (`WEB4`, `RZ5`, `GP34b`). `area: title` is convention, not parsed.
- A row is the head line plus following lines that start with whitespace. A blank line or an unindented line ends it.
- Number a new ID after the highest in its prefix (prefix table in `docs/plan/README.md`).
- IDs are unique over the index and all shards (`workspace-check` fails a duplicate). Ids in `docs/archive/SPRINT-done-*.md` count as ticked for `needs:`. Do not reuse them: the tools do not catch it, but the row would shadow a landed one.
- `area` is one short word from the Areas table in `docs/SPRINT.md` (render, physics, game, assets, net, proof, ...).

Fields, in this order:

| field | rule |
|---|---|
| `Diff:` | regular (any coder), good (Sonnet class), hard (Opus class: GPU, concurrency, determinism, subtle math) |
| `Tier:` | A alpha, B v1, C VISION bar |
| `Size:` | S one session (up to about 150 changed lines in SPRINT-22), M two to four (about 400), L five to ten. Bigger than L: split. For L list landings: `Size L (landings: world and caps; ragdoll seeding; debris).` |
| `Owns` | the exact paths the row may edit, in backticks |
| `Accept:` `F2P:` `P2P:` `needs:` | see below |

## Accept, F2P, P2P

- Accept names a command and a number a second reader can run: `cargo test -p engine-game --features app --lib ragdoll_look` plus what it must show. Or a printed line, or a file that must exist. Prefer `node tools/gh-check.mjs cargo test ...` (skill `github-check`).
- A visual claim needs a capture you opened (skill `test-and-proof`). A perf claim needs a measured number and where (runner, or "(reference GPU)" for the RTX 3050).
- Always include the golden: `golden unchanged` or `golden d57b5c8ae3096614 x2`.
- `node tools/plan-atlas.mjs --plan <name>` lists an Accept that has no number.
- Every NEW row carries `F2P:` (fails before the change, passes after: name the test or the missing thing) and `P2P:` (passes before and after). By the board rule a row without both is not ready. The tools do not check it.

## needs and ready

- `needs:` goes last. The parser takes the FIRST `needs:` in the row text and reads up to the FIRST period: `needs: WEB2, WEB5.` or `needs: none.`
- Text after that period is not parsed. Notes go before `needs:` or after the period.
- Never write the word `needs:` earlier in the prose. Never put a period inside the list (`needs: RZ1. Also EG45` hides EG45): use commas.
- Every id must exist on the board or in the archive. A ticked row cannot need an open row (`workspace-check` fails it).
- A row is ready when it is open, every need is ticked, its text has no `BLOCKED:` or `LATER:`, and it sits under no `### Operator` heading. `LATER:` parks a row. Deleting the word unparks it.
- Backticked paths in a row count as "owned" for slicing (a span with no space, `<` or `>`, containing `/` or ending `.mjs .md .json .rs`). Do not backtick paths you only mention.

## Before you add a row

- Run `git grep` for the function first. A ticked row or an existing named function means NOOP.
- It names the measured number it moves. No number, no row.
- The proof is a command that runs the behaviour and can fail. `Select-String` for a word in a file is not proof.
- A stop is written: max attempts, files or minutes. At the stop the row ends PARTIAL. It does not grow.
- A research card becomes a row only when a consumer row exists. Otherwise park it in one line.
- One row, one outcome, small enough for one session (about 3 files or 200 lines). Bigger work splits into rows with disjoint paths.

## Tick the row

- Tick only after the row's Accept passed.
- Format at the end of the row's first line: `- [x] ID area: title ... Landed <sha7>.` Change `[ ]` to `[x]` and append ` Landed <sha7>.` Example: `... needs: none. Landed 979bc54.`
- The sha exists only after the code commit. Commit the code, run `git rev-parse --short=7 HEAD`, edit the row, commit the board file as its own commit (`board: ID landed`).
- A row with more landings stays `[ ]` and gets `Landed so far (<date>): <sha7> (<what>). Remaining: ...`.
- A second identical failure ends the row: add `BLOCKED:` and one row to `docs/BLOCKERS.md`. Never mark it green.

## Checks before the commit

- `node tools/workspace-check.mjs` after any board, doc or manifest edit. It checks ids unique, every `needs:` resolves, crate edges, and the LFS guard (a new non-LFS blob over 5 MB fails).
- `node tools/board-slice.mjs <ID>`: the row prints, under 8 KB.
- Code: `rustfmt --edition 2021 --check <files>` (never `cargo fmt`), `node tools/rust-quality.mjs`, tests through `node tools/gh-check.mjs` (skill `github-check`).
- New doc: add its line to the index that lists it in the same commit (`docs/INDEX.md`, `skills/SKILLS.md`). A new plan file gets one line in `docs/plan/README.md` (`node --test tools/plan-index.test.mjs`). Index tests fail otherwise.

## Pitfalls

- `git commit -- <path>` on a tracked file commits the whole file, foreign hunks included. Check `--numstat` first.
- `git pull` aborts when upstream touched a dirty file of yours. Commit your paths first, then pull (merge, no rebase).
- Push rejected: pull again, rerun nothing else, push. Never force.
- Pushing publishes only the private repo. The public mirror needs `node tools/public-export.mjs --commit --push` (skill `github-check`).
