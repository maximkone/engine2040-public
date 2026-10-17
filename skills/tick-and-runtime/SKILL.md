---
name: tick-and-runtime
description: "Use when you change the fixed-step tick, live window loop, frame budget, per-tick allocation, asset load or handle, worker threads, startup order or the crash hook: accumulator, no replay from tick 0, off-thread results at tick boundaries."
---

# Tick and runtime

Eight questions for a diff that touches the tick or loop, then the loop rules and repo facts. Name the crate in each finding (core, ecs, physics, net, arena, assets, perf, render, audio, ui, game). A pass needs evidence per touched item: a test, the golden hash, or measured ms. A promise in the commit text is not evidence.

Related: `rust-safety` (locks, threads), `perf-bench` (measure), `determinism-debug` (hash moved), `net-rollback`.

## 1. Eight questions

1. **Loop and timestep.** Does the sim advance in fixed steps from an accumulator (add real dt, run whole steps), with render using the leftover? Flag: variable dt reaching physics, AI or net; no cap on catch-up steps; no clamp on a huge dt (debugger pause, window drag). One slow frame that triggers two steps makes the next slower still: a spiral. Time is integer ticks, subtracted before narrowing.
2. **Frame budget.** What does this cost per tick, in ms, at the entity count that matters, in a release build, against which budget? Flag: a new per-frame pass with no number; work that grows with N and nobody says how.
3. **Allocation.** Is the hot loop free of heap allocation (`Vec::new`, `Box`, `format!`, `collect`)? Is per-tick scratch taken from an arena cleared once per tick? Flag: arena data kept past the tick; async job output written into the per-tick arena instead of a double-buffered pair; many same-size objects without a pool; churn that fragments over a long session. Prove with `engine_core::alloc_probe::measure`.
4. **Resource lifetime.** Does each asset have a stable id, one registry entry and a named tier (global, level, short, streamed)? Are runtime references handles with index plus generation? Flag: a synchronous load inside the tick; raw addresses or slot indices in saves or entities; a freed slot reused so an old handle silently reads the new object.
5. **Jobs and data races.** Is a job data plus code, with read-only inputs and its own output buffer? Do objects read each other's state from the previous step? Flag: ordering races that `Send`/`Sync` cannot catch (result used before ready, or a frame late); results merged in finish order instead of entity id; a new `Mutex` in a hot path with no answer to what it guards.
6. **Determinism.** Is the authoritative tick a pure function of state, ordered input log and seed streams? Is the golden hash unchanged, or is the move receipted? Flag: wall clock, thread timing, `HashMap` order, GPU output or provider output inside hashed state. Replay and rollback (net, arena) must re-run identical ticks.
7. **Startup and shutdown.** Do init calls follow the dependency graph, with shutdown in exact reverse, all called from one place (app)? Flag: lazy statics or global constructors where A's init silently relies on B's first use.
8. **Profile before optimizing.** Are there before and after numbers from the real workload (`perf` crate)? Flag: speculation; a rewrite of a cold path. Most code is not hot. The first fix is usually data layout (contiguous arrays) or removing allocation, not clever math.

Do: demand "fixed 1/60 step, at most 5 catch-up steps, render interpolates". Don't: accept last frame's measured dt fed to physics and "step twice to catch up".
Do: make per-tick scratch die with the tick; double-buffer job output that must outlive it. Don't: store a view into per-tick arena memory in a struct that lives to the next tick.
Do: ask for `perf` numbers plus the golden hash with the diff. Don't: approve "should be faster", or a hash that moved with no receipt.

## 2. Loop rules

1. Per-tick cost stays flat as the match grows. Never replay from tick 0 inside a live loop. Step stored state. Use a snapshot for rollback.
2. No thread per fork in a per-frame path. Use a persistent pool. The sim itself stays single-threaded.
3. Consume off-thread results at a frame or tick boundary. Record any result that touches the sim.
4. Unload an asset when its ref count reaches zero. Load its whole dependency closure, not one file.
5. Feed the audio callback through a single-writer queue (no lock in the callback).
6. Debug tools only read the sim. Pause, step and slow motion scale the accumulator, never the dt inside a tick.
7. Log with channels and levels. Mirror all output to a flushed file. Install a crash hook that dumps the tape tail and the last checkpoint.
8. Queue cross-object writes to the bucket end. Never write another object mid-step.

## 3. Repo facts

- `crates/core/src/time.rs`: `IntAcc` is the integer 60 Hz accumulator and the only sim clock. `STEP_NS` 16_666_666 plus `STEP_REM_THIRDS` 2 (so three steps are exactly 50_000_000 ns), `MAX_FRAME_NS` 100 ms clamp, `push_ns`/`push_duration`/`push_secs`. Wall time is quantized at the poll and never hashed. `alpha()` is for render only.
- `crates/core/src/tick.rs`: `MAX_STEPS` is 5. When the cap is hit, banked whole steps are forgiven, so a hitch never builds catch-up debt (anti-spiral).
- Crash hook: `crates/game/src/bin/arena.rs` calls `crash::install_run_hook` (`crates/game/src/crash.rs`) in `main`. `crates/game/tests/crash_hook.rs` proves it with a forced panic. A new binary must install it too (rule 7).
- Audio: `feed_tick_audio` (`crates/game/src/windowed.rs`) is the single-writer feed with `FeedStats`. The mixer mirror in `crates/game/src/bin/arena_frame/mod.rs` uses `try_lock` as best effort only when no device stream is live. See `rust-safety` section 2.
- Pool: `crates/ecs/src/pool.rs::JobPool::scope` is the persistent pool (rule 2).

Sources: geaen 5.1-5.2, 6.2.2, 7.2, 7.4.2, 7.5-7.7, 9.1, 9.5, 9.8, 15.5.3, 15.6 (cache background 3.4); gearus 8.6.3, 16.7.4; gems2 29.2; bos ch.5. Search: `node tools/books.mjs search "game loop logging crash"`.
