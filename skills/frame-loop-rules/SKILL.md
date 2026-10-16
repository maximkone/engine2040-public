---
name: frame-loop-rules
description: Use when you change the live window loop, a worker thread, an asset load, the audio callback, a debug tool, the logging or crash hook, or parallel object updates in crates game, core, ecs, assets or audio.
version: 0.1.0
author: skillworks
---

# Frame loop rules

Use when you change the live loop, a per-frame or per-tick path, a worker thread, an asset load, the audio callback, a debug tool, the logging or crash hook, or parallel object updates. Fixed-step timing and the accumulator are in `engine-subsystem-review`. These rules add the rest.

1. Keep per-tick cost flat as the match grows. Never replay from tick 0 inside a live loop. Step stored state, and use a snapshot for rollback. [geaen 7.2]
2. Do not spawn a thread for each fork in a per-frame path. Use a persistent pool. The sim itself stays single-threaded. [gearus 8.6.3]
3. Consume off-thread results at a frame or tick boundary. Record any result that touches the sim. [geaen 7.6.6, 6.2.2.5]
4. Unload an asset when its ref count reaches zero. Load its whole dependency closure, not one file. [geaen 6.2.2.1, 6.2.2.6]
5. Feed the audio callback through a single-writer queue. [gems2 29.2; bos ch.5]
6. Debug tools only read the sim. Pause, step and slow motion scale the accumulator, never the dt inside a tick. [geaen 7.4.2, 9.5]
7. Log with channels and levels. Mirror all output to a flushed file. Install a crash hook that dumps the tape tail and the last checkpoint. [geaen 9.1.2-9.1.5]
8. Queue cross-object writes to the bucket end. Never write another object mid-step. [gearus 16.7.4; geaen 15.6.3.4]

## Repo facts

- `crates/core/src/time.rs`: the integer clock and accumulator the loop uses.
- `crates/game/src/bin/arena.rs` installs the crash hook in `main` (`crash::install_run_hook` in `crates/game/src/crash.rs`), and `crates/game/tests/crash_hook.rs` proves it with a forced panic. Rule 7 is met for the window binary; a new binary must do the same.
- `crates/game/src/bin/arena_frame/mod.rs:107`: the window's mixer is fed inside `self.mixer.try_lock()` in the tick path, not through a single-writer queue (rule 5).

Search more: `node tools/books.mjs search "game loop logging crash"`
