# For AIs: read this first

This page is for an AI that opens engine2040 for the first time. It is short on purpose. Every path and `node` command named here exists in the repository; a test checks that (`node --test tools/for-ai.test.mjs`). The same page, as a link list, is `llms.txt` in the repository root.

## What the engine is

engine2040 is a Rust game engine made so that an AI can set up, run and check a whole game through one door. The proof game is a multiplayer Roman arena: fighters with melee, cuts and limb loss, ragdolls, walls that break. It runs on Windows today and in a browser tab (WebGPU plus WebAssembly) as a growing shell.

- **Deterministic.** The same complete input gives the same hash on every PC. The golden is seed 2040, 600 ticks, `d57b5c8ae3096614`. It moves only with an explicit receipt.
- **Layered.** `engine-core` is std-only. The simulation crates (arena, combat, physics, anim, net, input) hold integers and decide every outcome. Presentation (render, audio, `engine-physfx`) draws and may differ between machines. The map is `docs/ARCHITECTURE.md`; the crate list is `docs/CRATE-MAP.md`.
- **AI-native.** The engine ships an MCP server, the door (`crates/endpoint/src/bin/e2040-mcp.rs`). Every edit goes preview, apply, undo. Live AI proposes validated actions; replay consumes the recorded actions and calls no model.

## Prove it works: three commands, in this order

1. The golden. `cargo run --locked -- --verify` must print `final_hash=d57b5c8ae3096614` and `verified=true`.
2. The proof fixtures. `node tools/proof.mjs --skip-build` runs the seed and tape proofs (hash twice, tapes replayed). Plain `node tools/proof.mjs` adds fmt, build, tests and clippy.
3. The web build. `node tools/web-doctor.mjs` says what is missing (Rust target, wasm-bindgen, a browser); `node tools/web-build.mjs` writes `dist/web`. See the web page in this book for the headless-browser check.

After you edit a board row or a doc, also run `node tools/workspace-check.mjs`. It prints a FAIL line for anything you broke. `node tools/doctor.mjs` checks repo health first.

## The rules that win

- **Hash law** (`docs/HASH-LAW.md`). Only `+ - * /`, `sqrt`, comparisons and integer or `Fixed` math in simulation code. No `sin`, `cos`, `exp`, `powf`. No `HashMap` iteration in hashed state. Fixed 60 Hz tick. The golden does not move without a receipt. The float physics world (`crates/physfx`) is never hashed and never read by the sim.
- **One render writer at a time.** Rows that edit `crates/render/src/present/` queue behind each other (`docs/plan/web-and-realism.md`, section 3).
- **Commit by path.** Several AIs share one tree. Pull first with `git pull --no-rebase --no-edit origin master`. Commit only your own paths with `git commit -m <message> -- <paths>`. Never `git add -A`, never rebase, amend or force-push, never revert an edit you did not make, never `git clean` under `target/`.
- **Heavy cargo runs on GitHub.** Iterate with `cargo check -p <crate>`. Prove with `node tools/gh-check.mjs cargo test -p <crate> --lib <filter>` (one plain cargo line, exit 0 pass, 1 fail, 2 could not run, 3 still running).
- **No stubs.** Real code with a real caller, a test that fails before the change, `#[expect(reason = "...")]` and never `#[allow]`, no new dependency. `node tools/rust-quality.mjs` must stay green (`docs/RUST-QUALITY.md`).
- **Say what you proved.** Separate implemented, compiled, unit-tested, integrated and visually verified. A missing GPU adapter is unsupported coverage, not a pass.

## Where plans and rows are

- Order of work: `docs/FOCUS.md`, section "Now".
- The board is `docs/SPRINT.md` plus the shards `docs/SPRINT-22-web-realism.md` and friends. Never read it whole. `node tools/board-slice.mjs --focus` lists the Now rows, `node tools/board-slice.mjs --ready` the open ones, `node tools/board-slice.mjs <ID>` prints one row.
- Plans, one per subsystem: `docs/plan/README.md`. `node tools/plan-atlas.mjs --row <ID>` finds the plan of a row. The browser and realism plan is `docs/plan/web-and-realism.md`.
- Measured facts about browsers: `docs/research/web-2026-10-17/07-wasm-probe.md` and its siblings.
- The doc map is `docs/INDEX.md`. The tools are listed in `tools/README.md`. Style rules are `docs/CODELAW.md`.

## The door: verbs to start with

Start the server on a project folder with `cargo run --locked -p engine-endpoint --features mcp --bin e2040-mcp -- --help`, then point your MCP client at the built `e2040-mcp`. A session sees a small core and loads packs on demand. Every verb is callable by exact name, loaded or not.

| Verb | Use it to |
|---|---|
| `rpc.discover` | list every tool with its schemas |
| `api.search` | find an item in the engine index by words |
| `api.describe` | read one indexed item: signature, docs, units, owner, callers |
| `api.load` | show the verbs of one pack in the tool list (`break`, `anim`, `physics`, ...) |
| `scene.read` | get the scene revision and a page of objects |
| `scene.brief` | get a short text brief of the scene |
| `tx.preview` | dry-run an edit and get an approval |
| `tx.apply` | apply the previewed edit |
| `tx.undo` | undo the newest edit |
| `play.run` | run a headless match and read its checkpoints |
| `physics.read` | read what one blow does to a fighter's balance |
| `capture` | get a renderer receipt (a PNG when a GPU is wired) |
| `replay.verify` | resimulate a tape and report the first divergent tick |

Arguments, ranges, refusals and undo for every verb are in the [door reference](door-reference.md), printed from the source. The editing loop is: `scene.read` for the revision, `tx.preview`, `tx.apply` with the approval, `tx.undo` to revert. A refusal names the legal values and one recovery; read it before you retry.

<!-- doc-pin: crates/endpoint/src/resources.rs sha256-8:107eed12dd9a5f6b -->

## When you are stuck

[Troubleshooting](troubleshooting.md) lists the failures we have measured: no WebGPU adapter, a black canvas, blocked UDP, a wrong MIME type, a stale certificate. A second identical failure is a blocker: write it down (`docs/BLOCKERS.md`) and go on.
