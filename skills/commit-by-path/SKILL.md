---
name: commit-by-path
description: "Use when you commit in engine2040: several AIs share one tree on master, so commit only your own paths (or only your hunks through a temp index), tick the board row, run workspace-check, pull, push."
---

# Commit by path

One branch `master`, one tree `C:\engine2040`, several AIs editing at once. Standing permission: you may push `master` (CLAUDE.md). Never `git add -A`, `git commit -a`, rebase, amend, force-push, stash, `reset --hard`, `restore` or `git clean` (never under `target/`: it is one unit, `-X` on a subfolder wipes all of it).

## Before you edit
- `git pull --no-rebase --no-edit origin master`.
- `git status --short`: lines you did not make are someone else's. Staged lines (`D `, `M `) in the real index are theirs too: a bare `git commit` would ship them.
- Re-read a file right before you edit it. Never revert what you did not write.
- An `auto: backup ...` commit lands every few minutes, so HEAD moves while you work. Rebuild any HEAD-based text just before committing.

## Case A: the paths are wholly yours
1. New files: `git add -N <path> [<path> ...]` (intent to add; without it `git commit -- <path>` says the path is unknown).
2. Check nobody else is in them: `git diff --numstat -- <paths>`. Numbers you did not expect mean foreign hunks: use case B.
3. Write the message to a scratch file (not the repo), ending with the attribution line the session gives, then:
   `git commit -F <msgfile> -- <path1> <path2>`
   `--` plus paths commits only those paths, whatever else is staged.
4. `git pull --no-rebase --no-edit origin master`, then `git push origin master`.

## Case B: someone else has hunks in your file
`git commit -- <path>` would ship their half-done lines. Commit only yours with a temp index (bash form):
1. Make the file as HEAD plus only your edits: `git show HEAD:<path>` into a scratch file, apply your change to that copy by script (not by line-number patch: a `-U0` patch lands on the wrong line the moment HEAD moves).
2. `export GIT_INDEX_FILE=<scratch>/idx` then `git read-tree HEAD`.
3. `blob=$(git hash-object -w --path=<path> <scratch copy>)` (`--path` applies the CRLF and LFS filters).
4. `git update-index --add --cacheinfo 100644,$blob,<path>` (one per file).
5. `git commit -F <msgfile>`, then `unset GIT_INDEX_FILE`.
6. Resync the real index for your paths only: `git reset -q -- <path>` (a bare `git reset -q` drops other sessions' staged files).
7. Make the same edit in the working copy too, or their next path commit drops your lines.
`tools/gh-check.mjs` uses the same `GIT_INDEX_FILE` trick for its snapshot.

## Tick the board row
- Rows live in `docs/SPRINT-NN-*.md` shards. Read one row: `node tools/board-slice.mjs <ID>` (never read a shard whole).
- Tick only after the row's Accept passed. Format, at the end of the row's first line: `- [x] ID area: title ... Landed <sha7>.` (change `[ ]` to `[x]`, append ` Landed <sha7>.`). Example: `... needs: none. Landed 979bc54.`
- The sha exists only after the code commit: commit the code, `git rev-parse --short=7 HEAD`, edit the row, commit the board file as its own commit (`board: ID landed`).
- A row that needs more landings stays `[ ]` and gets `Landed so far (<date>): <sha7> (<what>). Remaining: ...`.
- A ticked row must not need an open row, and `needs:` ends at the first period: write `needs: WEB2, WEB5.`

## Checks before the commit
- `node tools/workspace-check.mjs` after any board, doc or manifest edit (board ids unique, every `needs:` resolves, crate edges, LFS size guard: a new non-LFS blob over 5 MB fails).
- Code: `rustfmt --edition 2021 --check <files>` (never `cargo fmt`), `node tools/rust-quality.mjs`, tests through `node tools/gh-check.mjs` (skill `github-check`).
- New docs: add the line to the index that lists them (`docs/INDEX.md`, `docs/plan/README.md`, `skills/SKILLS.md`) in the same commit; index tests fail otherwise.

## Pitfalls we hit
- `git commit -- <path>` on a tracked file commits the whole file, foreign hunks included. Check `--numstat` first.
- `git pull` aborts when upstream touched a dirty file of yours: commit your paths first, then pull (merge, no rebase).
- Push rejected: pull again, rerun nothing else, push. Never force.
- Pushing publishes only the private repo. The public mirror needs `node tools/public-export.mjs --commit --push` (skill `web-build-and-test`).
