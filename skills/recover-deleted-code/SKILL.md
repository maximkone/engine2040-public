---
name: recover-deleted-code
description: "Use when a file, module or test that a row, plan or old commit mentions is missing from the tree (the 2026-10-16 rot cleanup, a purge, a rewrite): find the commit that removed it, read the last copy, decide restore or rewrite."
---

# Recover deleted code

Nothing is lost: git has every file. On 2026-10-16 the rot cleanup removed 81 unwired Rust files (38,904 lines) in eight commits, most of them "auto: backup". The list with sizes and deleting commits is `docs/archive/2026-10-16-rot/RECOVERY.md` (table: path, lines, deleting commit, `exists again`). Verdict per group (stay deleted, restore at row X): `docs/plan/wiring-triage-2026-10-17.md` section 2.

## 1. Find it

- On the list? Open `docs/archive/2026-10-16-rot/RECOVERY.md`, take the deleting commit. Example: the physics LOD file was cut by `3d6ee4a1`.
- Not on the list: `git log --diff-filter=D --format="%h %s" -- <path>` shows the commit that deleted that path. Add `--all` if it may be on another ref.
- Path unknown, name known: `git log --diff-filter=D --name-only --format="%h" | Select-String <name>` (pwsh), or `git log --all --oneline -S "<symbol>"` finds commits that added or removed that text.
- Moved, not deleted? `git log --follow --format="%h %s" -- <new path>`. Also check for a rewrite: the list marks `exists again` yes when a smaller new file took the name (cloth, islands, query).

## 2. Read the last copy

- `git show <sha>^:<path>` prints it (`<sha>^` is the parent of the deleting commit). Check with `git show <sha>^:<path> | Select-Object -First 20`.
- Then read the header comment: it names the consumer. If the consumer is gone, the file was rot.
- Tests of the file lived in `crates/<c>/tests/it/` or in the file under `#[cfg(test)]`. The list names the test files too.

## 3. Decide: restore, rewrite, or leave

| case | do |
|---|---|
| A row or plan needs it and no live code does the job (`package.rs` for GM3, `look/metric.rs` for WEB5, `crowd.rs` for NV4) | restore, then wire it (step 4) |
| A smaller live file already covers it (Rapier replaced the solver stack) | leave deleted; say so in the row |
| The design changed (lagcomp with refused late inputs) | write the thin new thing; do not restore |
| You only want one function | copy that function, not the file |

Restoring is allowed (porting policy: owner 2026-10-10, any source, no license check). The rule is a real caller: a restored island comes back as dead code the next cleanup removes (skill `wiring-a-feature`).

## 4. Restore safely on the shared tree

1. Write the bytes without a shell re-encode. In pwsh, `>` and `Out-File` change line endings and encoding. Use a small temp `.mjs` outside the repo: `execFileSync("git", ["show", "<sha>^:<path>"])` as a Buffer, then `writeFileSync("<path>", buf)`.
2. Do not run `git checkout <sha>^ -- <path>` or `git restore`: they write the shared index and the working tree in one step.
3. Add the `mod` line in the crate's `lib.rs` (and in the integration-test main file (tests, module `it`) for a test file). The old file may name items that moved; fix names, do not edit tests to pass.
4. Local `cargo check -p <crate>` first, then `node tools/gh-check.mjs cargo test -p <crate> --lib <filter>` (skill `github-check`).
5. Run `node tools/rust-quality.mjs --crate <c>`. An old file may carry debt (casts, long fns) that the ratchet refuses (skill `rust-quality-ratchet`).
6. Update `docs/archive/2026-10-16-rot/RECOVERY.md`: set `exists again` to yes for the path, in the same commit.
7. Commit the paths you own (skill `commit-and-board`). The message names the source: `restore <path> from <sha>^`.

## 5. Traps

- The deleting commit is often an `auto: backup` that swept many files. `git show <sha> --stat` lists them all; pick only what you need.
- A file deleted and rewritten under the same name: `git show <sha>^:<path>` is the OLD one. Compare with the live file before you overwrite it.
- A ticked board row can point at a path that is gone. `node tools/ticked-rows-audit.mjs` lists those rows. Re-open the row or restore, do not leave it lying.
- Never delete files under `target/` to "clean up" while you work (skill `commit-and-board`).
