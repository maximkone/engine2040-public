---
name: arena-capture-and-view-line
description: "Use when you must see what the arena window draws without a person: run arena.exe for N frames, read the arena view, break and physfx receipt lines (pops, foot_slide_cm, sunk_cm_max, ragdolls, breaks, debris), take captures, or run the staged fight."
---

# Arena run, captures and the receipt lines

The `arena` binary is the window game (`crates/game/src/bin/arena.rs`, helpers in `arena_frame/mod.rs` and `arena_winit/mod.rs`). It needs the `app` feature. A short `--exit` run prints receipt lines that tell you what was drawn as numbers, so you do not need to look at pixels first. Door and golden runs: skill `door-and-headless-runbook`.

## 1. Build and run

- Is the exe fresh: `node tools/bins.mjs` (prints the exact rebuild line when it is stale).
- Build (local, one cargo at a time): `cargo build --release -p engine-game --features app --bin arena`. Output `target\release\arena.exe`. Without `--features app` cargo refuses the bin.
- Look run (what the product check does): `target\release\arena.exe --scene arena --fighters 4 --frames 300 --exit --require-framing`. It opens a real window on this GPU, runs, prints the lines, closes. Every `--exit` run has a wall bound of 5 minutes (`--deadline-ms <ms>` changes it) and never opens an audio device. With no display and `--frames` it falls back to a smoke without a window.
- Do not run it while a cargo build, a capture or another arena is running: timings and the window stall (skill `gh-check-playbook`).
- The whole stdout is mirrored into `target/logs/arena-<UTC>.log` (a crash writes `crash-<UTC>.toml` beside it). Read that file when the console scrolled.

Flags you will use (all in `arena.rs` and `windowed.rs`): `--scene arena` (the only scene), `--fighters N`, `--seed N`, `--ticks N`, `--frames N`, `--exit`, `--mute`, `--timed-smoke` (writes `target/perf/frame-times.csv`), `--require-framing` (exit 1 if any frame shows no fighter), `--view-brief <file>` (JSONL, one line per presented frame: camera and each body's act, clip, tint, bounds, screen rect), `--no-break-look`, `--pause-at-tick N` (prints `paused tick=N`; add `--step` to advance one tick per Space), `--staged-fight` (scripted human slot: seed 2040, 600 ticks, 4 fighters), `--camera-path arena600`.

## 2. Captures

`target\release\arena.exe --scene arena --fighters 4 --frames 300 --exit --capture-dir target/clips/<name> --capture-every 60 --fps 30` writes `frame_000001.png`..., `manifest.json`, `cutlog.json` and prints `arena capture: frames=N manifest="..." sha=<16 hex>` (the sha is the sequence hash; same inputs give the same sha). Full flags (`--tape`, `--rows`, `--sha`): `docs/CAPTURE.md`. The PNG comes from an explicit offscreen render of the same draw list, never from the swapchain. Open at least one PNG yourself before you report it (skill `test-and-proof`). A PNG under 60 KB is blank.
The headless staged fight (no window): `cargo run -p engine-game --features app --bin staged_fight`, output `target/ct15-staged-fight`.

## 3. The lines, in print order

```
arena smoke: ticks=600 frames=550 alpha=0.606 hud_cells=63 ... fight_draws=16
arena sim: ticks=600 hash=<16 hex>
arena frame-split: sim=..ms present=..ms fighters=..ms ... interval_ms p50=13.0 p99=1382.9 ...
arena framing: frames=550 fighters=4 seen=4 empty=0 fewest_in_view=4 tinted_share=0.00
arena view: frames=550 sim_ticks=0.9..599.6 bodies=2200 offscreen_share=0.01 nonfinite=0 error_parts=0 sunk_cm_max=14.2 idle_float_cm_max=3.7 pops=180 pop_acts=attack:35/idle:38/run:107 pop_cm_max=107.6 eye_in_body=0 eye_below_ground=0 closest_cm=798 cover_max=0.02 foot_slide_cm=0.1 planted_feet=65 ragdolls=3
arena break: breaks=15 debris=2 awake=0
physfx: bodies=2 contacts=0 us=0
```
(a real 4-fighter, 600-tick log of 2026-10-17; yours will differ). The `arena sim` hash is the frozen sim hash for that tick count: pausing or a different frame rate does not move it.

### arena view (source `crates/game/src/view_truth.rs`)

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
- `physfx: bodies=<n> contacts=<n> us=<n>`: the float physics layer (debris, ragdolls) at its tier: bodies alive, contacts, microseconds. It never feeds the hash.
- Both exist in `crates/game/src/break_window.rs` and `crates/physfx/src/types.rs`.

## 4. No window: the same view line

`FighterLook::view_run(seed, ticks, fighters)` folds a bout into one `arena view:` line with no GPU. Test: `node tools/gh-check.mjs cargo test -p engine-game --features app --lib view_run` (runs on the PC). Pop and slide proofs on real bodies: `clip_switches_do_not_pop_in_a_real_duel` and `planted_feet_hold_their_spot_in_a_real_duel` in `crates/game/src/fighter_look_proofs.rs`. No door verb returns this line yet (the `view_run` doc names one that does not exist).

## 5. Score it

`tools/product-check.mjs` reads these lines (`parseView`, `parseReceipt`) against `tools/product-bar.json`; run it only with `--no-build --no-file` (no packets). `node --test tools/product-check.test.mjs` shows the parse rules. Report: command, exit code, the line you read (quote it), and the PNG you opened.
