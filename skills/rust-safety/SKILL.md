---
name: rust-safety
description: "Use when a diff adds unsafe code, a static, an atomic, a lock or guard, a thread, channel or Arc, a real-time callback, or a numeric `as` cast or integer or float math in sim code: safety docs, lock spans, join, id-order merge, TryFrom, overflow mode."
---

# Rust safety

For the rest of the Rust bar see `engine-code` and `review`. For tick timing and allocation see `tick-and-runtime`.

Name the crate in each finding. A pass needs a test, a join, an order rule or a hash run. A fast run once is not proof.

## 1. Unsafe and statics

1. Each `unsafe fn` gets a `# Safety` doc that names its invariant. Each `unsafe` block gets a `// SAFETY:` comment. Never write `unsafe impl Send` or `Sync` without a written proof. Never silence a `Send`/`Sync` compile error with one.
2. A safe function checks its own preconditions. It never relies on its caller.
3. Per-session and per-match state lives on the session or match object. No process-wide mutable state for it.

Workspace lints (`Cargo.toml`): `unsafe_code = "deny"`, `unsafe_op_in_unsafe_fn = "deny"`, `undocumented_unsafe_blocks = "deny"`, `mem_forget = "deny"`, `await_holding_lock = "deny"`. A real exception is `#[expect(unsafe_code, reason = "...")]`. The root `Cargo.toml` lists the scoped sites: the counting allocator in `crates/core/src/alloc_probe.rs`, the test-only env writer in `crates/provider/src/test_env.rs`, `Scope::spawn` in `crates/ecs/src/pool.rs`, the xatlas FFI in `crates/geo`, and the DXGI VRAM read in `crates/render`. Do not add a sixth without a reason a reviewer accepts.

Models in the repo:

- `crates/core/src/alloc_probe.rs`: a `// SAFETY:` comment that names the invariant.
- `crates/geo/src/uv.rs::check_inputs`: the safe wrapper checks every precondition the C side relies on before it enters `unsafe`.
- `crates/endpoint/src/play/play_session.rs`: the capture hook and the re-query cache (`RequeryCache`) belong to the session object, not a static.
- Two statics exist on purpose: `WATCH_HUB` in `crates/endpoint/src/discover.rs` and `DOORS` in `crates/endpoint/src/packs/anim.rs`. Do not copy them for per-match state.

## 2. Locks, callbacks, threads

1. **Short lock spans.** Hold a guard for the shortest span. An `if let`, `match` or `while let` temporary lives to the end of its statement. Never hold a guard across a tick or a blocking call. Take at most one lock at a time, in a fixed order. A new mutex in a hot path needs a budget number.
2. **Poison.** Use `into_inner` only if the state is still valid. Otherwise return a typed error. Model: `crates/endpoint/src/play/requery.rs` turns a poisoned lock into `lock_poisoned`.
3. **No blocking in a real-time callback or the sim tick.** A `try_lock` that fails must queue or count the work. It must never drop work silently. Model: `feed_tick_audio` in `crates/game/src/windowed.rs` queues cue commands and counts `FeedStats::dropped_full`.
4. **Best-effort mirror.** The offline mixer mirror in `crates/game/src/bin/arena_frame/mod.rs` takes the mixer with `try_lock` and has no else branch. That is acceptable only because the lock-free feed already queued and counted every command and the code comment says so. Do not copy the pattern where nothing else carries the work.
5. **Spawn moves ownership.** Thread closures take what they need by `move`. No borrowed locals into `spawn`. Keep the handle.
6. **Every thread is scoped or joined** before its result is used and before shutdown. No fire-and-forget. Re-raise a worker panic with `resume_unwind` after the join. Model: `crates/ecs/src/pool.rs::JobPool::scope` waits for every job, then re-raises the lowest-numbered job panic.
7. **Channels carry ownership.** No use of a value after `send`. Choose blocking `recv` or polled `try_recv` on purpose; never block a tick thread. Do not ignore a send failure: it means the other end is gone. One receiver, cloned senders. Arrival order across senders is random: never build logic on it.
8. **Arc across threads, Rc inside one.** No `Rc` to another thread. No atomic cost where nothing is shared.
9. **Thread output stays replayable.** Merge job results in entity or tile id order, never in finish order or map-iteration order. Show the golden hash unchanged. Never claim a speedup from one small or debug run.

## 3. Numeric casts in sim code

Eight classes. Ask the question. Flag the pattern.

| class | ask | flag |
|---|---|---|
| Narrowing int | does every source value fit the target, else `TryFrom` | `v as u8` keeping low bits on overflow (core, net, save) |
| Sign change | can a negative reach an unsigned target, or a large unsigned reach a signed one | negative `as u32` becoming huge; use `TryFrom` or a range guard |
| Safe widening | is it a widening that keeps every value exact | rewriting `u8 as u32` or `i32 as i64` with extra machinery; leave it |
| Float to int | is truncation toward zero plus clamping at the edges what you want | out-of-range clamping, NaN giving 0, fraction loss inside hashed state |
| Fixed-point | does `Fixed` math scale by `SCALE` with a checked `i64` step before narrowing | raw `as` skipping the scale; divide-then-multiply losing the fraction |
| Index and word size | is the index a validated `usize`; is `usize` width treated as platform-dependent | `len as u32` stored in a save; `u32 as usize` indexing with no bounds check |
| Overflow mode | which mode did the author choose on purpose | plain `+` where modes differ; sim keeps checked, saturating or wrapping with a reason |
| Bool, char, enum | does the cast go outward to an integer only | integer to bool or char by `as`; use a comparison or a checked constructor |

Repo facts:

- `[profile.release]` sets `overflow-checks = true`. Plain `+` panics in dev and release alike. Wrapping must be explicit (`wrapping_add`, `Wrapping`).
- `checked_conversions`, `trivial_numeric_casts` and `lossy_float_literal` are on in workspace lints.
- Owners: `engine_core::checked` (`saturating_i64_to_i32`, `saturating_usize_to_u32`, `decimal_u32`), `Fixed::{checked_add, checked_mul, checked_div, saturating_add}`. Reuse them. See `crates/core/src/time.rs::IntAcc::push_ns` for a worked case: `u128` math, `u64::try_from` with an `expect` that names the proven bound.
- Never `mul_add` (`suboptimal_flops` stays allow): FMA changes rounding and breaks cross-platform hashes.

Do: keep a safe widening `as`, convert a fallible narrowing with `TryFrom`, pick checked, saturating or wrapping for sim math. Don't: leave a narrowing or sign-changing `as` on untrusted data.

Sources: trpl ch.16, ch.20; bos ch.1, ch.4, ch.5; progrust ch.1, ch.2; transition ch.3; idiomatic 10.7. Search: `node tools/books.mjs search "unsafe safe abstraction preconditions"`.
