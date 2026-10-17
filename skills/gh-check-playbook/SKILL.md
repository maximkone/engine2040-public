---
name: gh-check-playbook
description: "Use when you run node tools/gh-check.mjs or tools/web-run.mjs and need to pick the right form, or it printed exit 2 or 3, a push or dispatch error, 0 tests, STILL RUNNING, or a GPU test hangs: which command goes where and what each message means."
---

# gh-check playbook

Rules and exit codes are in skill `github-check`. This is the field guide: which form for which job, and what each odd message means.

## 1. Pick the form

| job | command |
|---|---|
| unit or integration test of a headless crate | `node tools/gh-check.mjs cargo test -p engine-net --lib <filter>` |
| lints of one crate | `node tools/gh-check.mjs cargo clippy -p <crate> --all-targets -- -D warnings` |
| does it compile | local `cargo check -p <crate>` first, `node tools/gh-check.mjs cargo check -p <crate>` for the proof |
| `crates/game` look, arena, window, audio-device code | add `--features app`: `cargo test -p engine-game --features app --lib <filter>`. Without it the app-gated tests do not exist and the run shows 0 tests. The tool runs `app` lines on the PC for you (the runner has no GPU) |
| render, window or GPU test | the tool sends it to the PC (features `window`, `app`, `ui`, `device`; a filter with gpu, window, wgpu, adapter, vulkan, dx12) |
| browser build, headless Chrome | `node tools/web-run.mjs <paths>` (section 4) |
| the whole suite, workspace-wide | publish to the mirror and read `node tools/public-export.mjs --ci` (skill `github-check`) |

One plain line only, starting `cargo check`, `cargo test` or `cargo clippy`. No `&&`, `;`, `|`, `>`, backticks or `$(`. In pwsh quote a test filter with spaces.

## 2. Start one, then the next

- Start one check and wait until it prints `sent wip-...`. Then start the next. Every call fetches and pushes through one shared mirror clone (`target/public-export/repo`); two calls starting in the same second are the likely cause of a ref-lock or snapshot error. After the `sent` line the runs go in parallel (up to 20) and several `--wait` lines can run side by side.
- A check takes 90 to 190 s. The call returns at 100 s with exit 3 and a line `STILL RUNNING ... run: node tools/gh-check.mjs --wait wip-xxxx`. That is not a failure. Run that exact `--wait` line. Do not rerun the cargo line: it starts a second run.
- A long one: `node tools/gh-check.mjs --budget 840 cargo test ...` with a tool timeout of 900000 ms waits in one call.

## 3. Messages and what to do

| you see | meaning | do |
|---|---|---|
| `refused: give one plain cargo ... command` | exit 2, the line has a shell operator or is not cargo | split it into separate calls |
| `no mirror clone yet: run node tools/farm.mjs once` | `target/public-export/repo` is missing | run `node tools/farm.mjs`, then retry |
| `dispatch failed (is check.yml on the mirror's main ...)` | the push worked, the workflow start did not | `gh auth status`; `node tools/farm.mjs` once; retry once |
| `could not run the check: ...` | the snapshot or the push failed (network, auth, a lock) | the tool already retried once and then ran the line on the PC; read the first line it printed |
| `the HEAD snapshot did not appear in 5 minutes` | another call is building the base snapshot for this HEAD | wait one minute, rerun once |
| `the runner gave no command output` | setup failed or the run was cancelled | tool retries, then runs local |
| exit 2 with `no result in 30 minutes` | runner stuck | file the step as env (skill `github-check`), do not loop |
| exit 1 | the command failed, tail printed | read the tail, quote the error line, fix (skill `github-check` "When it is red") |
| passed, `0 tests` | the filter matched nothing | the tool reruns on the PC; if still 0, fix the filter (name a real `fn`, `mod` or file) |
| a red in a file you did not write | the snapshot holds everyone's dirty files | say whose file; do not edit it |

A rejected `git push origin master` is a different thing (the private repo): pull again, push, never force (skill `commit-and-board`).

## 4. Runner vs local, and the browser run

- The runner is Linux, no GPU, no window. A GPU test that runs there prints SKIPPED and still counts as passed. For render, window or crash proofs read the passed count and the SKIPPED lines (skill `test-and-proof`).
- Local lines run with cargo's own lock. `Blocking waiting for file lock` is another build: wait, do not start a second.
- GPU and arena tests are timing-sensitive. They stall or time out when the PC is busy: another cargo or rustc running, `arena.exe` open, a capture. Run such a line alone, in the background (`run_in_background`) or with a long tool timeout, and never in parallel with a second one. `tools/product-check.mjs` marks a run `LOADED` for the same reason.
- `node tools/web-run.mjs [--budget <s>] [--file <tree path>=<local file>] <path> [<path> ...]`: runs the `web-build` workflow on a throwaway `web-build-*` branch. The tree is committed HEAD plus only the paths you name (a directory means every file in it; a path missing locally is removed). Other sessions' half-written crates stay out, unlike gh-check. Exit 0 ok, 1 failed, 2 could not run, 3 still running: run the printed `node tools/web-run.mjs --wait web-build-xxxx`. It needs the same mirror clone.
- Before a web run: `node tools/web-doctor.mjs` (what the PC lacks). The workflow does the real build; the PC need not have wasm-bindgen.

## 5. Evidence to write down

For the row (skill `tick-a-row`): the full command, the exit code, the passed count, the run id or "on this PC", and for web the page lines (skill `browser-test-on-this-pc`). Job logs of the mirror CI exist only after the whole run ends: `gh run view --repo maximkone/engine2040-public --job <id> --log`.

The tree goes to a PUBLIC branch for the run. Never leave a key, token or private note in a file you leave dirty.
