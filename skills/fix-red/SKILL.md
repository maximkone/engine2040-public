---
name: fix-red
description: "Use when a test or a gate is red and you have to fix it: reproduce the red once, find the file that owns it, write the smallest patch, prove it with the row's own command, land it by path."
---

# Fix red

Iron law: NO COMMIT WITHOUT THE RED SEEN FIRST. A patch whose test never failed is a guess.

The shape is Agentless: reproduce, localize, patch, prove, land. One cause at a time.

## 1. Reproduce

1. Take the exact command from the row's Accept line. Never swap it for a greener one.
2. Run it on a runner: `node tools/gh-check.mjs --budget 840 cargo test -p <crate> --lib <filter>`. Gh-check flags go before the cargo line.
3. Exit 0 passes, 1 fails with the tail printed, 2 could not run, 3 is still running: run the printed `node tools/gh-check.mjs --wait wip-xxxx` line. Never start the cargo line twice.
4. The same line on unchanged inputs answers from a kept result. Use `--fresh` only when you must see it again.
5. Keep the red output: the error line, the test name, the file and the line of the panic. Quote it in the report.
6. A filter that matches no test shows 0 tests. Fix the filter, never weaken the test.

## 2. Localize

1. The panic file and line own the failure. `docs/CRATE-MAP.md` names the crate that owns a file.
2. `node tools/affected.mjs --files <path>` maps the file to its crate and prints the closure to retest.
3. `git log -L <start>,<end>:<file>` shows who last changed those lines. `git log -1 -- <file>` shows the last commit on the file.
4. A red in a file you do not own belongs to its owner. Say whose file it is, do not edit it.

## 3. Patch

1. Patch only the file that owns the failure, and only the lines that cause it.
2. One cause, one change. No refactor, no new file, no weakened assert, no `#[allow]`. Use `#[expect(reason = "...")]` if a lint must be expected.
3. New behavior needs a test first (skill `test-and-proof`).
4. The machine can patch part of this: row AF1 adds the mechanical-fix tool for the paths you own. It is not in the tree yet, so hand-edit until it lands.

## 4. Prove

1. Rerun the same command with the same flags and the same scope. It must go green.
2. Then the crate's lib tests: `node tools/gh-check.mjs --budget 840 cargo test -p <crate> --lib`.
3. Local cheap gates: `node tools/rust-quality.mjs --owned <your paths>` and `rustfmt --edition 2021 --check <your files>`.
4. After a board or doc edit: `node tools/workspace-check.mjs`.
5. Sim code changed: `node tools/gh-check.mjs --budget 840 cargo test -p engine2040 --release --test proof_cli default_and_max_seed` must print d57b5c8ae3096614.
6. The proof is the row's own command. A greener command proves nothing (skill `github-check`).

## 5. Land

1. Commit by path: `git commit -F <msgfile> -- <paths>`. Never `git add -A`, never someone else's file.
2. `git fetch origin master`, then `git merge --no-edit origin master`. No rebase, no amend, no force-push, no autostash.
3. `git push origin master`.
4. Tick the row: `- [ ]` becomes `- [x]`, and ` Landed <sha7>. <evidence>` goes at the end of the row's first line. Commit the tick by path too (skill `commit-and-board`).
5. The evidence names the command, the exit code, the passed count and the run id. Every path in it exists.

## 6. When it stays red

- Retry once with a diagnosis. A second identical failure ends the step: one row in `docs/BLOCKERS.md`, never green (AGENTS.md).
- The cause hunt is skill `debug-triage`. A moved hash is skill `determinism-debug`.
