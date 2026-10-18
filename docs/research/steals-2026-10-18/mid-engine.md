# Steal card: Mid-D-Man/mid-engine (2026-10-18)

## What it is

"The Middle Man — Modular Anti-Engine": a Rust toolkit of many small, FFI-first crates (`mid-alloc`/`mid-arena`, `mid-math`, `mid-log`, `mid-ecs`, `mid-net`, `mid-time`, `mid-ptr`, `mid-collections`). Claims SIMD-optimized math with 16-byte-aligned `#[repr(C)]` primitives, a hybrid data-oriented ECS (archetype + sparse set) and a lock-free async logger. Targets: network tick 128 Hz / 7.8 ms, physics 60 Hz / 16.6 ms, 100 000+ entities per core. 0 stars, 11 MB tree, HEAD `3365589dfcef` (2026-10-18T09:27Z), language Rust. Read live 2026-10-18 (README, crates, docs; read-only). Nothing built or run.

## Licence

**None.** The GitHub API returns `license.spdx_id` = `null`, and a tree search for `LICENSE`/`COPYING` finds no file (checked live today). Treat as all rights reserved: **ideas and patterns only, no bytes vendored.**

## Steals (ranked by value per hour)

1. **Declared access sets with an exact conflict check.** `crates/mid-ecs/src/access.rs:5-46` (what a piece of work reads and writes, as sets, plus why two accesses conflict; one query's own entities can never conflict; a component `T` and a resource `A` never conflict; and — the sharp rule — a write of `T` also counts as a read of `T`'s change ticks, so a writer conflicts with a `Changed<T>` filter), `:59-101` (`AccessConflicts` with `names`, `is_empty`, `is_all`, `Display`), `:124-130` (`Access`: component and resource reads and writes).
   - Maps onto engine2040: `docs/research/steals-2026-10-12/arch.md` row XX8 asked for declared access plus a refused conflicting pair (Bevy shape). `crates/ecs/src/schedule.rs:36-47` names boxed closures in 4 fixed phases with **no read/write sets**, so nothing can say two systems conflict. Lands in `crates/ecs/src/schedule.rs` plus a `sched_verify` test.
   - **Kill:** if the access sets cannot be built for the fixed 5-column spine without a rewrite, ship the change-tick read rule alone (a writer conflicts with a `Changed` reader).

2. **Generational index: even = vacant, odd = occupied.** `crates/mid-collections/src/generational_index.rs:1-64` (one `u32` generation per slot, bumped on **every** occupied↔vacant transition; every handle carries the generation it was issued with, so a stale handle is caught dead, not aliased; the free list needs no sentinel because `free_head == slots.len()` already means "grow"; LIFO reuse; `generation.wrapping_add(1)` copied from `slotmap`), `:69-90` (the handle and its raw index).
   - Maps onto engine2040: `crates/assets/src/handle.rs:25-39` already uses a generational handle (the index names a slot, the generation rejects stale uses, zero never names a live asset). The steal is the **nameable invariant** (odd = occupied, bump on every transition) and the sentinel-free free list, applied to ECS entity ids.
   - **Kill:** a generation wrap must not alias a live handle in a 2^32 soak; if it can, hold a higher bit or a second word.

3. **Rate-limited logging by site, with a summary on window expiry.** `crates/mid-log/src/ratelimit.rs:52-63` (`SiteKey { file, line }`, `window_start`), `:82-130` (allow up to `max_per_window` inside the window, then suppress and count; when the window expires emit one "… previous message repeated N more time(s)" summary and reset the site), `:132-152` (`flush_expired` emits a static `Warn` summary for sites that expired with suppressions).
   - Maps onto engine2040: `crates/observe` (`event_queue.rs`, `supervisor.rs`) and the crash diagnostics want a bounded log out of a hot path.
   - **Kill:** it must not suppress a genuinely distinct message that shares a line — include the level (or a caller tag) in the site key.

4. **A clock that is one import and no dependency tree.** `crates/mid-time/src/lib.rs:73-120` (on wasm32 `std::time::Instant::now()` panics, so the whole platform split is a single `#[link(wasm_import_module = "env")] extern "C" fn mid_time_now_ms() -> f64`; the loader supplies `performance.now()`; no `wasm-bindgen` / `web-sys` / `js-sys` / `web-time`), with their own honest caveat `:35-40` ("unverified on wasm32 — no wasm target or JS runtime exists anywhere in this project's tooling yet").
   - Maps onto engine2040: `crates/web` already takes a `now_ms: f64` (`src/anim.rs:110`, `src/arena.rs:32` `browser_clock`), but through `wasm-bindgen`/`js-sys` (`crates/web/Cargo.toml`). The steal is the rule — the web clock is one `extern "C"` import and nothing else.
   - **Kill:** do not adopt until a real wasm32 build passes (their caveat), and the clock must stay monotonic within one page load.

## Also seen, not stolen

`docs/benching-standards.md:37-50` — `set -o pipefail` is not optional: `cargo bench … | tee raw.txt` without it makes the step's exit code `tee`'s (always 0), so a real benchmark failure shows green (12 of their bench workflows lack it). `:52-70` — GitHub's `$GITHUB_STEP_SUMMARY` is capped at 1024 KiB and drops the **entire** step summary past it; compiler warnings piped into the summary blew it. Both cost engine2040's `gh-check`/`bench-compare` one line each.

## Not read

`crates/mid-ecs/src/world.rs` (95 KB), `ffi.rs` (206 KB), `crates/mid-math/src/swizzle/engine.rs` (76 KB), `crates/mid-arena/src/bump_arena.rs` (43 KB), `docs/mid-ecs.md` (262 KB).

DONE 4 steals, donor files read live 2026-10-18. Nothing built or run. No licence file: ideas only.
