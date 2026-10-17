# Skill-on vs skill-off pairs: rust-craft-rules

Three situations. "Off" is what an agent says from memory; "On" is what the skill makes it say. Pass = the On answer applies the rule, searches or cites the source for each claim, and names the file that proves it.

## P1: unsafe impl Send for a raw pointer (rust)

Situation: A builder wraps a raw pointer in a struct and adds `unsafe impl Send for Handle {}` so the struct can go to a worker thread.

Off: "Add unsafe impl Send. The pointer is only used on one thread, so it is safe."

On: "Rule 1: never write unsafe impl Send or Sync without a written proof. Name the invariant in a `// SAFETY:` comment, as `crates/core/src/alloc_probe.rs:31` does. Search `node tools/books.mjs search "implementing send and sync unsafe"` and cite [trpl ch.16]. 'Only used on one thread' is a claim, not a proof."

What skill-less misses: the proof. The builder states a usage pattern and calls it a proof, and a later change to the call site breaks the safety with no warning.

## P2: process-wide static for match data (rust)

Situation: A builder adds `static mut` for the match event log, so tests can read it without passing the match object around.

Off: "A static mut is the simplest way. Tests can read it from anywhere."

On: "Rule 3: pass per-match state through the match object. Add no process-wide mutable state for it. Counter-example to avoid: a process-wide static holding the hook (the old `STEP_HOOK` static is gone); the hook is now an `Option<StepCaptureHook>` on the play slot, wired by `with_step_capture_hook` at `crates/endpoint/src/play/play_session.rs:804`. Search `node tools/books.mjs search "mutable static variable"` and cite [trpl ch.20]. Give each test its own match object."

What skill-less misses: the shared state. Two matches or two parallel tests share the log, so the results depend on the order the tests run in.

## P3: try_lock with no else branch (rust)

Situation: A builder feeds the audio mixer from the tick with `if let Ok(mut m) = mixer.try_lock() { m.play(...) }` and no else branch. The lock is busy only rarely, so the code looks fine in testing.

Off: "try_lock keeps the tick from blocking. The lock is only busy now and then, so the drops are rare."

On: "Rule 5: a try_lock that fails must queue or count the work, never drop it silently. Search `node tools/books.mjs search "rust atomics locks real time"` and cite [bos ch.5]. The window mixer at `crates/game/src/bin/arena_frame/mod.rs:114` drops audible voices on a refused lock and counts nothing; add the counter there (the feed above it already queues and counts `FeedStats::dropped_full`)."

What skill-less misses: that the commands vanish under load, with no count. Nobody sees the loss, so the audio looks wrong with no visible error.
