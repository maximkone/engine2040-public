# Steal card: RustyZoo/RustyZoo (2026-10-18)

## What it is

An open-source Rust reimplementation of **Zoo Tycoon (2001)** in the spirit of OpenRCT2: a standalone program that loads the data of your own copy of the game (it ships no assets). Pre-alpha — the repo is a core library plus a `window-smoke` command — but a single crate `src/` already carries a full simulation: guests, animals, exhibits, money, month cycle, staff goals, an A* path finder, and a headless `sim` command that prints a whole-state hash. 1 star, 2.8 MB tree, HEAD `57a0611e59ec` (2026-10-17T20:53Z), language Rust (SDL3 for the window). Read live 2026-10-18 (crates, docs listing; read-only). Nothing built or run.

## Licence

**GPL-3.0** — `LICENSE` first lines "GNU GENERAL PUBLIC LICENSE / Version 3, 29 June 2007" (read live today); GitHub API `license.spdx_id` = `GPL-3.0`. Ideas only; no bytes vendored.

## Steals (ranked by value per hour)

1. **A replayable wall-clock seconds source.** `src/sim/rng.rs:1` (the game's generator and the seconds it is seeded from), `:23-25` (`state()` exposed so the generator is hashed), `:53-77` (`Seconds` enum: `Clock` reads the wall clock; `Test { seed, n }` returns `seed + n` and increments `n`; `Replay(VecDeque<u32>)` pops the recorded seconds, falling back to the wall clock when they run out).
   - Maps onto engine2040: the tape/replay path (`crates/proof/src/tape.rs`) and the "never replay from tick 0" rule. A wall-clock seed is the classic non-reproducible input; making the seconds a recorded, consumable stream is the steal.
   - **Kill:** a `Replay` stream that runs out must not silently fall back to the wall clock on a proof run — fail, or require the recorded length to match.

2. **A state-hash law with fixed widths and list order.** `src/sim/hash.rs:1-4` (FNV-1a over the bytes of fixed-width fields, little-endian; whole numbers at the width of their field; decimals by the bits of their `f32`; a string or list after its length as a `u32`; **the tables are walked in list order, never through a hash map**), `:30-72` (the typed feeders: `put`/`u8`/`u16`/`u32`/`i32`/`u64`/`f32`/`len`/`text`/`option`/`event`), `:126-165` (the zoo: names, tiles, entities, counters), `:282-319` (the sim: totals, `rng.state()`, the `Seconds` variant, removals, and both event buffers), `:312-318` (both buffers of the double-buffered event queue are hashed, in order).
   - Maps onto engine2040: `docs/HASH-LAW.md` and `crates/core/src/tick.rs` `fold_hash`. The steal is the explicit rule "never iterate a hash map in a hashed path", plus hashing both buffers of a double-buffered queue.
   - **Kill:** any `HashMap`/`HashSet` iteration introduced into a hashed path changes the hash — a test must fail, never a re-pin.

3. **A* with a hard node cap and a refusal flag.** `src/path/search.rs:79-81` (the search carries a `capped: bool` — "a node was refused because the cap was used up"), `:186-187` (`if self.nodes.len() >= self.s.node_max { self.capped = true; … }`), `:334-374` (the search returns the best path found so far, and the caller keeps it when capped), with the config keys `node_max` (`src/sim.rs:112`) and the unreachable sweep (`src/sim.rs:98-99` `clear_unreachable_freq`, `min_unreachable_count`).
   - Maps onto engine2040: `crates/nav` (`mesh`, `flow`, `reach`, `tiles`) — a bounded search that reports "not found, capped" instead of stalling the tick, and a periodic sweep that clears unreachable nodes.
   - **Kill:** the cap must never change a path on the golden fixture (raise `node_max` until the golden path is unaffected), and a capped result must be distinguishable from "no path".

4. **A double-buffered event queue with per-event delay aging.** `src/sim/events.rs:13-43` (`post` pushes to `next`; `listen` filters `current` by target/channel and `delay < 2` **without consuming**; `age()` re-pushes current events with `delay > 1` decremented, drops the rest, then swaps the two buffers), `:47-53` (`restore` rebuilds the queue from a saved map's two buffers), `:55-58` (`buffers()` exposes both for the state hash).
   - Maps onto engine2040: cue/event ordering in the fixed tick and the rules tape (`crates/rules/src/tape.rs`). The steal is a queue where the read rule, the aging rule and the deterministic end-of-step swap are one place, and both buffers are hashed.
   - **Kill:** the happy-path order and the aging must be pinned by a test (`same cue tape → same buffer bytes x2`), and an event must not be readable twice.

## Not read

`src/main.rs` (108 KB), `src/render.rs` + `render/order.rs`, `src/vfs.rs` (72 KB), `src/sprite.rs` (79 KB), `src/behaviour/run.rs`, the guest/staff goal bodies.

DONE 4 steals, donor files read live 2026-10-18. Nothing built or run.
