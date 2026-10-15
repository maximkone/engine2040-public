---
name: fearless-concurrency-checks
description: "Use when a builder or reviewer adds threads, channels, or shared counters to engine crates and must prove the handoff is ordered and safe."
version: 0.1.0
author: skillworks
---

# Fearless concurrency checks

Nine checks for a diff that shares work across threads. Name the crate in each finding (core, ecs, physics, net, assets, game, audio, render). PASS needs a join, an order rule, or a hash run, never a fast run once.

1. **Spawn moves ownership.** Ask: does each thread closure take what it needs by move and outlive its refs with a static bound? (crate: game)
   Flag: borrowed locals sent to spawn; main drops or reuses the value while the thread may still run; no handle kept.
2. **Join every thread.** Ask: is each handle joined before its result is used and before shutdown? (crate: game)
   Flag: fire-and-forget spawn; main exits and kills work early; join placed so threads never overlap when they should.
3. **Channels carry ownership.** Ask: does send move the value and does the receiver own it after, with blocking recv or polled try_recv chosen on purpose? (crate: net)
   Flag: use of a value after send; blocking recv on a tick thread; ignoring a send failure that means the other end is gone.
4. **One receiver, cloned senders.** Ask: are extra producers made by cloning the sender, and is arrival order treated as random? (crate: net)
   Flag: two receivers expected; logic that needs sender A before sender B; test that passes once by luck of timing.
5. **Lock guards data briefly.** Ask: is shared state behind a mutex, locked only around the short update, with poison handled? (crate: physics)
   Flag: data read without the lock; guard held across a tick or a blocking call; panic while holding the lock ignored.
6. **Arc across threads, Rc inside one.** Ask: is shared ownership across threads atomic-counted, with single-thread counters kept off the hot path? (crate: assets)
   Flag: reference-counted handle sent to another thread; atomic cost paid where no thread ever shares; counter loop that could use atomics.
7. **Send and Sync mean safe to move and share.** Ask: does each type sent or shared across threads meet the marker bounds, with no hand-written unsafe impl to dodge them? (crate: ecs)
   Flag: compile error silenced by an unsafe impl; raw pointer or cell type smuggled to another thread without a safety note.
8. **One lock at a time.** Ask: does work take at most one lock, in a fixed order, released before the next tick? (crate: core)
   Flag: two locks taken in opposite order on two paths; lock cycle that stalls; new mutex in a hot path with no budget number.
9. **Thread output stays replayable.** Ask: are job results merged in entity or tile order, never in finish or map-iteration order, with the golden hash shown? (crate: arena)
   Flag: results collected by arrival time; hash walked from a hash map; speedup claimed from one small or debug run.

## Do / Don't
- Do: move inputs into the thread, join it, and merge results in id order with the hash unchanged. Don't: borrow locals, skip the join, or trust arrival order and one fast run.

source: Klabnik et al., The Rust Programming Language, 3rd ed., ch. 16
