---
name: door-and-headless-runbook
description: "Use when you must run engine2040 without a person or window (staged fight, golden proof, perf CSV, the e2040 MCP door, the playtest crate), or run the arena window and read its receipt lines (arena view, break, physfx, captures). Gives exact run lines and output paths."
---

# Door and headless runbook

Every line below names a binary or test that exists in this repo. Run from the repo root in pwsh. Builds share `target/`: a `Blocking waiting for file lock` line means wait (AGENTS.md, shell and search). Tests and clippy go through `node tools/gh-check.mjs` (skill `github-check`).

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
- Client config: `{"command": "e2040-mcp", "args": ["--project-dir", "<dir>"]}`. Tool names carry the `e2040_` prefix (`e2040_api_search`, `e2040_api_describe`). Search before you describe a path you have not seen. Adding a verb: skill `ai-door`.

## Playtest crate
- Tests: `node tools/gh-check.mjs cargo test --locked -p engine-playtest keypoint` (`docs/book/recipes-drive-from-mcp.md`, the play-test section). Package `engine-playtest`, path `crates/playtest`.

## Window run and the view line

The `arena` bin (`crates/game/src/bin/arena.rs`; helpers in `arena_frame/mod.rs` and `arena_winit/mod.rs`) needs `--features app`. A short `--exit` run prints receipt lines that say what was drawn, as numbers, so you do not need to look at pixels first.

### Build and run
- Is the exe fresh? `node tools/bins.mjs` prints the rebuild line when stale. Build (one cargo at a time): `cargo build --release -p engine-game --features app --bin arena` gives `target\release\arena.exe`.
- Look run (what the product check does): `target\release\arena.exe --scene arena --fighters 4 --frames 300 --exit --require-framing`. It opens a real window on this GPU, runs, prints the lines and closes. Every `--exit` run has a 5-minute wall bound (`--deadline-ms <ms>` changes it) and never opens an audio device. With no display and `--frames` it falls back to a smoke run with no window.
- Do not run it while a cargo build, a capture or another arena runs: timings and the window stall (skill `github-check`, section 4).
- stdout is mirrored into `target/logs/arena-<UTC>.log`; a crash writes `crash-<UTC>.toml` beside it.
- Flags: `--scene arena` (the only scene), `--fighters N`, `--seed N`, `--ticks N`, `--frames N`, `--exit`, `--mute`, `--timed-smoke` (writes `target/perf/frame-times.csv`), `--require-framing` (exit 1 if a frame shows no fighter), `--view-brief <file>` (JSONL, one line per presented frame), `--no-break-look`, `--pause-at-tick N` (add `--step` to advance one tick per Space), `--staged-fight` (scripted human slot: seed 2040, 600 ticks, 4 fighters), `--camera-path arena600`.

### Captures
- `target\release\arena.exe --scene arena --fighters 4 --frames 300 --exit --capture-dir target/clips/<name> --capture-every 60 --fps 30` writes `frame_000001.png`..., `manifest.json`, `cutlog.json` and prints `arena capture: frames=N manifest="..." sha=<16 hex>` (same inputs, same sha).
- Tape run: `cargo run -p engine-game --features app --bin arena -- --capture-dir target/clips/<name> --capture-every 2 --tape <tape> --rows <ROW1> --sha <build-sha> --fps 30` (`docs/CAPTURE.md`, built-in dump). Flags `--tape`, `--rows`, `--sha` are documented there.
- The PNG is an offscreen render of the same draw list, never the swapchain. Open at least one PNG yourself before you report it (skill `test-and-proof`). A PNG under 60 KB is blank.

### Receipt lines, in print order

```
arena smoke: ticks=600 frames=550 alpha=0.606 hud_cells=63 ... fight_draws=16
arena sim: ticks=600 hash=<16 hex>
arena frame-split: sim=..ms present=..ms fighters=..ms ... interval_ms p50=13.0 p99=1382.9 ...
arena framing: frames=550 fighters=4 seen=4 empty=0 fewest_in_view=4 tinted_share=0.00
arena view: frames=550 sim_ticks=0.9..599.6 bodies=2200 offscreen_share=0.01 nonfinite=0 error_parts=0 sunk_cm_max=14.2 idle_float_cm_max=3.7 pops=180 pop_acts=attack:35/idle:38/run:107 pop_cm_max=107.6 eye_in_body=0 eye_below_ground=0 closest_cm=798 cover_max=0.02 foot_slide_cm=0.1 planted_feet=65 ragdolls=3
arena break: breaks=15 debris=2 awake=0
physfx: bodies=2 contacts=0 us=0
```

A real 4-fighter, 600-tick log of 2026-10-17; yours will differ. The `arena sim` hash is the frozen sim hash for that tick count: pausing or a different frame rate does not move it.

### arena view (`crates/game/src/view_truth.rs`)

Every drawn body as numbers from the CPU skin. Units: centimeters in the line, sim ticks for time.

| field | reads as | red when |
|---|---|---|
| `frames`, `sim_ticks=a..b` | frames folded and the tick span shown | fight must start by tick 30 and span at least 60 ticks |
| `bodies`, `offscreen_share` | bodies drawn in total; share with no on-screen rect | share high means the camera lost them |
| `nonfinite`, `error_parts` | NaN or infinite vertices; parts drawn with the skin error material | any above 0 |
| `sunk_cm_max` | deepest a body sank into the sand | above 3 (floor in `tools/product-bar.json`); target under 2 |
| `idle_float_cm_max` | most an idle body floated above the sand | trend only |
| `pops`, `pop_acts`, `pop_cm_max` | joints that missed their constant-velocity guess by more than 10 cm in one frame (a clip snap); by act | target 0; `run` pops come from the run clip phase |
| `eye_in_body`, `eye_below_ground` | frames with the camera inside a body or under the sand | any above 0 |
| `closest_cm`, `cover_max` | nearest body to the eye; biggest picture share (0..1) | trend only |
| `foot_slide_cm`, `planted_feet` | p95 slide of a planted foot off its spot, over this many foot-frames | above a few cm means a skating foot; 0 `planted_feet` means no proof |
| `ragdolls` | most bodies drawn from a ragdoll in one frame | 0 after a knockdown means the ragdoll is not wired |

Ragdolled bodies are not judged for pops. No `arena view:` line at all means the view truth did not fold (fighter look failed to bind).

### break and physfx

- `arena break: breaks=<n> debris=<n> awake=<n>`: breaks = authority `BreakEvent`s the window saw; debris = debris bodies alive; awake = bodies still moving. `awake=0` means the pile settled. `breaks=0` is normal when nothing hit the crate in a short run. No `arena break` line: the picture is off (`--no-break-look`) or it dropped itself and printed `arena notice: break picture dropped: <why>`.
- `physfx: bodies=<n> contacts=<n> us=<n>`: the float physics layer (debris, ragdolls) at its tier. It never feeds the hash.
- Both come from `crates/game/src/break_window.rs` and `crates/physfx/src/types.rs`.

### No window: the same view line

`FighterLook::view_run(seed, ticks, fighters)` folds a bout into one `arena view:` line with no GPU. Test: `node tools/gh-check.mjs cargo test -p engine-game --features app --lib view_run` (runs on the PC). Pop and slide proofs on real bodies: `clip_switches_do_not_pop_in_a_real_duel` and `planted_feet_hold_their_spot_in_a_real_duel` in `crates/game/src/fighter_look_proofs.rs`. No door verb returns this line yet.

### Score it

`tools/product-check.mjs` reads these lines (`parseView`, `parseReceipt`) against `tools/product-bar.json`; run it only with `--no-build` (it writes its report and captures, nothing else). `node --test tools/product-check.test.mjs` shows the parse rules. Report: the command, the exit code, the line you read (quoted), and the PNG you opened.

## Checks before you report
- A run line that names a file: check the file exists first (`Test-Path <path>`).
- Report the command, the exit code, and the output path. Never report a PNG you did not open.
