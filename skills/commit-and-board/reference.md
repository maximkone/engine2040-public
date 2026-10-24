# Commit and board — reference

Detail moved out of `SKILL.md`: the temp-index procedure, the board row fields, the Accept rules and the close-a-row steps. The rules and the commands stay in `SKILL.md`.

## Case B: someone else has hunks in your file

`git commit -- <path>` would ship their half-done lines. Commit only yours through a temp index (`tools/gh-check.mjs` uses the same trick).

1. Write your copy as HEAD plus only your edits: a scratch `.mjs` reads `git show HEAD:<path>` as a Buffer (not through pwsh, which re-encodes bytes) and applies the change. No `-U0` patch by line number.
2. In pwsh: `$env:GIT_INDEX_FILE = "<scratch>\idx"`, then `git read-tree HEAD`.
3. `$blob = git hash-object -w --path=<path> <scratch copy>` (`--path` applies the CRLF and LFS filters), then `git update-index --add --cacheinfo "100644,$blob,<path>"`.
4. `git commit -F <msgfile>`, then `Remove-Item Env:GIT_INDEX_FILE`.
5. `git reset -q -- <path>` resyncs the real index for your path only. A bare `git reset -q` drops other sessions' staged files.
6. Make the same edit in the working copy too, or their next path commit drops your lines.

## Board row fields

One bullet per row. Head: `- [ ] ID` at line start (ID: capitals, a digit, an optional lowercase letter, `GP34b`). The row is that line plus the indented lines after it.
The board is `docs/SPRINT.md` (index) plus shards `docs/SPRINT-NN-*.md`. A row's spec is its plan in `docs/plan/` (`node tools/plan-atlas.mjs --row <ID>`).

- Number a new ID after the highest in its prefix (table in `docs/plan/README.md`). IDs are unique over the index, the shards and `docs/archive/SPRINT-done-*.md`, which count as ticked.
- `area` is one short word from the Areas table in `docs/SPRINT.md`.
- Fields, in this order:

| field | rule |
|---|---|
| `Diff:` | regular (any coder), good (Sonnet class), hard (Opus class: GPU, concurrency, determinism, subtle math) |
| `Tier:` | A alpha, B v1, C VISION bar |
| `Size:` | S one session (about 150 changed lines), M two to four (about 400), L five to ten. Bigger: split |
| `Owns` | the exact paths the row may edit, in backticks |
| `Accept:` `F2P:` `P2P:` `needs:` | see below |

## Accept, F2P, P2P, needs

- Accept names a command and a number a second reader can run, or a printed line, or a file that must exist. Prefer `node tools/gh-check.mjs cargo test ...` (skill `github-check`). A visual result needs a capture you opened (skill `test-and-proof`); a perf result a measured number and where. Always the golden: `golden unchanged` or `golden d57b5c8ae3096614 x2`.
- Every NEW row carries `F2P:` (fails before, passes after: name the test) and `P2P:` (passes before and after). `node tools/plan-atlas.mjs --plan <name>` lists an Accept with no number.
- `needs:` goes last. The parser takes the FIRST `needs:` and reads up to the FIRST period: `needs: WEB2, WEB5.` or `needs: none.` Never write the word earlier in the prose; never put a period in the list. Every id must exist on the board or in the archive; a ticked row cannot need an open one.
- Ready = open, every need ticked, no `BLOCKED:` or `LATER:` in its text, not under a `### Operator` heading. Backticked paths count as "owned" for slicing: do not backtick paths you only mention.

## Before you add a row

- `git grep` the function first. A ticked row or an existing named function means NOOP.
- Name the measured number it moves. No number, no row. The proof is a command that can fail: `Select-String` for a word is not proof.
- Write the stop (max attempts, files or minutes). At the stop the row ends PARTIAL; it does not grow.
- One row, one outcome, about 3 files or 200 lines. A research card becomes a row only when a consumer row exists.

## Close a row: the evidence

Tick only when the row's own `Accept:` passed, with the command it names (or a stronger one). The evidence is part of the tick:

- The exact command, its result and a count: `node tools/gh-check.mjs cargo test -p engine-game --features app --lib break_wire` then `5 passed`. A run id, or "on this PC" for GPU lines. `golden d57b5c8ae3096614 unchanged` when the row says so.
- A cargo test with 0 tests is not a pass; neither is a filter that matches no `fn`, `mod` or file.
- Any part of the Accept you did not do: do not tick. Log progress instead.

Steps:

1. Commit the code first. The sha is your own commit: `git log -1 --format=%h -- <one of your paths>`.
2. Edit the FIRST line of the row (`- [ ] ID`): `[ ]` to `[x]`, and append ` Landed <sha7>. <evidence>`. Indented lines stay; the row stays one bullet.
3. Use the Edit tool with the whole first line as `old_string`. Not `sed`: `/` in the evidence ends an `s///`, `&` and `[ ]` are regex, and pwsh eats backticks and `$`.
4. `node tools/workspace-check.mjs` and `node tools/board-slice.mjs <ID>` (prints, under 8 KB). `node tools/ticked-rows-audit.mjs` flags a ticked row whose crate, `--test` target, filter or `crates/` path went stale; it never runs cargo.
5. Commit the board file on its own: `board: <ID> landed` (batch: `board: tick audit batch 3 - GA13 GP40 ...`), body lists the proofs. If another session has hunks in that shard, use case B.

Partial: a row with more landings stays `[ ]`. Append ` Progress <date>: <what exists, test names, run ids>. Left: <what the Accept still needs>.` A later landing makes it `Landed`.

Blocked or wrong:

- Second identical failure: add `BLOCKED:` to the row and one line to `docs/BLOCKERS.md` (never read it whole). Never tick green, and never tick on a red or link-failed tree.
- A ticked row whose code is gone or whose proof was a skipped test: re-open it (`[x]` back to `[ ]`, with a `Progress` note why).
- Never write the word `needs:` in the evidence: the parser takes the first one. Evidence with a backticked path that does not exist is flagged by the audit: quote real paths only.

## Before you add a row

- `git grep` the function first. A ticked row or an existing named function means NOOP.
- Name the measured number it moves. No number, no row. The proof is a command that can fail: `Select-String` for a word is not proof.
- Write the stop (max attempts, files or minutes). At the stop the row ends PARTIAL; it does not grow.
- One row, one outcome, about 3 files or 200 lines. A research card becomes a row only when a consumer row exists.

## Case A in full

1. New files: `git add -N <path> ...` (intent to add). Without it `git commit -- <path>` says the path is unknown.
2. Check nobody else is in them: `git diff --numstat -- <paths>`. Unexpected numbers are foreign hunks: use case B.
3. Run `git diff --check -- <paths>` and read `git diff -- <paths>`. Confirm every hunk is yours and serves the request.
4. Write the message to a scratch file outside the repo, ending with the attribution line the session gives. Then `git commit -F <msgfile> -- <path1> <path2>`. `--` plus paths commits only those, whatever else is staged.
5. One row'\''s files go in one commit together (the `mod x;` line and `x.rs`), or the mirror export and the Pages build break.
6. `git pull --no-rebase --no-edit origin master`, then `git push origin master`. Rejected: pull again, push again. Never force.

## Checks before the commit

- `node tools/workspace-check.mjs` after any board, doc or manifest edit (ids, `needs:`, crate edges, LFS guard: a new non-LFS blob over 5 MB fails).
- Code: `rustfmt --edition 2021 --check <files>` (never `cargo fmt`), `node tools/rust-quality.mjs`, tests through `node tools/gh-check.mjs`. A refused commit: skill `rust-quality-ratchet`.
- New doc: its line goes into the index that lists it, in the same commit (`docs/INDEX.md`, `skills/SKILLS.md`). A new plan gets one line in `docs/plan/README.md`.
- The `needs:` parser rule and the ready test: see the Accept section above.
