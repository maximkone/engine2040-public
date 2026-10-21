# Tools reference: tools/*.mjs

Scope: the scripts in `tools/` that a developer or an AI runs by hand, from CI, or from a git hook, plus the JSON data files they read. Facts come from each file's header, argv handling and exit paths, read on 2026-10-17. Run every command from the repo root `C:\engine2040` as `node tools/<name>.mjs`. CI runs Node 24. Tests: `node --test tools/<name>.test.mjs`.

"see code" in the table means the header and a quick read did not state the exit path. Read `main()` before relying on it.

## 1. Table of scripts

| Tool | Purpose | Test file | Exit codes |
|---|---|---|---|
| `gh-check.mjs` | One cargo check, test or clippy line on a GitHub runner | `gh-check.test.mjs` | 0 pass, 1 fail, 2 could not run, 3 still running |
| `workspace-check.mjs` | Workspace audit before landing | `workspace-check.test.mjs` | 0 pass, 1 audit FAIL |
| `proof.mjs` | Local gate runner: fmt, build, tests, clippy, fixtures, `--perf`, `--mutants`, `--drift` | `proof.test.mjs` | 0 green, 1 fail, 2 bad arguments |
| `rust-quality.mjs` | Rust quality scorecard and ratchet | `rust-quality.test.mjs` | 0 pass, 1 regression, 2 error |
| `docs-coverage.mjs` | Public-item doc coverage ratchet | `docs-coverage.test.mjs` | 0 pass, 1 regression, 2 error |
| `wasm-gate.mjs` | Std calls on the web path that fail in a browser | `wasm-gate.test.mjs` | 0 pass, 1 regression, 2 error |
| `golden-matrix.mjs` | Compares golden tapes and corpus or ragdoll rows | `golden-matrix.test.mjs` | 0 agree, 1 divergence or skip, 2 usage |
| `web-build.mjs` | Builds `dist/web` | `web-build.test.mjs` | 0 complete, 1 step failed, 2 bad call |
| `web-browse.mjs` | Headless Chrome check of `dist/web` | `web-browse.test.mjs` | 0 proven, 1 proof failed, 2 cannot run, 3 no adapter with `--require-adapter` |
| `web-doctor.mjs` | Lists what the web target needs | `web-doctor.test.mjs` | 0 READY, 1 NOT READY |
| `wasm-info.mjs` | `.wasm` sections as JSON | `wasm-info.test.mjs` | see code |
| `potato-check.mjs` | Potato gate G9 run of the arena | `potato-check.test.mjs` | 0 ok, 1 floor red (`--floor`), 2 no iGPU evidence or bad bar, 3 withheld on a loaded box |
| `product-check.mjs` | Built-game floors from captures and timing | `product-check.test.mjs` | 0 green, 1 floor broke, 2 not run |
| `finish.mjs` | FINISH-RICH gate scoreboard | `finish.test.mjs` | 0 printed, 2 missing input file |
| `board-slice.mjs` | One board row, ready and focus lists | `board-slice.test.mjs` | 0 ok, 1 unknown row or self-check FAIL, 2 usage |
| `plan-atlas.mjs` | Generated plan map and queries | `plan-atlas.test.mjs` | 0 ok, 1 unknown or empty, 2 usage |
| `ticked-rows-audit.mjs` | Ticked rows whose test, crate or path is gone | `ticked-rows-audit.test.mjs` | 0 none, 1 suspect, 2 usage or git failure |
| `part-score.mjs` | One live line per focus front | `part-score.test.mjs` | see code |
| `mp-evidence.mjs` | Newest MP and ON evidence per area | `mp-evidence.test.mjs` | 0 help, 2 usage, else see code |
| `mp6-check.mjs` | Checks the six MP6 receipts | `mp6-check.test.mjs` | 0 hold, 1 fail, 2 missing or not JSON |
| `p4-netsim.mjs` | 64-client network model for gate P4 | `p4-netsim.test.mjs` | 0 pass, 1 fail, 2 usage |
| `secret-gate.mjs` | Secret scan of the staged diff | `secret-gate.test.mjs` | 0 clean, 1 secrets, 2 bad arguments |
| `license-gate.mjs` | Unlicensed packages, repo licence files, header lint | `license-gate.test.mjs` | 1 when unlicensed count > 0 |
| `arch-fitness.mjs` | Crate graph cycle check | `arch-fitness.test.mjs` | always 0 (section 14) |
| `feature-unify.mjs` | Features inside the workspace-hack union | `feature-unify.test.mjs` | 0 pass, 1 FAIL |
| `crate-map.mjs` | Writes or checks `docs/CRATE-MAP.md` | `crate-map.test.mjs` | 0, 1 with `--check` when stale, 2 usage |
| `affected.mjs` | Changed packages and their tool tests | `affected.test.mjs` | 0, 1 on error |
| `bins.mjs` | Release binaries built after the last code commit | `bins.test.mjs` | 0 all fresh, 1 stale or missing |
| `cargo-guard.mjs` | One cargo command with time and message caps | `cargo-guard.test.mjs` | child code; 2 usage, 124 time cap, 127 spawn failed |
| `compile-cache.mjs` | Warm compiler-cache plan | `compile-cache.test.mjs` | see code |
| `compile-cache-spike.mjs` | Cold lane build vs a warm sccache in fresh dirs (KEEP/DROP) | `compile-cache-spike.test.mjs` | 0 measured or help, 2 usage or a failed build |
| `clean.mjs` | Removes trash under `target/` | `clean.test.mjs` | 2 bad arguments |
| `doctor.mjs` | Health report and consented repair | `doctor.test.mjs` | 0 healthy or healed, 1 needs a person, 2 bad consent |
| `seal-keys.mjs` | Makes seal keys, runs a command with them | `seal-keys.test.mjs` | child code via `--`; 2 bad arguments |
| `farm.mjs` | Exports HEAD to the mirror and waits for CI | `farm.test.mjs` | 0 green, 1 red, 2 no run, timeout or all skipped |
| `public-export.mjs` | Snapshot of HEAD to the public mirror; `--ci` | `public-export.test.mjs` | 0 ok, 1 not exported, not committed or failed |
| `ship.mjs` | Release pack, stage and clean-room run | `ship.test.mjs` | see code (1 failed step, 2 refused or usage) |
| `mutants.mjs` | cargo-mutants on a patch | `mutants.test.mjs` | 0 PASS, 1 FAIL, 2 usage or tool error |
| `books.mjs` | BM25 search over book text | `books.test.mjs` | 1 or 2 on error |
| `gh-search.mjs` | Shell-safe GitHub search | `gh-search.test.mjs` | 0 ok, 1 self-check FAIL |
| `web-search.mjs` | Shell-safe web search | `web-search.test.mjs` | 0 ok, 1 self-check FAIL |
| `cook-char.mjs` | Character manifest check; Blender cook | `cook-char.test.mjs` | 0 ok, 1 manifest FAIL, 2 refused or no Blender, 3 cook failed |
| `gltf-intake.mjs` | glTF intake with CC0 licence gate | `gltf-intake.test.mjs` | 0 ok, 2 named refusal, 1 crash |
| `fetch-content.mjs` | Downloads listed content with md5 | `fetch-content.test.mjs` | 0 ok, 1 md5 mismatch, 2 usage |
| `city-osm.mjs` | OSM slice to glTF | `city-osm.test.mjs` | 2 on error, else see code |
| `steal-land.mjs` | Next steal line, donor fetch, gate | `steal-land.test.mjs` | 1 gate FAIL, 2 usage, else see code |
| `mcp-install.mjs` | Copies `e2040-mcp` to `.opencode/bin/` | `mcp-install.test.mjs` | 0 help, 1 failed, 2 bad arguments |
| `level-check.mjs` | Rome arena layout gate | `level-check.test.mjs` | 0 ok, 1 FAIL, 2 usage or missing input |
| `beat-lint.mjs` | Ladder beat chart gate | `beat-lint.test.mjs` | 0 pass, 1 fail, 2 usage |
| `wiring-scan.mjs` | Pub items with no production user | `wiring-scan.test.mjs` | 0, 1 with `--check` when count rises, 1 with `--diff` on a new unwired item |
| `wiring-cov.mjs` | Zero-hit functions from cargo-llvm-cov | `wiring-cov.test.mjs` | 0 report, 1 cargo step failed, 2 tool missing or flags |
| `bench-compare.mjs` | Warm-up, compare, export for benches | `bench-compare.test.mjs` | 0 pass, 1 fail, 2 usage |
| `tape-checksum.mjs` | Per-tick FNV-1a64 chain, first divergence | `tape-checksum.test.mjs` | 0 printed, 2 usage or error |
| `desync-diff.mjs` | Desync report over canonical bytes | `desync-diff.test.mjs` | 1 divergence, 2 usage or error |
| `replay-to-mp4.mjs` | Tape to MP4 through ffmpeg | `replay-to-mp4.test.mjs` | 2 no ffmpeg or bad call, 1 failure, 0 ok |
| `showcase.mjs` | PNG sequence to MP4 and GIF | `showcase.test.mjs` | see code |
| `frame-moved.mjs` | Did the look move between captures | `frame-moved.test.mjs` | 2 on error, else see code |
| `frame-triage.mjs` | Fighter-pixel triage of showcase frames | `frame-triage.test.mjs` | 1 triage fail, else see code |
| `frame-v2v3.mjs` | Sky and motion heuristics on orbit captures | `frame-v2v3.test.mjs` | see code |
| `fighter-count.mjs` | Fighter-vs-box pixel counter | `fighter-count.test.mjs` | 0 ok, 1 failure, 2 bad arguments |
| `q1-check.mjs` | Read-only check of `target/q1` | `q1-check.test.mjs` | 0, 1 when gaps exist |
| `ra8-check.mjs` | Read-only RA8 manifest presence check | `ra8-check.test.mjs` | see code |
| `ra8-manifest.mjs` | RA8 look manifest from real PNGs | `ra8-manifest.test.mjs` | 0 ok, 1 error, 2 bad arguments |
| `iqa-gate.mjs` | SSIM gate for a reference and a capture PNG | `iqa-gate.test.mjs` | 0 pass, 1 not pass or selfcheck fail, 2 error |
| `rdc-diff.mjs` | Pixel diff for RenderDoc texture PNGs | `rdc-diff.test.mjs` | 0 help or equal, 2 bad arguments |
| `rerun-timeline.mjs` | Diffs two per-tick JSON timelines | `rerun-timeline.test.mjs` | 1 self-check FAIL, 2 usage |
| `playtest-log.mjs` | Checks a playtest CSV | `playtest-log.test.mjs` | see code |
| `pe-exports.mjs` | Lists PE export names | `pe-exports.test.mjs` | 2 usage or error |
| `physfx-backends.mjs` | RZ1 soak on Rapier wasm and Jolt wasm, one table | `physfx-backends.test.mjs` | 0 both backends ran, 1 a backend failed to load or step, 2 usage |
| `flaky-quarantine.mjs` | Quarantines tests that fail repeatedly | `flaky-quarantine.test.mjs` | 0 ok, 1 fail, 2 usage |
| `duration-fmt.mjs` | Duration formatter CLI | `duration-fmt.test.mjs` | 0 pass, 1 fail, 2 usage |
| `argv.mjs` | Library: shell-safe spawn and CLI failure helpers | `argv.test.mjs` | not a CLI |
| `sim-source-scan.mjs` | Library: Rust source scans for `workspace-check` | `sim-source-scan.test.mjs` | not a CLI |

Library modules without a CLI are in section 11.

Conventions: run from the repo root; named refusals (`E_LICENCE`, `E_INPUT_ABSENT`, ...) go to stderr with exit 2 unless a section says otherwise; ratchets read baselines under `tools/` and counts only go down; cargo proofs run on GitHub through `gh-check.mjs`.

## 2. Gates and proofs

### gh-check.mjs

- Purpose: runs one plain cargo line on a GitHub runner against the working tree (HEAD plus dirty and untracked files, minus the mirror DENY list).
- Run: `node tools/gh-check.mjs cargo test -p <crate> --lib <filter>`; `cargo clippy -p <crate> --all-targets -- -D warnings`; `cargo check ...`; `--wait wip-<id>`.
- Flags: `--budget <s>` (1 to 1740; default 100 or env `GH_CHECK_BUDGET`); `--remote-only` (no PC fallback); `--local` (skip GitHub).
- Refuses any line that is not one `cargo check|test|clippy` with no shell operators (`& ; | \` < > $(`): `refused: give one plain cargo check|test|clippy ...`, exit 2.
- Runs on the PC instead when the line has feature `window`, `app`, `ui` or `device`, or `--all-features`, or a test filter or target with `gpu`, `window`, `wgpu`, `winit`, `swapchain`, `adapter`, `vulkan`, `dx12` or `win32`, or the package has a source file that probes an adapter or is `cfg(windows)`. Prints `local, not GitHub: <reason>`.
- Remote path: snapshot from `target/public-export/repo`, overlay of the working tree, throwaway `wip-<id>` branch of `maximkone/engine2040-public`, dispatch of `check.yml`, poll, print the last 70 log lines, delete the branch.
- Output: `sent wip-...`; `[<n>s] PASS on the runner: <cmd> (run <url>)` or `FAIL`; `STILL RUNNING on the runner, not a failure` with the `--wait` line.
- A pass with 0 tests prints `PASS but 0 tests ran on the runner` and reruns on the PC (not with `--remote-only`).
- Exit: 0 pass, 1 fail, 2 could not run (no mirror clone, dispatch failed, no output after one retry), 3 still running.
- `--workspace` and `--all` are refused on the PC.
- Needs the `gh` CLI and the mirror clone. Without a clone it prints `no mirror clone yet: run node tools/farm.mjs once`.

### workspace-check.mjs

- Purpose: audit before landing. Header lists: Cargo metadata, resolved dependency graph, module reachability, sim determinism, genre ledger, baseline gate, board needs, new LFS blobs.
- Run: `node tools/workspace-check.mjs`. `--help` or `-h` prints help, exit 0.
- Pass lines: `PASS steal ledger: ...`; `PASS workspace: <n> packages share version/edition/license/rust-version`; `PASS dependency edges and std-only engine-core`; `PASS default headless graph: <n> workspace packages, no external dependencies`; `PASS modules: all <n> source files are compiled`; `PASS board: <n> tasks in docs/SPRINT.md, ids unique, every needs resolves`; `PASS lfs guard: no newly tracked non-LFS blob over 5 MB`; `PASS test layout: <n> crates keep one integration-test binary (tests/it)`; `PASS maps: <n> crates, <n> paths match the tree`.
- Fail: `[FAIL] ...` lines on stdout; `FAIL workspace audit: <message>` on stderr.
- Exit: 0 pass, 1 FAIL.
- Edges come from the `allowedEdges` Map in the file. `engine-core` is universal.

### proof.mjs

- Run: `node tools/proof.mjs` (full); `--skip-build` (fixtures only); `--perf` (timing tests, release, one thread, plus instruction counts on Linux); `--mutants` (cargo-mutants against HEAD); `--drift` (reads the green records); `--help` or `-h`.
- A run that ends GREEN writes `target/proof/full-latest.json` (`fixtures-latest.json` for `--skip-build`) with the sha and time; a red run leaves the old record alone. `--drift` FAILs a record that is 25 h old or whose sha is neither HEAD nor an ancestor, and FAILs when no record exists.
- Static legs in both modes: the rust quality ratchet, the docs coverage ratchet and `mdbook build docs/book` (green only with no WARN or ERROR line; a `[SKIP]` line when mdbook is not installed).
- A cargo leg whose filter matches no test is RED (`zero tests ran`). The gpu-live line prints `not_wired` unless a `target/reports/pass-truth-<date>.md` device report carries live rows.
- Other arguments print the usage line and exit 2.
- Output: `RESULT PASS (all local gates)` or `RESULT PASS (proof fixtures only)` or `RESULT FAIL`. Perf: `RESULT PASS (timing tests, release, one thread)`, `RESULT FAIL (PQ1 input latency)`, `RESULT FAIL (frame times)`, `RESULT FAIL (instruction counts)`. Mutants: `RESULT PASS (mutants: ...)` or `RESULT FAIL (mutants)`.
- Exit: 0 green, 1 fail, 2 bad arguments.
- Help text lists: nextest profile `proof` (`.config/nextest.toml`), partitioned runs, gungraun (Linux, 2% Ir budget, baseline `perf_gate`), fuzz (`engine-proof` feature `fuzz`).
- Default run is local cargo. AGENTS says heavy cargo goes to `gh-check.mjs`.

### rust-quality.mjs

- Run: `node tools/rust-quality.mjs`; `--crate <name>` (repeatable); `--list <metric> [--limit N]` (default limit 40); `--json`; `--owned <csv>`; `--init` (writes baseline once); `--tighten`; `--accept <crate> --reason "<text>"` (reason at least 10 characters); `--no-cache`; `--help` or `-h`.
- Output: `RESULT PASS: score <n> (<grade>), 0 ratchet regressions over <n> crate(s)`; `RESULT FAIL: <n> ratchet regression(s)`; `RESULT tightened <n> entries`; `RESULT <n> <metric> hit(s)`; `WROTE tools/rust-quality.baseline.json (<n> crates)`.
- Exit: 0 pass, 1 regression, 2 error.
- Scores production code only: test modules, `tests/`, `benches/`, `examples/` are excluded except `tests` and `board_ids`.
- Lists `SIM_CRATES` and `HOT_CRATES` at the top of the file. Used by the pre-commit hook (section 14).

### docs-coverage.mjs

- Run: `node tools/docs-coverage.mjs`; `--crate <name>` (repeatable; `engine-` prefix optional); `--json`; `--update-baseline` (writes baseline, prints `WROTE <path> (<n> crates)`); `--root <dir>`; `--limit N` (undocumented items listed per crate, default 5).
- Output: a table, then `[FAIL] undocumented <item>`, `[FAIL] ratchet ...`, `[INFO] ...`, then `RESULT PASS: 0 coverage regressions over <n> crate(s)` or `RESULT FAIL: <n> coverage regression(s)`.
- No baseline: writes one and prints `[INFO] no baseline found; recorded current coverage to ...`.
- Exit: 0 pass, 1 regression, 2 error.
- Baseline: `tools/docs-coverage.baseline.json`.

### wasm-gate.mjs

- Finds calls that fail or panic in a browser (`std::fs`, `Instant::now`, `SystemTime::now`, threads, sockets) in non-test source of the `WEB_CRATES` list in the file.
- Run: `node tools/wasm-gate.mjs`; `--list` (every hit as `file:line`); `--tighten`; `--init` (first baseline; refuses when one exists); `--help`.
- Skips `tests/`, `benches/`, `examples/`, `tests.rs`, `*_tests.rs`, and items whose `#[cfg(...)]` is false for `wasm32-unknown-unknown`.
- Output: `RESULT PASS: 0 ratchet regressions over <n> file(s) with hazards`; `RESULT FAIL: <n> ratchet regression(s)`; `RESULT tightened <n> count(s)`; `RESULT wrote tools/wasm-gate.baseline.json for <n> file(s)`.
- Exit: 0 pass, 1 regression, 2 error. Baseline: `tools/wasm-gate.baseline.json`.

### golden-matrix.mjs

- Compares golden tapes, corpus rows and ragdoll rows across machines. Writes nothing.
- Run: `node tools/golden-matrix.mjs <a.e2040> <b.e2040> [c ...] [--rows <a.txt> <b.txt> [c.txt ...]] [--ragdoll <a.txt> <b.txt> [c.txt ...]]`. Tapes come first. Each group after a flag needs two or more files.
- Tape format: 28-byte little-endian header: magic `E2040RP\0`, u16 format (1), u16 fixture (1), u64 seed, u32 ticks, u32 input count; then 16 bytes per input (u32 tick, u32 body, f32 vx, f32 vy); then 16 bytes per checkpoint (u64 legacy, u64 state). Length must match exactly.
- Output per file: `MATCH <name> vs <first>`, `DIVERGE <name> vs <first> (<host> vs <host>): <note>`, `SKIP <name>: <note>`; then `RESULT PASS|FAIL`, `CORPUS RESULT PASS|FAIL` or `RAGDOLL RESULT PASS|FAIL`.
- Exit: 0 agree, 1 divergence or skipped pair, 2 usage or unreadable file.
- Producers: `engine2040 --seed <s> --ticks <n> --record <path>`; `matrix_line` and `ragdoll_line` bins in `crates/proof`.

### secret-gate.mjs

- Scans added lines of the staged diff for secret patterns and entropy above 3.8.
- Run: `node tools/secret-gate.mjs`; `--self-test` (seeds one temp secret and checks it is counted); `--help`.
- Output: `PASS: <n> staged files scanned, 0 secrets`; `FAIL: <n> secrets in <n> staged files`; `PASS: self-test detected <n> seeded secret in 1 file`.
- Exit: 0 pass, 1 secrets or self-test failed, 2 bad arguments.
- The pre-commit hook runs it on every commit (section 14); `node tools/secret-gate.mjs` scans on demand.

### license-gate.mjs

- Leg 2 of the licence gate. Run by the local `pre-push` hook after `cargo deny check licenses`.
- Counts unlicensed packages (fail), unused allow entries (warn); checks repo licence files and source headers.
- Run: `node tools/license-gate.mjs`. Exit 1 only when unlicensed count is above 0.

### arch-fitness.mjs

- Acyclic crate graph. Reads the allow-list rows from `tools/workspace-check.mjs` and runs a DFS.
- Run: `node tools/arch-fitness.mjs`. Prints `PASS dependency cycle` or `FAIL cycle: a -> b -> a`.
- Exit is always 0 (section 14).
- Exports: `findCycle(edges)`, `loadAllowedEdges(repoDir)`.

### feature-unify.mjs

- Every feature of a workspace crate must sit inside the union in `crates/workspace-hack/Cargo.toml`.
- Run: `node tools/feature-unify.mjs`. Output `PASS feature-unify: every feature inside the union` or `FAIL feature-unify:` plus lines. Exit 0 or 1.

## 4. Board, plans and docs

### board-slice.mjs

- Reads `docs/SPRINT.md` (index) plus every `docs/SPRINT-*.md` shard, and done rows from `docs/archive/SPRINT-done-*.md`.
- Run: `node tools/board-slice.mjs <ID> [--json]`; `--next [<text>] [--json]`; `--ready [--open]`; `--focus [--open]`; `--find <text>`; `--self-check`; `--help`.
- Exit: 0 ok, 1 unknown row or self-check FAIL, 2 usage.
- A row is ready when open, every `needs:` is ticked, no `BLOCKED:` or `LATER:` in its text, not an operator row.
- Test file `board-slice-cases.test.mjs` has no module of its own; it imports this one.

### plan-atlas.mjs

- Generated map of `docs/plan/` plans and rows.
- Run: `node tools/plan-atlas.mjs --write` (regenerates `docs/plan/ATLAS.md`; never hand-edit); `--row <ID>`; `--crate <name>`; `--priority <look|bodies|aidoor|speed|proof|other>`; `--plan <name>`; `--find <text>`; `--help`.
- Exit: 0 ok, 1 unknown or empty result, 2 usage.

### ticked-rows-audit.mjs

- Finds ticked `- [x]` rows whose evidence is gone. Static, no cargo, no edits. Checks: `crate-missing`, `test-target-missing`, `test-target-moved`, `filter-missing`, `path-missing`, `path-moved`.
- Run: `node tools/ticked-rows-audit.mjs [--json] [--no-history] [--root <dir>]`.
- Exit: 0 no suspect, 1 suspect, 2 usage or git failure.

### finish.mjs

- Prints each gate in `docs/FINISH-RICH.md` (A1 to A10, B, C1 to C5, P1 to P10) as met, partial or open, with open rows and an evidence line. Ticks come from the board.
- Run: `node tools/finish.mjs [--finish <path>] [--sprint <path>] [--archive <dir>] [--json]`; `--help` prints usage, exit 0.
- Exit: 0 after printing; 2 when a `--finish` or `--sprint` file is missing.
- `docs/plan/status-2026-10-17.md` says the scoreboard needed repair the same day. Re-run before quoting a gate.

### part-score.mjs

- One live line per front: `look`, `bodies`, `aidoor`, `net`, `speed`.
- Run: `node tools/part-score.mjs [look|bodies|aidoor|net|speed] [--json] [--round]`; `--help`. Reads the board, `tools/perf-baseline.json` and git.

### crate-map.mjs

- Writes `docs/CRATE-MAP.md`: one row per `crates/<dir>`, from Cargo.toml and the first `//!` line of `src/lib.rs`.
- Run: `node tools/crate-map.mjs` (writes); `--check` (exit 1 when the file is stale); other arguments print `usage: node tools/crate-map.mjs [--check]`, exit 2.

### books.mjs

- `node tools/books.mjs search "<query>" [--book <id>[,<id>]] [--k 5]`; `list`; `toc <id>`. Work folder: env `BOOKS_DIR`, default `C:/empire/lab/skillworks/work` (outside the repo).

## 5. Web and wasm

### web-build.mjs

- Builds the browser shell into `dist/web`.
- Run: `node tools/web-build.mjs [--out dist/web] [--commit <hash>] [--no-opt]`.
- Steps: cargo build of package `engine-web`, target `wasm32-unknown-unknown`, profile `web`; wasm-bindgen CLI at the version `Cargo.lock` locks; `wasm-opt -Oz` with bulk-memory, nontrapping float-to-int, sign-ext, multivalue, reference-types and mutable-globals enabled (skipped with `--no-opt`).
- Output: `index.html`, `loader.js`, `coi-serviceworker.js` (WEB14 isolation worker), `engine.js`, `engine_bg.wasm`, `build.json` (sizes, gzip sizes, SHA-256, tool versions, commit), plus wasm-bindgen's `snippets/`.
- Size limit 8 MiB for `dist/web` before content.
- Exit: 0 complete and under limit, 1 a step failed (message names it), 2 bad call.
- Page source: `crates/web/index.html`.

### web-browse.mjs

- Serves `dist/web` on loopback, loads it in headless Chrome, reads the JSON line from `#report`, screenshots, checks the canvas is not blank.
- Run: `node tools/web-browse.mjs --chrome <path> [--dir dist/web] [--out out/web] [--require-adapter] [--only <n>]`. `--chrome` is required.
- Flag sets, in order (`FLAG_SETS`): `swiftshader-angle`, `swiftshader-vulkan`, `adapter-only`, `swiftshader-unsafe`, `lavapipe`, `vulkan-feature-only`, `bare`. `--only <n>` runs one by index. Env `WEB_CHROME_EXTRA` adds an `extra` set.
- Proof 1 (WebGPU): first set giving an adapter; checks report (ok, adapter, limits), non-blank screenshot, frames keep coming, `window.loseDevice` gives `data-generation 2`. No adapter: writes `attempts.json` with each reason and prints `NO ADAPTER: every flag set was tried; see attempts.json for each reason.`
- Proof 2 (no WebGPU): `navigator.gpu` removed; the loud message must show and the line must read `ok:false reason no-webgpu`.
- Exit: 0 proven (missing adapter fails only with `--require-adapter`), 1 proof failed, 2 could not run, 3 no adapter with `--require-adapter`.

### web-doctor.mjs and wasm-info.mjs

- `node tools/web-doctor.mjs`: `OK <name> <version>` or `MISSING <name>` plus `  fix: <command>`; last line `web target: READY` (exit 0) or `web target: NOT READY <n> missing` (exit 1).
- `node tools/wasm-info.mjs <file.wasm>`: one JSON object (imports by kind, memories and tables with limits, exports, producer features).

## 6. Content, assets and intake

### cook-char.mjs

- Checks the CT37/CT23 character manifest (licences, digest dedupe, counts, budgets, refused list); runs the headless Blender cook (GS24), one process per clip.
- Run: `node tools/cook-char.mjs --check [--manifest content/manifest.json]`; `node tools/cook-char.mjs --cook <in.bvh|fbx|gltf|glb> <out.glb|gltf> [--script <path>] [--blender <exe>]`; `--help`.
- Blender from PATH, `--blender`, or env `E2040_BLENDER`.
- Licence rule: sources resolve to `CC0-1.0` or `CC-BY-4.0`. Refused marks: `geno`, `lafan1`, `mixamo`, `qal`, `unreal`.
- Output: check findings `FAIL <line>` on stderr (exit 1). Clean manifest: `RESULT cook-char ok landed=<n>/80 locomotion=planned combat=30+50 manifest=<path>`. Refusals `E_MANIFEST_ABSENT`, `E_MANIFEST_EMPTY`, `E_PARSE` (exit 2).
- Exit: 0 ok, 1 FAIL, 2 refused or no Blender or bad call, 3 cook failed.

### gltf-intake.mjs

- One-command glTF intake: validate, clean (dedup and prune, no lossy step), validate output, copy only under a CC0 licence.
- Run: `node tools/gltf-intake.mjs content/fighters/glb/<name>.glb <out.glb> [--licence SPDX] [--manifest path] [--author name] [--licence-read text] [--provenance path]`; `--help`.
- Licence: must resolve to `CC0-1.0` from `--licence` or `--manifest` (default `content/fighters/manifest.json`), else `E_LICENCE`.
- Inputs under `content/fighters/` are read only; an output there is `E_READONLY`.
- Refusals on stderr, exit 2: `E_INPUT_ABSENT`, `E_INPUT_EMPTY`, `E_LICENCE`, `E_PARSE`, `E_INPUT_INVALID`, `E_OUTPUT_INVALID`, `E_READONLY`. Crash: `E_INTAKE_CRASH <message>`, exit 1.
- Success: `RESULT gltf-intake ok in=<bytes> out=<bytes> issues=<n> triangles=<n>` plus one JSON provenance line (source, sha256 in and out, licence). `issues` counts validator errors only.

### fetch-content.mjs

- Downloads listed files with md5 and licence sidecars.
- Run: `node tools/fetch-content.mjs --list <file> [--out <dir>] [--dry-run] [--rate-ms N]`; `--help`.
- List rows: `<source> <id> <url> [md5] [license] [author]`. Sources `polyhaven`, `ambientcg`, `url`. `-` as url resolves the source API root. `#` lines and blanks skipped.
- Default out `content/_incoming`; each asset in `<out>/<source>/<id>/` with `<id>.license.json`. One request per second by default. Matching md5 is skipped; a wrong md5 deletes the file and exits 1.
- `--dry-run` prints total bytes and licences, writes nothing.
- Output: `RESULT fetch-content ok downloaded=<n> skipped=<n> bytes=<n>`. Exit 0, 1 md5 mismatch, 2 usage.

### city-osm.mjs, level-check.mjs, beat-lint.mjs

- `city-osm <in.osm|in.json> <out.gltf> [--licence SPDX] [--style p] [--provenance p]`: offline OSM slice to glTF 2.0; caller is `gltf-intake`.
- `level-check [--placement P] [--zones Z] [--levels L] [--arena-kit K]`: Rome arena layout gate; exit 0 ok, 1 FAIL, 2 usage.
- `beat-lint [--ladder PATH]`: ladder beat chart gate (GP40) over `content/design/ladder.rows`; exit 0 pass, 1 fail, 2 usage.

### steal-land.mjs, mcp-install.mjs

- `steal-land --next <part>` / `--fetch <id>` / `--gate` (FAIL on rules, else PASS). Works on `docs/plan/steals.md`; fetches into `target/steals/<id>/`.
- `mcp-install [--project-dir DIR] [--remote-url URL] [--bin PATH] [--check]`: copies the release `e2040-mcp` binary to `.opencode/bin/`. `--check` changes nothing.

## 7. Captures and visual checks

### product-check.mjs

- Built-game floors. Builds `target/release/arena.exe` if older than the code, runs it twice: look run `--scene arena --fighters 8 --frames 300 --exit --require-framing --capture-dir ... --capture-every 60 --sha ... --view-brief ...`; timing run `--scene arena --fighters 8 --frames 300 --exit`.
- Scores against `tools/product-bar.json` and `tools/pass-bar.json`.
- Writes `target/reports/product/latest.json`, `latest.md`, `history.jsonl`; captures in `target/reports/product/runs/<sha>-<utc>/` (newest 6 kept).
- For each broken floor prints a `FAIL <floor>: <why>` line and records it in `latest.md`.
- `--if-new`: returns at once when the tree is the last checked (`SAME tree as <sha> (<verdict>)`). `--no-build`, `--json`.
- Output line: `product-check: <GREEN|RED> <sha> fps=... empty=... tinted=... pops=... sunk=... blank=... frozen=... val=...`, with `red=` when present.
- Exit: 0 green, 1 floor broke, 2 not run (`NOT RUN at <sha>: <why>`).

### potato-check.mjs

- Gate G9. Runs `target/release/arena.exe --scene arena --fighters 8 --frames 600 --exit` with `E2040_ADAPTER=integrated E2040_LIMITS=downlevel`, samples working set every 250 ms, writes `target/reports/potato-<date>.md`.
- Run: `node tools/potato-check.mjs`; `--floor`; `--help`.
- Bars in `tools/potato-bar.json`: p50 50 ms, p99 80 ms, working set 3072 MB. Output: potato line, one line per bar, load line `potato load: QUIET` or `LOADED <n> cargo + <m> rustc running`.
- Adapter evidence: GS6 line `adapter: <name> <backend> <kind> vendor=0x<id>`, else `arena per-pass: adapter="<name>"`. None prints `NO_IGPU`.
- Exit: 0 ok or bars printed; 1 bar failed with `--floor`; 2 no integrated adapter, bar unreadable, or help; 3 `--floor` on a loaded box: `potato floor: WITHHELD`.

### showcase.mjs

- Stitches a PNG sequence into MP4 and GIF. Input `target/showcase/<date>/<clip>/` with `frame*.png` and `manifest.json`.
- Run: `node tools/showcase.mjs --clip <dir> [--fps 30] [--width 640]`; `node tools/showcase.mjs <clipDir>`; `--help`.

### frame-moved.mjs

- Compares two captures, says whether the look moved.
- Run: `node tools/frame-moved.mjs --new <png|dir> [--old <png|dir>]`; `--baseline <png>`; `--since-last-playtest`; `--json`. Errors exit 2.

### frame-triage.mjs

- J1 showcase triage. Counts fighter-kit pixels (`isFighterPixel` from `fighter-count.mjs`); passes only above `FIGHTER_FLOOR_PIXELS`. The manifest's claimed count is reported and never decides. Orbit dims 1280x720.
- Exit 1 on triage fail.

### frame-v2v3.mjs, fighter-count.mjs, q1-check.mjs, ra8-check.mjs, ra8-manifest.mjs

- `frame-v2v3 <frameA.png> <frameB.png> [--json]`: sky and motion heuristics.
- `fighter-count [--dir <frames-dir>] [--json]`: exit 0 ok, 1 failure, 2 bad arguments.
- `q1-check`: read-only check of `target/q1`; exit 1 when `gaps` is not empty.
- `ra8-check`: read-only; reports missing PNGs, never builds.
- `ra8-manifest --src <dir> --out <file> [--renderer R] [--device D] [--seed N]`: writes the look manifest from real PNG bytes; refuses to write into `target/`.

### iqa-gate.mjs

- PQ4 capture-pair gate. Reads two PNGs, prints one JSON line with SSIM (`10000` = 1.0; default threshold `9800`). ssimulacra2 and butteraugli are null offline.
- Run: `node tools/iqa-gate.mjs [--threshold N] <reference.png> <capture.png>`; `--selfcheck`; `--help`. Exit 0 pass, 1 not pass or selfcheck fail, 2 error.

### rdc-diff.mjs

- `node tools/rdc-diff.mjs <dirA|pngA> <dirB|pngB> <outPng> [--frame N] [--epsilon N]`; `--self-check`. Exit 0 help or equal, 2 bad arguments.

## 8. Replay, netcode and multiplayer

### tape-checksum.mjs

- Per-tick FNV-1a64 chain over a JSON tape array; first divergence between two tapes.
- Run: `node tools/tape-checksum.mjs --tapes <a.json> <b.json>`. Output `first-divergence: equal (<n> ticks)` or `first-divergence: <tick>` plus `a[<t>]=... b[<t>]=...`. Usage error exit 2.

### desync-diff.mjs

- Desync report over canonical system bytes at one tick.
- Run: `node tools/desync-diff.mjs --expected <bin> --actual <bin> --system <name> --tick <u32> --out <dir> [--expected-hash <hex> --actual-hash <hex>]`. Exit 1 divergence, 2 usage or error.

### rerun-timeline.mjs

- `node tools/rerun-timeline.mjs <a.json> <b.json>` diffs two per-tick timelines; `--self-check` expects `first_divergent=57 replay_diff_rows=0`.

### replay-to-mp4.mjs

- GS25: tape to numbered PNGs (arena capture run), then ffmpeg at 60 fps.
- Run: `node tools/replay-to-mp4.mjs <tape> --out clip.mp4 [--max-ticks N] [--pipe] [--keep-frames]`; `--help`.
- Encoder: `h264_nvenc` when ffmpeg lists it, else `libx264 veryfast crf 18`. `--pipe` holds frames in memory; large clips use file mode.
- Exit: 2 no ffmpeg or bad call, 1 failure, 0 ok.

### p4-netsim.mjs

- `node tools/p4-netsim.mjs [--ticks N] [--reconnect on|off] [--corrupt C:T] [--selftest]`: deterministic 64-client model, seed 2040, 60 Hz. Exit 0 pass, 1 fail, 2 usage.

### mp6-check.mjs

- Checks the six MP6 receipts written by `net-bout mp6 --out DIR`. Read-only.
- Run: `node tools/mp6-check.mjs [DIR]` (default `target/mp6`). Exit 0 all hold, 1 check fails, 2 receipt missing or not JSON.

### mp-evidence.mjs

- Newest MP and ON evidence row per area, read-only. Run: `node tools/mp-evidence.mjs`; `--help`.

## 9. Mirror, CI and maintenance

### public-export.mjs

- Exports `git archive HEAD` minus internal files to the public mirror `maximkone/engine2040-public`, branch `main`. LFS pointers become real bytes.
- Run: `node tools/public-export.mjs` (builds and scans `target/public-export/tree`); `--commit` (also commits in `target/public-export/repo`, needs a passing verify receipt for HEAD); `--commit --push` (pushes `origin main`); `--ci` (last runs, failed job and step); `--remote <url>` (first time); `--force` (export despite problems).
- Denied: `.opencode/` (except `modmap.md` and `repomap.md`), `opencode.jsonc`, `arsenal.json`, `orders.csv`, `team/`, `sprint/`, `archive/`, `clients/`, `docs/archive/` (except `SPRINT-done-*.md`), `docs/ROYALTY.md`, `docs/EMPIRE-INBOX.md`, `AGENTS.md`, `AGENTS.test.mjs`, `CLAUDE.md`, `tools/public-export.mjs`, `tools/public-export.test.mjs`.
- Scan: GitHub tokens, `sk-` keys, AWS keys, private key blocks; `mod` lines without their file (`not self-contained`); missing LFS objects.
- Output: `verify <sha>: denied-present=... secrets=... lfsMissing=... missingModules=... -> <path>`; `PROBLEM <path>`; `NOT EXPORTED: <n> problem(s). Fix them or pass --force.`; `NOT COMMITTED: ...`; `tree ready: <path>`; `committed in <repo>: ...`; `pushed`.
- Lock `target/public-export.lock`; a second run prints `another export is running ... nothing done`.
- Exit: 0 normal; 1 NOT EXPORTED, NOT COMMITTED or FAILED.

### farm.mjs

- `node tools/farm.mjs` exports HEAD to the mirror and waits for CI, printing red jobs. Flags: `--no-export`, `--max <minutes>` (default 30), `--packets`. Output `RESULT GREEN|RED|SKIPPED`; error lines in `target/farm-last.txt`. Exit 0 green, 1 red, 2 no run, timeout, or all skipped.

Mirror workflows (`.github/workflows/`): `check.yml` (dispatch, one cargo line), `ci.yml` (push to main, PR, dispatch; jobs `quick`, `test`, `clippy`, `release-proof`, `window-compile`, `arm-golden`), `golden-matrix.yml` (push to `golden-matrix-*`), `wasm-check.yml`, `wasm-probe.yml` (`wasm-probe-*`), `web-build.yml` (`web-build-*`), `pages.yml` (push to main, Node 24, GitHub Pages). All jobs run only on `maximkone/engine2040-public`.

### clean.mjs

- Removes trash under `target/`.
- Run: `node tools/clean.mjs --dry-run` (default when no `--yes` or `--auto`); `--auto`; `--deep --yes` (also release cache); `--include-debug` (also `target/debug`).
- Removes `target/tmp`, `target/debug-workspace-check`, hunt, prov, j4 and perf logs, old per-lane build dirs, and pool dirs no lane uses. Never touches `*.patch`, `lanes-worktrees.json`, `sprint.lock`, `sprint.halt`, `sprint.handoff`.
- Unknown argument: usage line, exit 2. Names lane paths (section 14).

### doctor.mjs

- Finds what breaks development and repairs the safe parts.
- Run: `node tools/doctor.mjs` (report only); `--fix` (prints what would change, changes nothing); `--fix --consent=<csv>` (applies consented scopes `toolchain`, `deny`, `cache`).
- Env: `E2040_DOCTOR_ROOT` (fixture root), `E2040_DOCTOR_FREE_GB` (simulated free disk).
- Lane evidence (`target/lanes/*.patch`) is never deleted. Installs and cache eviction need consent.
- Exit: 0 healthy or healed, 1 needs a person, 2 bad consent or usage.

### bins.mjs

- `node tools/bins.mjs` (exit 0 all fresh, 1 otherwise); `--json`. Stale or missing binaries print the cargo command that rebuilds them.

### affected.mjs

- Maps a diff to changed packages, reverse dependencies, and matching `tools/*.test.mjs`, so gates run the affected subset.
- Run: `node tools/affected.mjs [--base <ref>] [--files <paths...>] [--json] [--mark-green]`; also `--tiers`, `--snapshot-write`, `--snapshot-diff`.
- Exit: 1 on error (`affected: <message>`), 0 otherwise.

### cargo-guard.mjs

- Runs one cargo command and kills it past a time or message cap.
- Run: `node tools/cargo-guard.mjs [--] <cmd> [args...]`. Env `CARGO_TIME_LIMIT` (seconds; 0 = no limit), `CARGO_MSG_LIMIT` (0 = no limit).
- Exit: child code; 2 no command; 124 time cap; 127 spawn failed.

### seal-keys.mjs

- Keeps two 64-hex keys (`E2040_PERF_SEAL_KEY`, `E2040_EVIDENCE_KEY`) in `<home>/.empire/state/engine2040-seal-keys.json`, outside every repo. Never printed or committed.
- Run: `node tools/seal-keys.mjs --ensure` (prints `OK`); `node tools/seal-keys.mjs -- <cmd> [args]` (runs the command with both keys in env).

### ship.mjs

- GM3 ship proof: pack, stage and clean-room run in one entry point. Donor MANIFESTED, BUNDLE_MAGIC and STUB lines are verbatim.
- Run: `node tools/ship.mjs --help`; `pack [--dist <path>] [--target <path>] [--replace] [--bin <arena-exe>] [--editor-bin <exe>] [--bundle <bin>] [--frames N | --minutes N] [--log <path>] [--build-log <path>] [--allow-stub] [--skip-build]`; `stage` (same as `pack --stage-only`); `run [pack-style flags] [--receipt <path>]`; `run --dry-run [--fixture <path>] [--work <path>] [--frames N] [--log <path>]`.
- `run --dry-run` never touches the clean-run folder named in the header (`C:/e2040-clean/`).
- Exit: 0 pass, 1 failed step, 2 refused or usage.

### mutants.mjs

- Runs `cargo mutants --in-diff <patch> -p <crate>`; fails closed when a mutant in changed lines survives.
- Run: `node tools/mutants.mjs --patch <file|-> -p <crate> [--root <dir>] [--timeout-min 10] [--max-missed N] [--jobs N] [--output <dir>] [--report <file>] [-- <extra>]`; or `--from-out <mutants.out dir | outcomes.json> [--max-missed N]` (re-judges without cargo).
- Output: one `path:line: <mutation>` per missed mutant, counts, score, then `RESULT PASS` or `RESULT FAIL`. Score = (caught + timeout) / (caught + timeout + missed). PASS needs missed at or below `--max-missed` (default 0).
- Exit: 0 PASS, 1 FAIL, 2 usage or tool error. Install: `cargo install --locked cargo-mutants`.

## 10. Search wrappers and utilities

### gh-search.mjs and web-search.mjs

- Shell-safe wrappers. Arguments go to the child as an array, so `>=` in a query is not a shell redirect.
- `gh-search`: `--query "<q>" [--limit 10] [--type repos|code|issues|prs] [--dry-run] [--self-check] [--help]`.
- `web-search`: `--query "<q>" [--limit 10] [--dry-run] [--self-check] [--help]`; or words as positional arguments.
- Exit: 0 ok; 1 when `--self-check` fails (`RESULT FAIL: shell-safe ... checks failed`).

### bench-compare.mjs

- Run: `node tools/bench-compare.mjs [--warmup N] [--runs N] [--export-md f] [--export-json f] [--format md|json|both] [--self-test] [--sort-demo] [-- "<cmd>" ...]`. Exit 0 pass, 1 fail, 2 usage.

### wiring-scan.mjs

- Lists `pub fn|struct|enum|trait|const` items with no production user. Test code skipped.
- Run: `node tools/wiring-scan.mjs [crates] [--json] [--baseline] [--check] [--diff[=REV]] [--unlocks=ROW[,ROW]] [--orphans] [--help]`. Writes `target/reports/wiring-scan.json`.
- `--diff[=REV]` is the unwired-code gate (AD5): a `pub fn`, `struct` or `enum` added since REV (default HEAD; working tree and untracked files count) needs a production user in another file, else exit 1. `--unlocks=ROW` names an open board row that will consume the new items and exempts them (a closed or unknown row is refused). `--orphans` is the default report, the list of existing orphans; nothing is deleted.
- `--check` exits 1 when the count rises above the pinned baseline. `workspace-check.mjs` reuses its verdict cache.
- Baselines: `tools/wiring-scan.baseline.json`, `tools/wiring-baseline.json`. Run lock in `target/locks/`.

### wiring-cov.mjs

- Runs `cargo llvm-cov run` on the arena binary (never `cargo test`), nightly, report only. Run: `node tools/wiring-cov.mjs [--out <report.json>] [--all | --crates a,b]`; `--from-json <export.json>`. Report `target/reports/wiring-cov.json`. Missing tool prints `cargo install cargo-llvm-cov`.

## 11. Library modules

No entry point: `sim-source-scan.mjs`, `board-slice.mjs`, `board-memo.mjs`, `argv.mjs`, `fnv1a64.mjs` (mirrors `crates/core/src/hash.rs`), `hash-util.mjs`, `cast-safe.mjs`, `sort-once.mjs`, `pool-queue.mjs`. `drr-queue.mjs` is imported and also has a CLI guard. Each has a same-name `.test.mjs`.

## 12. Data files

| File | Read by | Contents |
|---|---|---|
| `tools/product-bar.json` | `product-check.mjs` | `floors` (red thresholds: `framing_empty`, `tinted_share_max` 0.25, `validation_errors`, `blank_frames`, `frozen_pairs`, `stall_p99_ms_max`, `fps_drop_max`, `view_*`) and `targets` (finish-line targets) |
| `tools/pass-bar.json` | `product-check.mjs` | `passes` table |
| `tools/potato-bar.json` | `potato-check.mjs` | p50, p99 and working-set bars (G9) |
| `tools/perf-baseline.json` | `part-score.mjs` | performance baseline |
| `tools/rust-quality.baseline.json` | `rust-quality.mjs`, `affected.test.mjs` | per-crate debt counts |
| `tools/docs-coverage.baseline.json` | `docs-coverage.mjs` | per-crate documented counts |
| `tools/wasm-gate.baseline.json` | `wasm-gate.mjs` | per-file hazard counts |
| `tools/wiring-scan.baseline.json`, `tools/wiring-baseline.json` | `wiring-scan.mjs` | wiring pins |

## 13. Test files without a module

These `tools/*.test.mjs` files have no same-name module. Some import a module under another name, some test a deleted tool. Running `node --test tools/` includes them. Check each before relying on it.

`argv-contract`, `board-slice-cases`, `ci`, `compare-split`, `cover-gap`, `float-gate`, `frame-check`, `ibl-presets`, `ibl-vectors`, `index-check`, `knobs-check`, `ladder-split`, `plan-index`, `pool-queue-batch`, `rng-vectors`, `rollback-vectors`, `ship-dupe-gap`, `skills-index`, `skin-vectors`, `sky-preset`, `sky-presets`, `sky-presets3`, `spawn-check`, `stale-refs`, `stale-targets`, `toolchain-pin`, `w4-compile-cache-plan`.

## 14. Suspicious items

- `arch-fitness.mjs` exits 0 even on a cycle. It prints `FAIL cycle` and sets no exit code.
- `secret-gate.mjs` is called by the local `pre-commit` hook (installed from the tracked `tools/hooks/pre-commit`). The local `pre-push` hook runs `git lfs pre-push`, `cargo deny check licenses`, and `node tools/license-gate.mjs`; `pre-commit` also runs `tools/rust-quality.mjs`. Hooks live in `.git/hooks/`, not versioned.
- `product-check.mjs` files no queue packets: the queue was removed 2026-10-17, so a broken floor is a `FAIL` line and a `latest.md` entry (the keeper that filed packets was cut, `docs/plan/finish-line.md`).
- Several tools name lane machinery that the standalone repo cut: `clean.mjs` (`lanes-worktrees.json`, `target/lanes`), `doctor.mjs` (`target/lanes/*.patch`), `mutants.mjs` (`target/lanes/<id>.patch`, `.lanes/<id>`). `books.mjs` and `ship.mjs` use paths outside the repo (`C:/empire/...`, `C:/e2040-clean/`).
- `docs/WORKFLOW.md` names `tools/sprint-lock.mjs`, `tools/sprint-loop.mjs`, `builder` and `planner` agents. None exists in `tools/`.
- `docs/FOCUS.md` names `tools/drift-check.mjs` and `tools/steal-next.mjs`. Neither exists in `tools/`.
- `proof.mjs` with no flags runs cargo locally; AGENTS says cargo proofs go to `gh-check.mjs`. Mirror pushes go to `main`; this repo is `master`. 28 test files have no module (section 13). "see code" rows in the table are unverified.
