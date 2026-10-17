---
name: github-check
description: "Use for any cargo test, clippy or check proof in engine2040, or any build that waits on the local lock: send it to a GitHub runner with node tools/gh-check.mjs, and read the mirror CI with public-export --ci."
---

# GitHub is the workforce (owner 2026-10-17)

The PC has one cargo build lock. GitHub runners have none. Heavy proofs go to GitHub; the PC only iterates.

## One narrow command: `tools/gh-check.mjs`

- `node tools/gh-check.mjs cargo test -p <crate> --lib <filter>`
- `node tools/gh-check.mjs cargo clippy -p <crate> --all-targets -- -D warnings`
- `node tools/gh-check.mjs cargo check -p <crate>`
- ONE plain `cargo check|test|clippy` line. No `&&`, `;`, `|`, `>`, `<`, backticks or `$(`. The workflow refuses the rest.
- It builds your working tree (HEAD plus every dirty and untracked file), pushes it to a throwaway `wip-*` branch of the public mirror, runs the line on a runner and deletes the branch. Up to 20 run at once and none waits on the PC lock.
- A check takes about 90 to 190 s even for a 3 s test (checkout, toolchain, cache): one call per proof, never one per edit. Iterate with local `cargo check -p <crate>`.
- The call returns by itself after 100 s (`--budget <s>` changes it, up to 1740), before the 120 s tool timeout of OpenCode and Claude Code kills it. A check that is not done by then prints `STILL RUNNING ... run: node tools/gh-check.mjs --wait wip-...` and exits 3. That is not a failure: do other work, then run that `--wait` line (not the cargo line again, which would start a second run). `--budget 840` with `timeout: 900000` waits in one call.
- Exit codes: 0 = passed, 1 = failed (the output tail is printed: read it and fix), 2 = could not run, 3 = still running.
- On auto the tool decides where a line runs, so you never count retries: a run that could not start is tried once more, then the same line runs on the PC (never `--workspace`); GPU, window and Windows-only lines (features `window`, `app`, `ui`, `device`, `--all-features`, a gpu or window test filter, a test in a file that probes for an adapter or is `cfg(windows)`) run on the PC at once; a cargo test that passed with 0 tests reruns on the PC. `--remote-only` never falls back (exit 2), `--local` skips GitHub.

## What the runner cannot do

- It is Linux with no GPU and no window: tests that need an adapter print SKIPPED and pass, Windows-only code is not compiled. The tool sends the lines it can recognise to the PC itself (above); a skipping test it cannot recognise still shows as a pass, so for a render, window or crash proof read the passed count.
- It builds everyone's uncommitted files too. A red in a file you do not own is its owner's: say so, do not edit it.
- The tree goes to the PUBLIC mirror branch for the run (denylist and secret scan apply). Never leave a key, token or private note in the tree.
- Workspace-wide proofs (`--workspace`) are fine here and forbidden local.

## The whole suite: the mirror

- `node tools/public-export.mjs` (verify) then `node tools/public-export.mjs --commit --push` publishes the committed HEAD. GitHub then runs fmt, ratchet, gate tests, clippy, four test groups, the release proof, Windows and ARM in parallel.
- `node tools/public-export.mjs --ci` lists the last runs and the failed job and step. Job logs exist only after the whole run ends: `gh run view --repo maximkone/engine2040-public --job <id> --log`.
- Reproduce the `quick` job on committed HEAD inside `target/public-export/tree`: `node tools/rust-quality.mjs --no-cache`, `node tools/workspace-check.mjs`, `node --test "tools/**/*.test.mjs"`. Always `--no-cache`: the cache hid a ratchet regression for hours.
- Another session's half-done file in HEAD can turn the mirror red. Say whose file it is; do not edit it.

## Stop

A second identical exit 1 ends the step (AGENTS.md). Exit 2 means the tool already tried GitHub twice and the PC once: end `RESULT: PARTIAL` naming the command and the printed reason.
