# Steal card: openOMSI-org/openOMSI (2026-10-18)

## What it is

A from-scratch Rust recreation of the bus simulator **OMSI 2**, compatible with existing maps, buses and mods (it loads an installed OMSI 2 and ships no content). 64-bit, multithreaded, wgpu renderer (Metal / Vulkan / DX12), plus launcher, Android and Apple targets and a dedicated server. 38 crates under `crates/` (`omsi-sim`, `omsi-render`, `omsi-scene`, `omsi-guard`, …). 647 stars, 47 MB tree, HEAD `1f6ff3890038` (2026-10-18T13:52Z), language Rust. Read live 2026-10-18 (docs, crates; read-only). Nothing built or run.

## Licence

**AGPL-3.0** — `LICENSE` first lines "GNU AFFERO GENERAL PUBLIC LICENSE / Version 3, 19 November 2007" (read live today); GitHub API `license.spdx_id` = `AGPL-3.0`. Ideas only; no bytes vendored.

## Steals (ranked by value per hour)

1. **A start guard: a release that breaks the start must not strand the player.** `crates/omsi-guard/src/lib.rs:1-31` (a watcher process beside every launcher start; a start journal; healing; a broken-release list; safe mode; the guard's own log so it works where the game's log is not up yet), `:63-70` (the program's first act answers `--self-test` and `--guard-watch`), `:72-85` (`--self-test` proves the binary loads and its own folder is readable — the one breakage a watcher inside the same program cannot catch — before an update replaces anything). `crates/omsi-guard/src/journal.rs:9-13` (`Kind::Launcher`/`Game`), `:24-46` (`End::Clean`/`Error`/`Crash`/`Hang`), `:49-70` (a start failed if it never reached its first frames other than cleanly, or nobody saw it end and its process is gone).
   - Maps onto engine2040: `crates/game/src/crash.rs` is a std-only panic hook with a tape tail — no stack, no native faults (BOOK-GAPS:103), and PLF6 minidump is parked. The guard is the release-side half of that story and belongs with the startup-order rule (skill `tick-and-runtime`).
   - **Kill:** the guard must add < 5 MB and delay exit < 2 s, and safe mode must still render a frame (basic pipelines, no plugins).

2. **The simulation on its own thread, pipelined with the picture.** `crates/omsi-sim/src/sim_thread.rs:1-35` (a step runs on a persistent "sim" thread while the main thread draws and presents, and it is back before anything else of the frame reads it; the offscreen run takes the same code path inline), `:50-90` (one persistent thread, `sync_channel(1)` = one job at a time, an `Outcome` of `(state, seconds, panic)`, `catch_unwind`, and `join` re-raising the panic on the main thread).
   - Maps onto engine2040: `crates/ecs/src/jobs.rs` already runs bake/import/verify off the hot path on a shared `JobPool`. The steal is the **hot-path** shape: one owed step per tick, joined before the frame is counted, a panic carried back and re-raised at join, and a headless path that runs the step inline so a receipt never depends on the thread.
   - **Kill:** the golden at tick 600 must be identical with the thread on and off, and no step may ever be consumed twice.

3. **A refactor gate that is pictures, not manifests.** `docs/REFACTOR_CHECKS.md:1-4` (a refactor must not change what the game does; these checks show it beside `cargo test`), `:6-24` (`scripts/golden.sh` renders fixed offscreen scenes with the before and after binaries, prints `identical` or the share of differing pixels plus max/mean channel difference, and writes an amplified diff picture), `:30-46` (960×540, fixed seed 1 and date, a table of 9 scenes each with an explicit allowance: none, 0.01 %, or 1 % for the drive scene), `:48-60` (a fresh empty `$HOME` per scene; `windy_trees` off because the renderer's animation clock is real time; the remaining animated pixels are named and covered by the allowance), `:62-66` (three renders, all within allowance).
   - Maps onto engine2040: the `test-and-proof` rule "pixels not manifests" wants exactly this — an explicit per-scene allowance table, the non-repeatable source named, and an amplified diff a judge can open. Lands in the render golden harness and the `tools/frame-moved.mjs` family.
   - **Kill:** any scene whose allowance is above 1 % must be split, not excused.

4. **A source scanner for declared-vs-used fields.** `crates/omsi-srcscan/src/lib.rs:1-10` (holds what the code declares against what it does: which fields of a type a function reaches through its methods and the free functions it is handed to), `:16-33` (walk `.rs` files in path order), `:37-80` (blank comments, strings and char literals keeping length and line breaks, then follow `self.f`, `self.m(..)`, `Self::m(..)` and a parameter handed on).
   - Maps onto engine2040: the review audit "a pub item with no caller" and `tools/workspace-check.mjs` are manual/metadata. A text scanner gives a per-function reach list without a compiler.
   - **Kill:** a false positive on a clean tree would fail a gate, so it must be proof-tested against our own tree before it can gate; code that hides a field behind a macro or a trait method of another type is out of scope by design and must be reported, not guessed.

## Also seen, not stolen

`crates/omsi-lan/src/variable_schema.rs:1-23` — every script variable a model references is resolved against the script program, and an unresolved one is tagged `u`: a content/script binding audit. Noted for a content gate.

## Not read

`crates/omsi-sim/src/ai_traffic/*` (tick.rs 77 KB), `crates/omsi-render/src/passes/*`, `crates/omsi-plugin/src/lib.rs` (56 KB), `crates/omsi-scene/src/*` big files, `docs/DEBUG_FLAGS.md` (44 KB).

DONE 4 steals, donor files read live 2026-10-18. Nothing built or run.
