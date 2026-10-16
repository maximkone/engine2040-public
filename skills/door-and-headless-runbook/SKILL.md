---
name: door-and-headless-runbook
description: "Use when running engine2040 without a window or a person: the headless staged fight, the golden proof, arena capture, perf CSV, the e2040 MCP door (e2040-mcp) and the playtest crate. Gives the exact pwsh run lines and where each writes its output."
---

# Door and headless runbook

Every line below names a binary or test that exists in this repo. Run from the repo root in pwsh. Builds share `target/`: a `Blocking waiting for file lock` line means wait (AGENTS.md::Fast paths).

## Golden proof
- Full proof: `node tools/proof.mjs`. Fixtures only: `node tools/proof.mjs --skip-build`.
- Golden test: `cargo test -p engine-proof --lib golden_hash_untouched` (crates/proof/src/a9.rs::golden_hash_untouched).
- The v1 golden is `GOLDEN_600=d57b5c8ae3096614` (600 ticks, seed 2040). It moves only with a receipt (docs/HASH-LAW.md).

## Headless staged fight
- `cargo run -p engine-game --bin staged_fight`. It writes frame PNGs, a frame manifest (a JSON array, one row per frame) and a receipt under `target/ct15-staged-fight` (crates/game/src/bin/staged_fight.rs::DEFAULT_OUT_DIR). Same manifest, seed and script give the same PNGs and receipt twice.

## Arena capture (window build)
- `cargo run -p engine-game --bin arena -- --capture-dir target/clips/<name> --capture-every 2 --tape <tape> --rows <ROW1> --sha <build-sha> --fps 30` (docs/CAPTURE.md::Built-in dump).
- Output: `frame_000001.png` files, `manifest.json` and `cutlog.json`. For smooth clips add `--camera-path arena600`.

## Perf CSV
- Capture run: `cargo run --release -p engine-perf --features profile -- --capture [frames]` (crates/perf/src/main.rs::capture_arg).
- The CSV goes to stdout, so redirect it: `cargo run --release -p engine-perf --bin perf > target/perf-run.csv`. Header: `scenario,seed,ticks,agents,sprites,mean_ms,p50_ms,p99_ms,max_ms,cpu_workload_rate,final_hash` (crates/perf/src/main.rs::main).

## Game binary
- Root package: `cargo run -p engine2040` (Cargo.toml name `engine2040`, src/main.rs).

## MCP door (e2040-mcp)
- Help, headless: `cargo run --locked -p engine-endpoint --features mcp --bin e2040-mcp -- --help` (docs/book/recipes-drive-from-mcp.md::1. Serve the project folder). The plain build refuses the binary without the `mcp` feature.
- The OpenCode client runs the copy in `.opencode/bin/e2040-mcp.exe --project-dir .opencode/e2040-scratch` (opencode.jsonc::mcp.e2040).
- Refresh that copy: `node tools/mcp-install.mjs` (copies the release binary; `--check` only verifies). Rebuild release first.
- A client config line: `{"command": "e2040-mcp", "args": ["--project-dir", "<dir>"]}`.
- Tool names carry the `e2040_` prefix (for example `e2040_api_search`, `e2040_api_describe`). Search before you describe a path you have not seen.

## Playtest crate
- Tests: `cargo test --locked -p engine-playtest keypoint` (docs/book/recipes-drive-from-mcp.md::3. Play-test the proposal before it lands). The crate is `crates/playtest`, package `engine-playtest`.

## Checks before you report
- A run line that names a file: check the file exists first (`Test-Path <path>`).
- Report the command, the exit code, and the output path. Never report a PNG you did not open.
