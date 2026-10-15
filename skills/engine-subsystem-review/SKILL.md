---
name: engine-subsystem-review
description: Use when a planner or judge reviews a change to a Rust game engine (engine2040 or similar) that touches the tick or game loop, timing, hot-path allocation, asset or resource lifetime, threads or jobs, replay or rollback determinism, or subsystem init order, or when a change claims a speedup without a measurement. Load before writing the plan or the verdict. Not for render math (see render-math-checklist).
version: 0.1.0
author: skillworks
---

# Engine subsystem review

Eight questions to put to a diff. Name the crate in each finding (core, ecs, physics, net, arena, assets, perf, render, audio, ui). PASS needs evidence per touched item (a test, the golden hash, or measured ms), never a promise in the commit text.

1. **Loop and timestep.** Ask: does simulation advance in fixed steps from an accumulator (add real dt, run whole steps), with render using the leftover?
   Flag: variable dt reaching physics, AI or net; no cap on catch-up steps; no clamp on a huge dt (debugger pause, window drag). One slow frame that triggers two steps makes the next one slower still, a spiral. Time is kept as integer ticks, subtracted before narrowing.
2. **Frame budget.** Ask: what does this cost per tick, in ms, at the entity count that matters, in a release build, against which budget?
   Flag: a new per-frame pass with no number; work that grows with N and nobody says how.
3. **Allocation.** Ask: is the hot loop free of heap allocation (Vec::new, Box, format!, collect)? Is per-tick scratch taken from an arena that is cleared once per tick?
   Flag: arena data kept past the tick; async job output written into the per-tick arena instead of a double-buffered pair; many same-size objects without a pool; churn that fragments over a long session.
4. **Resource lifetime.** Ask: does each asset have a stable id, one registry entry, and a named tier (global, level, short, streamed)? Are runtime references handles with index plus generation?
   Flag: a synchronous load inside the tick; raw addresses or slot indices stored in saves or entities; a freed slot reused so an old handle silently reads the new object.
5. **Jobs and data races.** Ask: is a job data plus code, with read-only inputs and its own output buffer? Do objects read each other's state from the previous step?
   Flag: ordering races that Send/Sync cannot catch (result used before ready, or a frame late); results merged in finish order instead of entity id; a new Mutex in a hot path with no answer to what shared state it guards.
6. **Determinism.** Ask: is the authoritative tick a pure function of state, ordered input log and seed streams? Is the golden hash unchanged, or is the move receipted?
   Flag: wall clock, thread timing, HashMap iteration order, GPU output or provider/model output inside the hashed state. Replay and rollback (net, arena) must re-run identical ticks.
7. **Startup and shutdown.** Ask: do init calls follow the dependency graph, with shutdown in exact reverse, all called from one place (app)?
   Flag: lazy statics or global constructors where subsystem A's init silently relies on B's first use; no defined order between them.
8. **Profile before optimizing.** Ask: are there before/after numbers from the real workload (perf crate)?
   Flag: speculation; a rewrite of a cold path. Most code is not hot. The first fix is usually data layout (contiguous arrays, fewer cache misses) or removing allocation, not clever math.

## Do / Don't
- Do: demand "fixed 1/60 step, at most 5 catch-up steps, render interpolates". Don't: accept last frame's measured dt fed to physics and "step twice to catch up".
- Do: make per-tick scratch die with the tick and double-buffer job output that must outlive it. Don't: store a view into per-tick arena memory in a struct that lives to the next tick.
- Do: ask for perf-crate numbers plus the golden hash with the diff. Don't: approve "should be faster", or a hash that moved with no receipt.

source: Gregory, Game Engine Architecture (2nd ed.), ch. 5.1-5.2, 6.2.2, 7.5-7.7, 9.8, 15.5.3, 15.6 (cache background 3.4)
