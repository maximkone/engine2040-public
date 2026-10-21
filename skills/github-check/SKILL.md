---
name: github-check
description: "Use when you need a cargo test, clippy or check proof, a gate or CI check is red, a test binary fails to link, or the local cargo lock blocks: run node tools/gh-check.mjs on a GitHub runner, fix red the smallest way. Odd messages (exit 2 or 3, 0 tests, dispatch errors) are here too."
---

# GitHub is the workforce (owner 2026-10-17)

The PC has one cargo build lock. GitHub runners have none. Heavy proofs go to GitHub; the PC only iterates (`cargo check -p <crate>`).

Every cargo on this PC shares one compile-slot pool (`tools/jobserver.ps1 status`). OpenCode denies plain cargo test and clippy; `node tools/gh-check.mjs --local <line>` is the override.

## 1. Pick the form

| job | command |
|---|---|
| unit or integration test of a headless crate | `node tools/gh-check.mjs cargo test -p engine-net --lib <filter>` |
| lints of one crate | `node tools/gh-check.mjs cargo clippy -p <crate> --all-targets -- -D warnings` |
| does it compile | local `cargo check -p <crate>` to iterate; `node tools/gh-check.mjs cargo check -p <crate>` for the proof |
| `crates/game` look, arena, window, audio-device code | `node tools/gh-check.mjs cargo test -p engine-game --features app --lib <filter>`. Without `--features app` the app tests do not exist and the run shows 0 tests |
| render, window or GPU test | the same kind of line; the tool sends it to the PC (features `window`, `app`, `ui`, `device`; a filter with gpu, window, wgpu, adapter, vulkan, dx12) |
| browser build, headless Chrome | `node tools/web-run.mjs <paths>` (section 5) |
| the whole suite | publish to the mirror and read `node tools/public-export.mjs --ci` (section 5) |

One plain line only, starting `cargo check`, `cargo test` or `cargo clippy` (after `node tools/gh-check.mjs`). No `&&`, `;`, `|`, `>`, `<`, backticks or `$(`: the workflow refuses them. In pwsh quote a test filter with spaces.

## 2. How a run goes

- It builds your working tree (HEAD plus every dirty and untracked file), pushes it to a throwaway `wip-*` branch of the public mirror, runs the line on a runner and deletes the branch. Up to 20 run at once; none waits on the PC lock.
- Start one check and wait until it prints `sent wip-...`, then start the next. Calls starting in the same second share one mirror clone (`target/public-export/repo`) and can hit a ref-lock or snapshot error.
- A check takes 90 to 190 s, even for a 3 s test. The call returns at 100 s (`--budget <s>`, up to 1740) with exit 3 and `STILL RUNNING ... run: node tools/gh-check.mjs --wait wip-xxxx`. That is not a failure: do other work, then run that `--wait` line. Never rerun the cargo line, which starts a second run. `--budget 840` with `timeout: 900000` waits in one call.
- Exit codes: 0 passed, 1 failed (the tail is printed: read it and fix), 2 could not run, 3 still running.
- On auto the tool decides where a line runs:
  - a run that could not start is tried once more, then the same line runs on the PC (never `--workspace`);
  - lines that need Windows or a `window`, `app`, `ui` or `device` feature only to build go to a Windows runner: checks, clippy, `--no-run`, a test in a `cfg(windows)` file, tests that open no GPU or window;
  - only GPU, window and sound-device tests run on the PC;
  - reuse: the same line on unchanged inputs prints `REUSED PASS` or `REUSED FAIL` at once (a pass is kept 24 h, a fail 30 min; `--fresh` runs again); a line another AI is already running is joined;
  - `--only <path,path>` builds HEAD plus just those files (never kept for reuse): use it when another AI's unfinished file breaks the build;
  - a cargo test that passed with 0 tests reruns on the PC.
- `--remote-only` never falls back (exit 2). `--local` skips GitHub.

## 3. Messages and what to do

| you see | meaning | do |
|---|---|---|
| `refused: give one plain cargo ... command` | exit 2, the line has a shell operator or is not cargo | split it into separate calls |
| `no mirror clone yet: run node tools/farm.mjs once` | `target/public-export/repo` is missing | run `node tools/farm.mjs`, then retry |
| `dispatch failed (is check.yml on the mirror's main ...)` | the push worked, the workflow start did not | `gh auth status`; `node tools/farm.mjs` once; retry once |
| `could not run the check: ...` | the snapshot or the push failed (network, auth, a lock) | the tool already retried once and then ran the line on the PC; read the first line it printed |
| `the HEAD snapshot did not appear in 5 minutes` | another call is building the base snapshot for this HEAD | wait one minute, rerun once |
| `the runner gave no command output` | setup failed or the run was cancelled | the tool retries, then runs local |
| exit 2 with `no result in 30 minutes` | runner stuck | file the step as env (section 6), do not loop |
| exit 1 | the command failed, tail printed | read the tail, quote the error line, fix (section 6) |
| passed, `0 tests` | the filter matched nothing | the tool reruns on the PC; if still 0, fix the filter (name a real `fn`, `mod` or file) |
| a red in a file you did not write | the snapshot holds everyone's dirty files | say whose file; do not edit it |

A rejected `git push origin master` is a different thing (the private repo): skill `commit-and-board`.

## 4. What the runner cannot do

- The runner is Linux with no GPU and no window; lines with window, app, ui or device features run on a Windows runner, also with no GPU. A test that needs an adapter prints SKIPPED and still counts as passed. For render, window or crash proofs read the passed count and the SKIPPED lines (skill `test-and-proof`).
- Local lines run with cargo's own lock. `Blocking waiting for file lock` is another build: wait, do not start a second.
- GPU and arena tests are timing-sensitive. Run one alone (`run_in_background` or a long tool timeout), never beside a second one, and not while another cargo, `arena.exe` or a capture runs. `tools/product-check.mjs` marks such a run `LOADED`.
- It builds everyone's uncommitted files too. A red in a file you do not own is its owner's: say so, do not edit it.
- The tree goes to the PUBLIC mirror branch for the run (denylist and secret scan apply). Never leave a key, token or private note in it.
- Workspace-wide proofs (`--workspace`) are fine here and forbidden local.

## 5. The whole suite and the web run

- `node tools/public-export.mjs` builds and scans `target/public-export/tree` from `git archive HEAD`. Then `node tools/public-export.mjs --commit --push` publishes committed HEAD (it refuses without a fresh receipt; first time only, `--remote <url>`).
- GitHub then runs fmt, ratchet, gate tests, clippy, four test groups, the release proof, Windows and ARM. `node tools/public-export.mjs --ci` lists the last runs and the failed step. Job logs exist only after the whole run ends: `gh run view --repo maximkone/engine2040-public --job <id> --log`.
- Reproduce the `quick` job on committed HEAD inside `target/public-export/tree`: `node tools/rust-quality.mjs --no-cache`, `node tools/workspace-check.mjs`, `node --test "tools/**/*.test.mjs"`. Always `--no-cache`: the cache hid a ratchet regression for hours.
- `node tools/web-run.mjs [--budget <s>] [--file <tree path>=<local file>] <path> ...` runs the `web-build` workflow on a throwaway `web-build-*` branch: committed HEAD plus only the paths you name. Exit 0 ok, 1 failed, 2 could not run, 3 still running (run the printed `--wait web-build-xxxx`). Run `node tools/web-doctor.mjs` first. Skill `web-wasm`.
- Another session's half-done file in HEAD can turn the mirror red. Say whose file it is; do not edit it.

## 6. When it is red

1. Quote the failing step and the error line before any edit. Never guess from the check name.
2. Rerun the exact red command, same flags, same scope. Never swap in a greener check.
3. One smallest fix for one cause, in paths you own. No refactor, no extra files, no weakened assert or gate.
4. Rerun the same command; it must go green. Report the failing command and both exits.
5. Write the expected outcome before the rerun. No skipped legs, no wall-clock asserts (skill `test-and-proof`).
6. For the cause hunt (stacks, many errors at once) use skill `debug-triage`.

### A link error with a green check

A test binary fails with `rust-lld: undefined symbol` while `cargo check -p <crate>`, fmt and quality are green.
- That is a toolchain or environment fingerprint, not a code bug. Say so in your first reply.
- Do not revert, replan or abandon owned code on a link-red tree.
- File exactly one `docs/BLOCKERS.md` row with the fingerprint line (crate, symbol, command). Code rows wait until the env clears.
- Re-verify when the tree is quiet. Clean only by exact crate: `cargo clean -p <crate>`. Never delete under `target/` by hand, never `git clean` it.
- The same link FAIL twice ends the step. The third try is env-fix-first, never a third code try.
- A run that exits 124 (timeout on a heavy runner) counts as env, never as a code failure.

## 7. Stop and evidence

A second identical exit 1 ends the step (AGENTS.md): one row in `docs/BLOCKERS.md`, never green. Exit 2 means the tool already tried GitHub twice and the PC once: end `RESULT: PARTIAL` naming the command and the printed reason.

For the row (skill `commit-and-board`): the full command, the exit code, the passed count, the run id or "on this PC", and for web the page lines (skill `web-wasm`, section 8). The tree goes to a PUBLIC branch for the run: never leave a key, token or private note in a dirty file.
