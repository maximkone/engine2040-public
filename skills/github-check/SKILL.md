---
name: github-check
description: "Use when you need a cargo test, clippy or check proof, a gate or CI check is red, a test binary fails to link, or the local cargo lock blocks: run node tools/gh-check.mjs on a GitHub runner, fix red the smallest way. Odd messages: gh-check-playbook."
---

# GitHub is the workforce (owner 2026-10-17)

The PC has one cargo build lock. GitHub runners have none. Heavy proofs go to GitHub; the PC only iterates.

## One narrow command: `tools/gh-check.mjs`

Which form for which job, and what each odd message means (exit 2 or 3, 0 tests, push or dispatch errors, GPU lines, web-run): skill `gh-check-playbook`.

- `node tools/gh-check.mjs cargo test -p <crate> --lib <filter>`
- `node tools/gh-check.mjs cargo clippy -p <crate> --all-targets -- -D warnings`
- `node tools/gh-check.mjs cargo check -p <crate>`
- ONE plain `cargo check|test|clippy` line. No `&&`, `;`, `|`, `>`, `<`, backticks or `$(`. The workflow refuses the rest.
- It builds your working tree (HEAD plus every dirty and untracked file), pushes it to a throwaway `wip-*` branch of the public mirror, runs the line on a runner and deletes the branch. Up to 20 run at once and none waits on the PC lock.
- A check takes about 90 to 190 s even for a 3 s test (checkout, toolchain, cache): one call per proof, never one per edit. Iterate with local `cargo check -p <crate>`.
- The call returns by itself after 100 s (`--budget <s>` changes it, up to 1740), before the 120 s tool timeout of OpenCode and Claude Code kills it. A check not done by then prints `STILL RUNNING ... run: node tools/gh-check.mjs --wait wip-...` and exits 3. That is not a failure: do other work, then run that `--wait` line (not the cargo line again, which would start a second run). `--budget 840` with `timeout: 900000` waits in one call.
- Exit codes: 0 = passed, 1 = failed (the output tail is printed: read it and fix), 2 = could not run, 3 = still running.
- On auto the tool decides where a line runs, so you never count retries:
  - a run that could not start is tried once more, then the same line runs on the PC (never `--workspace`);
  - lines that need Windows or a `window`, `app`, `ui` or `device` feature (or `--all-features`) only to build go to a Windows runner: every check and clippy, `--no-run`, a test in a `cfg(windows)` file, and tests that open no GPU or window;
  - only GPU, window and sound-device tests run on the PC (a gpu or window test filter or target, a test in a file that probes for an adapter, a `device`-feature test);
  - reuse: the same line on unchanged inputs (its crates with their dependencies, the manifests, the lock, `content/`, `fixtures/`, any path its code names outside them) prints `REUSED PASS` or `REUSED FAIL` at once (a pass is kept 24 h, a fail 30 min; `--fresh` runs it again), and a line another AI is already running is joined, not run twice;
  - `--only <path,path>` builds HEAD plus just those files: use it when another AI's unfinished file breaks the build (never kept for reuse);
  - a cargo test that passed with 0 tests reruns on the PC.
- `--remote-only` never falls back (exit 2). `--local` skips GitHub. In OpenCode a plain `cargo test`, `cargo clippy` or `cargo nextest` is denied; `--local` is the way to run such a line on this PC when it really must (pool, low priority, kept results).

## What the runner cannot do

- The default runner is Linux; lines with a window, app, ui or device feature run on a Windows runner. Neither has a GPU or a desktop. Tests that need an adapter print SKIPPED and pass. On Linux, Windows-only code is not compiled. The tool sends the lines it can recognise to the PC itself. A skipping test it cannot recognise still shows as a pass, so for a render, window or crash proof read the passed count (skill `test-and-proof`).
- It builds everyone's uncommitted files too. A red in a file you do not own is its owner's: say so, do not edit it.
- The tree goes to the PUBLIC mirror branch for the run (denylist and secret scan apply). Never leave a key, token or private note in the tree.
- Workspace-wide proofs (`--workspace`) are fine here and forbidden local.

## The whole suite: the mirror

Get your tree to the mirror (the one copy of this text):
- `node tools/public-export.mjs` first. It builds and scans `target/public-export/tree` from `git archive HEAD` and writes a verify receipt.
- Then `node tools/public-export.mjs --commit --push` publishes the committed HEAD. `--commit` refuses without a fresh receipt for this HEAD. First time only: `--remote <url>`.
- GitHub then runs fmt, ratchet, gate tests, clippy, four test groups, the release proof, Windows and ARM in parallel.
- `node tools/public-export.mjs --ci` lists the last runs and the failed job and step. Job logs exist only after the whole run ends: `gh run view --repo maximkone/engine2040-public --job <id> --log`.
- Reproduce the `quick` job on committed HEAD inside `target/public-export/tree`: `node tools/rust-quality.mjs --no-cache`, `node tools/workspace-check.mjs`, `node --test "tools/**/*.test.mjs"`. Always `--no-cache`: the cache hid a ratchet regression for hours.
- Another session's half-done file in HEAD can turn the mirror red. Say whose file it is; do not edit it.

## When it is red

1. Quote the failing step and the error line before any edit. Never guess from the check name.
2. Rerun the exact red command, same flags, same scope. Never swap in a greener check.
3. One smallest fix for one cause, in paths you own. No refactor, no extra files, no weakened assert or gate. A red in a file you do not own is its owner's.
4. Rerun the same command. It must go green. A different green check proves nothing. Report the failing command and both exits.
5. Write the expected outcome before the rerun. No skipped legs, no wall-clock asserts (skill `test-and-proof`).
6. For the cause hunt (stacks, many errors at once) use skill `debug-triage`.

### A link error with a green check

A test binary fails with `rust-lld: undefined symbol` while `cargo check -p <crate>`, fmt and quality are green.
- That is a toolchain or environment fingerprint, not a code bug. Say so in your first reply.
- Do not revert, replan or abandon owned code on a link-red tree.
- File exactly one `docs/BLOCKERS.md` row with the fingerprint line (crate, symbol, command). Code rows wait until the env clears.
- Re-verify when the tree is quiet. Clean only by exact crate: `cargo clean -p <crate>`. Never delete under `target/` by hand and never `git clean` it.
- The same link FAIL twice ends the step. The third try is env-fix-first, never a third code try.
- A run that exits 124 (timeout on a heavy runner) counts as env, never as a code failure.

## Stop

A second identical exit 1 ends the step (AGENTS.md): one row in `docs/BLOCKERS.md`, never green. Exit 2 means the tool already tried GitHub twice and the PC once: end `RESULT: PARTIAL` naming the command and the printed reason.
