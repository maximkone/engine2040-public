---
name: door-and-headless-runbook
description: "Use when you must run engine2040 without a person or window (staged fight, golden proof, perf CSV, the e2040 MCP door, the playtest crate), or run the arena window and read its receipt lines (arena view, break, physfx, captures). Gives exact run lines and output paths."
---

# Door and headless runbook

Every line below names a binary or test that exists in this repo. Run from the repo root in pwsh. Builds share `target/`: a `Blocking waiting for file lock` line means wait (AGENTS.md, shell and search). Tests and clippy go through `node tools/gh-check.mjs` (skill `github-check`). Detail moved to `reference.md`: the MCP door walkthrough, receipt lines and the view-law table, the scene report, scoring, night GPU.

## Golden proof
- Full proof: `node tools/proof.mjs`. Fixtures only: `node tools/proof.mjs --skip-build`. Other flags: `--perf`, `--mutants`.
- Golden test: `node tools/gh-check.mjs cargo test -p engine-proof --lib golden_hash_untouched` (`crates/proof/src/a9.rs::golden_hash_untouched`).
- The v1 golden is `GOLDEN_600=d57b5c8ae3096614` (600 ticks, seed 2040). It moves only with a receipt (`docs/HASH-LAW.md`). A hash that diverges: skill `determinism-debug`. Never re-pin it.

## Headless staged fight
- `cargo run -p engine-game --features app --bin staged_fight`. The bin needs `--features app`. It writes frame PNGs, a frame manifest (a JSON array, one row per frame) and a receipt under `target/ct15-staged-fight` (`crates/game/src/bin/staged_fight.rs::DEFAULT_OUT_DIR`). Same manifest, seed and script give the same PNGs and receipt twice.

## Perf CSV
- Capture run: `cargo run --release -p engine-perf --features profile -- --capture [frames]` (`crates/perf/src/main.rs`, `capture_arg`).
- The CSV goes to stdout: `cargo run --release -p engine-perf --bin perf > target/perf-run.csv`. Header: `scenario,seed,ticks,agents,sprites,mean_ms,p50_ms,p99_ms,max_ms,cpu_workload_rate,final_hash`.
- How to measure, pair runs and report noise: skill `perf-bench`.

## Game binary
- `cargo run -p engine2040` (`Cargo.toml` name `engine2040`, `src/main.rs`). Run it release, twice, for the golden line.

## MCP door (e2040-mcp)
- Help, headless: `cargo run --locked -p engine-endpoint --features mcp --bin e2040-mcp -- --help` (`docs/book/recipes-drive-from-mcp.md`, serve the project folder). The plain build refuses the binary without the `mcp` feature.
- Run a copy so a rebuild cannot relink the binary under a running server: `cargo build --release --locked -p engine-endpoint --features mcp --bin e2040-mcp`, copy `target/release/e2040-mcp.exe` outside `target/`, and point the client at the copy with `--project-dir <scratch dir>`.
- Client config: `{"command": "e2040-mcp", "args": ["--project-dir", "<dir>"]}`. `tools/mcp-install.mjs` writes the server key `e2040`, and OpenCode shows the key as a prefix (`e2040_api_search`). The door itself answers only the bare name: a raw call to `e2040_api_search` returned `unknown_tool` (checked 2026-10-21). Use the names `rpc.discover` prints. Search before you describe a path you have not seen. Adding a verb: skill `ai-door`.
- Run the stale-copy check first and the prop-brief walkthrough from `reference.md` before you build anything through the door.

## Playtest crate
- Tests: `node tools/gh-check.mjs cargo test --locked -p engine-playtest keypoint` (`docs/book/recipes-drive-from-mcp.md`, the play-test section). Package `engine-playtest`, path `crates/playtest`.

## Window run
The `arena` bin (`crates/game/src/bin/arena.rs`; helpers in `arena_frame/mod.rs` and `arena_winit/mod.rs`) needs `--features app`. A short `--exit` run prints receipt lines that say what was drawn, as numbers, so you do not need to look at pixels first.

### Build and run
- Is the exe fresh? `node tools/bins.mjs` prints the rebuild line when stale. Build (one cargo at a time): `cargo build --release -p engine-game --features app --bin arena` gives `target\release\arena.exe`.
- Look run (what the product check does): `target\release\arena.exe --scene arena --fighters 4 --frames 300 --exit --require-framing`. It opens a real window on this GPU, runs, prints the lines and closes. Every `--exit` run has a 5-minute wall bound (`--deadline-ms <ms>` changes it) and never opens an audio device. With no display and `--frames` it falls back to a smoke run with no window.
- Do not run it while a cargo build, a capture or another arena runs: timings and the window stall (skill `github-check`, section 4). One window at a time: `node tools/arena-run.mjs -- <args>`.
- stdout is mirrored into `target/logs/arena-<UTC>.log`; a crash writes `crash-<UTC>.toml` beside it.
- Flags: `--scene arena` (the only scene), `--fighters N`, `--seed N`, `--ticks N`, `--frames N`, `--exit`, `--mute`, `--timed-smoke` (writes `target/perf/frame-times.csv`), `--require-framing` (exit 1 if a frame shows no fighter), `--view-brief <file>` (JSONL, one line per presented frame), `--no-break-look`, `--pause-at-tick N` (add `--step` to advance one tick per Space), `--staged-fight` (scripted human slot: seed 2040, 600 ticks, 4 fighters), `--camera-path arena600`.

### Captures
- `target\release\arena.exe --scene arena --fighters 4 --frames 300 --exit --capture-dir target/clips/<name> --capture-every 60 --fps 30` writes `frame_000001.png`..., `manifest.json`, `cutlog.json` and prints `arena capture: frames=N manifest="..." sha=<16 hex> view=ok` (same inputs, same sha).
- **Read `view=` before you look or report.** It is the `arena view` verdict for the same frames: `view=ok`, or `view=FAIL(<fields>)` naming what broke (`view_eye_in_body`, `view_eye_below_ground`, `view_sunk_cm_max`, `view_nonfinite`, `view_error_parts`, ...). A FAIL capture is not evidence of anything except the bug it names (`ViewLaws::verdict`, `crates/game/src/view_truth.rs`). Field meanings: `reference.md`.
- **A capture of staged boxes fails the run.** When the fighter look cannot load (a missing clip, a body at the wrong scale, a bad manifest) the window prints `arena notice: <why>; drawing staged boxes`; with `--capture-dir` armed it also prints `an armed capture of staged boxes is no evidence` and exits 1. Fix the named file; never report those frames.
- Realistic bodies: `E2040_BODIES=rocketbox` loads `content/fighters/rocketbox` (the Rocketbox family) instead of the CT7 bundle. `node tools/asset-gen/fighter.mjs ... --prove` runs this capture for you and fails unless the exit is 0 and every capture line says `view=ok`.
- Tape run: `cargo run -p engine-game --features app --bin arena -- --capture-dir target/clips/<name> --capture-every 2 --tape <tape> --rows <ROW1> --sha <build-sha> --fps 30` (`docs/CAPTURE.md`, built-in dump). Flags `--tape`, `--rows`, `--sha` are documented there.
- The PNG is an offscreen render of the same draw list, never the swapchain. Open at least one PNG yourself before you report it (skill `test-and-proof`). A PNG under 60 KB is blank.

### Reading the lines
Receipt lines in print order, the `arena view` field table, `arena break`/`physfx`, the headless `view_run` line, and the `view.scene` report with its `view.steer` ops all live in `reference.md`. Read it before you report a `view=` line or a scene finding. `tools/product-check.mjs` scores the lines against `tools/product-bar.json`; run it with `--no-build` only.

## Checks before you report
- A run line that names a file: check the file exists first (`Test-Path <path>`).
- Report the command, the exit code, and the output path. Never report a PNG you did not open.
