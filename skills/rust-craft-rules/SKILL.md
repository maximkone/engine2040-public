---
name: rust-craft-rules
description: Use when you write or review Rust that adds unsafe code, a static, an atomic, a lock or lock guard, a real-time callback, or a thread in crates, before you merge it.
version: 0.1.0
author: skillworks
---

# Rust craft rules

Use when you add unsafe code, a static, a lock or guard, a real-time callback, or a thread. The general rules (typed errors, no unwrap on IO data, `try_from`, no clones in hot loops) are in `engine-code`. The ordering rules were in `cache-and-ordering-checks` (removed 2026-10-12, never loaded); they are not repeated here.

1. Each unsafe fn gets a `# Safety` doc that names its invariant. Never write `unsafe impl Send` or `Sync` without a written proof. [trpl ch.20; trpl ch.16; bos ch.4]
2. A safe function checks its own preconditions. It never relies on its caller to have checked them. [trpl ch.20; transition ch.3 3.1]
3. Pass per-session and per-match state through the session or match object. Add no process-wide mutable state for it. [trpl ch.20; idiomatic 10.7]
4. Hold a lock guard for the shortest span. An `if let`, `match` or `while let` temporary lives to the end of its statement. On poison, use `into_inner` only if the state is valid. Otherwise return a typed error. [bos ch.1; trpl ch.16]
5. Never block in a real-time callback or in the sim tick. A `try_lock` that fails must queue or count the work. It must never drop the work silently. [bos ch.5]
6. Every thread is scoped or joined. Re-raise a worker panic with `resume_unwind` after the join. [progrust ch.1; trpl ch.16]

## Repo facts

- `crates/core/src/alloc_probe.rs`: model for a `// SAFETY:` comment that names the invariant (rule 1).
- `crates/geo/src/uv.rs` `check_inputs`: model. The safe wrapper around the xatlas C call checks every precondition the C side relies on before it enters `unsafe` (rule 2).
- `crates/endpoint/src/play/play_session.rs`: model. The capture hook and the re-query cache belong to the session object, not to a static (rule 3). Two process-wide statics that exist on purpose: `crates/endpoint/src/discover.rs` `WATCH_HUB` and `crates/endpoint/src/packs/anim.rs` `DOORS`; do not copy them for per-match state.
- `crates/endpoint/src/play/requery.rs`: model. A poisoned lock becomes the typed error `lock_poisoned` (rule 4).
- `crates/game/src/bin/arena.rs`: counter-example. The mixer `try_lock` in the tick path has no fallback, so audio commands drop silently when it is busy (rule 5).
- `crates/ecs/src/pool.rs` `JobPool::scope`: model. It waits for every job, then re-raises the lowest-numbered job panic with `resume_unwind` (rule 6).

Search more: `node tools/books.mjs search "unsafe safe abstraction preconditions"`
